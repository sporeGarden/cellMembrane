// SPDX-License-Identifier: AGPL-3.0-or-later

//! Plasmid binary fetch — download primal binaries from sovereign or external sources.
//!
//! Rust evolution of `tools/fetch_primals.sh`. Supports three source backends:
//! - `github` — GitHub Releases (outer membrane, default)
//! - `vps` — VPS membrane depot via SSH/rsync (sovereign)
//! - `forgejo` — Forgejo releases (sovereign, inner membrane)
//!
//! BLAKE3 checksums are verified in-process using the `blake3` crate.

use crate::ShadowOutcome;
use crate::error::{Result, ShadowError};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Primal binary names derived from the service registry at compile time.
///
/// Previously a hand-maintained `const` list — now sourced directly from
/// `cellmembrane-types::MembraneService::all()` so additions/removals to the
/// registry propagate automatically with zero manual sync.
fn nucleus_primals() -> Vec<&'static str> {
    cellmembrane_types::MembraneService::all()
        .iter()
        .filter(|s| s.is_primal)
        .map(|s| s.binary)
        .collect()
}

/// Source backend for binary downloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchSource {
    /// GitHub Releases (outer membrane, default).
    GitHub,
    /// VPS membrane depot via SSH/rsync (sovereign).
    Vps,
    /// Forgejo releases (sovereign, inner membrane).
    Forgejo,
}

impl std::fmt::Display for FetchSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GitHub => f.write_str("github"),
            Self::Vps => f.write_str("vps"),
            Self::Forgejo => f.write_str("forgejo"),
        }
    }
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[must_use]
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

/// GitHub/Forgejo release API response (only the fields we need).
#[derive(Deserialize)]
struct ReleaseResponse {
    tag_name: String,
}

/// Resolve the latest release tag from a source using native HTTP.
#[cfg(feature = "http")]
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
            let client = reqwest::Client::new();
            let resp: ReleaseResponse = client
                .get(url)
                .header("User-Agent", "membrane-shadow/0.1")
                .send()
                .await
                .map_err(|e| ShadowError::Parse(format!("GitHub API request failed: {e}")))?
                .json()
                .await
                .map_err(|e| ShadowError::Parse(format!("GitHub API parse failed: {e}")))?;
            Ok(resp.tag_name)
        }
        FetchSource::Forgejo => {
            let api = &config.forgejo_api;
            let base = api.trim_end_matches("/api/v1");
            let url = format!("{base}/api/v1/repos/ecoPrimals/plasmidBin/releases/latest");
            let client = reqwest::Client::new();
            let resp: ReleaseResponse = client
                .get(&url)
                .header("User-Agent", "membrane-shadow/0.1")
                .send()
                .await
                .map_err(|e| ShadowError::Parse(format!("Forgejo API request failed: {e}")))?
                .json()
                .await
                .map_err(|e| ShadowError::Parse(format!("Forgejo API parse failed: {e}")))?;
            Ok(resp.tag_name)
        }
    }
}

/// Fallback resolve_tag when http feature is disabled.
#[cfg(not(feature = "http"))]
async fn resolve_tag(
    _source: FetchSource,
    explicit: Option<&str>,
    _config: &crate::ShadowConfig,
) -> Result<String> {
    explicit
        .map(ToString::to_string)
        .ok_or_else(|| ShadowError::Parse("cannot resolve latest tag without http feature".into()))
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
            download_via_http(&url, dest).await
        }
        FetchSource::Forgejo => {
            let api = &config.forgejo_api;
            let base = api.trim_end_matches("/api/v1");
            let url = format!("{base}/ecoPrimals/plasmidBin/releases/download/{tag}/{asset}");
            download_via_http(&url, dest).await
        }
        FetchSource::Vps => {
            let vps_bin_dir = std::env::var("VPS_MEMBRANE_BIN_DIR")
                .unwrap_or_else(|_| "/opt/ecoPrimals/plasmidBin/primals".into());
            let remote_path = format!("{vps_bin_dir}/{asset}");
            download_via_ssh(&config.ssh_host, &remote_path, dest).await
        }
    }
}

/// Download a file from a remote host via SSH (replaces rsync dependency).
///
/// Pipes `cat <remote_path>` through SSH directly to the local file.
/// Zero external dependencies beyond the system SSH client.
async fn download_via_ssh(host: &str, remote_path: &str, dest: &Path) -> bool {
    let output = tokio::process::Command::new("ssh")
        .args([
            "-o",
            "ConnectTimeout=30",
            "-o",
            "BatchMode=yes",
            host,
            "cat",
            remote_path,
        ])
        .output()
        .await;

    match output {
        Ok(o) if o.status.success() && !o.stdout.is_empty() => {
            tokio::fs::write(dest, &o.stdout).await.is_ok()
        }
        _ => false,
    }
}

