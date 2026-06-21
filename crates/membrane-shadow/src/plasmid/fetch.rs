// SPDX-License-Identifier: AGPL-3.0-or-later

//! `plasmid.fetch` — Download primal binaries from sovereign or external sources.

use crate::ShadowOutcome;
use crate::error::{Result, ShadowError};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::{detect_target_triple, nucleus_primals};

/// Source backend for binary downloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchSource {
    /// GitHub Releases (outer membrane, default).
    GitHub,
    /// VPS membrane depot via SSH/rsync (sovereign, LAN only).
    Vps,
    /// Forgejo releases (sovereign, inner membrane).
    Forgejo,
    /// WAN HTTPS depot served by Caddy on the outer membrane.
    /// Usable by any gate with internet access (no SSH required).
    Wan,
}

impl std::fmt::Display for FetchSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GitHub => f.write_str("github"),
            Self::Vps => f.write_str("vps"),
            Self::Forgejo => f.write_str("forgejo"),
            Self::Wan => f.write_str("wan"),
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
            "wan" => Ok(Self::Wan),
            _ => Err(ShadowError::Parse(format!(
                "unknown source '{s}' (expected: github, vps, forgejo, wan)"
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

/// Outcome of fetching a single primal binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FetchStatus {
    /// Downloaded successfully.
    Ok,
    /// Already present — skipped.
    Exists,
    /// Download failed (network, checksum, or filesystem error).
    DownloadFailed,
}

impl std::fmt::Display for FetchStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ok => write!(f, "ok"),
            Self::Exists => write!(f, "exists"),
            Self::DownloadFailed => write!(f, "download_failed"),
        }
    }
}

/// Result of fetching a single primal binary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchResult {
    /// Primal name.
    pub primal: String,
    /// Outcome of the fetch.
    pub status: FetchStatus,
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

/// Execute a full plasmid fetch operation.
///
/// Fetches musl binaries for all primals. Additionally fetches gnu binaries
/// for GPU primals (`barracuda`, `coralreef`) when the gate needs GPU support.
///
/// # Errors
///
/// Returns `Err` on IO failures or if the release tag cannot be resolved.
pub async fn fetch(config: &crate::ShadowConfig, args: &FetchArgs) -> Result<ShadowOutcome> {
    let arch = detect_target_triple();
    let dest_root = resolve_dest(args.dest.as_deref());
    let bin_dir = dest_root.join("primals").join(&arch);
    let tag = resolve_tag(args.source, args.release_tag.as_deref(), config).await?;

    #[allow(clippy::option_if_let_else)] // lifetimes differ: &'a str vs &'static str
    let primals: Vec<&str> = match args.primal.as_deref() {
        Some(p) => vec![p],
        None => nucleus_primals(),
    };

    if args.dry_run {
        return Ok(format_dry_run(&primals, &arch, &tag, &bin_dir, args.source));
    }

    tokio::fs::create_dir_all(&bin_dir)
        .await
        .map_err(ShadowError::Io)?;

    download::cleanup_partial_downloads(&bin_dir).await;

    let bd = bin_dir.clone();
    let t = tag.clone();
    let mut checksums = tokio::task::spawn_blocking(move || checksum::load_checksums(&bd, &t))
        .await
        .unwrap_or_default();
    if checksums.is_empty() && args.source == FetchSource::Wan {
        checksums = checksum::fetch_wan_checksums(&arch).await;
        if !checksums.is_empty() {
            let dr = dest_root.clone();
            let a = arch.clone();
            let cs = checksums.clone();
            if let Err(e) =
                tokio::task::spawn_blocking(move || checksum::persist_checksums(&dr, &a, &cs)).await
            {
                tracing::warn!(error = %e, "persist_checksums task failed");
            }
        }
    }
    let mut results =
        fetch_primals(&primals, &bin_dir, &arch, &tag, &checksums, args, config).await;

    if should_fetch_gpu(&primals) {
        let gnu_results = fetch_gpu_primals(&primals, &dest_root, &tag, args, config).await;
        results.extend(gnu_results);
    }

    Ok(format_fetch_outcome(
        args.source,
        &arch,
        &tag,
        &bin_dir,
        &results,
    ))
}

