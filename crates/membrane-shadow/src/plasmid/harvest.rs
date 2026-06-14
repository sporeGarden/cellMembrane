// SPDX-License-Identifier: AGPL-3.0-or-later

//! `plasmid.harvest` — Build primal binaries from source, verify, and stage.
//!
//! Implements the zero-touch binary harvest pipeline:
//! 1. Read `sources.toml` to discover repos + build args
//! 2. Compare HEAD commits against `provenance.toml` to detect drift
//! 3. Clone changed repos (shallow)
//! 4. Cross-compile for target triple (musl static)
//! 5. Compute BLAKE3 checksum
//! 6. Stage binary to plasmidBin depot
//! 7. Update `checksums.toml` and `provenance.toml`

use crate::ShadowOutcome;
use crate::error::{Result, ShadowError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

use super::{detect_target_triple, nucleus_primals, toolchain};

/// Parsed CLI arguments for `plasmid.harvest`.
pub struct HarvestArgs {
    /// Single primal to harvest (None = all with changes).
    pub primal: Option<String>,
    /// Force rebuild even if commit hasn't changed.
    pub force: bool,
    /// Show what would be built without executing.
    pub dry_run: bool,
    /// Override plasmidBin depot path.
    pub depot_dir: Option<String>,
    /// Override target triple (e.g. `aarch64-unknown-linux-musl` for cross-compile).
    pub target: Option<String>,
}

/// Outcome of harvesting a single primal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarvestResult {
    /// Binary name.
    pub binary: String,
    /// Outcome status.
    pub status: HarvestStatus,
    /// Human-readable detail.
    pub detail: String,
}

/// Status of a single primal harvest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HarvestStatus {
    /// Built and staged successfully.
    Built,
    /// No changes detected — skipped.
    Current,
    /// Build failed.
    Failed,
    /// Skipped (private repo without access, etc.).
    Skipped,
}

/// Source entry from `sources.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct SourceEntry {
    pub repo: String,
    #[serde(default)]
    pub private: bool,
    #[serde(default)]
    pub build_args: Option<String>,
    #[serde(default)]
    pub binary_name: Option<String>,
}

/// Provenance entry from `provenance.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvenanceEntry {
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
}

/// Full provenance file structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvenanceFile {
    #[serde(default)]
    pub generated: Option<String>,
    #[serde(default)]
    pub builder: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub rustc: Option<String>,
    #[serde(flatten)]
    pub entries: BTreeMap<String, ProvenanceEntry>,
}

/// Wrapper for `sources.toml` deserialization.
#[cfg(test)]
#[derive(Deserialize)]
struct SourcesFile {
    sources: BTreeMap<String, SourceEntry>,
}

/// Checksum entry from `checksums.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChecksumEntry {
    pub blake3: String,
    pub size: u64,
}

/// Harvest primals: detect changes, build, checksum, stage.
pub async fn harvest(args: &HarvestArgs) -> Result<ShadowOutcome> {
    let depot_dir = resolve_depot(args.depot_dir.as_deref())?;
    let sources = load_sources(&depot_dir)?;
    let provenance = load_provenance(&depot_dir);
    let target = args.target.clone().unwrap_or_else(detect_target_triple);

    let primals_to_harvest = determine_primals(args, &sources)?;

    let mut results: Vec<HarvestResult> = Vec::new();

    for primal in &primals_to_harvest {
        let Some(source) = sources.get(primal.as_str()) else {
            results.push(HarvestResult {
                binary: primal.clone(),
                status: HarvestStatus::Skipped,
                detail: "not in sources.toml".into(),
            });
            continue;
        };

        let needs_rebuild = args.force
            || has_upstream_changes(primal, source, provenance.as_ref(), &depot_dir).await;

        if !needs_rebuild {
            results.push(HarvestResult {
                binary: primal.clone(),
                status: HarvestStatus::Current,
                detail: "commit unchanged".into(),
            });
            continue;
        }

        if args.dry_run {
            results.push(HarvestResult {
                binary: primal.clone(),
                status: HarvestStatus::Built,
                detail: format!("dry-run: would clone {} and build", source.repo),
            });
            continue;
        }

        let result = harvest_one(primal, source, &target, &depot_dir).await;
        results.push(result);
    }

    if !args.dry_run {
        let built: Vec<&HarvestResult> = results
            .iter()
            .filter(|r| matches!(r.status, HarvestStatus::Built))
            .collect();
        if !built.is_empty() {
            if let Err(e) = update_depot_metadata(&depot_dir, &target, &built).await {
                eprintln!("warn: failed to update depot metadata: {e}");
            }
            publish_depot_checksums(&depot_dir).await;
        }
    }

    Ok(format_harvest_outcome(&results))
}

