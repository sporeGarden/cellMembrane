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
mod nucleus_restart;
pub(crate) mod post_sync;
mod resolve;
mod sync_engine;
pub mod types;

pub use cascade::{CascadeMode, CascadeOpts, PostSyncPhase, cascade_with_opts};
pub use types::*;

use crate::error::Result;
use cellmembrane_types::PushTarget;
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

    if !git_ok(&local_path, &["fetch", "--all", "--quiet"]).await {
        tracing::warn!(repo = %local_path.display(), "git fetch --all failed");
    }

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
    let refs: Vec<String> = positions
        .iter()
        .map(|p| format!("{}/{branch}", p.remote))
        .collect();

    let mut count = 0u32;
    for (i, ref_a) in refs.iter().enumerate() {
        let mut is_ahead_of_any = false;
        for (j, ref_b) in refs.iter().enumerate() {
            if i == j {
                continue;
            }
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

/// Temporal sync respecting the manifest's `push_target` setting.
pub async fn sync_with_target(
    workspace_root: &Path,
    repo_path: &str,
    push_target: PushTarget,
) -> Result<TemporalSyncResult> {
    sync_with_policy(workspace_root, repo_path, push_target, None).await
}

/// Temporal sync with optional manifest for divergence policy resolution.
pub async fn sync_with_policy(
    workspace_root: &Path,
    repo_path: &str,
    push_target: PushTarget,
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
            sync_engine::sync_converge(&local_path, repo_path, &matrix, push_target).await
        }
        SyncClassification::Diverge => {
            sync_engine::sync_diverge(
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

/// Resolve workspace root. Delegates to [`crate::resolve_workspace_root`].
pub(crate) fn resolve_workspace_root() -> Result<PathBuf> {
    crate::resolve_workspace_root()
}

/// Re-export freshness tracking functions (for backward compat from dispatch).
pub(crate) use crate::freshness::check_installed_freshness;

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
        assert!(sync_engine::REGENERABLE_METADATA.contains(&"checksums.toml"));
        assert!(sync_engine::REGENERABLE_METADATA.contains(&"provenance.toml"));
        assert!(sync_engine::REGENERABLE_METADATA.contains(&"freshness.toml"));
        assert!(sync_engine::REGENERABLE_METADATA.contains(&"signatures.toml"));
        assert_eq!(sync_engine::REGENERABLE_METADATA.len(), 4);
    }
}
