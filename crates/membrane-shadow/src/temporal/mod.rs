// SPDX-License-Identifier: AGPL-3.0-or-later

//! Temporal sync — waterFall DAG-based multi-remote convergence.
//!
//! Replaces the bash `temporal_check_repo` / `temporal_sync_repo` functions
//! with typed Rust operations. Runs locally — no SSH required. The DAG
//! (commit graph) is the only clock: whichever remote is furthest ahead
//! leads, followers are pushed to parity.
//!
//! Shadow domain mapping:
//!   - `temporal.check` → quorumSignal (qS): sense remote positions
//!   - `temporal.sync`  → waterFall (wF): pull leader, push followers

mod cascade;
pub mod types;

pub use cascade::cascade;
pub use types::*;

use crate::error::Result;
use std::path::{Path, PathBuf};

// ── Git helpers (delegated to git_ops) ───────────────────────────────

use crate::git_ops::{git_output as git, git_success as git_ok, rev_list_count};

/// Detect tree-parity: history diverges but tree content is identical.
///
/// Picks the remote with the most commits as the canonical leader (preserves
/// richer history). Returns `Some((leader, followers))` if all remote trees
/// produce an empty `git diff`, meaning the working trees are identical.
async fn detect_tree_parity(
    local_path: &Path,
    positions: &[RemotePosition],
    branch: &str,
) -> Option<(String, Vec<String>)> {
    if positions.len() < 2 {
        return None;
    }

    // Leader = remote with most total commits (behind + ahead gives a rough measure
    // of "richest history"; in practice, the one with behind=0 is the true leader).
    let leader = positions
        .iter()
        .min_by_key(|p| p.behind)
        .map(|p| p.remote.clone())?;

    let leader_ref = format!("{leader}/{branch}");
    let mut followers = Vec::new();

    for pos in positions {
        if pos.remote == leader {
            continue;
        }
        let follower_ref = format!("{}/{branch}", pos.remote);
        let diff_output = git(local_path, &["diff", "--stat", &leader_ref, &follower_ref])
            .await
            .unwrap_or_default();
        if diff_output.trim().is_empty() {
            followers.push(pos.remote.clone());
        } else {
            // Trees actually differ — this is a real divergence, not a rebase artifact
            return None;
        }
    }

    if followers.is_empty() {
        return None;
    }

    Some((leader, followers))
}

// ── Public API ───────────────────────────────────────────────────────

