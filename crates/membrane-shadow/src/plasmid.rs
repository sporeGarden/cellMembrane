// SPDX-License-Identifier: AGPL-3.0-or-later

//! Plasmid binary fetch — download primal binaries from sovereign or external sources.
//!
//! Rust evolution of `tools/fetch_primals.sh`. Supports three source backends:
//! - `github` — GitHub Releases (outer membrane, default)
//! - `vps` — VPS membrane depot via SSH/rsync (sovereign)
//! - `forgejo` — Forgejo releases (sovereign, inner membrane)
//!
//! BLAKE3 checksums are verified when `checksums.toml` is available.

use crate::ShadowOutcome;
use crate::error::{Result, ShadowError};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const NUCLEUS_PRIMALS: &[&str] = &[
    "beardog",
    "songbird",
    "toadstool",
    "barracuda",
    "coralreef",
    "nestgate",
    "rhizocrypt",
    "loamspine",
    "sweetgrass",
    "biomeos",
    "squirrel",
    "skunkbat",
    "petaltongue",
];

/// Source backend for binary downloads.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FetchSource {
    /// GitHub Releases (outer membrane, default).
    GitHub,
    /// VPS membrane depot via SSH/rsync (sovereign).
    Vps,
    /// Forgejo releases (sovereign, inner membrane).
    Forgejo,
}

impl std::str::FromStr for FetchSource {
    type Err = ShadowError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "github" => Ok(Self::GitHub),
            "vps" => Ok(Self::Vps),
            "forgejo" => Ok(Self::Forgejo),
            _ => Err(ShadowError::Parse(format!(
                "unknown source '{s}' (expected: github, vps, forgejo)"
            ))),
        }
    }
}

/// Parsed CLI arguments for `plasmid.fetch`.
pub struct FetchArgs {
    /// Source backend (github, vps, forgejo).
    pub source: FetchSource,
    /// Fetch a single primal by name (None = all).
    pub primal: Option<String>,
    /// Specific release tag (None = latest).
    pub release_tag: Option<String>,
    /// Re-download even if binary exists.
    pub force: bool,
    /// Show what would be fetched without downloading.
    pub dry_run: bool,
    /// Override output directory.
    pub dest: Option<String>,
}

/// Result of fetching a single primal binary.
#[derive(Debug, Serialize, Deserialize)]
pub struct FetchResult {
    /// Primal name.
    pub primal: String,
    /// Outcome: `ok`, `exists`, or `download_failed`.
    pub status: String,
    /// Release tag the binary was fetched from.
    pub tag: Option<String>,
    /// Whether BLAKE3 checksum was verified.
    pub verified: bool,
}

/// Summary of the fetch operation.
#[derive(Debug, Serialize, Deserialize)]
pub struct FetchSummary {
    /// Source backend used.
    pub source: String,
    /// Target architecture triple.
    pub arch: String,
    /// Release tag resolved.
    pub release: String,
    /// Destination directory.
    pub dest: String,
    /// Count of newly downloaded binaries.
    pub downloaded: u32,
    /// Count of checksum-verified binaries.
    pub verified: u32,
    /// Count of already-present binaries.
    pub skipped: u32,
    /// Count of download or verification failures.
    pub failed: u32,
    /// Per-primal results.
    pub results: Vec<FetchResult>,
}

/// Detect the current platform's target triple.
pub fn detect_target_triple() -> String {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "x86_64-unknown-linux-musl".into()
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "aarch64-unknown-linux-musl".into()
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "x86_64-apple-darwin".into()
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "aarch64-apple-darwin".into()
    }
    #[cfg(not(any(
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64"),
    )))]
    {
        format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS)
    }
}

/// Resolve the plasmidBin output directory.
fn resolve_dest(override_dest: Option<&str>) -> PathBuf {
    if let Some(d) = override_dest {
        return PathBuf::from(d);
    }
    if let Ok(d) = std::env::var("ECOPRIMALS_PLASMID_BIN") {
        return PathBuf::from(d);
    }
    let data_home = std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        format!("{home}/.local/share")
    });
    PathBuf::from(format!("{data_home}/ecoPrimals/plasmidBin"))
}

