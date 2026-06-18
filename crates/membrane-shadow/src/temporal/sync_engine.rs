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

/// Files that are machine-generated and safely discardable before a pull.
pub(super) const REGENERABLE_METADATA: &[&str] =
    &["checksums.toml", "provenance.toml", "freshness.toml"];

/// Check if a repository has uncommitted changes that would block a pull.
pub(super) async fn has_dirty_worktree(local_path: &Path) -> bool {
    let output = git(local_path, &["status", "--porcelain"])
        .await
        .unwrap_or_default();
    output.lines().any(|l| {
        let trimmed = l.trim();
        trimmed.len() >= 3
            && !REGENERABLE_METADATA
                .iter()
                .any(|m| trimmed[3..].trim().ends_with(m))
    })
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
        let trimmed = line.trim();
        if trimmed.len() < 4 {
            continue;
        }
        let xy = &trimmed[..2];
        let file = trimmed[3..].trim();

        let dominated_by_metadata = REGENERABLE_METADATA.iter().any(|m| file.ends_with(m));
        if !dominated_by_metadata {
            continue;
        }

        let is_unstaged_modify = xy == " M" || xy == "MM" || xy == "??" || xy == "UU";
        if is_unstaged_modify && git_ok(local_path, &["checkout", "--", file]).await {
            discarded.push(file.to_string());
        }
    }

    discarded
}

pub(super) async fn sync_converge(
    local_path: &Path,
    repo_path: &str,
    matrix: &TemporalMatrix,
    push_target: &str,
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
    PullOutcome::Err(format!(
        "pull {leader} failed (ff-only — diverged, rebase conflicted)"
    ))
}

pub(super) async fn sync_diverge(
    workspace_root: &Path,
    local_path: &Path,
    repo_path: &str,
    matrix: &TemporalMatrix,
    push_target: &str,
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

    let repo_path_owned = repo_path.to_string();

    if let Some(m) = manifest {
        let entry = m.repos.values().find(|e| e.local_path == repo_path);
        let policy = entry.map_or_else(
            || m.sync.divergence_policy.as_str(),
            |e| m.divergence_policy_for(e),
        );
        let positions: Vec<(String, u32, u32)> = matrix
            .positions
            .iter()
            .map(|p| (p.remote.clone(), p.ahead, p.behind))
            .collect();
        let args = crate::impulse::SyncDivergeArgs {
            repo_path: repo_path_owned.clone(),
            positions,
            repo_policy: policy.to_string(),
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
