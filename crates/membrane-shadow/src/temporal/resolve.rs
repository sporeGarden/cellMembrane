// SPDX-License-Identifier: AGPL-3.0-or-later

//! Divergence resolution strategies — authority-first push ordering.
//!
//! This module contains the graduated merge/rebase/force strategies that
//! resolve multi-remote divergence. The key invariant: Forgejo (sovereign)
//! is always converged to first; mirror remotes receive `--force-with-lease`.

use std::path::Path;

use super::types::{RemotePosition, TemporalMatrix, TemporalSyncResult};
use crate::error::Result;
use crate::git_ops::{PushOutcome, git_push_classified, git_success as git_ok, rev_list_count};
use cellmembrane_types::{DivergencePolicy, PushTarget};
use tracing::{error, warn};

/// The sovereign remote name, resolved from `MEMBRANE_SOVEREIGN_REMOTE` env var
/// or defaulting to "forgejo". Authority-first push always converges to this
/// remote before pushing to mirrors.
pub(super) fn sovereign_remote() -> &'static str {
    static SOVEREIGN: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    SOVEREIGN.get_or_init(|| {
        cellmembrane_types::service::env_or(
            cellmembrane_types::service::ENV_SOVEREIGN_REMOTE,
            cellmembrane_types::service::DEFAULT_SOVEREIGN_REMOTE,
        )
    })
}

/// Apply graduated merge strategy based on divergence policy from manifest.
///
/// Returns `Some(summary)` if the policy was applied successfully (repo converged),
/// or `None` if the policy defers resolution (`impulse-only`, `flag`, `agentic`).
pub(super) async fn apply_divergence_policy(
    local_path: &Path,
    matrix: &TemporalMatrix,
    policy: DivergencePolicy,
    push_target: PushTarget,
) -> Option<String> {
    let branch = &matrix.branch;

    let leader = matrix
        .positions
        .iter()
        .min_by_key(|p| p.behind)
        .map(|p| &p.remote)?;

    match policy {
        DivergencePolicy::MergeFf => {
            if !git_ok(
                local_path,
                &["pull", leader, branch, "--ff-only", "--quiet"],
            )
            .await
            {
                return None;
            }
            let pushed =
                push_to_followers(local_path, matrix, leader, push_target, branch, false).await;
            Some(format!(
                "policy:merge-ff pull {leader}, push {}",
                pushed.join(" ")
            ))
        }
        DivergencePolicy::MergeRebase => {
            if !git_ok(local_path, &["pull", "--rebase", leader, branch, "--quiet"]).await {
                return None;
            }
            let pushed =
                push_to_followers(local_path, matrix, leader, push_target, branch, true).await;
            Some(format!(
                "policy:merge-rebase rebase {leader}, push {}",
                pushed.join(" ")
            ))
        }
        DivergencePolicy::ImpulseOnly | DivergencePolicy::Flag => None,
        DivergencePolicy::Agentic => {
            resolve_agentic(local_path, matrix, leader, push_target, branch).await
        }
    }
}

/// Agentic divergence resolver — authority-first graduated escalation.
///
/// Strategy:
///   1. If leader is not the sovereign remote, prefer converging to sovereign first
///   2. Try fast-forward from authority → try rebase from authority
///   3. Push mirrors with force-with-lease (preventing circular divergence)
///   4. Signal conflict only if convergence to authority fails
///
/// The key invariant: never rebase from a mirror remote. The sovereign (Forgejo)
/// is the single source of truth; mirrors (GitHub) may receive independent pushes
/// that should be overwritten via force-with-lease, not rebased onto.
async fn resolve_agentic(
    local_path: &Path,
    matrix: &TemporalMatrix,
    leader: &str,
    push_target: PushTarget,
    branch: &str,
) -> Option<String> {
    let sov = sovereign_remote();

    // Determine the actual convergence target: prefer sovereign even if the
    // "leader" (by commit count) is a mirror.
    let authority = if matrix
        .positions
        .iter()
        .any(|p| p.remote == sov && p.behind > 0)
    {
        sov
    } else {
        leader
    };

    // Phase 1: attempt fast-forward from authority (no data mutation risk)
    if git_ok(
        local_path,
        &["pull", authority, branch, "--ff-only", "--quiet"],
    )
    .await
    {
        let pushed =
            push_to_followers(local_path, matrix, authority, push_target, branch, false).await;
        return Some(format!(
            "policy:agentic resolved via ff from {authority}, push {}",
            pushed.join(" ")
        ));
    }

    // Phase 2: attempt rebase onto authority only (never rebase onto mirror)
    if authority == sov
        && git_ok(
            local_path,
            &["pull", "--rebase", authority, branch, "--quiet"],
        )
        .await
    {
        let pushed =
            push_to_followers(local_path, matrix, authority, push_target, branch, true).await;
        return Some(format!(
            "policy:agentic resolved via rebase from {authority}, push {}",
            pushed.join(" ")
        ));
    }

    // Rebase failed or leader is a mirror — abort any in-progress rebase state
    if !git_ok(local_path, &["rebase", "--abort"]).await {
        tracing::warn!(repo = %local_path.display(), "rebase --abort failed (may not have been in progress)");
    }

    // Phase 3: signal conflict — emit to stderr and return None for impulse handling
    error!(
        repo = %matrix.repo_path,
        authority,
        "agentic resolver CONFLICT — convergence failed"
    );
    None
}