fn determine_primals(
    args: &HarvestArgs,
    sources: &BTreeMap<String, SourceEntry>,
) -> Result<Vec<String>> {
    if let Some(name) = args.primal.as_deref() {
        if !sources.contains_key(name) {
            return Err(ShadowError::Parse(format!(
                "'{name}' not found in sources.toml"
            )));
        }
        Ok(vec![name.to_string()])
    } else {
        let registry_primals = nucleus_primals();
        Ok(registry_primals
            .into_iter()
            .filter(|p| sources.contains_key(*p))
            .map(ToString::to_string)
            .collect())
    }
}

async fn has_upstream_changes(
    primal: &str,
    source: &SourceEntry,
    provenance: Option<&ProvenanceFile>,
    depot_dir: &Path,
) -> bool {
    let Some(prov) = provenance else {
        return true;
    };
    let Some(entry) = prov.entries.get(primal) else {
        return true;
    };
    let Some(prev_commit) = entry.commit.as_deref() else {
        return true;
    };

    fetch_head_commit(&source.repo, depot_dir)
        .await
        .is_none_or(|head| !head.starts_with(prev_commit) && !prev_commit.starts_with(&head))
}

/// Fetch HEAD from both outer (GitHub) and inner (Forgejo) membranes.
///
/// Returns the commit that is farthest ahead — if either remote has a newer
/// commit than provenance, we should detect drift. This ensures golgiBody
/// sees GitHub pushes and peptidoglycan sees both layers.
async fn fetch_head_commit(repo: &str, _depot_dir: &Path) -> Option<String> {
    let forgejo_host = std::env::var(cellmembrane_types::service::ENV_FORGEJO_SSH_HOST)
        .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_FORGEJO_GIT_ADDR.into());

    let forgejo = try_ls_remote_head(&format!("ssh://git@{forgejo_host}/{repo}.git")).await;
    let github = try_ls_remote_head(&format!("https://github.com/{repo}.git")).await;

    // Prefer GitHub (outer) if available — it's where pushes land first.
    // Forgejo mirrors from GitHub so it can lag behind.
    github.or(forgejo)
}

async fn try_ls_remote_head(url: &str) -> Option<String> {
    let output = tokio::process::Command::new("git")
        .args(["ls-remote", url, "HEAD"])
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.split_whitespace().next().map(|s| s[..8].to_string())
}

async fn harvest_one(
    primal: &str,
    source: &SourceEntry,
    target: &str,
    depot_dir: &Path,
) -> HarvestResult {
    let build_root = std::env::temp_dir().join("membrane-harvest");
    let clone_dir = build_root.join(primal);

    if let Err(detail) = clone_source(primal, source, &build_root, &clone_dir).await {
        let status = if source.private {
            HarvestStatus::Skipped
        } else {
            HarvestStatus::Failed
        };
        return HarvestResult {
            binary: primal.into(),
            status,
            detail,
        };
    }

    let head_commit = get_local_head(&clone_dir).await.unwrap_or_default();

    if let Err(detail) = toolchain::build_binary(source, target, &clone_dir).await {
        return HarvestResult {
            binary: primal.into(),
            status: HarvestStatus::Failed,
            detail,
        };
    }

    let binary_name = source.binary_name.as_deref().unwrap_or(primal);
    let bin_path = clone_dir
        .join("target")
        .join(target)
        .join("release")
        .join(binary_name);

    if !bin_path.exists() {
        return HarvestResult {
            binary: primal.into(),
            status: HarvestStatus::Failed,
            detail: format!("binary not found at {}", bin_path.display()),
        };
    }

    // BUILD-ELF-01: validate architecture before staging
    if let Err(detail) = validate_elf_arch(&bin_path, target).await {
        return HarvestResult {
            binary: primal.into(),
            status: HarvestStatus::Failed,
            detail,
        };
    }

    toolchain::strip_binary(&bin_path, primal, target).await;

    match stage_to_depot(primal, &bin_path, depot_dir, target) {
        Ok((size, blake3)) => {
            let _ = std::fs::remove_dir_all(&clone_dir);
            HarvestResult {
                binary: primal.into(),
                status: HarvestStatus::Built,
                detail: format!(
                    "{}KB blake3={} commit={}",
                    size / 1024,
                    &blake3[..16],
                    &head_commit[..std::cmp::min(8, head_commit.len())]
                ),
            }
        }
        Err(detail) => HarvestResult {
            binary: primal.into(),
            status: HarvestStatus::Failed,
            detail,
        },
    }
}