/// Whether any requested primals need GPU (gnu) builds.
fn should_fetch_gpu(primals: &[&str]) -> bool {
    primals
        .iter()
        .any(|p| cellmembrane_types::arch::is_gpu_primal(p))
}

/// Fetch gnu-target binaries for GPU primals into a parallel depot directory.
async fn fetch_gpu_primals(
    primals: &[&str],
    dest_root: &Path,
    tag: &str,
    args: &FetchArgs,
    config: &crate::ShadowConfig,
) -> Vec<FetchResult> {
    let gnu_arch = cellmembrane_types::TargetArch::X86_64Gnu.triple();
    let gnu_bin_dir = dest_root.join("primals").join(gnu_arch);
    if tokio::fs::create_dir_all(&gnu_bin_dir).await.is_err() {
        return Vec::new();
    }

    let gpu_primals: Vec<&str> = primals
        .iter()
        .copied()
        .filter(|p| cellmembrane_types::arch::is_gpu_primal(p))
        .collect();

    if gpu_primals.is_empty() {
        return Vec::new();
    }

    let checksums = checksum::fetch_wan_checksums(gnu_arch).await;
    fetch_primals(
        &gpu_primals,
        &gnu_bin_dir,
        gnu_arch,
        tag,
        &checksums,
        args,
        config,
    )
    .await
}

// ── Path resolution ──────────────────────────────────────────────────────────

fn resolve_dest(override_dest: Option<&str>) -> PathBuf {
    super::resolve_path(
        override_dest,
        cellmembrane_types::service::ENV_PLASMIDBIN_LEGACY,
        || {
            crate::resolve_xdg_data_home()
                .join("ecoPrimals")
                .join(cellmembrane_types::service::PLASMID_BIN_DIR)
        },
    )
}

// ── Release tag resolution ───────────────────────────────────────────────────

#[derive(Deserialize)]
struct ReleaseResponse {
    tag_name: String,
}

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
        FetchSource::Vps | FetchSource::Wan => Ok("vps-live".into()),
        FetchSource::GitHub => {
            let org = std::env::var(cellmembrane_types::service::ENV_GITHUB_ORG)
                .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_GITHUB_ORG.into());
            let url = format!("https://api.github.com/repos/{org}/plasmidBin/releases/latest");
            fetch_release_tag(&url).await
        }
        FetchSource::Forgejo => {
            let api = &config.forgejo_api;
            let base = api.trim_end_matches("/api/v1");
            let org = std::env::var(cellmembrane_types::service::ENV_FORGEJO_ORG)
                .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_FORGEJO_ORG.into());
            let url = format!("{base}/api/v1/repos/{org}/plasmidBin/releases/latest");
            fetch_release_tag(&url).await
        }
    }
}

#[cfg(feature = "http")]
async fn fetch_release_tag(url: &str) -> Result<String> {
    let client = reqwest::Client::new();
    let resp: ReleaseResponse = client
        .get(url)
        .header("User-Agent", "membrane-shadow/0.1")
        .send()
        .await
        .map_err(|e| ShadowError::Parse(format!("release API request failed: {e}")))?
        .json()
        .await
        .map_err(|e| ShadowError::Parse(format!("release API parse failed: {e}")))?;
    Ok(resp.tag_name)
}

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

use super::checksum;
use super::download;

// ── Fetch orchestration ──────────────────────────────────────────────────────

