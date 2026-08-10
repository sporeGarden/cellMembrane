// SPDX-License-Identifier: AGPL-3.0-or-later

//! rootPulse sovereignty ledger — commit cascade state and verify against it.
//!
//! Integrates with the NUCLEUS neural-api to register/verify cascade HEADs
//! via the rootPulse provenance trio:
//! `rhizoCrypt` (dehydrate) -> `BearDog` (sign) -> `NestGate` (store)
//! -> `LoamSpine` (commit) -> `sweetGrass` (attribute).

use std::collections::BTreeMap;

use crate::error::{Result, ShadowError};

/// Register cascade state with the rootPulse provenance trio via NUCLEUS neural-api.
///
/// Resolves the neural-api endpoint via the transport resolver — local UDS if on
/// this gate, TCP over `WireGuard` mesh, or songBird relay for remote gates.
/// Returns `Ok(session_id)` on success or an error if NUCLEUS is unreachable.
pub async fn rootpulse_commit(
    wave_id: u32,
    gate: &str,
    heads: &BTreeMap<String, String>,
) -> Result<String> {
    let endpoint = resolve_neural_api_endpoint().ok_or_else(|| {
        ShadowError::config("NUCLEUS neural-api endpoint not found — rootpulse commit skipped")
    })?;

    let session_id = format!("wave-{wave_id}-cascade-{}", crate::utc_now_compact());
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

    let response = crate::jsonrpc::call_endpoint(&endpoint, &request).await?;

    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&response) {
        if parsed.get("error").is_some() && parsed.get("result").is_none() {
            return Err(ShadowError::config(format!(
                "rootpulse commit graph error: {response}"
            )));
        }
    }

    Ok(session_id)
}

/// Register a harvest batch with the rootPulse provenance trio via NUCLEUS neural-api.
///
/// Unlike `rootpulse_commit` which records cascade repo HEADs, this records
/// build events: which primals were rebuilt, their source commits, target
/// triples, and BLAKE3 checksums. This creates a provenance chain from
/// source commit → built binary → depot.
pub async fn rootpulse_harvest_commit(
    gate: &str,
    harvest_entries: &[HarvestProvenanceEntry],
) -> Result<String> {
    let endpoint = resolve_neural_api_endpoint().ok_or_else(|| {
        ShadowError::config(
            "NUCLEUS neural-api endpoint not found — rootpulse harvest commit skipped",
        )
    })?;

    let session_id = format!("harvest-{gate}-{}", crate::utc_now_compact());
    let agent_did = format!("did:primal:cellMembrane:{gate}");

    let metadata = serde_json::json!({
        "session_id": session_id,
        "agent_did": agent_did,
        "gate": gate,
        "event_type": "depot.harvest",
        "entries": harvest_entries,
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
            "metadata": metadata,
        }),
        44,
    );

    let response = crate::jsonrpc::call_endpoint(&endpoint, &request).await?;

    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&response) {
        if parsed.get("error").is_some() && parsed.get("result").is_none() {
            return Err(ShadowError::config(format!(
                "rootpulse harvest commit graph error: {response}"
            )));
        }
    }

    Ok(session_id)
}

/// A single primal build event for rootPulse harvest provenance.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HarvestProvenanceEntry {
    /// Primal binary name.
    pub primal: String,
    /// Source commit hash.
    pub commit: String,
    /// Target triple (e.g. `x86_64-unknown-linux-musl`).
    pub target: String,
    /// BLAKE3 hash of the built binary.
    pub blake3: String,
}