async fn clone_source(
    primal: &str,
    source: &SourceEntry,
    build_root: &Path,
    clone_dir: &Path,
) -> std::result::Result<(), String> {
    if clone_dir.exists() {
        let _ = std::fs::remove_dir_all(clone_dir);
    }
    std::fs::create_dir_all(build_root).ok();

    let forgejo_host = std::env::var(cellmembrane_types::service::ENV_FORGEJO_SSH_HOST)
        .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_FORGEJO_GIT_ADDR.into());
    let forgejo_url = format!("ssh://git@{forgejo_host}/{}.git", source.repo);
    let github_url = format!("https://github.com/{}.git", source.repo);

    // Try Forgejo SSH first (works for private repos, inner membrane)
    if toolchain::try_clone(&forgejo_url, clone_dir).await {
        return Ok(());
    }

    // Fall back to GitHub HTTPS (outer membrane, public repos)
    if toolchain::try_clone(&github_url, clone_dir).await {
        return Ok(());
    }

    if source.private {
        Err(format!(
            "private repo — neither Forgejo SSH nor GitHub accessible ({primal})"
        ))
    } else {
        Err("git clone failed on both Forgejo and GitHub".into())
    }
}

pub(super) use toolchain::{
    ANDROID_TARGET, ENV_ANDROID_NDK_HOME, resolve_ndk_linker, validate_elf_arch,
};

fn stage_to_depot(
    primal: &str,
    bin_path: &Path,
    depot_dir: &Path,
    target: &str,
) -> std::result::Result<(u64, String), String> {
    let staging_dir = depot_dir.join("primals").join(target);
    std::fs::create_dir_all(&staging_dir).ok();
    let dest = staging_dir.join(primal);
    let tmp = staging_dir.join(format!(".{primal}.new"));

    std::fs::copy(bin_path, &tmp).map_err(|e| format!("copy to depot failed: {e}"))?;
    std::fs::rename(&tmp, &dest).map_err(|e| format!("atomic rename failed: {e}"))?;

    let size = std::fs::metadata(&dest).map_or(0, |m| m.len());
    let blake3 = compute_blake3_file(&dest);
    Ok((size, blake3))
}

async fn get_local_head(repo_dir: &Path) -> Option<String> {
    crate::git_ops::git_output(repo_dir, &["rev-parse", "--short=8", "HEAD"])
        .await
        .ok()
}

pub(super) use super::depot::{
    compute_blake3_file, load_provenance, load_sources, resolve_depot, update_depot_metadata,
};

/// Commit and push updated checksums.toml + provenance.toml to git.
/// Non-fatal — harvest succeeds even if git publish fails.
async fn publish_depot_checksums(depot_dir: &std::path::Path) {
    if !depot_dir.join(".git").is_dir() {
        return;
    }

    if !crate::git_ops::git_success(depot_dir, &["add", "checksums.toml", "provenance.toml"]).await
    {
        return;
    }

    let has_staged = !crate::git_ops::git_success(depot_dir, &["diff", "--cached", "--quiet"]).await;
    if !has_staged {
        return;
    }

    let commit_msg = format!(
        "harvest: update checksums + provenance ({})",
        chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ")
    );

    let _ = crate::git_ops::git_output(depot_dir, &["commit", "-m", &commit_msg]).await;
    let _ = crate::git_ops::push_all_remotes(depot_dir).await;
}

fn format_harvest_outcome(results: &[HarvestResult]) -> ShadowOutcome {
    let built = results
        .iter()
        .filter(|r| matches!(r.status, HarvestStatus::Built))
        .count();
    let current = results
        .iter()
        .filter(|r| matches!(r.status, HarvestStatus::Current))
        .count();
    let failed = results
        .iter()
        .filter(|r| matches!(r.status, HarvestStatus::Failed))
        .count();
    let skipped = results
        .iter()
        .filter(|r| matches!(r.status, HarvestStatus::Skipped))
        .count();

    let msg =
        format!("harvest: {built} built, {current} current, {skipped} skipped, {failed} failed");

    ShadowOutcome {
        ok: failed == 0,
        message: msg,
        data: serde_json::to_value(results).ok(),
    }
}

