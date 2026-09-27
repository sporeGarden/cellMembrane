// SPDX-License-Identifier: AGPL-3.0-or-later
//! Temporal sync types — positions, classifications, matrices, and reports.

use serde::{Deserialize, Serialize};

/// Per-remote temporal position relative to local HEAD.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemotePosition {
    /// Remote name (e.g. `origin`, `forgejo`).
    pub remote: String,
    /// Commits in local HEAD not in remote (local ahead).
    pub ahead: u32,
    /// Commits in remote not in local HEAD (remote ahead).
    pub behind: u32,
}

impl RemotePosition {
    /// True when local and remote share the same tip.
    #[must_use]
    pub const fn is_parity(&self) -> bool {
        self.ahead == 0 && self.behind == 0
    }
}

impl std::fmt::Display for RemotePosition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}(+{},-{})", self.remote, self.ahead, self.behind)
    }
}

/// Classification of a repo's temporal state across all remotes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SyncClassification {
    /// All remotes match local HEAD.
    Parity,
    /// A clear leader exists — can fast-forward converge.
    Converge,
    /// Multiple remotes have divergent unique commits — needs human review.
    Diverge,
    /// Repo directory missing or not a git repository.
    Missing,
    /// No remotes configured.
    NoRemote,
}

impl std::fmt::Display for SyncClassification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parity => write!(f, "PARITY"),
            Self::Converge => write!(f, "CONVERGE"),
            Self::Diverge => write!(f, "DIVERGE"),
            Self::Missing => write!(f, "MISSING"),
            Self::NoRemote => write!(f, "NO_REMOTE"),
        }
    }
}

/// Recommended action from temporal classification.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SyncAction {
    /// Nothing to do.
    None,
    /// Pull from the named leader remote.
    Pull {
        /// Remote with the most commits ahead.
        leader: String,
    },
    /// Push to remotes that are behind local HEAD.
    Push,
    /// Trees are identical but history diverged (rebase artifact).
    /// Force-push follower to match leader — safe because content is the same.
    TreeParity {
        /// Remote whose ref to adopt as the canonical history.
        leader: String,
        /// Remote(s) to force-push to match.
        followers: Vec<String>,
    },
    /// Diverged — flag for human review, do not modify.
    Flag,
}

impl std::fmt::Display for SyncAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "ok"),
            Self::Pull { leader } => write!(f, "pull {leader}"),
            Self::Push => write!(f, "push followers"),
            Self::TreeParity { leader, followers } => {
                write!(f, "tree-parity: {leader} → {}", followers.join(", "))
            }
            Self::Flag => write!(f, "FLAG: human review"),
        }
    }
}

/// Full temporal check result for a single repo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalMatrix {
    /// Relative repo path (e.g. `primals/biomeOS`).
    pub repo_path: String,
    /// Current branch.
    pub branch: String,
    /// Classification of the convergence state.
    pub classification: SyncClassification,
    /// Per-remote position data.
    pub positions: Vec<RemotePosition>,
    /// Recommended action.
    pub action: SyncAction,
}

impl std::fmt::Display for TemporalMatrix {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let positions: Vec<String> = self.positions.iter().map(ToString::to_string).collect();
        write!(
            f,
            "{:<35} {:<9} {} -> {}",
            self.repo_path,
            self.classification.to_string(),
            positions.join(" "),
            self.action
        )
    }
}

/// Result of a temporal sync operation on a single repo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalSyncResult {
    /// Relative repo path.
    pub repo_path: String,
    /// Whether the sync succeeded.
    pub ok: bool,
    /// What happened.
    pub summary: String,
    /// Remotes that were pulled from.
    pub pulled_from: Option<String>,
    /// Remotes that were pushed to.
    pub pushed_to: Vec<String>,
}

// ── Cascade Gossip Types ────────────────────────────────────────────