/// G69 Phase 3: Record a binary supersession in the depot lineage graph.
///
/// Before a depot binary is overwritten by a new build, this records the
/// evolution event via the `depot_lineage` biomeOS graph — signing the hash,
/// appending to the primal's spine, and creating an attribution braid.
/// Graceful degradation: returns Ok(None) if NUCLEUS is unreachable.
#[allow(
    clippy::too_many_arguments,
    reason = "neural-api graph params naturally require distinct fields"
)]
pub async fn archive_superseded_binary(
    primal: &str,
    arch: &str,
    old_blake3: &str,
    new_blake3: &str,
    commit: &str,
    wave: &str,
    builder_gate: &str,
    binary_size: u64,
) -> Result<Option<String>> {
    let Some(endpoint) = resolve_neural_api_endpoint() else {
        tracing::debug!(
            primal,
            arch,
            "depot lineage skipped — NUCLEUS neural-api not reachable"
        );
        return Ok(None);
    };

    let request = crate::jsonrpc::request_with_params(
        "graph.execute",
        &serde_json::json!({
            "graph_id": "depot_lineage",
            "params": {
                "PRIMAL_NAME": primal,
                "BINARY_HASH": old_blake3,
                "BINARY_SIZE": binary_size.to_string(),
                "ARCH": arch,
                "BUILDER_GATE": builder_gate,
                "WAVE": wave,
                "COMMIT_SHA": commit,
                "FAMILY_ID": "default",
            },
            "metadata": {
                "event_type": "binary.supersede",
                "old_blake3": old_blake3,
                "new_blake3": new_blake3,
                "primal": primal,
                "arch": arch,
            },
        }),
        45,
    );

    match crate::jsonrpc::call_endpoint(&endpoint, &request).await {
        Ok(response) => {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&response) {
                if parsed.get("error").is_some() && parsed.get("result").is_none() {
                    tracing::warn!(
                        primal,
                        arch,
                        error = %response,
                        "depot lineage graph error — binary still overwritten"
                    );
                    return Ok(None);
                }
            }
            tracing::info!(
                primal,
                arch,
                old_blake3,
                new_blake3,
                "depot lineage recorded: binary.supersede"
            );
            Ok(Some(format!("lineage:{primal}:{arch}:{old_blake3}")))
        }
        Err(e) => {
            tracing::warn!(
                primal,
                arch,
                error = %e,
                "depot lineage call failed — binary still overwritten"
            );
            Ok(None)
        }
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
    let Some(endpoint) = resolve_neural_api_endpoint() else {
        return Vec::new();
    };

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

    let Ok(response) = crate::jsonrpc::call_endpoint(&endpoint, &request).await else {
        return Vec::new();
    };

    parse_verify_response(&response, heads)
}