/// Classify a single repo's temporal position across all remotes.
///
/// Shadow for: `quorumSignal temporal.check`
///
/// Fetches all remotes, measures ahead/behind per remote relative to
/// local HEAD, and classifies as `Parity`, `Converge`, or `Diverge`.
#[allow(clippy::too_many_lines)]
pub async fn check(workspace_root: &Path, repo_path: &str) -> Result<TemporalMatrix> {
    let local_path = workspace_root.join(repo_path);

    if !local_path.join(".git").exists() {
        return Ok(TemporalMatrix {
            repo_path: repo_path.to_string(),
            branch: String::new(),
            classification: SyncClassification::Missing,
            positions: vec![],
            action: SyncAction::None,
        });
    }

    let branch = git(&local_path, &["rev-parse", "--abbrev-ref", "HEAD"])
        .await
        .unwrap_or_else(|_| "main".to_string());

    // Fetch all remotes quietly
    let _ = git_ok(&local_path, &["fetch", "--all", "--quiet"]).await;

    let remotes_str = git(&local_path, &["remote"]).await?;
    let remotes: Vec<&str> = remotes_str.lines().filter(|l| !l.is_empty()).collect();

    if remotes.is_empty() {
        return Ok(TemporalMatrix {
            repo_path: repo_path.to_string(),
            branch,
            classification: SyncClassification::NoRemote,
            positions: vec![],
            action: SyncAction::None,
        });
    }

    // Measure per-remote position
    let mut positions = Vec::with_capacity(remotes.len());
    let mut has_leader = false;
    let mut leader_remote = String::new();
    let mut leader_behind: u32 = 0;
    let mut has_followers = false;

    for remote in &remotes {
        let remote_ref = format!("{remote}/{branch}");
        if !git_ok(&local_path, &["rev-parse", &remote_ref]).await {
            continue;
        }

        let ahead_range = format!("{remote_ref}..HEAD");
        let behind_range = format!("HEAD..{remote_ref}");

        let ahead = rev_list_count(&local_path, &ahead_range).await;
        let behind = rev_list_count(&local_path, &behind_range).await;

        positions.push(RemotePosition {
            remote: (*remote).to_string(),
            ahead,
            behind,
        });

        if behind > 0 && behind > leader_behind {
            leader_behind = behind;
            leader_remote = (*remote).to_string();
            has_leader = true;
        }
        if ahead > 0 {
            has_followers = true;
        }
    }

    let all_parity = positions.iter().all(RemotePosition::is_parity);
    if all_parity {
        return Ok(TemporalMatrix {
            repo_path: repo_path.to_string(),
            branch,
            classification: SyncClassification::Parity,
            positions,
            action: SyncAction::None,
        });
    }

    // Divergence: check if multiple remotes have unique commits relative to each other
    let mut diverge_count = 0u32;
    for pos_a in &positions {
        let ref_a = format!("{}/{branch}", pos_a.remote);
        let mut is_ahead_of_any = false;
        for pos_b in &positions {
            if pos_a.remote == pos_b.remote {
                continue;
            }
            let ref_b = format!("{}/{branch}", pos_b.remote);
            let cross_range = format!("{ref_b}..{ref_a}");
            let cross = rev_list_count(&local_path, &cross_range).await;
            if cross > 0 {
                is_ahead_of_any = true;
                break;
            }
        }
        if is_ahead_of_any {
            diverge_count += 1;
        }
    }

    let (classification, action) = if diverge_count > 1 {
        // Check if trees are actually identical (rebase artifact — history diverges
        // but content is the same). Use `git diff --stat` between the two most common
        // remotes — if empty, trees are at parity and we can safely force-align.
        let tree_parity = detect_tree_parity(&local_path, &positions, &branch).await;
        if let Some((leader, followers)) = tree_parity {
            (
                SyncClassification::Diverge,
                SyncAction::TreeParity { leader, followers },
            )
        } else {
            (SyncClassification::Diverge, SyncAction::Flag)
        }
    } else if has_leader {
        (
            SyncClassification::Converge,
            SyncAction::Pull {
                leader: leader_remote,
            },
        )
    } else if has_followers {
        (SyncClassification::Converge, SyncAction::Push)
    } else {
        (SyncClassification::Parity, SyncAction::None)
    };

    Ok(TemporalMatrix {
        repo_path: repo_path.to_string(),
        branch,
        classification,
        positions,
        action,
    })
}

/// Execute temporal sync on a single repo: pull from leader, push to followers.
///
/// Shadow for: `waterFall temporal.sync`
///
/// `push_target`: `"all"` pushes to every follower remote (legacy),
/// `"forgejo"` pushes only to the forgejo remote (VPS mediator model).
///
/// Returns `Err` only on infrastructure failures. Divergence is reported
/// as an `Ok` result with `ok: false` — the DAG is never force-mutated.
pub async fn sync(workspace_root: &Path, repo_path: &str) -> Result<TemporalSyncResult> {
    sync_with_target(workspace_root, repo_path, "all").await
}

/// Temporal sync respecting the manifest's `push_target` setting.
pub async fn sync_with_target(
    workspace_root: &Path,
    repo_path: &str,
    push_target: &str,
) -> Result<TemporalSyncResult> {
    sync_with_policy(workspace_root, repo_path, push_target, None).await
}

