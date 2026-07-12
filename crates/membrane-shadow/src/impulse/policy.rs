// SPDX-License-Identifier: AGPL-3.0-or-later

use super::types::{ImpulseAck, ImpulseFile};
use cellmembrane_types::DivergencePolicy;
use std::fmt;
use std::path::Path;

/// Typed divergence classification for cascade sync.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DivergeType {
    /// A single remote is ahead of local (e.g. `forgejo_ahead`).
    RemoteAhead(String),
    /// Local is ahead of all remotes.
    LocalAhead,
    /// Multiple remotes have diverged from each other.
    MultiRemoteDiverge,
    /// General divergence (no clear leader).
    Diverge,
}

impl fmt::Display for DivergeType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RemoteAhead(name) => write!(f, "{name}_ahead"),
            Self::LocalAhead => f.write_str("local_ahead"),
            Self::MultiRemoteDiverge => f.write_str("multi_remote_diverge"),
            Self::Diverge => f.write_str("diverge"),
        }
    }
}

/// Typed resolution action suggested by divergence policy.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestedAction {
    PullLeaderPushFollowers,
    RebaseAndPush,
    HumanReview,
    AgenticResolve,
    PushAll,
    PullRemotePushOthers(String),
}

impl fmt::Display for SuggestedAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PullLeaderPushFollowers => f.write_str("pull_leader_push_followers"),
            Self::RebaseAndPush => f.write_str("rebase_and_push"),
            Self::HumanReview => f.write_str("human_review"),
            Self::AgenticResolve => f.write_str("agentic_resolve"),
            Self::PushAll => f.write_str("push_all"),
            Self::PullRemotePushOthers(name) => write!(f, "pull_{name}_push_others"),
        }
    }
}

#[must_use]
pub(super) fn classify_diverge_type(positions: &[(String, u32, u32)]) -> DivergeType {
    let ahead_remotes: Vec<_> = positions.iter().filter(|(_, a, _)| *a > 0).collect();
    let behind_remotes: Vec<_> = positions.iter().filter(|(_, _, b)| *b > 0).collect();

    match (ahead_remotes.len(), behind_remotes.len()) {
        (0, 1) => DivergeType::RemoteAhead(behind_remotes[0].0.clone()),
        (1, 0) => DivergeType::LocalAhead,
        (_, _) if ahead_remotes.len() >= 2 || behind_remotes.len() >= 2 => {
            DivergeType::MultiRemoteDiverge
        }
        _ => DivergeType::Diverge,
    }
}

#[must_use]
pub(super) fn suggest_action(
    diverge_type: &DivergeType,
    repo_policy: DivergencePolicy,
) -> SuggestedAction {
    match repo_policy {
        DivergencePolicy::MergeFf => SuggestedAction::PullLeaderPushFollowers,
        DivergencePolicy::MergeRebase => SuggestedAction::RebaseAndPush,
        DivergencePolicy::ImpulseOnly => SuggestedAction::HumanReview,
        DivergencePolicy::Agentic => SuggestedAction::AgenticResolve,
        DivergencePolicy::Flag => match diverge_type {
            DivergeType::RemoteAhead(name) => SuggestedAction::PullRemotePushOthers(name.clone()),
            DivergeType::LocalAhead => SuggestedAction::PushAll,
            _ => SuggestedAction::HumanReview,
        },
    }
}

#[must_use]
pub(super) fn is_expired(expires: &str, now: &chrono::DateTime<chrono::Utc>) -> bool {
    if expires.is_empty() {
        return false;
    }
    chrono::DateTime::parse_from_str(expires, "%Y-%m-%dT%H:%M:%S%:z").is_ok_and(|exp| now > &exp)
}

#[cfg(test)]
pub(super) fn is_fully_acked(impulse: &ImpulseFile) -> bool {
    is_fully_acked_with_externals(impulse, &[])
}

/// Check if impulse is fully acked, considering both inline acks and external ack files.
#[must_use]
pub(super) fn is_fully_acked_with_externals(impulse: &ImpulseFile, external_acks: &[ImpulseAck]) -> bool {
    if !impulse.meta.ack_required || impulse.to.gates.is_empty() {
        return false;
    }
    if impulse.to.gates.iter().any(|g| g == "*") {
        return false;
    }
    impulse.to.gates.iter().all(|g| {
        impulse.acks.iter().any(|a| &a.gate == g) || external_acks.iter().any(|a| &a.gate == g)
    })
}

