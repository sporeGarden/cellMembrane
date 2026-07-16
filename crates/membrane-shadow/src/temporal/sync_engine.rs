// SPDX-License-Identifier: AGPL-3.0-or-later

//! Temporal sync engine — converge/diverge resolution with graduated pull fallbacks.
//!
//! Implements the inner sync logic: dirty-worktree detection, ff-only/stash/rebase
//! fallback chains, tree-parity resolution, and divergence policy application.

use std::path::Path;

use tracing::info;

use super::resolve;
use super::types::{SyncAction, TemporalMatrix, TemporalSyncResult};
use crate::error::Result;
use crate::git_ops::{git_output as git, git_success as git_ok};
use cellmembrane_types::PushTarget;

/// Files that are machine-generated and safely discardable before a pull.
pub(super) const REGENERABLE_METADATA: &[&str] = &[
    cellmembrane_types::service::CHECKSUMS_FILE,
    cellmembrane_types::service::PROVENANCE_FILE,
    cellmembrane_types::service::FRESHNESS_FILE,
    cellmembrane_types::service::SIGNATURES_FILE,
];

/// Check if a repository has uncommitted changes that would block a pull.
pub(super) async fn has_dirty_worktree(local_path: &Path) -> bool {
    let output = git(local_path, &["status", "--porcelain"])
        .await
        .unwrap_or_default();
    output.lines().any(is_porcelain_dirty)
}

/// Discard local modifications to regenerable metadata files before pulling.
///
/// Returns the list of files that were discarded, so the caller can report it.
pub(super) async fn discard_regenerable_dirty_files(local_path: &Path) -> Vec<String> {
    let status_output = git(local_path, &["status", "--porcelain"])
        .await
        .unwrap_or_default();
    let mut discarded = Vec::new();

    for line in status_output.lines() {
        let trimmed = line.trim_end();
        if trimmed.len() < 4 {
            continue;
        }
        let xy = &trimmed[..2];
        let file = trimmed[3..].trim();

        let dominated_by_metadata = REGENERABLE_METADATA.iter().any(|m| file.ends_with(m));
        if !dominated_by_metadata {
            continue;
        }

        if is_discardable_xy(xy) && git_ok(local_path, &["checkout", "--", file]).await {
            discarded.push(file.to_string());
        }
    }

    discarded
}

pub(super) async fn sync_converge(
    local_path: &Path,
    repo_path: &str,
    matrix: &TemporalMatrix,
    push_target: PushTarget,
) -> Result<TemporalSyncResult> {
    let branch = &matrix.branch;
    let mut pulled_from = None;

    match &matrix.action {
        SyncAction::Pull { leader } => {
            match try_pull_converge(local_path, repo_path, leader, branch).await {
                PullOutcome::Ok(l) => pulled_from = Some(l),
                PullOutcome::Err(summary) => {
                    return Ok(TemporalSyncResult {
                        repo_path: repo_path.to_string(),
                        ok: false,
                        summary,
                        pulled_from: None,
                        pushed_to: vec![],
                    });
                }
            }
        }
        SyncAction::Push | SyncAction::None => {}
        SyncAction::Flag | SyncAction::TreeParity { .. } => {
            return Ok(TemporalSyncResult {
                repo_path: repo_path.to_string(),
                ok: false,
                summary: "unexpected Flag/TreeParity action on Converge classification".to_string(),
                pulled_from: None,
                pushed_to: vec![],
            });
        }
    }

    let pushed_to = resolve::push_converge_followers(
        local_path,
        &matrix.positions,
        pulled_from.as_deref(),
        push_target,
        branch,
    )
    .await;

    let summary = match (&pulled_from, pushed_to.is_empty()) {
        (Some(l), false) => format!("pull {l}, push {}", pushed_to.join(" ")),
        (Some(l), true) => format!("pull {l}"),
        (None, false) => format!("push {}", pushed_to.join(" ")),
        (None, true) => "parity".to_string(),
    };

    Ok(TemporalSyncResult {
        repo_path: repo_path.to_string(),
        ok: true,
        summary,
        pulled_from,
        pushed_to,
    })
}

enum PullOutcome {
    Ok(String),
    Err(String),
}

