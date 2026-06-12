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

pub(super) mod cascade;
mod post_sync;
mod resolve;
pub mod types;

pub use cascade::{CascadeMode, CascadeOpts, PostSyncPhase, cascade_with_opts};
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

    let positions = measure_remote_positions(&local_path, &remotes, &branch).await;

    let (classification, action) = classify_sync_state(&local_path, &positions, &branch).await;

    Ok(TemporalMatrix {
        repo_path: repo_path.to_string(),
        branch,
        classification,
        positions,
        action,
    })
}

async fn measure_remote_positions(
    local_path: &Path,
    remotes: &[&str],
    branch: &str,
) -> Vec<RemotePosition> {
    let mut positions = Vec::with_capacity(remotes.len());

    for remote in remotes {
        let remote_ref = format!("{remote}/{branch}");
        if !git_ok(local_path, &["rev-parse", &remote_ref]).await {
            continue;
        }

        let ahead_range = format!("{remote_ref}..HEAD");
        let behind_range = format!("HEAD..{remote_ref}");

        let ahead = rev_list_count(local_path, &ahead_range).await;
        let behind = rev_list_count(local_path, &behind_range).await;

        positions.push(RemotePosition {
            remote: (*remote).to_string(),
            ahead,
            behind,
        });
    }

    positions
}

async fn classify_sync_state(
    local_path: &Path,
    positions: &[RemotePosition],
    branch: &str,
) -> (SyncClassification, SyncAction) {
    if positions.iter().all(RemotePosition::is_parity) {
        return (SyncClassification::Parity, SyncAction::None);
    }

    let (leader_remote, _) = resolve::select_leader(positions);
    let has_leader = !leader_remote.is_empty();
    let has_followers = positions.iter().any(|p| p.ahead > 0);

    let diverge_count = count_divergent_remotes(local_path, positions, branch).await;

    if diverge_count > 1 {
        let tree_parity = detect_tree_parity(local_path, positions, branch).await;
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
    }
}