/// Temporal sync with optional manifest for divergence policy resolution.
#[allow(clippy::too_many_lines)]
pub async fn sync_with_policy(
    workspace_root: &Path,
    repo_path: &str,
    push_target: &str,
    manifest: Option<&crate::manifest::EcosystemManifest>,
) -> Result<TemporalSyncResult> {
    let local_path = workspace_root.join(repo_path);

    if !local_path.join(".git").exists() {
        return Ok(TemporalSyncResult {
            repo_path: repo_path.to_string(),
            ok: false,
            summary: "not cloned".to_string(),
            pulled_from: None,
            pushed_to: vec![],
        });
    }

    let matrix = check(workspace_root, repo_path).await?;

    match matrix.classification {
        SyncClassification::Parity => Ok(TemporalSyncResult {
            repo_path: repo_path.to_string(),
            ok: true,
            summary: "parity".to_string(),
            pulled_from: None,
            pushed_to: vec![],
        }),

        SyncClassification::Converge => {
            let branch = &matrix.branch;
            let mut pulled_from = None;
            let mut pushed_to = Vec::new();

            match &matrix.action {
                SyncAction::Pull { leader } => {
                    if git_ok(
                        &local_path,
                        &["pull", leader, branch, "--ff-only", "--quiet"],
                    )
                    .await
                    {
                        pulled_from = Some(leader.clone());
                    } else {
                        return Ok(TemporalSyncResult {
                            repo_path: repo_path.to_string(),
                            ok: false,
                            summary: format!("pull {leader} failed (ff-only)"),
                            pulled_from: None,
                            pushed_to: vec![],
                        });
                    }
                }
                SyncAction::Push | SyncAction::None => {}
                SyncAction::Flag | SyncAction::TreeParity { .. } => {
                    return Ok(TemporalSyncResult {
                        repo_path: repo_path.to_string(),
                        ok: false,
                        summary: "unexpected Flag/TreeParity action on Converge classification"
                            .to_string(),
                        pulled_from: None,
                        pushed_to: vec![],
                    });
                }
            }

            // Push to follower remotes, filtered by push_target.
            // "forgejo" = only push to the forgejo remote (VPS mediator handles GitHub).
            // "all" = push to every remote that is behind (legacy dual-push).
            for pos in &matrix.positions {
                if pulled_from.as_deref() == Some(&pos.remote) {
                    continue;
                }
                if push_target == "forgejo" && pos.remote != "forgejo" {
                    continue;
                }
                let remote_ref = format!("{}/{branch}", pos.remote);
                let ahead_range = format!("{remote_ref}..HEAD");
                let ahead = rev_list_count(&local_path, &ahead_range).await;
                if ahead > 0 && git_ok(&local_path, &["push", &pos.remote, branch, "--quiet"]).await
                {
                    pushed_to.push(pos.remote.clone());
                }
            }

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

        SyncClassification::Diverge => {
            if let SyncAction::TreeParity { leader, followers } = &matrix.action {
                let branch = &matrix.branch;
                // Reset local to leader ref, then force-push followers
                if !git_ok(
                    &local_path,
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
                    if git_ok(
                        &local_path,
                        &["push", "--force-with-lease", follower, branch],
                    )
                    .await
                    {
                        pushed_to.push(follower.clone());
                    }
                }

                Ok(TemporalSyncResult {
                    repo_path: repo_path.to_string(),
                    ok: true,
                    summary: format!("tree-parity resolved: {leader} → {}", pushed_to.join(", ")),
                    pulled_from: Some(leader.clone()),
                    pushed_to,
                })
            } else {
                // Real divergence — apply policy if manifest is available
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
                        repo_path: repo_path.to_string(),
                        positions,
                        repo_policy: policy.to_string(),
                    };

                    // Fire divergence impulse for visibility
                    let _ = crate::impulse::post_sync_diverge(workspace_root, &args).await;

                    // Apply graduated merge strategy based on policy
                    let resolved =
                        apply_divergence_policy(&local_path, &matrix, policy, push_target).await;

                    if let Some(summary) = resolved {
                        return Ok(TemporalSyncResult {
                            repo_path: repo_path.to_string(),
                            ok: true,
                            summary,
                            pulled_from: None,
                            pushed_to: vec![],
                        });
                    }
                }

                Ok(TemporalSyncResult {
                    repo_path: repo_path.to_string(),
                    ok: false,
                    summary: format!("DIVERGE — {matrix}"),
                    pulled_from: None,
                    pushed_to: vec![],
                })
            }
        }

        SyncClassification::Missing | SyncClassification::NoRemote => Ok(TemporalSyncResult {
            repo_path: repo_path.to_string(),
            ok: false,
            summary: matrix.classification.to_string(),
            pulled_from: None,
            pushed_to: vec![],
        }),
    }
}