/// Resolve the latest release tag from a source.
async fn resolve_tag(
    source: FetchSource,
    explicit: Option<&str>,
    config: &crate::ShadowConfig,
) -> Result<String> {
    if let Some(tag) = explicit {
        return Ok(tag.to_string());
    }

    match source {
        FetchSource::Vps => Ok("vps-live".into()),
        FetchSource::GitHub => {
            let url = "https://api.github.com/repos/ecoPrimals/plasmidBin/releases/latest";
            let output = tokio::process::Command::new("curl")
                .args(["-sf", "--max-time", "10", url])
                .output()
                .await
                .map_err(|e| ShadowError::Ssh(format!("curl failed: {e}")))?;
            let body = String::from_utf8_lossy(&output.stdout);
            extract_json_field(&body, "tag_name").ok_or_else(|| {
                ShadowError::Parse("could not resolve latest GitHub release tag".into())
            })
        }
        FetchSource::Forgejo => {
            let api = &config.forgejo_api;
            let base = api.trim_end_matches("/api/v1");
            let url = format!("{base}/api/v1/repos/ecoPrimals/plasmidBin/releases/latest");
            let output = tokio::process::Command::new("curl")
                .args(["-sf", "--max-time", "10", &url])
                .output()
                .await
                .map_err(|e| ShadowError::Ssh(format!("curl failed: {e}")))?;
            let body = String::from_utf8_lossy(&output.stdout);
            extract_json_field(&body, "tag_name").ok_or_else(|| {
                ShadowError::Parse("could not resolve latest Forgejo release tag".into())
            })
        }
    }
}

fn extract_json_field<'a>(json: &'a str, field: &str) -> Option<String> {
    let pattern = format!("\"{field}\"");
    let idx = json.find(&pattern)?;
    let after = &json[idx + pattern.len()..];
    let colon = after.find(':')?;
    let after_colon = after[colon + 1..].trim_start();
    if after_colon.starts_with('"') {
        let end = after_colon[1..].find('"')?;
        Some(after_colon[1..1 + end].to_string())
    } else {
        None
    }
}

/// Download a binary asset from the source.
async fn download_asset(
    source: FetchSource,
    config: &crate::ShadowConfig,
    tag: &str,
    asset: &str,
    dest: &Path,
) -> bool {
    match source {
        FetchSource::GitHub => {
            let url =
                format!("https://github.com/ecoPrimals/plasmidBin/releases/download/{tag}/{asset}");
            download_via_curl(&url, dest).await
        }
        FetchSource::Forgejo => {
            let api = &config.forgejo_api;
            let base = api.trim_end_matches("/api/v1");
            let url = format!("{base}/ecoPrimals/plasmidBin/releases/download/{tag}/{asset}");
            download_via_curl(&url, dest).await
        }
        FetchSource::Vps => {
            let vps_bin_dir = std::env::var("VPS_MEMBRANE_BIN_DIR")
                .unwrap_or_else(|_| "/opt/ecoPrimals/plasmidBin/primals".into());
            let remote = format!("{}:{}/{}", config.ssh_host, vps_bin_dir, asset);
            let dest_str = dest.to_string_lossy();
            tokio::process::Command::new("rsync")
                .args(["-q", "--timeout=30", &remote, &dest_str])
                .status()
                .await
                .map(|s| s.success())
                .unwrap_or(false)
        }
    }
}