/// Parse a sovereignty verification JSON-RPC response against known HEADs.
///
/// Pure function — no I/O. Extracts `result.ledger_heads` from the response
/// and compares each repo HEAD. Returns per-repo verification results.
fn parse_verify_response(
    response: &str,
    heads: &BTreeMap<String, String>,
) -> Vec<SovereigntyCheck> {
    let parsed: serde_json::Value = match serde_json::from_str(response) {
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

/// Resolve the NUCLEUS neural-api endpoint via the transport resolver.
///
/// Tries local Identity capability first (finds `neural-api-default.sock` or
/// `biomeos.sock`), then cross-gate resolution via manifest roles. Returns
/// `None` if no reachable endpoint exists — the caller should degrade gracefully.
fn resolve_neural_api_endpoint() -> Option<cellmembrane_types::TransportEndpoint> {
    let ctx = crate::resolve::ResolutionContext::from_env();

    if let Some(ep) = crate::resolve::resolve_endpoint(
        &ctx,
        &ctx.local_gate,
        cellmembrane_types::service::ServiceCapability::Identity,
    ) {
        if let cellmembrane_types::TransportEndpoint::Uds { ref path } = ep {
            if std::path::Path::new(path).exists() {
                return Some(ep);
            }
        } else {
            return Some(ep);
        }
    }

    crate::resolve::resolve_by_role(&ctx, "identity")
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
        let session = format!("wave-{}-cascade-{}", 116, crate::utc_now_compact());
        assert!(session.starts_with("wave-116-cascade-"));
        assert!(session.len() > 25);
    }

    #[test]
    fn agent_did_format() {
        let did = format!("did:primal:cellMembrane:{}", "sporeGate");
        assert_eq!(did, "did:primal:cellMembrane:sporeGate");
    }

    #[test]
    fn resolve_neural_api_endpoint_uses_resolver() {
        let ep = resolve_neural_api_endpoint();
        if let Some(cellmembrane_types::TransportEndpoint::Uds { path }) = &ep {
            let known = ["neural-api", "biomeos", "ai"];
            assert!(
                known.iter().any(|k| path.contains(k)),
                "resolved path should reference neural-api, biomeos, or ai alias, got: {path}"
            );
        }
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

    fn test_heads() -> BTreeMap<String, String> {
        let mut heads = BTreeMap::new();
        heads.insert("biomeOS".into(), "abc12345deadbeef".into());
        heads.insert("cellMembrane".into(), "def67890cafebabe".into());
        heads
    }

    #[test]
    fn parse_verify_all_match() {
        let response = r#"{"jsonrpc":"2.0","result":{"ledger_heads":{"biomeOS":"abc12345deadbeef","cellMembrane":"def67890cafebabe"}},"id":43}"#;
        let checks = parse_verify_response(response, &test_heads());
        assert_eq!(checks.len(), 2);
        assert!(checks.iter().all(|c| c.verified), "all should match");
        assert!(checks.iter().all(|c| c.detail == "sovereign match"));
    }

    #[test]
    fn parse_verify_mismatch() {
        let response = r#"{"jsonrpc":"2.0","result":{"ledger_heads":{"biomeOS":"abc12345deadbeef","cellMembrane":"TAMPERED_HASH"}},"id":43}"#;
        let checks = parse_verify_response(response, &test_heads());
        let cm = checks.iter().find(|c| c.repo == "cellMembrane").unwrap();
        assert!(!cm.verified);
        assert!(cm.detail.contains("MISMATCH"));
        assert!(cm.detail.contains("def67890"));
        assert!(cm.detail.contains("TAMPERED_HASH"));

        let bio = checks.iter().find(|c| c.repo == "biomeOS").unwrap();
        assert!(bio.verified);
    }

    #[test]
    fn parse_verify_missing_from_ledger() {
        let response =
            r#"{"jsonrpc":"2.0","result":{"ledger_heads":{"biomeOS":"abc12345deadbeef"}},"id":43}"#;
        let checks = parse_verify_response(response, &test_heads());
        let cm = checks.iter().find(|c| c.repo == "cellMembrane").unwrap();
        assert!(!cm.verified);
        assert!(cm.detail.contains("(not in ledger)"));
    }

    #[test]
    fn parse_verify_error_response() {
        let response =
            r#"{"jsonrpc":"2.0","error":{"code":-32600,"message":"ledger empty"},"id":43}"#;
        let checks = parse_verify_response(response, &test_heads());
        assert_eq!(checks.len(), 2);
        assert!(checks.iter().all(|c| !c.verified));
        assert!(checks[0].detail.contains("not yet initialized"));
    }

    #[test]
    fn parse_verify_invalid_json() {
        let checks = parse_verify_response("not json at all", &test_heads());
        assert_eq!(checks.len(), 2);
        assert!(checks.iter().all(|c| !c.verified));
        assert!(checks[0].detail.contains("unreachable"));
    }

    #[test]
    fn parse_verify_missing_ledger_heads_key() {
        let response = r#"{"jsonrpc":"2.0","result":{"something_else": true},"id":43}"#;
        let checks = parse_verify_response(response, &test_heads());
        assert_eq!(checks.len(), 2);
        assert!(checks.iter().all(|c| !c.verified));
        assert!(checks[0].detail.contains("no ledger state"));
    }

    #[test]
    fn parse_verify_empty_heads() {
        let response = r#"{"jsonrpc":"2.0","result":{"ledger_heads":{}},"id":43}"#;
        let checks = parse_verify_response(response, &BTreeMap::new());
        assert!(checks.is_empty());
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

    #[test]
    fn harvest_provenance_entry_serializes() {
        let entry = HarvestProvenanceEntry {
            primal: "songbird".into(),
            commit: "42aba605".into(),
            target: "x86_64-unknown-linux-musl".into(),
            blake3: "121546db28b35be6".into(),
        };
        let json = serde_json::to_value(&entry).unwrap();
        assert_eq!(json["primal"], "songbird");
        assert_eq!(json["commit"], "42aba605");
        assert_eq!(json["target"], "x86_64-unknown-linux-musl");
        assert_eq!(json["blake3"], "121546db28b35be6");
    }

    #[test]
    fn harvest_session_id_format() {
        let gate = "sporeGate";
        let session = format!("harvest-{gate}-{}", crate::utc_now_compact());
        assert!(session.starts_with("harvest-sporeGate-"));
        assert!(session.len() > 25);
    }
}