async fn fetch_primals(
    primals: &[&str],
    bin_dir: &Path,
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
                status: FetchStatus::Exists,
                tag: None,
                verified: false,
            });
            continue;
        }

        let _ = tokio::fs::remove_file(&local_path).await;

        let arch_asset = format!("{primal}-{arch}");
        let got = download::download_asset(
            args.source,
            config,
            tag,
            &arch_asset,
            arch,
            primal,
            &local_path,
        )
        .await
            || download::download_asset(
                args.source,
                config,
                tag,
                primal,
                arch,
                primal,
                &local_path,
            )
            .await;

        if !got {
            // PARTIAL-FETCH-RESUME: one retry after short backoff
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let retry_got = download::download_asset(
                args.source,
                config,
                tag,
                &arch_asset,
                arch,
                primal,
                &local_path,
            )
            .await
                || download::download_asset(
                    args.source,
                    config,
                    tag,
                    primal,
                    arch,
                    primal,
                    &local_path,
                )
                .await;
            if !retry_got {
                results.push(FetchResult {
                    primal: (*primal).to_string(),
                    status: FetchStatus::DownloadFailed,
                    tag: Some(tag.to_string()),
                    verified: false,
                });
                continue;
            }
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Err(e) =
                tokio::fs::set_permissions(&local_path, std::fs::Permissions::from_mode(0o755))
                    .await
            {
                tracing::warn!(error = %e, path = %local_path.display(), "failed to set executable permissions");
            }
        }

        let is_verified = if let Some(expected) = checksums.get(*primal) {
            checksum::verify_blake3_async(local_path.clone(), expected.clone()).await
        } else {
            false
        };

        results.push(FetchResult {
            primal: (*primal).to_string(),
            status: FetchStatus::Ok,
            tag: Some(tag.to_string()),
            verified: is_verified,
        });
    }

    results
}

// ── Formatting ───────────────────────────────────────────────────────────────

fn format_dry_run(
    primals: &[&str],
    arch: &str,
    tag: &str,
    bin_dir: &Path,
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

fn format_fetch_outcome(
    source: FetchSource,
    arch: &str,
    tag: &str,
    bin_dir: &Path,
    results: &[FetchResult],
) -> ShadowOutcome {
    let downloaded = u32::try_from(
        results
            .iter()
            .filter(|r| r.status == FetchStatus::Ok)
            .count(),
    )
    .unwrap_or(u32::MAX);
    let verified = u32::try_from(results.iter().filter(|r| r.verified).count()).unwrap_or(u32::MAX);
    let skipped = u32::try_from(
        results
            .iter()
            .filter(|r| r.status == FetchStatus::Exists)
            .count(),
    )
    .unwrap_or(u32::MAX);
    let failed = u32::try_from(
        results
            .iter()
            .filter(|r| r.status == FetchStatus::DownloadFailed)
            .count(),
    )
    .unwrap_or(u32::MAX);

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
            let mark = match r.status {
                FetchStatus::Ok if r.verified => "OK verified",
                FetchStatus::Ok => "OK",
                FetchStatus::Exists => "EXISTS",
                FetchStatus::DownloadFailed => "FAIL",
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

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::checksum::compute_blake3;
    use super::*;

    #[test]
    fn fetch_source_parse_roundtrip() {
        assert_eq!(
            "github".parse::<FetchSource>().unwrap(),
            FetchSource::GitHub
        );
        assert_eq!("vps".parse::<FetchSource>().unwrap(), FetchSource::Vps);
        assert_eq!(
            "forgejo".parse::<FetchSource>().unwrap(),
            FetchSource::Forgejo
        );
        assert_eq!("wan".parse::<FetchSource>().unwrap(), FetchSource::Wan);
        assert!("invalid".parse::<FetchSource>().is_err());
    }

    #[test]
    fn fetch_source_wan_display() {
        assert_eq!(FetchSource::Wan.to_string(), "wan");
    }

    #[test]
    fn detect_triple_contains_arch() {
        let triple = detect_target_triple();
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
    fn blake3_verify_known_content() {
        let tmp = std::env::temp_dir().join("b3-test-known");
        std::fs::write(&tmp, b"test data").unwrap();
        let hash = compute_blake3(&tmp).unwrap();
        assert!(checksum::verify_blake3(&tmp, &hash));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn blake3_verify_wrong_hash() {
        let tmp = std::env::temp_dir().join("b3-test-wrong");
        std::fs::write(&tmp, b"actual content").unwrap();
        assert!(!checksum::verify_blake3(
            &tmp,
            "0000000000000000000000000000000000000000000000000000000000000000"
        ));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn load_checksums_returns_empty_for_missing() {
        let checksums = checksum::load_checksums(Path::new("/tmp/nonexistent-dir"), "v0.1");
        assert!(checksums.is_empty());
    }
}