/// Push to follower remotes with authority-first ordering.
///
/// Sovereign remote is pushed first with a normal push. Mirror remotes always
/// get `--force-with-lease` to handle their independent state without creating
/// circular divergence from relay chains or parallel teams.
pub(super) async fn push_to_followers(
    local_path: &Path,
    matrix: &TemporalMatrix,
    leader: &str,
    push_target: PushTarget,
    branch: &str,
    force_lease: bool,
) -> Vec<String> {
    let sov = sovereign_remote();
    let mut pushed = Vec::new();

    let sovereign_first = {
        let mut remotes: Vec<&RemotePosition> = matrix
            .positions
            .iter()
            .filter(|p| p.remote != leader)
            .collect();
        remotes.sort_by_key(|p| i32::from(p.remote != sov));
        remotes
    };

    for pos in sovereign_first {
        if push_target == PushTarget::Forgejo && pos.remote != "forgejo" {
            continue;
        }
        let use_force = force_lease || pos.remote != sov;
        let args: Vec<&str> = if use_force {
            vec!["push", "--force-with-lease", &pos.remote, branch]
        } else {
            vec!["push", &pos.remote, branch, "--quiet"]
        };
        match git_push_classified(local_path, &args).await {
            PushOutcome::Ok => pushed.push(pos.remote.clone()),
            PushOutcome::ShallowRejected | PushOutcome::NonFastForward => {
                warn!(
                    remote = %pos.remote,
                    repo = %local_path.display(),
                    "push rejected: shallow/non-ff — reshallow or manual intervention needed"
                );
            }
            PushOutcome::Failed => {}
        }
    }
    pushed
}

/// Resolve tree-parity divergence: content is identical but history diverges.
///
/// Resets to the leader's ref and force-pushes to followers (safe because
/// the working trees are confirmed identical via `git diff --stat`).
pub(super) async fn resolve_tree_parity(
    local_path: &Path,
    repo_path: &str,
    branch: &str,
    leader: &str,
    followers: &[String],
) -> Result<TemporalSyncResult> {
    if !git_ok(
        local_path,
        &["reset", "--hard", &format!("{leader}/{branch}")],
    )
    .await
    {
        return Ok(TemporalSyncResult {
            repo_path: repo_path.to_string(),
            ok: false,
            summary: format!("tree-parity reset to {leader} failed"),
            pulled_from: None,
            pushed_to: vec![],
        });
    }

    let mut pushed_to = Vec::new();
    for follower in followers {
        match git_push_classified(
            local_path,
            &["push", "--force-with-lease", follower, branch],
        )
        .await
        {
            PushOutcome::Ok => pushed_to.push(follower.clone()),
            PushOutcome::ShallowRejected | PushOutcome::NonFastForward => {
                warn!(
                    remote = %follower,
                    repo = %local_path.display(),
                    "tree-parity push rejected: shallow/non-ff — reshallow needed"
                );
            }
            PushOutcome::Failed => {}
        }
    }

    Ok(TemporalSyncResult {
        repo_path: repo_path.to_string(),
        ok: true,
        summary: format!("tree-parity resolved: {leader} → {}", pushed_to.join(", ")),
        pulled_from: Some(leader.to_string()),
        pushed_to,
    })
}

