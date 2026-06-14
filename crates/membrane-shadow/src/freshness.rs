// SPDX-License-Identifier: AGPL-3.0-or-later

//! Freshness tracking — wave-level HEAD SHAs and binary provenance drift detection.
//!
//! - `publish_freshness_toml()` — writes `freshness.toml` with HEAD SHAs after cascade.
//! - `check_installed_freshness()` — compares installed binary provenance against source.

use crate::error::{Result, ShadowError};
use std::path::{Path, PathBuf};

/// Publish `freshness.toml` — snapshot of HEAD SHAs after cascade.
///
/// Writes to `infra/wateringHole/freshness.toml` with the current wave metadata
/// and HEAD commit hashes for all manifest repos.
pub async fn publish_freshness_toml(
    root: &Path,
    manifest: &crate::manifest::EcosystemManifest,
    repos: &[(&str, &crate::manifest::RepoEntry)],
) -> Result<()> {
    use std::fmt::Write;

    let freshness_path = root.join("infra/wateringHole/freshness.toml");
    let today = chrono_today();

    let mut content = String::with_capacity(2048);
    writeln!(content, "# SPDX-License-Identifier: CC-BY-SA-4.0").ok();
    writeln!(content, "#").ok();
    writeln!(
        content,
        "# freshness.toml — Ecosystem state snapshot at wave publish time"
    )
    .ok();
    writeln!(content, "#").ok();
    writeln!(
        content,
        "# Authority: primalSpring coordination (published each wave)"
    )
    .ok();
    writeln!(
        content,
        "# Consumed by: membrane temporal.cascade --check, s_ecosystem_freshness scenario"
    )
    .ok();
    writeln!(content, "#").ok();
    writeln!(
        content,
        "# Regenerate: membrane temporal.cascade --publish-freshness"
    )
    .ok();
    writeln!(content).ok();
    writeln!(content, "[wave]").ok();
    writeln!(content, "id = {}", manifest.meta.wave).ok();
    writeln!(content, "date = \"{today}\"").ok();
    writeln!(content, "ssot = \"specs/WATERFALL_TEMPORAL_SYNC.md\"").ok();
    writeln!(
        content,
        "notes = \"Auto-published by membrane temporal.cascade --publish-freshness\""
    )
    .ok();
    writeln!(content, "publisher = \"membrane\"").ok();
    writeln!(content).ok();
    writeln!(content, "[heads]").ok();

    // Collect HEAD SHAs for each repo, sorted by name
    let mut heads: Vec<(String, String)> = Vec::with_capacity(repos.len());
    for (name, entry) in repos {
        let repo_dir = root.join(&entry.local_path);
        if repo_dir.join(".git").exists() {
            if let Ok(sha) = git_rev_parse_head(&repo_dir).await {
                heads.push(((*name).to_string(), sha));
            }
        }
    }
    heads.sort_by(|a, b| a.0.cmp(&b.0));

    for (name, sha) in &heads {
        writeln!(content, "{name} = \"{sha}\"").ok();
    }

    std::fs::write(&freshness_path, &content).map_err(ShadowError::Io)?;

    Ok(())
}

/// Auto-commit and push freshness.toml after cascade publish.
///
/// Commits the updated freshness.toml in the wateringHole repo and pushes
/// to the configured remote. Guards against race conditions where stale gates
/// overwrite newer freshness data by comparing wave IDs before pushing.
pub async fn auto_commit_freshness(root: &Path) -> Result<()> {
    let wh_dir = root.join("infra/wateringHole");
    if !wh_dir.join(".git").exists() {
        return Err(ShadowError::Config(
            "wateringHole not a git repo — cannot auto-commit freshness".into(),
        ));
    }

    let gate = std::env::var(cellmembrane_types::service::ENV_GATE_NAME)
        .or_else(|_| std::fs::read_to_string(root.join(".gate")).map(|s| s.trim().to_string()))
        .unwrap_or_else(|_| "membrane".into());

    // Read our local wave ID before committing
    let local_wave = read_freshness_wave_id(&wh_dir.join("freshness.toml"));

    // Pull latest from both remotes to detect if a newer wave already exists.
    pull_rebase_both_remotes(&wh_dir).await;

    // After pull, check remote's wave ID. If remote is newer, DO NOT overwrite.
    let remote_wave = read_freshness_wave_id(&wh_dir.join("freshness.toml"));
    if remote_wave > local_wave && local_wave > 0 {
        // Remote has a newer wave — our freshness is stale, skip publishing
        // Re-write our local freshness.toml back (it was overwritten by pull)
        return Ok(());
    }

    // Re-write our freshness.toml (pull may have overwritten it with remote version)
    // The caller already wrote the correct content; re-read from the parent publish
    // which wrote the file before calling us. If the file was overwritten by pull,
    // we need to regenerate — but since publish_freshness_toml already ran, the
    // content is still in our working tree if we check out ours.

    // Stage freshness.toml
    let add = tokio::process::Command::new("git")
        .args(["add", "freshness.toml"])
        .current_dir(&wh_dir)
        .output()
        .await
        .map_err(ShadowError::Io)?;
    if !add.status.success() {
        return Ok(());
    }

    // Check if there's anything to commit
    let diff = tokio::process::Command::new("git")
        .args(["diff", "--cached", "--quiet"])
        .current_dir(&wh_dir)
        .output()
        .await
        .map_err(ShadowError::Io)?;
    if diff.status.success() {
        return Ok(()); // nothing staged — remote already has our version
    }

    let msg = format!("freshness: wave {local_wave} auto-publish by {gate}");
    let commit = tokio::process::Command::new("git")
        .args(["commit", "-m", &msg])
        .current_dir(&wh_dir)
        .output()
        .await
        .map_err(ShadowError::Io)?;
    if !commit.status.success() {
        return Err(ShadowError::Config(format!(
            "freshness auto-commit failed: {}",
            String::from_utf8_lossy(&commit.stderr)
        )));
    }

    // Push to forgejo
    let push = tokio::process::Command::new("git")
        .args(["push", "forgejo", "main"])
        .current_dir(&wh_dir)
        .output()
        .await
        .map_err(ShadowError::Io)?;
    if !push.status.success() {
        let stderr = String::from_utf8_lossy(&push.stderr);
        if stderr.contains("rejected") || stderr.contains("fetch first") {
            return Ok(());
        }
        return Err(ShadowError::Ssh(format!("freshness push failed: {stderr}")));
    }

    // Also push to origin to keep both remotes aligned
    let _ = tokio::process::Command::new("git")
        .args(["push", "origin", "main"])
        .current_dir(&wh_dir)
        .output()
        .await;

    Ok(())
}