/// Attempt pull with graduated fallbacks: ff-only -> stash+pull -> rebase.
async fn try_pull_converge(
    local_path: &Path,
    repo_path: &str,
    leader: &str,
    branch: &str,
) -> PullOutcome {
    let discarded = discard_regenerable_dirty_files(local_path).await;

    if git_ok(
        local_path,
        &["pull", leader, branch, "--ff-only", "--quiet"],
    )
    .await
    {
        if !discarded.is_empty() {
            info!(
                repo = repo_path,
                count = discarded.len(),
                files = %discarded.join(", "),
                "auto-discarded regenerable file(s)"
            );
        }
        return PullOutcome::Ok(leader.to_string());
    }

    if has_dirty_worktree(local_path).await {
        let stashed = git_ok(
            local_path,
            &["stash", "push", "-m", "temporal-cascade-auto"],
        )
        .await;
        if !stashed {
            return PullOutcome::Err(format!("pull {leader} blocked (stash failed)"));
        }
        info!(repo = repo_path, "auto-stashed dirty worktree");
        let pull_ok = git_ok(
            local_path,
            &["pull", leader, branch, "--ff-only", "--quiet"],
        )
        .await
            || git_ok(local_path, &["pull", "--rebase", leader, branch, "--quiet"]).await;
        if !git_ok(local_path, &["stash", "pop", "--quiet"]).await {
            tracing::debug!(repo = repo_path, "stash pop failed (may be empty)");
        }
        if pull_ok {
            info!(repo = repo_path, "stash-recovery succeeded");
            return PullOutcome::Ok(leader.to_string());
        }
        if !git_ok(local_path, &["rebase", "--abort"]).await {
            tracing::debug!(repo = repo_path, "rebase abort failed (no active rebase)");
        }
        return PullOutcome::Err(format!("pull {leader} failed after stash (diverged)"));
    }

    if git_ok(local_path, &["pull", "--rebase", leader, branch, "--quiet"]).await {
        info!(repo = repo_path, "diderm rebase reconciled");
        return PullOutcome::Ok(leader.to_string());
    }
    if !git_ok(local_path, &["rebase", "--abort"]).await {
        tracing::debug!(repo = repo_path, "rebase abort cleanup failed");
    }

    // CAC: if rebase conflicted but trees are actually identical, reset to
    // remote — content identity supersedes temporal identity.
    let remote_ref = format!("{leader}/{branch}");
    if crate::git_ops::trees_match(local_path, &remote_ref).await {
        info!(
            repo = repo_path,
            "CAC: trees identical despite divergent history — resetting to {leader}"
        );
        if git_ok(local_path, &["reset", "--hard", &remote_ref]).await {
            return PullOutcome::Ok(leader.to_string());
        }
    }

    PullOutcome::Err(format!(
        "pull {leader} failed (ff-only — diverged, rebase conflicted)"
    ))
}

/// Classify a single `git status --porcelain` line as genuinely dirty.
///
/// A line is dirty if it's at least 3 characters (XY + space + filename)
/// and the file is NOT one of the regenerable metadata files. Uses
/// `trim_end` to preserve the leading XY status codes.
fn is_porcelain_dirty(line: &str) -> bool {
    let trimmed = line.trim_end();
    trimmed.len() >= 3
        && !REGENERABLE_METADATA
            .iter()
            .any(|m| trimmed[3..].trim().ends_with(m))
}

/// Whether a git status XY prefix represents an unstaged/untracked change
/// suitable for auto-discard of regenerable files.
fn is_discardable_xy(xy: &str) -> bool {
    xy == " M" || xy == "MM" || xy == "??" || xy == "UU"
}

pub(super) async fn sync_diverge(
    workspace_root: &Path,
    local_path: &Path,
    repo_path: &str,
    matrix: &TemporalMatrix,
    push_target: PushTarget,
    manifest: Option<&crate::manifest::EcosystemManifest>,
) -> Result<TemporalSyncResult> {
    if let SyncAction::TreeParity { leader, followers } = &matrix.action {
        return resolve::resolve_tree_parity(
            local_path,
            repo_path,
            &matrix.branch,
            leader,
            followers,
        )
        .await;
    }

    // CAC: before firing impulse or applying policy, check if local tree
    // matches any remote tree. Content identity supersedes temporal identity.
    if let Some(result) = try_local_tree_parity(local_path, repo_path, matrix).await {
        return Ok(result);
    }

    let repo_path_owned = repo_path.to_string();

    if let Some(m) = manifest {
        let entry = m.repos.values().find(|e| e.local_path == repo_path);
        let policy = entry.map_or_else(|| m.sync.divergence_policy, |e| m.divergence_policy_for(e));
        let positions: Vec<(String, u32, u32)> = matrix
            .positions
            .iter()
            .map(|p| (p.remote.clone(), p.ahead, p.behind))
            .collect();
        let args = crate::impulse::SyncDivergeArgs {
            repo_path: repo_path_owned.clone(),
            positions,
            repo_policy: policy,
        };

        if let Err(e) = crate::impulse::post_sync_diverge(workspace_root, &args).await {
            tracing::warn!(repo = %repo_path_owned, error = %e, "sync diverge impulse failed");
        }

        let resolved =
            resolve::apply_divergence_policy(local_path, matrix, policy, push_target).await;
        if let Some(summary) = resolved {
            return Ok(TemporalSyncResult {
                repo_path: repo_path_owned,
                ok: true,
                summary,
                pulled_from: None,
                pushed_to: vec![],
            });
        }
    }

    Ok(TemporalSyncResult {
        repo_path: repo_path_owned,
        ok: false,
        summary: format!("DIVERGE — {matrix}"),
        pulled_from: None,
        pushed_to: vec![],
    })
}

