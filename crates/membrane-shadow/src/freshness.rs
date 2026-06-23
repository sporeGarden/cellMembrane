// SPDX-License-Identifier: AGPL-3.0-or-later

//! Freshness tracking — wave-level HEAD SHAs and binary provenance drift detection.
//!
//! - `publish_freshness_toml()` — writes `freshness.toml` with HEAD SHAs after cascade.
//! - `check_installed_freshness()` — compares installed binary provenance against source.

use crate::error::{Result, ShadowError};
pub use crate::sovereignty_ledger::{SovereigntyCheck, rootpulse_commit, sovereignty_verify};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tracing::warn;

const FRESHNESS_HEADER: &str = "\
# SPDX-License-Identifier: CC-BY-SA-4.0
#
# freshness.toml — Ecosystem state snapshot at wave publish time
#
# Authority: primalSpring coordination (published each wave)
# Consumed by: membrane temporal.cascade --check, s_ecosystem_freshness scenario
#
# Regenerate: membrane temporal.cascade --publish-freshness
";

/// Serializable representation of `freshness.toml`.
#[derive(Debug, Serialize, Deserialize)]
struct FreshnessFile {
    wave: WaveSection,
    #[serde(default)]
    heads: BTreeMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct WaveSection {
    id: u32,
    #[serde(default)]
    date: String,
    #[serde(default)]
    ssot: String,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    publisher: String,
}

/// Publish `freshness.toml` — snapshot of HEAD SHAs after cascade.
///
/// Writes to `infra/wateringHole/freshness.toml` with the current wave metadata
/// and HEAD commit hashes for all manifest repos.
pub async fn publish_freshness_toml(
    root: &Path,
    manifest: &crate::manifest::EcosystemManifest,
    repos: &[(&str, &crate::manifest::RepoEntry)],
) -> Result<()> {
    let freshness_path = root
        .join(cellmembrane_types::service::INFRA_WATERING_HOLE)
        .join("freshness.toml");

    // Preserve existing heads from repos this gate doesn't track (merge, not overwrite).
    let mut heads = tokio::fs::read_to_string(&freshness_path)
        .await
        .ok()
        .and_then(|c| toml::from_str::<FreshnessFile>(&c).ok())
        .map_or_else(BTreeMap::new, |f| f.heads);

    for (name, entry) in repos {
        let repo_dir = root.join(&entry.local_path);
        if repo_dir.join(".git").exists() {
            if let Ok(sha) = git_rev_parse_head(&repo_dir).await {
                heads.insert((*name).to_string(), sha);
            }
        }
    }

    let file = FreshnessFile {
        wave: WaveSection {
            id: manifest.meta.wave,
            date: chrono_today(),
            ssot: "specs/WATERFALL_TEMPORAL_SYNC.md".into(),
            notes: "Auto-published by membrane temporal.cascade --publish-freshness".into(),
            publisher: "membrane".into(),
        },
        heads,
    };

    let body = toml::to_string_pretty(&file).map_err(ShadowError::Serialize)?;
    let content = format!("{FRESHNESS_HEADER}\n{body}");
    crate::atomic_write_async(&freshness_path, content.as_bytes())
        .await
        .map_err(ShadowError::Io)?;

    Ok(())
}

/// Auto-commit and push freshness.toml after cascade publish.
///
/// Pulls from remote first (to get full freshness as merge base), re-publishes
/// the freshness with our local heads merged in, then commits and pushes.
/// Guards against race conditions where stale gates overwrite newer data.
pub async fn auto_commit_freshness(
    root: &Path,
    manifest: &crate::manifest::EcosystemManifest,
    repos: &[(&str, &crate::manifest::RepoEntry)],
) -> Result<()> {
    let wh_dir = root.join(cellmembrane_types::service::INFRA_WATERING_HOLE);
    if !wh_dir.join(".git").exists() {
        return Err(ShadowError::Config(
            "wateringHole not a git repo — cannot auto-commit freshness".into(),
        ));
    }

    let gate = match std::env::var(cellmembrane_types::service::ENV_GATE_NAME) {
        Ok(g) => g,
        Err(_) => tokio::fs::read_to_string(root.join(".gate"))
            .await
            .map_or_else(
                |_| crate::gate::resolve_local_gate_identity(),
                |s| s.trim().to_string(),
            ),
    };

    let local_wave = read_freshness_wave_id_async(&wh_dir.join("freshness.toml")).await;

    pull_rebase_both_remotes(&wh_dir).await;

    let remote_wave = read_freshness_wave_id_async(&wh_dir.join("freshness.toml")).await;
    if remote_wave > local_wave && local_wave > 0 {
        return Ok(());
    }

    // Re-publish freshness after pull so we merge our heads into the full remote set.
    publish_freshness_toml(root, manifest, repos).await?;

    if !crate::git_ops::git_success(&wh_dir, &["add", "freshness.toml"]).await {
        return Ok(());
    }

    if crate::git_ops::git_success(&wh_dir, &["diff", "--cached", "--quiet"]).await {
        return Ok(());
    }

    let msg = format!("freshness: wave {local_wave} auto-publish by {gate}");
    crate::git_ops::run_git(&wh_dir, &["commit", "-m", &msg]).await?;

    let push_result = crate::git_ops::push_all_remotes(&wh_dir).await;
    if !push_result.failed.is_empty() {
        warn!(
            failed_remotes = %push_result.failed.join(", "),
            "freshness push failed (reconciliation attempted)"
        );
    }

    Ok(())
}

/// Pull-rebase from both forgejo and origin to sync before publishing.
///
/// On conflict, resolves freshness.toml with `--ours` strategy (local wins).
async fn pull_rebase_both_remotes(wh_dir: &Path) {
    for remote in cellmembrane_types::service::DEFAULT_PUSH_REMOTES {
        match crate::git_ops::git_output(wh_dir, &["pull", "--rebase", remote, "main"]).await {
            Ok(_) => {}
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("CONFLICT") {
                    tracing::info!("freshness.toml conflict detected, resolving with --ours");
                    let _ = crate::git_ops::git_success(
                        wh_dir,
                        &["checkout", "--ours", "freshness.toml"],
                    )
                    .await;
                    let _ = crate::git_ops::git_success(wh_dir, &["add", "freshness.toml"]).await;
                    let _ = crate::git_ops::git_success(wh_dir, &["rebase", "--continue"]).await;
                } else if !msg.contains("Already up to date") {
                    warn!(%msg, "freshness pull failed, aborting rebase");
                    let _ = crate::git_ops::git_success(wh_dir, &["rebase", "--abort"]).await;
                }
            }
        }
    }
}