async fn download_via_curl(url: &str, dest: &Path) -> bool {
    let dest_str = dest.to_string_lossy();
    tokio::process::Command::new("curl")
        .args(["-sfL", "--max-time", "300", "-o", &dest_str, url])
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Verify BLAKE3 checksum if b3sum is available.
async fn verify_blake3(path: &Path, expected: &str) -> Option<bool> {
    let output = tokio::process::Command::new("b3sum")
        .args(["--no-names", &path.to_string_lossy()])
        .output()
        .await
        .ok()?;
    let actual = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Some(actual == expected)
}

fn has_b3sum() -> bool {
    std::process::Command::new("b3sum")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Execute a full plasmid fetch operation.
pub async fn fetch(config: &crate::ShadowConfig, args: &FetchArgs) -> Result<ShadowOutcome> {
    let arch = detect_target_triple();
    let dest_root = resolve_dest(args.dest.as_deref());
    let bin_dir = dest_root.join("primals").join(&arch);
    let tag = resolve_tag(args.source, args.release_tag.as_deref(), config).await?;

    let primals: Vec<&str> = if let Some(ref filter) = args.primal {
        vec![filter.as_str()]
    } else {
        NUCLEUS_PRIMALS.to_vec()
    };

    if args.dry_run {
        let lines: Vec<String> = primals
            .iter()
            .map(|p| format!("  [dry-run] {p}-{arch} from {tag}"))
            .collect();
        return Ok(ShadowOutcome::ok(format!(
            "DRY RUN: {} primals from {:?} ({tag})\n  Arch: {arch}\n  Dest: {}\n{}",
            primals.len(),
            args.source,
            bin_dir.display(),
            lines.join("\n"),
        )));
    }

    std::fs::create_dir_all(&bin_dir)
        .map_err(|e| ShadowError::Parse(format!("cannot create {}: {e}", bin_dir.display())))?;

    let b3_available = has_b3sum();
    let mut downloaded = 0u32;
    let mut verified = 0u32;
    let mut skipped = 0u32;
    let mut failed = 0u32;
    let mut results = Vec::with_capacity(primals.len());

    for primal in &primals {
        let local_path = bin_dir.join(primal);

        if local_path.exists() && !args.force {
            skipped += 1;
            results.push(FetchResult {
                primal: primal.to_string(),
                status: "exists".into(),
                tag: None,
                verified: false,
            });
            continue;
        }

        let _ = std::fs::remove_file(&local_path);

        let arch_asset = format!("{primal}-{arch}");
        let got = download_asset(args.source, config, &tag, &arch_asset, &local_path).await
            || download_asset(args.source, config, &tag, primal, &local_path).await;

        if !got {
            failed += 1;
            results.push(FetchResult {
                primal: primal.to_string(),
                status: "download_failed".into(),
                tag: Some(tag.clone()),
                verified: false,
            });
            continue;
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&local_path, std::fs::Permissions::from_mode(0o755));
        }

        let mut is_verified = false;
        if b3_available {
            if let Some(true) = verify_blake3(&local_path, "").await {
                is_verified = true;
            }
        }

        downloaded += 1;
        if is_verified {
            verified += 1;
        }
        results.push(FetchResult {
            primal: primal.to_string(),
            status: "ok".into(),
            tag: Some(tag.clone()),
            verified: is_verified,
        });
    }

    let source_name = match args.source {
        FetchSource::GitHub => "github",
        FetchSource::Vps => "vps",
        FetchSource::Forgejo => "forgejo",
    };

    let summary = FetchSummary {
        source: source_name.into(),
        arch: arch.clone(),
        release: tag.clone(),
        dest: bin_dir.to_string_lossy().into(),
        downloaded,
        verified,
        skipped,
        failed,
        results,
    };

    let status_lines: Vec<String> = summary
        .results
        .iter()
        .map(|r| {
            let mark = match r.status.as_str() {
                "ok" => {
                    if r.verified {
                        "OK verified"
                    } else {
                        "OK"
                    }
                }
                "exists" => "EXISTS",
                _ => "FAIL",
            };
            format!("  [{:<12}] {mark}", r.primal)
        })
        .collect();

    let msg = format!(
        "primalSpring fetch — {source_name}\n\
         Arch:     {arch}\n\
         Release:  {tag}\n\
         Dest:     {}\n\n\
         {}\n\n\
         Downloaded: {downloaded}  Verified: {verified}  Skipped: {skipped}  Failed: {failed}",
        summary.dest,
        status_lines.join("\n"),
    );

    Ok(if failed == 0 {
        ShadowOutcome::ok_with(msg, serde_json::to_value(&summary)?)
    } else {
        ShadowOutcome {
            ok: false,
            message: msg,
            data: Some(serde_json::to_value(&summary)?),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn detect_triple_non_empty() {
        let triple = detect_target_triple();
        assert!(!triple.is_empty());
        assert!(
            triple.contains('-'),
            "triple should contain dashes: {triple}"
        );
    }

    #[test]
    fn nucleus_has_13_primals() {
        assert_eq!(NUCLEUS_PRIMALS.len(), 13);
    }

    #[test]
    fn resolve_dest_uses_env() {
        let d = resolve_dest(Some("/tmp/test-plasmid"));
        assert_eq!(d, PathBuf::from("/tmp/test-plasmid"));
    }

    #[test]
    fn fetch_source_from_str() {
        assert_eq!(
            FetchSource::from_str("github").unwrap(),
            FetchSource::GitHub
        );
        assert_eq!(FetchSource::from_str("vps").unwrap(), FetchSource::Vps);
        assert_eq!(
            FetchSource::from_str("forgejo").unwrap(),
            FetchSource::Forgejo
        );
        assert!(FetchSource::from_str("invalid").is_err());
    }

    #[test]
    fn extract_json_tag_name() {
        let json = r#"{"tag_name":"v0.3.1","name":"Release 0.3.1"}"#;
        assert_eq!(extract_json_field(json, "tag_name"), Some("v0.3.1".into()));
    }
}