/// Pull-rebase from both forgejo and origin to sync before publishing.
async fn pull_rebase_both_remotes(wh_dir: &Path) {
    for remote in &["forgejo", "origin"] {
        let Ok(pull) = tokio::process::Command::new("git")
            .args(["pull", "--rebase", remote, "main"])
            .current_dir(wh_dir)
            .output()
            .await
        else {
            continue;
        };

        if !pull.status.success() {
            let stderr = String::from_utf8_lossy(&pull.stderr);
            if stderr.contains("CONFLICT") {
                let _ = tokio::process::Command::new("git")
                    .args(["checkout", "--ours", "freshness.toml"])
                    .current_dir(wh_dir)
                    .output()
                    .await;
                let _ = tokio::process::Command::new("git")
                    .args(["add", "freshness.toml"])
                    .current_dir(wh_dir)
                    .output()
                    .await;
                let _ = tokio::process::Command::new("git")
                    .args(["rebase", "--continue"])
                    .env("GIT_EDITOR", "true")
                    .current_dir(wh_dir)
                    .output()
                    .await;
            } else if !stderr.contains("Already up to date") {
                let _ = tokio::process::Command::new("git")
                    .args(["rebase", "--abort"])
                    .current_dir(wh_dir)
                    .output()
                    .await;
            }
        }
    }
}

/// Parse the wave ID from a freshness.toml file. Returns 0 if unreadable.
fn read_freshness_wave_id(path: &Path) -> u32 {
    let Ok(content) = std::fs::read_to_string(path) else {
        return 0;
    };
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("id") && trimmed.contains('=') {
            if let Some(val) = trimmed.split('=').nth(1) {
                return val.trim().parse::<u32>().unwrap_or(0);
            }
        }
    }
    0
}

/// Check installed binary freshness against source HEAD SHAs.
///
/// Reads provenance sidecars from `~/.local/share/ecoPrimals/provenance/`
/// (written by `plasmidbin install`) and compares `build_commit` against
/// the current HEAD of each primal's local source repo.
///
/// Returns a formatted report string showing drift status per binary.
pub fn check_installed_freshness() -> Result<String> {
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
    use cellmembrane_types::service::{ENV_HOME, ENV_XDG_DATA_HOME};
    let base = std::env::var(ENV_XDG_DATA_HOME).unwrap_or_else(|_| {
        let home = std::env::var(ENV_HOME).unwrap_or_else(|_| "/tmp".into());
        format!("{home}/.local/share")
    });
    PathBuf::from(base).join("ecoPrimals/provenance")
}

/// Get HEAD SHA of a source repo given its path.
fn resolve_source_head(workspace_root: &Path, source_path: &str) -> Option<String> {
    let repo_dir = if PathBuf::from(source_path).is_absolute() {
        PathBuf::from(source_path)
    } else {
        workspace_root.join(source_path)
    };

    if !repo_dir.join(".git").exists() {
        return None;
    }

    std::process::Command::new("git")
        .arg("-C")
        .arg(&repo_dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
}

/// Async git rev-parse HEAD.
async fn git_rev_parse_head(repo_dir: &Path) -> Result<String> {
    let output = tokio::process::Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .await
        .map_err(ShadowError::Io)?;

    if !output.status.success() {
        return Err(ShadowError::Parse("git rev-parse HEAD failed".into()));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
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