/// Parse the wave ID from a freshness.toml file. Returns 0 if unreadable.
#[cfg(test)]
fn read_freshness_wave_id(path: &Path) -> u32 {
    let Ok(content) = std::fs::read_to_string(path) else {
        return 0;
    };
    toml::from_str::<FreshnessFile>(&content).map_or(0, |f| f.wave.id)
}

/// Async variant — reads the file without blocking the runtime.
async fn read_freshness_wave_id_async(path: &Path) -> u32 {
    let Ok(content) = tokio::fs::read_to_string(path).await else {
        return 0;
    };
    toml::from_str::<FreshnessFile>(&content).map_or(0, |f| f.wave.id)
}

/// Check installed binary freshness against source HEAD SHAs.
///
/// Reads provenance sidecars from `~/.local/share/ecoPrimals/provenance/`
/// (written by `plasmidbin install`) and compares `build_commit` against
/// the current HEAD of each primal's local source repo.
///
/// Returns a formatted report string showing drift status per binary.
pub(crate) fn check_installed_freshness() -> Result<String> {
    use std::fmt::Write;

    let provenance_dir = dirs_provenance();
    if !provenance_dir.exists() {
        return Ok("No provenance sidecars found (plasmidbin not yet installed binaries).".into());
    }

    let root = crate::temporal::resolve_workspace_root()?;
    let mut report = String::from("=== Binary Freshness Check ===\n");
    let mut fresh = 0u32;
    let mut stale = 0u32;
    let mut unknown = 0u32;

    let Ok(entries) = std::fs::read_dir(&provenance_dir) else {
        return Ok("Cannot read provenance directory.".into());
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "toml") {
            continue;
        }

        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };

        let Ok(prov) = toml::from_str::<ProvenanceSidecar>(&contents) else {
            continue;
        };

        let primal_name = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let source_head = resolve_source_head(&root, &prov.source_path);

        let age = prov.installed_at.as_deref().unwrap_or("?");
        let hash_short = prov
            .binary_blake3
            .as_deref()
            .map_or_else(|| "-".to_string(), |h| h[..8.min(h.len())].to_string());

        let status = match &source_head {
            Some(head) if head == &prov.build_commit => {
                fresh += 1;
                format!("FRESH (b3={hash_short}) [{age}]")
            }
            Some(head) => {
                stale += 1;
                format!(
                    "STALE (installed={}, HEAD={}, b3={hash_short}) [{age}]",
                    &prov.build_commit[..8.min(prov.build_commit.len())],
                    &head[..8.min(head.len())]
                )
            }
            None => {
                unknown += 1;
                format!("UNKNOWN (source not found, b3={hash_short}) [{age}]")
            }
        };

        writeln!(report, "  {primal_name:<20} {status}").ok();
    }

    writeln!(report, "\nfresh={fresh} stale={stale} unknown={unknown}").ok();
    Ok(report)
}