/// Public wrapper to check upstream changes for a primal — used by `status`.
///
/// Uses lenient mode: if remote HEAD cannot be fetched (network failure),
/// assume current rather than reporting false drift.
pub(super) async fn has_upstream_changes_pub(
    primal: &str,
    source: &SourceEntry,
    provenance: Option<&ProvenanceFile>,
    depot_dir: &Path,
) -> bool {
    let Some(prov) = provenance else {
        return true;
    };
    let Some(entry) = prov.entries.get(primal) else {
        return true;
    };
    let Some(prev_commit) = entry.commit.as_deref() else {
        return true;
    };

    fetch_head_commit(&source.repo, depot_dir)
        .await
        .is_some_and(|head| !head.starts_with(prev_commit) && !prev_commit.starts_with(&head))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_entry_deserialize() {
        let toml_str = r#"
[sources.beardog]
repo = "https://git.primals.eco/ecoPrimals/bearDog.git"
private = true
build_args = "--features server"
"#;
        let parsed: SourcesFile = toml::from_str(toml_str).unwrap();
        let entry = &parsed.sources["beardog"];
        assert_eq!(entry.repo, "https://git.primals.eco/ecoPrimals/bearDog.git");
        assert!(entry.private);
        assert_eq!(entry.build_args.as_deref(), Some("--features server"));
        assert!(entry.binary_name.is_none());
    }

    #[test]
    fn source_entry_minimal() {
        let toml_str = r#"
[sources.songbird]
repo = "https://git.primals.eco/ecoPrimals/songBird.git"
"#;
        let parsed: SourcesFile = toml::from_str(toml_str).unwrap();
        let entry = &parsed.sources["songbird"];
        assert!(!entry.private);
        assert!(entry.build_args.is_none());
    }

    #[test]
    fn provenance_file_roundtrip() {
        let mut entries = BTreeMap::new();
        entries.insert(
            "beardog".into(),
            ProvenanceEntry {
                version: Some("0.9.1".into()),
                commit: Some("abc123".into()),
                source: Some("forgejo".into()),
            },
        );
        let prov = ProvenanceFile {
            generated: Some("2026-06-07".into()),
            builder: Some("eastGate".into()),
            target: Some("x86_64-unknown-linux-musl".into()),
            rustc: Some("1.96.0".into()),
            entries,
        };
        let serialized = toml::to_string_pretty(&prov).unwrap();
        let deserialized: ProvenanceFile = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.generated.as_deref(), Some("2026-06-07"));
        assert_eq!(
            deserialized.entries["beardog"].commit.as_deref(),
            Some("abc123")
        );
    }

    #[test]
    fn checksum_entry_serde() {
        let entry = ChecksumEntry {
            blake3: "deadbeef".into(),
            size: 42_000,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let back: ChecksumEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back.blake3, "deadbeef");
        assert_eq!(back.size, 42_000);
    }

    #[test]
    fn harvest_result_status_display() {
        let result = HarvestResult {
            binary: "beardog".into(),
            status: HarvestStatus::Built,
            detail: "compiled OK".into(),
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["status"], "Built");
        assert_eq!(json["binary"], "beardog");
    }

    #[test]
    fn format_harvest_outcome_all_current() {
        let results = vec![
            HarvestResult {
                binary: "a".into(),
                status: HarvestStatus::Current,
                detail: "no change".into(),
            },
            HarvestResult {
                binary: "b".into(),
                status: HarvestStatus::Current,
                detail: "no change".into(),
            },
        ];
        let outcome = format_harvest_outcome(&results);
        assert!(outcome.ok);
        assert!(outcome.message.contains("0 built"));
        assert!(outcome.message.contains("2 current"));
    }

    #[test]
    fn format_harvest_outcome_with_failure() {
        let results = vec![
            HarvestResult {
                binary: "a".into(),
                status: HarvestStatus::Built,
                detail: "ok".into(),
            },
            HarvestResult {
                binary: "b".into(),
                status: HarvestStatus::Failed,
                detail: "build error".into(),
            },
        ];
        let outcome = format_harvest_outcome(&results);
        assert!(!outcome.ok);
        assert!(outcome.message.contains("1 built"));
        assert!(outcome.message.contains("1 failed"));
    }

    #[test]
    fn load_sources_from_tempdir() {
        let tmp = std::env::temp_dir().join("harvest_test_sources");
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("sources.toml"),
            r#"
[sources.beardog]
repo = "https://example.com/beardog.git"
[sources.songbird]
repo = "https://example.com/songbird.git"
private = true
"#,
        )
        .unwrap();

        let sources = load_sources(&tmp).unwrap();
        assert_eq!(sources.len(), 2);
        assert!(sources.contains_key("beardog"));
        assert!(sources["songbird"].private);

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn load_provenance_missing_returns_none() {
        let tmp = std::env::temp_dir().join("harvest_test_no_prov");
        std::fs::create_dir_all(&tmp).unwrap();
        assert!(load_provenance(&tmp).is_none());
        std::fs::remove_dir_all(&tmp).ok();
    }
}