/// Download a URL to a local path using reqwest (pure Rust HTTP).
#[cfg(feature = "http")]
async fn download_via_http(url: &str, dest: &Path) -> bool {
    let Ok(client) = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
    else {
        return false;
    };

    let response = match client.get(url).send().await {
        Ok(r) if r.status().is_success() => r,
        _ => return false,
    };

    let Ok(bytes) = response.bytes().await else {
        return false;
    };

    tokio::fs::write(dest, &bytes).await.is_ok()
}

#[cfg(not(feature = "http"))]
async fn download_via_http(_url: &str, _dest: &Path) -> bool {
    false
}

/// Compute BLAKE3 hash of a file in pure Rust (no external b3sum needed).
fn compute_blake3(path: &Path) -> std::io::Result<String> {
    let data = std::fs::read(path)?;
    Ok(blake3::hash(&data).to_hex().to_string())
}

/// Verify BLAKE3 checksum against an expected hash.
fn verify_blake3(path: &Path, expected: &str) -> bool {
    if expected.is_empty() {
        return false;
    }
    compute_blake3(path).is_ok_and(|actual| actual == expected)
}

/// Load BLAKE3 checksums from a `checksums.toml` file if available.
///
/// Format: `[checksums]\nprimal_name = "hex_hash"`
fn load_checksums(bin_dir: &Path, tag: &str) -> std::collections::HashMap<String, String> {
    #[derive(Deserialize)]
    struct ChecksumFile {
        #[serde(default)]
        checksums: std::collections::HashMap<String, String>,
    }

    let checksums_path = bin_dir
        .parent()
        .unwrap_or(bin_dir)
        .join(format!("checksums-{tag}.toml"));

    let alt_path = bin_dir.join("checksums.toml");

    let path = if checksums_path.exists() {
        checksums_path
    } else if alt_path.exists() {
        alt_path
    } else {
        return std::collections::HashMap::new();
    };

    let Ok(contents) = std::fs::read_to_string(&path) else {
        return std::collections::HashMap::new();
    };

    toml::from_str::<ChecksumFile>(&contents)
        .map(|f| f.checksums)
        .unwrap_or_default()
}

/// Execute a full plasmid fetch operation.
///
/// # Errors
///
/// Returns `Err` on IO failures or if the release tag cannot be resolved.
pub async fn fetch(config: &crate::ShadowConfig, args: &FetchArgs) -> Result<ShadowOutcome> {
    let arch = detect_target_triple();
    let dest_root = resolve_dest(args.dest.as_deref());
    let bin_dir = dest_root.join("primals").join(&arch);
    let tag = resolve_tag(args.source, args.release_tag.as_deref(), config).await?;

    #[allow(clippy::option_if_let_else)]
    let primals: Vec<&str> = match args.primal.as_deref() {
        Some(p) => vec![p],
        None => nucleus_primals(),
    };

    if args.dry_run {
        return Ok(format_dry_run(&primals, &arch, &tag, &bin_dir, args.source));
    }

    std::fs::create_dir_all(&bin_dir).map_err(ShadowError::Io)?;

    let checksums = load_checksums(&bin_dir, &tag);
    let results = fetch_primals(&primals, &bin_dir, &arch, &tag, &checksums, args, config).await;

    Ok(format_fetch_outcome(
        args.source,
        &arch,
        &tag,
        &bin_dir,
        &results,
    ))
}

fn format_dry_run(
    primals: &[&str],
    arch: &str,
    tag: &str,
    bin_dir: &std::path::Path,
    source: FetchSource,
) -> ShadowOutcome {
    let lines: Vec<String> = primals
        .iter()
        .map(|p| format!("  [dry-run] {p}-{arch} from {tag}"))
        .collect();
    ShadowOutcome::ok(format!(
        "DRY RUN: {} primals from {source} ({tag})\n  Arch: {arch}\n  Dest: {}\n{}",
        primals.len(),
        bin_dir.display(),
        lines.join("\n"),
    ))
}