/// Load external ack files for a given impulse ID from `impulses/acks/`.
pub(super) fn load_external_acks(workspace_root: &Path, impulse_id: &str) -> Vec<ImpulseAck> {
    let acks_dir = workspace_root
        .join(cellmembrane_types::service::INFRA_WATERING_HOLE)
        .join("impulses/acks");
    if !acks_dir.exists() {
        return vec![];
    }

    let prefix = format!("{impulse_id}_");
    let Ok(entries) = std::fs::read_dir(&acks_dir) else {
        return vec![];
    };

    let mut acks = Vec::new();
    for entry in entries.flatten() {
        let fname = entry.file_name().to_string_lossy().to_string();
        if fname.starts_with(&prefix)
            && std::path::Path::new(&fname)
                .extension()
                .is_some_and(|e| e == "toml")
        {
            if let Ok(contents) = std::fs::read_to_string(entry.path()) {
                if let Ok(ack) = toml::from_str::<ImpulseAck>(&contents) {
                    acks.push(ack);
                }
            }
        }
    }
    acks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::impulse::types::*;

    #[test]
    fn classify_single_behind() {
        let pos = vec![("forgejo".into(), 0, 3)];
        assert_eq!(
            classify_diverge_type(&pos),
            DivergeType::RemoteAhead("forgejo".into())
        );
        assert_eq!(classify_diverge_type(&pos).to_string(), "forgejo_ahead");
    }

    #[test]
    fn classify_local_ahead() {
        let pos = vec![("origin".into(), 2, 0)];
        assert_eq!(classify_diverge_type(&pos), DivergeType::LocalAhead);
        assert_eq!(classify_diverge_type(&pos).to_string(), "local_ahead");
    }

    #[test]
    fn classify_multi_diverge() {
        let pos = vec![("origin".into(), 1, 0), ("forgejo".into(), 2, 0)];
        assert_eq!(classify_diverge_type(&pos), DivergeType::MultiRemoteDiverge);
    }

    #[test]
    fn classify_no_divergence() {
        let pos = vec![("origin".into(), 0, 0)];
        assert_eq!(classify_diverge_type(&pos), DivergeType::Diverge);
    }

    #[test]
    fn suggest_merge_ff() {
        assert_eq!(
            suggest_action(&DivergeType::Diverge, DivergencePolicy::MergeFf),
            SuggestedAction::PullLeaderPushFollowers
        );
    }

    #[test]
    fn suggest_impulse_only() {
        assert_eq!(
            suggest_action(&DivergeType::Diverge, DivergencePolicy::ImpulseOnly),
            SuggestedAction::HumanReview
        );
    }

    #[test]
    fn suggest_flag_remote_ahead() {
        let dt = DivergeType::RemoteAhead("forgejo".into());
        assert_eq!(
            suggest_action(&dt, DivergencePolicy::Flag),
            SuggestedAction::PullRemotePushOthers("forgejo".into())
        );
        assert_eq!(
            suggest_action(&dt, DivergencePolicy::Flag).to_string(),
            "pull_forgejo_push_others"
        );
    }

    #[test]
    fn suggest_flag_local_ahead() {
        assert_eq!(
            suggest_action(&DivergeType::LocalAhead, DivergencePolicy::Flag),
            SuggestedAction::PushAll
        );
    }

    #[test]
    fn suggest_agentic() {
        assert_eq!(
            suggest_action(&DivergeType::Diverge, DivergencePolicy::Agentic),
            SuggestedAction::AgenticResolve
        );
    }

    #[test]
    fn is_expired_empty_is_false() {
        assert!(!is_expired("", &chrono::Utc::now()));
    }

    #[test]
    fn is_expired_future_is_false() {
        let now = chrono::Utc::now();
        let future = (now + chrono::Duration::hours(1))
            .format("%Y-%m-%dT%H:%M:%S%:z")
            .to_string();
        assert!(!is_expired(&future, &now));
    }

    #[test]
    fn is_expired_past_is_true() {
        let now = chrono::Utc::now();
        let past = (now - chrono::Duration::hours(1))
            .format("%Y-%m-%dT%H:%M:%S%:z")
            .to_string();
        assert!(is_expired(&past, &now));
    }

    #[test]
    fn fully_acked_empty_gates() {
        let impulse = ImpulseFile {
            impulse: ImpulseMeta {
                id: "test".into(),
                impulse_type: ImpulseType::Frago,
                priority: Priority::Routine,
                wave: 1,
            },
            from: ImpulseFrom {
                gate: "a".into(),
                team: String::new(),
                project: String::new(),
                git_ref: String::new(),
            },
            to: ImpulseTo {
                gates: vec![],
                teams: vec![],
            },
            content: ImpulseContent {
                subject: "test".into(),
                body: String::new(),
            },
            meta: ImpulseOpMeta {
                created: String::new(),
                expires: String::new(),
                ack_required: true,
            },
            signature: None,
            acks: vec![],
        };
        assert!(!is_fully_acked(&impulse));
    }

    #[test]
    fn fully_acked_broadcast_never_true() {
        let impulse = ImpulseFile {
            impulse: ImpulseMeta {
                id: "test".into(),
                impulse_type: ImpulseType::Frago,
                priority: Priority::Routine,
                wave: 1,
            },
            from: ImpulseFrom {
                gate: "a".into(),
                team: String::new(),
                project: String::new(),
                git_ref: String::new(),
            },
            to: ImpulseTo {
                gates: vec!["*".into()],
                teams: vec![],
            },
            content: ImpulseContent {
                subject: "test".into(),
                body: String::new(),
            },
            meta: ImpulseOpMeta {
                created: String::new(),
                expires: String::new(),
                ack_required: true,
            },
            signature: None,
            acks: vec![ImpulseAck {
                gate: "x".into(),
                timestamp: String::new(),
                note: String::new(),
            }],
        };
        assert!(!is_fully_acked(&impulse));
    }
}
