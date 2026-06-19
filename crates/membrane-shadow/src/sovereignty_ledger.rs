// SPDX-License-Identifier: AGPL-3.0-or-later

//! rootPulse sovereignty ledger — commit cascade state and verify against it.
//!
//! Integrates with the NUCLEUS neural-api to register/verify cascade HEADs
//! via the rootPulse provenance trio:
//! `rhizoCrypt` (dehydrate) -> `BearDog` (sign) -> `NestGate` (store)
//! -> `LoamSpine` (commit) -> `sweetGrass` (attribute).

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::error::{Result, ShadowError};

/// Register cascade state with the rootPulse provenance trio via NUCLEUS neural-api.
///
/// Returns `Ok(session_id)` on success or an error if NUCLEUS is unreachable.
pub async fn rootpulse_commit(
    wave_id: u32,
    gate: &str,
    heads: &BTreeMap<String, String>,
) -> Result<String> {
    let socket_path = resolve_neural_api_socket();
    if !socket_path.exists() {
        return Err(ShadowError::Config(
            "NUCLEUS neural-api socket not found — rootpulse commit skipped".into(),
        ));
    }

    let session_id = format!(
        "wave-{wave_id}-cascade-{}",
        chrono::Utc::now().format("%Y%m%dT%H%M%S")
    );
    let agent_did = format!("did:primal:cellMembrane:{gate}");

    let params = serde_json::json!({
        "session_id": session_id,
        "agent_did": agent_did,
        "wave_id": wave_id,
        "heads": heads,
        "gate": gate,
    });

    let request = crate::jsonrpc::request_with_params(
        "graph.execute",
        &serde_json::json!({
            "graph_id": "rootpulse_commit",
            "params": {
                "SESSION_ID": session_id,
                "AGENT_DID": agent_did,
                "FAMILY_ID": "default",
            },
            "metadata": params,
        }),
        42,
    );

    match crate::jsonrpc::call(&socket_path, &request).await {
        Ok(response) => {
            if response.contains("error") && !response.contains("result") {
                return Err(ShadowError::Config(format!(
                    "rootpulse commit graph error: {response}"
                )));
            }
            Ok(session_id)
        }
        Err(e) => Err(ShadowError::Ssh(format!("rootpulse commit failed: {e}"))),
    }
}

/// Sovereignty verification result for a single repo.
#[derive(Debug)]
pub struct SovereigntyCheck {
    /// Repository name.
    pub repo: String,
    /// Whether the repo HEAD matches the rootPulse ledger record.
    pub verified: bool,
    /// Human-readable verification detail.
    pub detail: String,
}

/// Verify cascade HEADs against the rootPulse ledger.
///
/// Queries the last rootpulse-committed state via the neural-api and compares
/// each repo HEAD against the ledger record. Any mismatch indicates potential
/// VCS tampering (GitHub/Forgejo diverged from sovereign record).
///
/// Returns per-repo verification results. If NUCLEUS is unavailable, returns
/// an empty vec (graceful degradation).
pub async fn sovereignty_verify(
    wave_id: u32,
    heads: &BTreeMap<String, String>,
) -> Vec<SovereigntyCheck> {
    let socket_path = resolve_neural_api_socket();
    if !socket_path.exists() {
        return Vec::new();
    }

    let request = crate::jsonrpc::request_with_params(
        "graph.execute",
        &serde_json::json!({
            "graph_id": "rootpulse_diff",
            "params": {
                "WAVE_ID": wave_id.to_string(),
                "CURRENT_HEADS": heads,
            },
        }),
        43,
    );

    let Ok(response) = crate::jsonrpc::call(&socket_path, &request).await else {
        return Vec::new();
    };

    let parsed: serde_json::Value = match serde_json::from_str(&response) {
        Ok(v) => v,
        Err(_) => return mark_all_unverified(heads, "ledger unreachable"),
    };

    if parsed.get("error").is_some() {
        return mark_all_unverified(heads, "rootpulse ledger not yet initialized");
    }

    let ledger_heads = parsed
        .get("result")
        .and_then(|r| r.get("ledger_heads"))
        .and_then(|h| h.as_object());

    ledger_heads.map_or_else(
        || mark_all_unverified(heads, "no ledger state returned"),
        |ledger| {
            heads
                .iter()
                .map(|(repo, head)| {
                    let verified = ledger
                        .get(repo)
                        .and_then(|v| v.as_str())
                        .is_some_and(|ledger_head| ledger_head == head);
                    let detail = if verified {
                        "sovereign match".into()
                    } else {
                        let ledger_val = ledger
                            .get(repo)
                            .and_then(|v| v.as_str())
                            .unwrap_or("(not in ledger)");
                        format!(
                            "MISMATCH: VCS={} ledger={ledger_val}",
                            &head[..8.min(head.len())]
                        )
                    };
                    SovereigntyCheck {
                        repo: repo.clone(),
                        verified,
                        detail,
                    }
                })
                .collect()
        },
    )
}