/// Push to follower remotes after a converge pull — authority-first ordering.
///
/// Called from `sync_converge` after pulling from a leader.
pub(super) async fn push_converge_followers(
    local_path: &Path,
    positions: &[RemotePosition],
    pulled_from: Option<&str>,
    push_target: PushTarget,
    branch: &str,
) -> Vec<String> {
    let sov = sovereign_remote();
    let mut pushed_to = Vec::new();

    let mut push_targets: Vec<&RemotePosition> = positions
        .iter()
        .filter(|p| pulled_from != Some(p.remote.as_str()))
        .collect();
    push_targets.sort_by_key(|p| i32::from(p.remote != sov));

    for pos in push_targets {
        if push_target == PushTarget::Forgejo && pos.remote != "forgejo" {
            continue;
        }
        let remote_ref = format!("{}/{branch}", pos.remote);
        let ahead_range = format!("{remote_ref}..HEAD");
        let ahead = rev_list_count(local_path, &ahead_range).await;
        if ahead == 0 {
            continue;
        }
        let args: Vec<&str> = if pos.remote == sov {
            vec!["push", &pos.remote, branch, "--quiet"]
        } else {
            vec!["push", "--force-with-lease", &pos.remote, branch]
        };
        match git_push_classified(local_path, &args).await {
            PushOutcome::Ok => pushed_to.push(pos.remote.clone()),
            PushOutcome::NonFastForward if pos.remote == sov => {
                let remote_ref = format!("{}/{branch}", pos.remote);
                if crate::git_ops::trees_match(local_path, &remote_ref).await {
                    tracing::info!(
                        remote = %pos.remote,
                        repo = %local_path.display(),
                        "tree-parity: retrying sovereign push with --force-with-lease"
                    );
                    let force_args = vec!["push", "--force-with-lease", &pos.remote, branch];
                    if git_push_classified(local_path, &force_args).await == PushOutcome::Ok {
                        pushed_to.push(pos.remote.clone());
                    }
                }
            }
            PushOutcome::ShallowRejected | PushOutcome::NonFastForward => {
                warn!(
                    remote = %pos.remote,
                    repo = %local_path.display(),
                    "push rejected: shallow/non-ff — reshallow or manual intervention needed"
                );
            }
            PushOutcome::Failed => {}
        }
    }
    pushed_to
}

/// Classify with authority-first leader selection: if the sovereign remote
/// has commits we're behind on, always prefer it as the pull leader.
pub(super) fn select_leader(positions: &[RemotePosition]) -> (String, u32) {
    let sov = sovereign_remote();

    // If sovereign has work we need, prefer it over any mirror
    if let Some(sp) = positions.iter().find(|p| p.remote == sov) {
        if sp.behind > 0 {
            return (sp.remote.clone(), sp.behind);
        }
    }

    // Fallback: remote with the most commits ahead of us
    positions
        .iter()
        .filter(|p| p.behind > 0)
        .max_by_key(|p| p.behind)
        .map_or_else(|| (String::new(), 0), |p| (p.remote.clone(), p.behind))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(remote: &str, ahead: u32, behind: u32) -> RemotePosition {
        RemotePosition {
            remote: remote.into(),
            ahead,
            behind,
        }
    }

    #[test]
    fn select_leader_prefers_sovereign_when_behind() {
        let positions = vec![pos("forgejo", 0, 5), pos("origin", 0, 10)];
        let (leader, count) = select_leader(&positions);
        assert_eq!(leader, "forgejo", "sovereign remote should be preferred");
        assert_eq!(count, 5);
    }

    #[test]
    fn select_leader_falls_back_to_most_behind_when_sovereign_parity() {
        let positions = vec![
            pos("forgejo", 0, 0),
            pos("origin", 0, 3),
            pos("github", 0, 7),
        ];
        let (leader, count) = select_leader(&positions);
        assert_eq!(leader, "github", "should pick remote with most behind");
        assert_eq!(count, 7);
    }

    #[test]
    fn select_leader_empty_positions() {
        let (leader, count) = select_leader(&[]);
        assert!(leader.is_empty());
        assert_eq!(count, 0);
    }

    #[test]
    fn select_leader_all_parity() {
        let positions = vec![pos("forgejo", 0, 0), pos("origin", 0, 0)];
        let (leader, count) = select_leader(&positions);
        assert!(leader.is_empty(), "no leader when all at parity");
        assert_eq!(count, 0);
    }

    #[test]
    fn select_leader_single_remote_behind() {
        let positions = vec![pos("origin", 0, 2)];
        let (leader, count) = select_leader(&positions);
        assert_eq!(leader, "origin");
        assert_eq!(count, 2);
    }

    #[test]
    fn select_leader_sovereign_at_parity_but_mirror_behind() {
        let positions = vec![pos("forgejo", 0, 0), pos("github", 0, 4)];
        let (leader, count) = select_leader(&positions);
        assert_eq!(leader, "github");
        assert_eq!(count, 4);
    }

    #[test]
    fn sovereign_remote_is_not_empty() {
        let sov = sovereign_remote();
        assert!(!sov.is_empty(), "sovereign remote should never be empty");
    }
}
