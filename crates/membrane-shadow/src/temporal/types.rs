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

}