/// Check temporal position for multiple repos, returning an aggregate report.
pub async fn check_all(workspace_root: &Path, repo_paths: &[&str]) -> Result<TemporalReport> {
    let mut report = TemporalReport {
        total: repo_paths.len() as u32,
        parity: 0,
        converged: 0,
        diverged: 0,
        missing: 0,
        repos: Vec::with_capacity(repo_paths.len()),
    };

    for path in repo_paths {
        let matrix = check(workspace_root, path).await?;
        match matrix.classification {
            SyncClassification::Parity => report.parity += 1,
            SyncClassification::Converge => report.converged += 1,
            SyncClassification::Diverge => report.diverged += 1,
            SyncClassification::Missing | SyncClassification::NoRemote => report.missing += 1,
        }
        report.repos.push(matrix);
    }

    Ok(report)
}

/// Resolve workspace root. Delegates to [`crate::resolve_workspace_root`].
pub fn resolve_workspace_root() -> Result<PathBuf> {
    crate::resolve_workspace_root()
}

/// Apply graduated merge strategy based on divergence policy from manifest.
///
/// Returns `Some(summary)` if the policy was applied successfully (repo converged),
/// or `None` if the policy defers resolution (`impulse-only`, `flag`, `agentic`).
async fn apply_divergence_policy(
    local_path: &Path,
    matrix: &TemporalMatrix,
    policy: &str,
    push_target: &str,
) -> Option<String> {
    let branch = &matrix.branch;

    // Find leader (least behind) and followers
    let leader = matrix
        .positions
        .iter()
        .min_by_key(|p| p.behind)
        .map(|p| &p.remote)?;

    match policy {
        "merge-ff" => {
            // Pull from leader with --ff-only, push to followers
            if !git_ok(
                local_path,
                &["pull", leader, branch, "--ff-only", "--quiet"],
            )
            .await
            {
                return None;
            }
            let mut pushed = Vec::new();
            for pos in &matrix.positions {
                if &pos.remote == leader {
                    continue;
                }
                if push_target == "forgejo" && pos.remote != "forgejo" {
                    continue;
                }
                if git_ok(local_path, &["push", &pos.remote, branch, "--quiet"]).await {
                    pushed.push(pos.remote.clone());
                }
            }
            Some(format!(
                "policy:merge-ff pull {leader}, push {}",
                pushed.join(" ")
            ))
        }
        "merge-rebase" => {
            // Rebase onto leader, then push
            if !git_ok(local_path, &["pull", "--rebase", leader, branch, "--quiet"]).await {
                return None;
            }
            let mut pushed = Vec::new();
            for pos in &matrix.positions {
                if &pos.remote == leader {
                    continue;
                }
                if push_target == "forgejo" && pos.remote != "forgejo" {
                    continue;
                }
                if git_ok(
                    local_path,
                    &["push", "--force-with-lease", &pos.remote, branch],
                )
                .await
                {
                    pushed.push(pos.remote.clone());
                }
            }
            Some(format!(
                "policy:merge-rebase rebase {leader}, push {}",
                pushed.join(" ")
            ))
        }
        "impulse-only" | "flag" => None,
        "agentic" => {
            eprintln!(
                "temporal: agentic policy for {} — deferred (not yet wired to resolver)",
                matrix.repo_path,
            );
            None
        }
        unknown => {
            eprintln!("temporal: unknown divergence policy {unknown:?} — treating as flag");
            None
        }
    }
}

/// Re-export freshness tracking functions (for backward compat from dispatch).
pub use crate::freshness::check_installed_freshness;
