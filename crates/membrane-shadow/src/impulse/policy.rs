// SPDX-License-Identifier: AGPL-3.0-or-later

use super::types::ImpulseFile;

pub fn classify_diverge_type(positions: &[(String, u32, u32)]) -> String {
    let ahead_remotes: Vec<_> = positions.iter().filter(|(_, a, _)| *a > 0).collect();
    let behind_remotes: Vec<_> = positions.iter().filter(|(_, _, b)| *b > 0).collect();

    match (ahead_remotes.len(), behind_remotes.len()) {
        (0, 1) => format!("{}_ahead", behind_remotes[0].0),
        (1, 0) => "local_ahead".to_string(),
        (_, _) if ahead_remotes.len() >= 2 || behind_remotes.len() >= 2 => {
            "multi_remote_diverge".to_string()
        }
        _ => "diverge".to_string(),
    }
}

pub fn suggest_action(diverge_type: &str, repo_policy: &str) -> String {
    match repo_policy {
        "merge-ff" => "pull_leader_push_followers".to_string(),
        "merge-rebase" => "rebase_and_push".to_string(),
        "impulse-only" => "human_review".to_string(),
        "agentic" => "agentic_resolve".to_string(),
        _ => match diverge_type {
            t if t.ends_with("_ahead") => format!("pull_{t}_push_others"),
            "local_ahead" => "push_all".to_string(),
            _ => "human_review".to_string(),
        },
    }
}

pub fn is_expired(expires: &str, now: &chrono::DateTime<chrono::Utc>) -> bool {
    if expires.is_empty() {
        return false;
    }
    chrono::DateTime::parse_from_str(expires, "%Y-%m-%dT%H:%M:%S%:z").is_ok_and(|exp| now > &exp)
}

pub fn is_fully_acked(impulse: &ImpulseFile) -> bool {
    if !impulse.meta.ack_required || impulse.to.gates.is_empty() {
        return false;
    }
    if impulse.to.gates.contains(&"*".to_string()) {
        return false;
    }
    impulse
        .to
        .gates
        .iter()
        .all(|g| impulse.acks.iter().any(|a| &a.gate == g))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::impulse::types::*;

    #[test]
    fn classify_single_behind() {
        let pos = vec![("forgejo".into(), 0, 3)];
        assert_eq!(classify_diverge_type(&pos), "forgejo_ahead");
    }

    #[test]
    fn classify_local_ahead() {
        let pos = vec![("origin".into(), 2, 0)];
        assert_eq!(classify_diverge_type(&pos), "local_ahead");
    }

    #[test]
    fn classify_multi_diverge() {
        let pos = vec![("origin".into(), 1, 0), ("forgejo".into(), 2, 0)];
        assert_eq!(classify_diverge_type(&pos), "multi_remote_diverge");
    }

    #[test]
    fn suggest_merge_ff() {
        assert_eq!(
            suggest_action("any", "merge-ff"),
            "pull_leader_push_followers"
        );
    }

    #[test]
    fn suggest_impulse_only() {
        assert_eq!(suggest_action("any", "impulse-only"), "human_review");
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