/// Count how many remotes have unique commits relative to others (divergence indicator).
async fn count_divergent_remotes(
    local_path: &Path,
    positions: &[RemotePosition],
    branch: &str,
) -> u32 {
    let mut count = 0u32;
    for pos_a in positions {
        let ref_a = format!("{}/{branch}", pos_a.remote);
        let mut is_ahead_of_any = false;
        for pos_b in positions {
            if pos_a.remote == pos_b.remote {
                continue;
            }
            let ref_b = format!("{}/{branch}", pos_b.remote);
            let cross_range = format!("{ref_b}..{ref_a}");
            if rev_list_count(local_path, &cross_range).await > 0 {
                is_ahead_of_any = true;
                break;
            }
        }
        if is_ahead_of_any {
            count += 1;
        }
    }
    count
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
            sync_converge(&local_path, repo_path, &matrix, push_target).await
        }
        SyncClassification::Diverge => {
            sync_diverge(
                workspace_root,
                &local_path,
                repo_path,
                &matrix,
                push_target,
                manifest,
            )
            .await
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

/// Files that are machine-generated and safely discardable before a pull.
/// These are always regenerable from the depot itself (hashes, provenance, timestamps).
const REGENERABLE_METADATA: &[&str] = &["checksums.toml", "provenance.toml", "freshness.toml"];

/// Check if a repository has uncommitted changes that would block a pull.
async fn has_dirty_worktree(local_path: &Path) -> bool {
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
async fn discard_regenerable_dirty_files(local_path: &Path) -> Vec<String> {
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

async fn sync_converge(
    local_path: &Path,
    repo_path: &str,
    matrix: &TemporalMatrix,
    push_target: &str,
) -> Result<TemporalSyncResult> {
    let branch = &matrix.branch;
    let mut pulled_from = None;

    match &matrix.action {
        SyncAction::Pull { leader } => {
            let discarded = discard_regenerable_dirty_files(local_path).await;
            if git_ok(
                local_path,
                &["pull", leader, branch, "--ff-only", "--quiet"],
            )
            .await
            {
                pulled_from = Some(leader.clone());
                if !discarded.is_empty() {
                    eprintln!(
                        "temporal: auto-discarded {} regenerable file(s) in {repo_path}: {}",
                        discarded.len(),
                        discarded.join(", "),
                    );
                }
            } else if has_dirty_worktree(local_path).await {
                // CASCADE-STALE-RECOVERY: auto-stash, pull, pop
                let stashed = git_ok(local_path, &["stash", "push", "-m", "temporal-cascade-auto"]).await;
                if stashed {
                    eprintln!("temporal: auto-stashed dirty worktree in {repo_path}");
                    let pull_ok = git_ok(
                        local_path,
                        &["pull", leader, branch, "--ff-only", "--quiet"],
                    )
                    .await;
                    let _ = git_ok(local_path, &["stash", "pop", "--quiet"]).await;
                    if pull_ok {
                        pulled_from = Some(leader.clone());
                        eprintln!("temporal: stash-recovery succeeded for {repo_path}");
                    } else {
                        return Ok(TemporalSyncResult {
                            repo_path: repo_path.to_string(),
                            ok: false,
                            summary: format!("pull {leader} failed after stash (diverged)"),
                            pulled_from: None,
                            pushed_to: vec![],
                        });
                    }
                } else {
                    return Ok(TemporalSyncResult {
                        repo_path: repo_path.to_string(),
                        ok: false,
                        summary: format!("pull {leader} blocked (stash failed)"),
                        pulled_from: None,
                        pushed_to: vec![],
                    });
                }
            } else {
                return Ok(TemporalSyncResult {
                    repo_path: repo_path.to_string(),
                    ok: false,
                    summary: format!("pull {leader} failed (ff-only — diverged)"),
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

async fn sync_diverge(
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

        let _ = crate::impulse::post_sync_diverge(workspace_root, &args).await;

        let resolved =
            resolve::apply_divergence_policy(local_path, matrix, policy, push_target).await;
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

/// Re-export freshness tracking functions (for backward compat from dispatch).
pub use crate::freshness::check_installed_freshness;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_position_parity_detection() {
        let parity = RemotePosition {
            remote: "origin".into(),
            ahead: 0,
            behind: 0,
        };
        assert!(parity.is_parity());

        let ahead = RemotePosition {
            remote: "origin".into(),
            ahead: 3,
            behind: 0,
        };
        assert!(!ahead.is_parity());

        let behind = RemotePosition {
            remote: "forgejo".into(),
            ahead: 0,
            behind: 2,
        };
        assert!(!behind.is_parity());
    }

    #[test]
    fn temporal_matrix_display_format() {
        let matrix = TemporalMatrix {
            repo_path: "primals/bearDog".into(),
            branch: "main".into(),
            classification: SyncClassification::Parity,
            positions: vec![
                RemotePosition {
                    remote: "origin".into(),
                    ahead: 0,
                    behind: 0,
                },
                RemotePosition {
                    remote: "forgejo".into(),
                    ahead: 0,
                    behind: 0,
                },
            ],
            action: SyncAction::None,
        };
        let display = format!("{matrix:?}");
        assert!(display.contains("bearDog"));
        assert!(display.contains("Parity"));
    }

    #[test]
    fn sync_classification_variants() {
        assert_ne!(SyncClassification::Parity, SyncClassification::Converge);
        assert_ne!(SyncClassification::Diverge, SyncClassification::Missing);
        assert_ne!(SyncClassification::Missing, SyncClassification::NoRemote);
    }

    #[test]
    fn sync_action_pull_carries_leader() {
        let action = SyncAction::Pull {
            leader: "forgejo".into(),
        };
        if let SyncAction::Pull { leader } = action {
            assert_eq!(leader, "forgejo");
        } else {
            panic!("expected Pull variant");
        }
    }

    #[test]
    fn sync_action_tree_parity_carries_data() {
        let action = SyncAction::TreeParity {
            leader: "origin".into(),
            followers: vec!["forgejo".into()],
        };
        if let SyncAction::TreeParity { leader, followers } = action {
            assert_eq!(leader, "origin");
            assert_eq!(followers, vec!["forgejo"]);
        } else {
            panic!("expected TreeParity variant");
        }
    }

    #[test]
    fn regenerable_metadata_contains_expected_files() {
        assert!(REGENERABLE_METADATA.contains(&"checksums.toml"));
        assert!(REGENERABLE_METADATA.contains(&"provenance.toml"));
        assert!(REGENERABLE_METADATA.contains(&"freshness.toml"));
        assert_eq!(REGENERABLE_METADATA.len(), 3);
    }
}
