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

    std::fs::write(&freshness_path, &content)
        .map_err(|e| ShadowError::Parse(format!("failed to write freshness.toml: {e}")))?;

    Ok(())
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

        let status = match &source_head {
            Some(head) if head == &prov.build_commit => {
                fresh += 1;
                "FRESH"
            }
            Some(head) => {
                stale += 1;
                &format!(
                    "STALE (installed={}, HEAD={})",
                    &prov.build_commit[..8.min(prov.build_commit.len())],
                    &head[..8.min(head.len())]
                )
            }
            None => {
                unknown += 1;
                "UNKNOWN (source not found)"
            }
        };

        writeln!(report, "  {primal_name:<20} {status}").ok();
    }

    writeln!(report, "\nfresh={fresh} stale={stale} unknown={unknown}").ok();
    Ok(report)
}

// ── Helpers ──────────────────────────────────────────────────────────

/// Get today's date as YYYY-MM-DD (no chrono dependency — uses system time).
fn chrono_today() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = secs / 86400;
    let mut y = 1970i32;
    let mut remaining = i64::from(u32::try_from(days).unwrap_or(u32::MAX));
    loop {
        let days_in_year: i64 = if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
            366
        } else {
            365
        };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        y += 1;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let month_days: [i64; 12] = if leap {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut m = 0u32;
    for &md in &month_days {
        if remaining < md {
            break;
        }
        remaining -= md;
        m += 1;
    }
    format!("{y}-{:02}-{:02}", m + 1, remaining + 1)
}

/// Provenance sidecar written by `plasmidbin install`.
#[derive(Debug, serde::Deserialize)]
struct ProvenanceSidecar {
    build_commit: String,
    source_path: String,
    #[allow(dead_code)]
    installed_at: Option<String>,
    #[allow(dead_code)]
    binary_blake3: Option<String>,
}

/// Standard provenance directory (XDG data dir).
fn dirs_provenance() -> PathBuf {
    let base = std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
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
