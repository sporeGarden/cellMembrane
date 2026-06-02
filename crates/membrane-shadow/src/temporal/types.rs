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

/// Aggregate result for a full temporal sync across multiple repos.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalReport {
    /// Total repos checked.
    pub total: u32,
    /// Repos at parity.
    pub parity: u32,
    /// Repos successfully converged.
    pub converged: u32,
    /// Repos with divergence flagged for review.
    pub diverged: u32,
    /// Repos missing or not cloned.
    pub missing: u32,
    /// Per-repo results.
    pub repos: Vec<TemporalMatrix>,
}