fn mark_all_unverified(heads: &BTreeMap<String, String>, reason: &str) -> Vec<SovereigntyCheck> {
    heads
        .keys()
        .map(|repo| SovereigntyCheck {
            repo: repo.clone(),
            verified: false,
            detail: format!("unverified: {reason}"),
        })
        .collect()
}

/// Resolve the NUCLEUS neural-api socket path.
fn resolve_neural_api_socket() -> PathBuf {
    if let Ok(path) = std::env::var(cellmembrane_types::service::ENV_NEURAL_API_SOCKET) {
        return PathBuf::from(path);
    }

    let socket_base = std::env::var(cellmembrane_types::service::ENV_SOCKET_BASE)
        .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_SOCKET_BASE.into());

    PathBuf::from(&socket_base).join(cellmembrane_types::service::NEURAL_API_SOCKET_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_all_unverified_produces_correct_output() {
        let mut heads = BTreeMap::new();
        heads.insert("repo-a".to_string(), "abc123".to_string());
        heads.insert("repo-b".to_string(), "def456".to_string());

        let checks = mark_all_unverified(&heads, "test reason");
        assert_eq!(checks.len(), 2);
        assert!(!checks[0].verified);
        assert!(checks[0].detail.contains("test reason"));
    }

    #[test]
    fn sovereignty_check_fields() {
        let check = SovereigntyCheck {
            repo: "cellMembrane".to_string(),
            verified: true,
            detail: "sovereign match".into(),
        };
        assert!(check.verified);
        assert_eq!(check.repo, "cellMembrane");
    }

    #[test]
    fn session_id_format() {
        let session = format!(
            "wave-{}-cascade-{}",
            116,
            chrono::Utc::now().format("%Y%m%dT%H%M%S")
        );
        assert!(session.starts_with("wave-116-cascade-"));
        assert!(session.len() > 25);
    }

    #[test]
    fn agent_did_format() {
        let did = format!("did:primal:cellMembrane:{}", "sporeGate");
        assert_eq!(did, "did:primal:cellMembrane:sporeGate");
    }

    #[test]
    fn default_socket_path_uses_constants() {
        let expected = PathBuf::from(cellmembrane_types::service::DEFAULT_SOCKET_BASE)
            .join(cellmembrane_types::service::NEURAL_API_SOCKET_NAME);
        assert!(
            expected.to_str().unwrap().contains("neural-api"),
            "default socket path should contain neural-api"
        );
    }

    #[test]
    fn mark_all_unverified_preserves_repos() {
        let mut heads = BTreeMap::new();
        heads.insert("biomeOS".into(), "aaa".into());
        heads.insert("cellMembrane".into(), "bbb".into());
        heads.insert("whitePaper".into(), "ccc".into());

        let checks = mark_all_unverified(&heads, "offline");
        assert_eq!(checks.len(), 3);
        let repos: Vec<&str> = checks.iter().map(|c| c.repo.as_str()).collect();
        assert!(repos.contains(&"biomeOS"));
        assert!(repos.contains(&"cellMembrane"));
        assert!(repos.contains(&"whitePaper"));
        for check in &checks {
            assert!(!check.verified);
            assert!(check.detail.contains("offline"));
        }
    }

    #[test]
    fn mark_all_unverified_empty_heads() {
        let heads = BTreeMap::new();
        let checks = mark_all_unverified(&heads, "no repos");
        assert!(checks.is_empty());
    }

    #[test]
    fn sovereignty_check_mismatch_detail() {
        let check = SovereigntyCheck {
            repo: "biomeOS".into(),
            verified: false,
            detail: "MISMATCH: VCS=abc12345 ledger=def67890".into(),
        };
        assert!(!check.verified);
        assert!(check.detail.contains("MISMATCH"));
        assert!(check.detail.contains("VCS="));
        assert!(check.detail.contains("ledger="));
    }

    #[test]
    fn graph_request_structure() {
        let session_id = "wave-116-cascade-20260619T120000";
        let agent_did = "did:primal:cellMembrane:sporeGate";
        let params = serde_json::json!({
            "session_id": session_id,
            "agent_did": agent_did,
            "wave_id": 116_u32,
            "heads": {"cellMembrane": "abc123"},
            "gate": "sporeGate",
        });
        let request_body = serde_json::json!({
            "graph_id": "rootpulse_commit",
            "params": {
                "SESSION_ID": session_id,
                "AGENT_DID": agent_did,
                "FAMILY_ID": "default",
            },
            "metadata": params,
        });
        assert_eq!(
            request_body["graph_id"].as_str().unwrap(),
            "rootpulse_commit"
        );
        assert_eq!(
            request_body["metadata"]["gate"].as_str().unwrap(),
            "sporeGate"
        );
        assert_eq!(
            request_body["params"]["SESSION_ID"].as_str().unwrap(),
            session_id
        );
    }
}