async fn fetch_primals(
    primals: &[&str],
    bin_dir: &std::path::Path,
    arch: &str,
    tag: &str,
    checksums: &std::collections::HashMap<String, String>,
    args: &FetchArgs,
    config: &crate::ShadowConfig,
) -> Vec<FetchResult> {
    let mut results = Vec::with_capacity(primals.len());

    for primal in primals {
        let local_path = bin_dir.join(primal);

        if local_path.exists() && !args.force {
            results.push(FetchResult {
                primal: (*primal).to_string(),
                status: "exists".into(),
                tag: None,
                verified: false,
            });
            continue;
        }

        let _ = std::fs::remove_file(&local_path);

        let arch_asset = format!("{primal}-{arch}");
        let got = download_asset(args.source, config, tag, &arch_asset, &local_path).await
            || download_asset(args.source, config, tag, primal, &local_path).await;

        if !got {
            results.push(FetchResult {
                primal: (*primal).to_string(),
                status: "download_failed".into(),
                tag: Some(tag.to_string()),
                verified: false,
            });
            continue;
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&local_path, std::fs::Permissions::from_mode(0o755));
        }

        let is_verified = checksums
            .get(*primal)
            .is_some_and(|expected| verify_blake3(&local_path, expected));

        results.push(FetchResult {
            primal: (*primal).to_string(),
            status: "ok".into(),
            tag: Some(tag.to_string()),
            verified: is_verified,
        });
    }

    results
}

fn format_fetch_outcome(
    source: FetchSource,
    arch: &str,
    tag: &str,
    bin_dir: &std::path::Path,
    results: &[FetchResult],
) -> ShadowOutcome {
    let downloaded = results.iter().filter(|r| r.status == "ok").count() as u32;
    let verified = results.iter().filter(|r| r.verified).count() as u32;
    let skipped = results.iter().filter(|r| r.status == "exists").count() as u32;
    let failed = results
        .iter()
        .filter(|r| r.status == "download_failed")
        .count() as u32;

    let summary = FetchSummary {
        source: source.to_string(),
        arch: arch.to_string(),
        release: tag.to_string(),
        dest: bin_dir.to_string_lossy().into(),
        downloaded,
        verified,
        skipped,
        failed,
        results: results.to_vec(),
    };

    let status_lines: Vec<String> = results
        .iter()
        .map(|r| {
            let mark = match r.status.as_str() {
                "ok" if r.verified => "OK verified",
                "ok" => "OK",
                "exists" => "EXISTS",
                _ => "FAIL",
            };
            format!("  [{:<12}] {mark}", r.primal)
        })
        .collect();

    let msg = format!(
        "primalSpring fetch — {source}\n\
         Arch:     {arch}\n\
         Release:  {tag}\n\
         Dest:     {}\n\n\
         {}\n\n\
         Downloaded: {downloaded}  Verified: {verified}  Skipped: {skipped}  Failed: {failed}",
        summary.dest,
        status_lines.join("\n"),
    );

    if failed == 0 {
        ShadowOutcome::ok_with(msg, serde_json::to_value(&summary).unwrap_or_default())
    } else {
        ShadowOutcome {
            ok: false,
            message: msg,
            data: serde_json::to_value(&summary).ok(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(nucleus_primals().len(), 13);
    }

    #[test]
    fn nucleus_primals_derived_from_registry() {
        let derived = nucleus_primals();
        let registry: Vec<&str> = cellmembrane_types::MembraneService::all()
            .iter()
            .filter(|s| s.is_primal)
            .map(|s| s.binary)
            .collect();
        assert_eq!(derived, registry, "nucleus_primals() must match registry");
    }

    #[test]
    fn resolve_dest_uses_override() {
        let d = resolve_dest(Some("/tmp/test-plasmid"));
        assert_eq!(d, PathBuf::from("/tmp/test-plasmid"));
    }

    #[test]
    fn fetch_source_from_str() {
        assert_eq!(
            "github".parse::<FetchSource>().unwrap(),
            FetchSource::GitHub
        );
        assert_eq!("vps".parse::<FetchSource>().unwrap(), FetchSource::Vps);
        assert_eq!(
            "forgejo".parse::<FetchSource>().unwrap(),
            FetchSource::Forgejo
        );
        assert!("invalid".parse::<FetchSource>().is_err());
    }

    #[test]
    fn blake3_verify_empty_expected_returns_false() {
        let tmp = std::env::temp_dir().join("b3-test-empty");
        std::fs::write(&tmp, b"test data").unwrap();
        assert!(!verify_blake3(&tmp, ""));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn blake3_verify_correct_hash() {
        let tmp = std::env::temp_dir().join("b3-test-correct");
        let data = b"primal binary content";
        std::fs::write(&tmp, data).unwrap();
        let expected = blake3::hash(data).to_hex().to_string();
        assert!(verify_blake3(&tmp, &expected));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn blake3_verify_wrong_hash() {
        let tmp = std::env::temp_dir().join("b3-test-wrong");
        std::fs::write(&tmp, b"actual content").unwrap();
        assert!(!verify_blake3(
            &tmp,
            "0000000000000000000000000000000000000000000000000000000000000000"
        ));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn load_checksums_returns_empty_for_missing() {
        let checksums = load_checksums(Path::new("/tmp/nonexistent-dir"), "v0.1");
        assert!(checksums.is_empty());
    }
}