/// Sync status for a single repo in a cascade notify event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CascadeSyncStatus {
    /// Repo was pulled/pushed to reach parity.
    Synced,
    /// Repo was already at parity — no action needed.
    AlreadyCurrent,
    /// Repo was freshly cloned.
    Cloned,
    /// Sync failed.
    Failed,
    /// Repo was skipped (not cloned, not in manifest).
    Skipped,
}

impl std::fmt::Display for CascadeSyncStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Synced => write!(f, "synced"),
            Self::AlreadyCurrent => write!(f, "current"),
            Self::Cloned => write!(f, "cloned"),
            Self::Failed => write!(f, "failed"),
            Self::Skipped => write!(f, "skipped"),
        }
    }
}

/// Per-repo sync result in a cascade gossip event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoSyncResult {
    /// Repo name (e.g. `cellMembrane`, `primals/songBird`).
    pub name: String,
    /// Content-addressed tree SHA (`HEAD^{tree}`) after sync.
    pub head_sha: String,
    /// What happened to this repo.
    pub status: CascadeSyncStatus,
}

/// Gossip event: a gate completed `temporal.cascade` on one or more repos.
///
/// Emitted via `mesh.publish { topic: "cascade.notify" }` after a successful
/// cascade sync. Mesh peers that are stale on any of these repos can then
/// pull autonomously — no SSH dispatch needed.
///
/// This is the **NanoWire Tier 2** unlock: `gate.pull` and `gate.check`
/// retire SSH in favor of gossip-driven convergence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CascadeNotify {
    /// Gate that completed the cascade.
    pub gate: String,
    /// Manifest wave at cascade time.
    pub wave: u32,
    /// Per-repo sync results.
    pub repos: Vec<RepoSyncResult>,
    /// Aggregate: total repos in gate profile.
    pub total: u32,
    /// Aggregate: repos synced (pulled or pushed).
    pub synced: u32,
    /// Aggregate: repos that failed.
    pub failed: u32,
    /// Aggregate: repos freshly cloned.
    pub cloned: u32,
    /// ISO 8601 timestamp of cascade completion.
    pub synced_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_position_serde_roundtrip() {
        let pos = RemotePosition {
            remote: "origin".into(),
            ahead: 3,
            behind: 1,
        };
        let json = serde_json::to_string(&pos).unwrap();
        let deser: RemotePosition = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.remote, "origin");
        assert_eq!(deser.ahead, 3);
        assert_eq!(deser.behind, 1);
    }

    #[test]
    fn remote_position_is_parity() {
        let parity = RemotePosition {
            remote: "forgejo".into(),
            ahead: 0,
            behind: 0,
        };
        assert!(parity.is_parity());

        let ahead = RemotePosition {
            remote: "origin".into(),
            ahead: 1,
            behind: 0,
        };
        assert!(!ahead.is_parity());

        let behind = RemotePosition {
            remote: "origin".into(),
            ahead: 0,
            behind: 2,
        };
        assert!(!behind.is_parity());
    }

    #[test]
    fn remote_position_display() {
        let pos = RemotePosition {
            remote: "origin".into(),
            ahead: 5,
            behind: 2,
        };
        assert_eq!(pos.to_string(), "origin(+5,-2)");
    }

    #[test]
    fn sync_classification_serde_roundtrip_all_variants() {
        let variants = [
            SyncClassification::Parity,
            SyncClassification::Converge,
            SyncClassification::Diverge,
            SyncClassification::Missing,
            SyncClassification::NoRemote,
        ];
        for variant in &variants {
            let json = serde_json::to_string(variant).unwrap();
            let deser: SyncClassification = serde_json::from_str(&json).unwrap();
            assert_eq!(&deser, variant);
        }
    }

    #[test]
    fn sync_classification_serializes_screaming_snake_case() {
        assert_eq!(
            serde_json::to_string(&SyncClassification::Parity).unwrap(),
            "\"PARITY\""
        );
        assert_eq!(
            serde_json::to_string(&SyncClassification::NoRemote).unwrap(),
            "\"NO_REMOTE\""
        );
    }

    #[test]
    fn sync_classification_display() {
        assert_eq!(SyncClassification::Parity.to_string(), "PARITY");
        assert_eq!(SyncClassification::Converge.to_string(), "CONVERGE");
        assert_eq!(SyncClassification::Diverge.to_string(), "DIVERGE");
        assert_eq!(SyncClassification::Missing.to_string(), "MISSING");
        assert_eq!(SyncClassification::NoRemote.to_string(), "NO_REMOTE");
    }

    #[test]
    fn sync_action_none_roundtrip() {
        let action = SyncAction::None;
        let json = serde_json::to_string(&action).unwrap();
        let deser: SyncAction = serde_json::from_str(&json).unwrap();
        assert!(matches!(deser, SyncAction::None));
    }

    #[test]
    fn sync_action_pull_roundtrip() {
        let action = SyncAction::Pull {
            leader: "forgejo".into(),
        };
        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains("\"leader\":\"forgejo\""));
        let deser: SyncAction = serde_json::from_str(&json).unwrap();
        if let SyncAction::Pull { leader } = deser {
            assert_eq!(leader, "forgejo");
        } else {
            panic!("expected Pull variant");
        }
    }

    #[test]
    fn sync_action_push_roundtrip() {
        let json = serde_json::to_string(&SyncAction::Push).unwrap();
        let deser: SyncAction = serde_json::from_str(&json).unwrap();
        assert!(matches!(deser, SyncAction::Push));
    }

    #[test]
    fn sync_action_tree_parity_roundtrip() {
        let action = SyncAction::TreeParity {
            leader: "forgejo".into(),
            followers: vec!["origin".into(), "github".into()],
        };
        let json = serde_json::to_string(&action).unwrap();
        let deser: SyncAction = serde_json::from_str(&json).unwrap();
        if let SyncAction::TreeParity { leader, followers } = deser {
            assert_eq!(leader, "forgejo");
            assert_eq!(followers, vec!["origin", "github"]);
        } else {
            panic!("expected TreeParity variant");
        }
    }

    #[test]
    fn sync_action_flag_roundtrip() {
        let json = serde_json::to_string(&SyncAction::Flag).unwrap();
        let deser: SyncAction = serde_json::from_str(&json).unwrap();
        assert!(matches!(deser, SyncAction::Flag));
    }

    #[test]
    fn sync_action_display() {
        assert_eq!(SyncAction::None.to_string(), "ok");
        assert_eq!(
            SyncAction::Pull {
                leader: "origin".into()
            }
            .to_string(),
            "pull origin"
        );
        assert_eq!(SyncAction::Push.to_string(), "push followers");
        assert_eq!(SyncAction::Flag.to_string(), "FLAG: human review");

        let tp = SyncAction::TreeParity {
            leader: "forgejo".into(),
            followers: vec!["origin".into(), "github".into()],
        };
        assert_eq!(tp.to_string(), "tree-parity: forgejo → origin, github");
    }

    #[test]
    fn temporal_matrix_serde_roundtrip() {
        let matrix = TemporalMatrix {
            repo_path: "primals/biomeOS".into(),
            branch: "main".into(),
            classification: SyncClassification::Converge,
            positions: vec![
                RemotePosition {
                    remote: "origin".into(),
                    ahead: 0,
                    behind: 3,
                },
                RemotePosition {
                    remote: "forgejo".into(),
                    ahead: 3,
                    behind: 0,
                },
            ],
            action: SyncAction::Pull {
                leader: "forgejo".into(),
            },
        };
        let json = serde_json::to_string(&matrix).unwrap();
        let deser: TemporalMatrix = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.repo_path, "primals/biomeOS");
        assert_eq!(deser.branch, "main");
        assert_eq!(deser.classification, SyncClassification::Converge);
        assert_eq!(deser.positions.len(), 2);
    }

    #[test]
    fn temporal_matrix_display() {
        let matrix = TemporalMatrix {
            repo_path: "primals/biomeOS".into(),
            branch: "main".into(),
            classification: SyncClassification::Parity,
            positions: vec![RemotePosition {
                remote: "origin".into(),
                ahead: 0,
                behind: 0,
            }],
            action: SyncAction::None,
        };
        let display = matrix.to_string();
        assert!(display.contains("primals/biomeOS"));
        assert!(display.contains("PARITY"));
        assert!(display.contains("origin(+0,-0)"));
        assert!(display.contains("ok"));
    }

    #[test]
    fn temporal_sync_result_roundtrip() {
        let result = TemporalSyncResult {
            repo_path: "primals/songbird".into(),
            ok: true,
            summary: "converged via ff".into(),
            pulled_from: Some("forgejo".into()),
            pushed_to: vec!["origin".into()],
        };
        let json = serde_json::to_string(&result).unwrap();
        let deser: TemporalSyncResult = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.repo_path, "primals/songbird");
        assert!(deser.ok);
        assert_eq!(deser.pulled_from.as_deref(), Some("forgejo"));
        assert_eq!(deser.pushed_to, vec!["origin"]);
    }

    #[test]
    fn cascade_sync_status_serde() {
        assert_eq!(
            serde_json::to_string(&CascadeSyncStatus::Synced).unwrap(),
            "\"synced\""
        );
        assert_eq!(
            serde_json::to_string(&CascadeSyncStatus::AlreadyCurrent).unwrap(),
            "\"already_current\""
        );
        let deser: CascadeSyncStatus = serde_json::from_str("\"failed\"").unwrap();
        assert_eq!(deser, CascadeSyncStatus::Failed);
    }

    #[test]
    fn cascade_sync_status_display() {
        assert_eq!(CascadeSyncStatus::Synced.to_string(), "synced");
        assert_eq!(CascadeSyncStatus::AlreadyCurrent.to_string(), "current");
        assert_eq!(CascadeSyncStatus::Cloned.to_string(), "cloned");
        assert_eq!(CascadeSyncStatus::Failed.to_string(), "failed");
        assert_eq!(CascadeSyncStatus::Skipped.to_string(), "skipped");
    }

    #[test]
    fn cascade_notify_roundtrip() {
        let notify = CascadeNotify {
            gate: "sporeGate".into(),
            wave: 159,
            repos: vec![
                RepoSyncResult {
                    name: "cellMembrane".into(),
                    head_sha: "abc123".into(),
                    status: CascadeSyncStatus::Synced,
                },
                RepoSyncResult {
                    name: "primals/songBird".into(),
                    head_sha: "def456".into(),
                    status: CascadeSyncStatus::AlreadyCurrent,
                },
            ],
            total: 15,
            synced: 14,
            failed: 0,
            cloned: 1,
            synced_at: "2026-09-27T23:00:00Z".into(),
        };
        let json = serde_json::to_string(&notify).unwrap();
        let deser: CascadeNotify = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.gate, "sporeGate");
        assert_eq!(deser.wave, 159);
        assert_eq!(deser.repos.len(), 2);
        assert_eq!(deser.repos[0].status, CascadeSyncStatus::Synced);
        assert_eq!(deser.repos[1].status, CascadeSyncStatus::AlreadyCurrent);
        assert_eq!(deser.total, 15);
        assert_eq!(deser.synced, 14);
    }

    #[test]
    fn repo_sync_result_roundtrip() {
        let r = RepoSyncResult {
            name: "detroit".into(),
            head_sha: "aabbcc".into(),
            status: CascadeSyncStatus::Failed,
        };
        let json = serde_json::to_string(&r).unwrap();
        let deser: RepoSyncResult = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.name, "detroit");
        assert_eq!(deser.head_sha, "aabbcc");
        assert_eq!(deser.status, CascadeSyncStatus::Failed);
    }
}
