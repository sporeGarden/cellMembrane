// SPDX-License-Identifier: AGPL-3.0-or-later

//! Enrollment cryptography — BTSP-verified genetic enrollment via songBird.
//!
//! Implements Phase 7 of `gate.enroll`: computes an HMAC-SHA256 enrollment proof
//! from `FAMILY_SEED` (mirroring bearDog's `enrollment.verify`) and calls
//! songBird's `mesh.enroll` JSON-RPC to cryptographically join the mesh.
//!
//! Extracted from `enroll.rs` for line budget management and separation of
//! concerns — enrollment orchestration vs. enrollment crypto.

use super::bootstrap::BootstrapPhase;

/// BTSP-verified genetic enrollment via songBird's `mesh.enroll` endpoint.
///
/// Computes an HMAC-SHA256 enrollment proof from `FAMILY_SEED` (the same
/// algorithm bearDog's `enrollment.verify` checks) and calls songBird's
/// `mesh.enroll` JSON-RPC to complete cryptographic enrollment into the mesh.
///
/// Requires: `FAMILY_SEED` or `BEARDOG_FAMILY_SEED` env var set.
/// Requires: songBird running locally with its UDS socket reachable.
pub(super) async fn mesh_enroll_phase(
    gate_name: &str,
    mesh_ip: &str,
    dry_run: bool,
) -> BootstrapPhase {
    let family_seed = load_family_seed();

    if family_seed.is_none() {
        return BootstrapPhase {
            name: "mesh.enroll".into(),
            ok: false,
            detail: "FAMILY_SEED or BEARDOG_FAMILY_SEED not set — cannot compute enrollment proof"
                .into(),
        };
    }
    let family_seed = family_seed.unwrap_or_default();

    let Some(pubkey) = super::wg::read_local_pubkey().await else {
        return BootstrapPhase {
            name: "mesh.enroll".into(),
            ok: false,
            detail: "cannot read local WG public key — run wg.keygen first".into(),
        };
    };

    let seed_generation = load_seed_generation();
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());

    let proof = compute_enrollment_proof(
        &family_seed,
        gate_name,
        &pubkey,
        timestamp,
        seed_generation,
    );

    if dry_run {
        return BootstrapPhase {
            name: "mesh.enroll".into(),
            ok: true,
            detail: format!(
                "dry-run: would call mesh.enroll for {gate_name} (gen={seed_generation}, proof={}...)",
                &proof[..8.min(proof.len())]
            ),
        };
    }

    let songbird_socket = resolve_relay_socket();
    if !songbird_socket.exists() {
        return BootstrapPhase {
            name: "mesh.enroll".into(),
            ok: false,
            detail: format!(
                "songBird socket not found at {} — is songBird running?",
                songbird_socket.display()
            ),
        };
    }

    let params = serde_json::json!({
        "node_id": gate_name,
        "public_key": pubkey,
        "timestamp": timestamp,
        "proof": proof,
        "address": format!("{mesh_ip}:{}", cellmembrane_types::service::DEFAULT_FEDERATION_PORT),
    });
    let request = crate::jsonrpc::request_with_params("mesh.enroll", &params, 1);

    match crate::jsonrpc::call(&songbird_socket, &request).await {
        Ok(response) => {
            let enrolled = serde_json::from_str::<serde_json::Value>(&response)
                .ok()
                .and_then(|j| j.get("result").cloned().or(Some(j)))
                .and_then(|r| r.get("enrolled").and_then(serde_json::Value::as_bool))
                .unwrap_or(false);

            if enrolled {
                BootstrapPhase {
                    name: "mesh.enroll".into(),
                    ok: true,
                    detail: format!("{gate_name} enrolled into mesh (gen={seed_generation})"),
                }
            } else {
                let reason = serde_json::from_str::<serde_json::Value>(&response)
                    .ok()
                    .and_then(|j| j.get("result").cloned().or(Some(j)))
                    .and_then(|r| {
                        r.get("reason")
                            .and_then(serde_json::Value::as_str)
                            .map(String::from)
                    })
                    .unwrap_or_else(|| "unknown".into());

                BootstrapPhase {
                    name: "mesh.enroll".into(),
                    ok: false,
                    detail: format!("mesh.enroll rejected for {gate_name}: {reason}"),
                }
            }
        }
        Err(e) => BootstrapPhase {
            name: "mesh.enroll".into(),
            ok: false,
            detail: format!("mesh.enroll RPC failed: {e}"),
        },
    }
}

/// Load `FAMILY_SEED` from environment (same precedence as bearDog).
fn load_family_seed() -> Option<Vec<u8>> {
    for var in ["BEARDOG_FAMILY_SEED", "FAMILY_SEED"] {
        if let Ok(val) = std::env::var(var) {
            if !val.is_empty() {
                return Some(val.into_bytes());
            }
        }
    }
    None
}