/// CAC: check if local HEAD tree matches any remote's tree. When content
/// is identical but history diverges (Newton-Leibniz), reset to the
/// remote and push to align history without losing content.
async fn try_local_tree_parity(
    local_path: &Path,
    repo_path: &str,
    matrix: &TemporalMatrix,
) -> Option<TemporalSyncResult> {
    let branch = &matrix.branch;

    for pos in &matrix.positions {
        let remote_ref = format!("{}/{branch}", pos.remote);
        if crate::git_ops::trees_match(local_path, &remote_ref).await {
            info!(
                repo = repo_path,
                remote = %pos.remote,
                "CAC: local tree matches remote — Newton-Leibniz, resetting to remote"
            );
            if git_ok(local_path, &["reset", "--hard", &remote_ref]).await {
                return Some(TemporalSyncResult {
                    repo_path: repo_path.to_string(),
                    ok: true,
                    summary: format!("CAC tree-parity: reset to {}", pos.remote),
                    pulled_from: Some(pos.remote.clone()),
                    pushed_to: vec![],
                });
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regenerable_metadata_contains_expected_files() {
        assert!(REGENERABLE_METADATA.contains(&"checksums.toml"));
        assert!(REGENERABLE_METADATA.contains(&"provenance.toml"));
        assert!(REGENERABLE_METADATA.contains(&"freshness.toml"));
        assert!(REGENERABLE_METADATA.contains(&"signatures.toml"));
        assert_eq!(REGENERABLE_METADATA.len(), 4);
    }

    #[test]
    fn porcelain_dirty_detects_modified_source() {
        assert!(is_porcelain_dirty(" M src/main.rs"));
        assert!(is_porcelain_dirty("MM src/lib.rs"));
        assert!(is_porcelain_dirty("?? new_file.rs"));
        assert!(is_porcelain_dirty("A  added.rs"));
    }

    #[test]
    fn porcelain_dirty_ignores_regenerable_files() {
        assert!(!is_porcelain_dirty(" M checksums.toml"));
        assert!(!is_porcelain_dirty("MM provenance.toml"));
        assert!(!is_porcelain_dirty("?? freshness.toml"));
    }

    #[test]
    fn porcelain_dirty_short_lines_not_dirty() {
        assert!(!is_porcelain_dirty(""));
        assert!(!is_porcelain_dirty("  "));
        assert!(!is_porcelain_dirty("M"));
    }

    #[test]
    fn porcelain_dirty_nested_regenerable_path() {
        assert!(!is_porcelain_dirty(" M infra/plasmidBin/checksums.toml"));
        assert!(!is_porcelain_dirty(" M some/path/provenance.toml"));
    }

    #[test]
    fn porcelain_dirty_similar_but_not_regenerable() {
        assert!(is_porcelain_dirty(" M checksums.toml.bak"));
        assert!(is_porcelain_dirty(" M my_provenance.toml.old"));
        assert!(is_porcelain_dirty(" M freshness_report.toml"));
    }

    #[test]
    fn discardable_xy_recognizes_unstaged_and_untracked() {
        assert!(is_discardable_xy(" M"));
        assert!(is_discardable_xy("MM"));
        assert!(is_discardable_xy("??"));
        assert!(is_discardable_xy("UU"));
    }

    #[test]
    fn discardable_xy_rejects_staged_only() {
        assert!(!is_discardable_xy("A "));
        assert!(!is_discardable_xy("D "));
        assert!(!is_discardable_xy("R "));
        assert!(!is_discardable_xy("M "));
    }

    #[test]
    fn discardable_xy_rejects_deleted() {
        assert!(!is_discardable_xy(" D"));
        assert!(!is_discardable_xy("MD"));
    }
}