// ── Helpers ──────────────────────────────────────────────────────────

/// Get today's date as YYYY-MM-DD.
fn chrono_today() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

/// Provenance sidecar written by `plasmidbin install`.
#[derive(Debug, serde::Deserialize)]
struct ProvenanceSidecar {
    build_commit: String,
    source_path: String,
    installed_at: Option<String>,
    binary_blake3: Option<String>,
}

/// Standard provenance directory (XDG data dir).
fn dirs_provenance() -> PathBuf {
    crate::resolve_xdg_data_home()
        .join("ecoPrimals")
        .join("provenance")
}

/// Get full HEAD SHA of a source repo given its path (sync).
fn resolve_source_head(workspace_root: &Path, source_path: &str) -> Option<String> {
    let repo_dir = if PathBuf::from(source_path).is_absolute() {
        PathBuf::from(source_path)
    } else {
        workspace_root.join(source_path)
    };
    crate::git_ops::resolve_head_full(&repo_dir)
}

/// Async git rev-parse HEAD.
async fn git_rev_parse_head(repo_dir: &Path) -> Result<String> {
    crate::git_ops::git_output(repo_dir, &["rev-parse", "HEAD"]).await
}

/// Read the current wave ID from `freshness.toml` in the workspace.
///
/// Returns `0` if the file is missing, unparseable, or has no wave ID.
/// This is the canonical source — do not duplicate.
#[must_use]
pub(crate) fn current_wave(workspace_root: &Path) -> u32 {
    let path = workspace_root
        .join(cellmembrane_types::service::INFRA_WATERING_HOLE)
        .join("freshness.toml");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return 0;
    };
    toml::from_str::<FreshnessFile>(&content).map_or(0, |f| f.wave.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chrono_today_format() {
        let today = chrono_today();
        assert_eq!(today.len(), 10);
        assert_eq!(&today[4..5], "-");
        assert_eq!(&today[7..8], "-");
        let year: u32 = today[..4].parse().unwrap();
        assert!(year >= 2024);
    }

    #[test]
    fn dirs_provenance_uses_xdg() {
        let dir = dirs_provenance();
        assert!(dir.to_string_lossy().contains("ecoPrimals/provenance"));
    }

    #[test]
    fn resolve_source_head_returns_none_for_missing() {
        let result = resolve_source_head(Path::new("/tmp"), "nonexistent-repo-xyz");
        assert!(result.is_none());
    }

    #[test]
    fn resolve_source_head_handles_absolute_path() {
        let result = resolve_source_head(Path::new("/tmp"), "/tmp/no-such-dir");
        assert!(result.is_none());
    }

    #[test]
    fn provenance_sidecar_deserializes() {
        let toml_str = r#"
build_commit = "abc12345"
source_path = "primals/bearDog"
installed_at = "2026-06-07T12:00:00Z"
binary_blake3 = "deadbeef0123456789"
"#;
        let prov: ProvenanceSidecar = toml::from_str(toml_str).unwrap();
        assert_eq!(prov.build_commit, "abc12345");
        assert_eq!(prov.source_path, "primals/bearDog");
        assert_eq!(prov.installed_at.as_deref(), Some("2026-06-07T12:00:00Z"));
        assert_eq!(prov.binary_blake3.as_deref(), Some("deadbeef0123456789"));
    }

    #[test]
    fn provenance_sidecar_optional_fields() {
        let toml_str = r#"
build_commit = "abc12345"
source_path = "primals/bearDog"
"#;
        let prov: ProvenanceSidecar = toml::from_str(toml_str).unwrap();
        assert!(prov.installed_at.is_none());
        assert!(prov.binary_blake3.is_none());
    }

    #[test]
    fn read_freshness_wave_id_parses_correctly() {
        use std::io::Write as IoWrite;
        let dir = std::env::temp_dir().join("freshness-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("freshness.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "[wave]").unwrap();
        writeln!(f, "id = 111").unwrap();
        writeln!(f, "date = \"2026-06-12\"").unwrap();
        drop(f);
        assert_eq!(read_freshness_wave_id(&path), 111);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_freshness_wave_id_returns_zero_for_missing() {
        assert_eq!(
            read_freshness_wave_id(Path::new("/tmp/nonexistent-freshness-xyz.toml")),
            0
        );
    }
}