/// Load enrollment seed generation from environment (default 0).
pub(super) fn load_seed_generation() -> u32 {
    std::env::var("BEARDOG_ENROLLMENT_SEED_GENERATION")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

/// Compute the HMAC-SHA256 enrollment proof.
///
/// Mirrors bearDog's enrollment crypto exactly:
/// ```text
/// key = HKDF-SHA256(family_seed, salt=FAMILY_ID, info="enrollment-v{gen}")
/// message = "node_id|public_key|timestamp|generation"
/// proof = base64(HMAC-SHA256(key, message))
/// ```
pub(super) fn compute_enrollment_proof(
    family_seed: &[u8],
    node_id: &str,
    public_key: &str,
    timestamp: u64,
    generation: u32,
) -> String {
    use base64::Engine;
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;

    let family_id = std::env::var("FAMILY_ID").unwrap_or_else(|_| "default".into());

    let info = format!("enrollment-v{generation}");
    let mut extract_mac = HmacSha256::new_from_slice(family_id.as_bytes()).expect("HMAC key init");
    extract_mac.update(family_seed);
    let prk = extract_mac.finalize().into_bytes();

    let mut expand_mac = HmacSha256::new_from_slice(&prk).expect("HMAC key init");
    expand_mac.update(info.as_bytes());
    expand_mac.update(&[1u8]);
    let enrollment_key: [u8; 32] = expand_mac.finalize().into_bytes().into();

    let message = format!("{node_id}|{public_key}|{timestamp}|{generation}");
    let mut proof_mac = HmacSha256::new_from_slice(&enrollment_key).expect("HMAC key init");
    proof_mac.update(message.as_bytes());
    let proof_bytes = proof_mac.finalize().into_bytes();

    base64::engine::general_purpose::STANDARD.encode(proof_bytes)
}

/// Resolve the mesh relay UDS socket path via capability discovery.
pub(super) fn resolve_relay_socket() -> std::path::PathBuf {
    let relay = cellmembrane_types::MembraneService::binary_for(
        cellmembrane_types::ServiceCapability::MeshRelay,
    );
    let paths = super::health::resolve_primal_socket_paths(relay);
    std::path::PathBuf::from(&paths[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enrollment_proof_is_deterministic() {
        let seed = b"test-family-seed";
        let p1 = compute_enrollment_proof(seed, "gate1", "pubkey123", 1000, 0);
        let p2 = compute_enrollment_proof(seed, "gate1", "pubkey123", 1000, 0);
        assert_eq!(p1, p2);
    }

    #[test]
    fn enrollment_proof_changes_with_inputs() {
        let seed = b"test-family-seed";
        let p1 = compute_enrollment_proof(seed, "gate1", "pubkey1", 1000, 0);
        let p2 = compute_enrollment_proof(seed, "gate2", "pubkey1", 1000, 0);
        let p3 = compute_enrollment_proof(seed, "gate1", "pubkey2", 1000, 0);
        let p4 = compute_enrollment_proof(seed, "gate1", "pubkey1", 2000, 0);
        let p5 = compute_enrollment_proof(seed, "gate1", "pubkey1", 1000, 1);
        assert_ne!(p1, p2);
        assert_ne!(p1, p3);
        assert_ne!(p1, p4);
        assert_ne!(p1, p5);
    }

    #[test]
    fn enrollment_proof_is_valid_base64() {
        use base64::Engine;
        let proof = compute_enrollment_proof(b"seed", "node", "key", 1, 0);
        let decoded = base64::engine::general_purpose::STANDARD.decode(&proof);
        assert!(decoded.is_ok(), "proof should be valid base64");
        assert_eq!(decoded.unwrap().len(), 32, "HMAC-SHA256 output is 32 bytes");
    }

    #[test]
    fn load_seed_generation_parses_or_defaults() {
        let result = load_seed_generation();
        assert!(result < 1000, "generation should be reasonable: {result}");
    }

    #[test]
    fn relay_socket_path_ends_with_sock() {
        let path = resolve_relay_socket();
        assert!(
            path.extension().is_some_and(|e| e == "sock"),
            "socket path should end with .sock: {path:?}"
        );
    }

    #[tokio::test]
    async fn mesh_enroll_dry_run() {
        let phase = mesh_enroll_phase("testGate", "10.0.0.1", true).await;
        assert_eq!(phase.name, "mesh.enroll");
        assert!(
            phase.detail.contains("dry-run") || phase.detail.contains("FAMILY_SEED"),
            "should be dry-run or missing seed: {}",
            phase.detail
        );
    }
}
