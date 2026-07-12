// SPDX-License-Identifier: AGPL-3.0-or-later

//! Depot signing — BLAKE3 + ed25519 provenance for cascade artifacts.
//!
//! Signs `checksums.toml` via bearDog's `crypto.sign_ed25519` UDS endpoint,
//! producing a `signatures.toml` that gates can verify on fetch. Verification
//! uses `ed25519-dalek` directly (no bearDog required).
//!
//! Sign flow:  checksums.toml → BLAKE3 digest → bearDog sign → signatures.toml
//! Verify flow: checksums.toml → BLAKE3 digest → ed25519 verify against pubkey

use std::path::Path;

use cellmembrane_types::signing::{
    DepotSignature, DepotTrustPolicy, SignatureAlgorithm, SignaturesFile,
};

/// Sign the depot's `checksums.toml` using bearDog's ed25519 signer.
///
/// Returns `None` if bearDog is unavailable or signing fails. This is
/// intentionally best-effort: cascade proceeds without signatures when
/// bearDog is not running (e.g. development environments).
fn sign_depot_checksums(depot_dir: &Path) -> Option<DepotSignature> {
    let checksums_path = depot_dir.join("checksums.toml");
    let checksums_content = std::fs::read(&checksums_path).ok()?;
    let checksums_blake3 = blake3::hash(&checksums_content).to_hex().to_string();

    let gate = crate::gate::resolve_local_gate_identity();
    let sign_result = request_beardog_sign(&checksums_blake3)?;

    let sig = DepotSignature {
        algorithm: SignatureAlgorithm::Ed25519,
        public_key: sign_result.public_key,
        checksums_blake3,
        signature: sign_result.signature,
        signer_gate: gate,
        signed_at: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
    };

    Some(sig)
}

/// Sign and persist `signatures.toml` in the depot directory.
///
/// Prepends the new signature to any existing ones (most recent first).
/// Returns `true` if signing and persistence succeeded.
pub fn sign_and_persist(depot_dir: &Path) -> bool {
    let Some(sig) = sign_depot_checksums(depot_dir) else {
        tracing::debug!("depot signing: bearDog unavailable — skipping");
        return false;
    };

    let sigs_path = depot_dir.join("signatures.toml");
    let mut file = load_signatures(&sigs_path);

    file.signatures.retain(|s| s.signer_gate != sig.signer_gate);
    file.signatures.insert(0, sig);

    match toml::to_string(&file) {
        Ok(content) => {
            if let Err(e) = std::fs::write(&sigs_path, &content) {
                tracing::warn!(
                    path = %sigs_path.display(),
                    error = %e,
                    "depot signing: failed to write signatures.toml"
                );
                return false;
            }
            tracing::debug!(
                signatures = file.signatures.len(),
                "depot signing: signatures.toml updated"
            );
            true
        }
        Err(e) => {
            tracing::warn!(error = %e, "depot signing: failed to serialize signatures");
            false
        }
    }
}

/// Verify a depot signature against the current `checksums.toml`.
///
/// Returns `true` if:
/// 1. The signature's `checksums_blake3` matches the actual file digest
/// 2. The ed25519 signature is valid for the stated public key
///
/// This is a standalone verification — no bearDog needed.
fn verify_depot_signature(depot_dir: &Path, sig: &DepotSignature) -> bool {
    let checksums_path = depot_dir.join("checksums.toml");
    let Ok(checksums_content) = std::fs::read(&checksums_path) else {
        return false;
    };
    let actual_blake3 = blake3::hash(&checksums_content).to_hex().to_string();

    if actual_blake3 != sig.checksums_blake3 {
        tracing::warn!(
            expected = %sig.checksums_blake3,
            actual = %actual_blake3,
            "depot verify: checksums.toml digest mismatch — file modified after signing"
        );
        return false;
    }

    verify_ed25519(&sig.checksums_blake3, &sig.signature, &sig.public_key)
}

/// Verify depot signatures according to trust policy.
///
/// Returns `true` if the policy is satisfied.
pub fn verify_depot_with_policy(depot_dir: &Path, policy: DepotTrustPolicy) -> bool {
    match policy {
        DepotTrustPolicy::IntegrityOnly => true,
        DepotTrustPolicy::VerifyIfPresent => {
            let sigs_path = depot_dir.join("signatures.toml");
            let file = load_signatures(&sigs_path);
            file.latest().map_or_else(
                || {
                    tracing::debug!("depot verify: no signatures.toml — skipping verification");
                    true
                },
                |sig| {
                    let valid = verify_depot_signature(depot_dir, sig);
                    if !valid {
                        tracing::warn!(
                            signer = %sig.signer_gate,
                            "depot verify: signature present but INVALID"
                        );
                    }
                    valid
                },
            )
        }
        DepotTrustPolicy::RequireSigned => {
            let sigs_path = depot_dir.join("signatures.toml");
            let file = load_signatures(&sigs_path);
            let Some(sig) = file.latest() else {
                tracing::warn!("depot verify: RequireSigned but no signatures.toml");
                return false;
            };
            let valid = verify_depot_signature(depot_dir, sig);
            if !valid {
                tracing::warn!(
                    signer = %sig.signer_gate,
                    "depot verify: signature INVALID — rejecting"
                );
            }
            valid
        }
    }
}

/// Load `signatures.toml` from a path, returning empty file on missing/corrupt.
fn load_signatures(path: &Path) -> SignaturesFile {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|content| toml::from_str(&content).ok())
        .unwrap_or_default()
}

/// Fetch `signatures.toml` from the WAN depot over HTTPS.
///
/// Returns an empty `SignaturesFile` if the depot is unreachable or the
/// file doesn't exist yet (signing activation pending).
#[cfg(feature = "http")]
pub async fn fetch_wan_signatures() -> SignaturesFile {
    let base_url = std::env::var(cellmembrane_types::service::ENV_WAN_DEPOT_URL)
        .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_WAN_DEPOT_URL.to_string());
    let url = format!("{base_url}/signatures.toml");

    let Ok(client) = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    else {
        tracing::debug!("WAN signatures: failed to build HTTP client");
        return SignaturesFile::default();
    };

    let resp = match client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!(error = %e, "WAN signatures: fetch failed");
            return SignaturesFile::default();
        }
    };

    if !resp.status().is_success() {
        tracing::debug!(status = %resp.status(), "WAN signatures: non-success response");
        return SignaturesFile::default();
    }

    match resp.text().await {
        Ok(body) => toml::from_str(&body).unwrap_or_else(|e| {
            tracing::debug!(error = %e, "WAN signatures: invalid TOML");
            SignaturesFile::default()
        }),
        Err(e) => {
            tracing::debug!(error = %e, "WAN signatures: failed to read response body");
            SignaturesFile::default()
        }
    }
}

#[cfg(not(feature = "http"))]
pub async fn fetch_wan_signatures() -> SignaturesFile {
    SignaturesFile::default()
}

/// Pure ed25519 verification using `ed25519-dalek`.
fn verify_ed25519(message: &str, signature_hex: &str, public_key_hex: &str) -> bool {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};

    let Ok(pk_bytes) = hex::decode(public_key_hex) else {
        tracing::debug!("ed25519 verify: invalid public key hex");
        return false;
    };
    let Ok(sig_bytes) = hex::decode(signature_hex) else {
        tracing::debug!("ed25519 verify: invalid signature hex");
        return false;
    };

    let Ok(pk_array) = <[u8; 32]>::try_from(pk_bytes) else {
        tracing::debug!("ed25519 verify: public key not 32 bytes");
        return false;
    };
    let Ok(sig_array) = <[u8; 64]>::try_from(sig_bytes) else {
        tracing::debug!("ed25519 verify: signature not 64 bytes");
        return false;
    };

    let Ok(verifying_key) = VerifyingKey::from_bytes(&pk_array) else {
        tracing::debug!("ed25519 verify: invalid public key");
        return false;
    };
    let signature = Signature::from_bytes(&sig_array);

    verifying_key
        .verify(message.as_bytes(), &signature)
        .is_ok()
}

struct SignResult {
    public_key: String,
    signature: String,
}

/// Request an ed25519 signature from bearDog via UDS.
///
/// Reuses the same UDS discovery pattern as impulse signing.
fn request_beardog_sign(data: &str) -> Option<SignResult> {
    #[cfg(not(unix))]
    {
        let _ = data;
        return None;
    }

    #[cfg(unix)]
    {
        use base64::Engine;

        let socket_name = signer_socket_name();
        let socket_path = crate::impulse::discover_socket(&socket_name)?;

        let message_b64 = base64::engine::general_purpose::STANDARD.encode(data.as_bytes());
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "crypto.sign_ed25519",
            "params": {
                "message": message_b64,
                "key_id": "depot-signer",
                "purpose": "depot"
            }
        });
        let request_str = serde_json::to_string(&request).ok()?;
        let response_bytes = uds_sign_request(&socket_path, &request_str)?;
        let response: serde_json::Value = serde_json::from_slice(&response_bytes).ok()?;
        let result = response.get("result")?;

        let pk_b64 = result.get("public_key")?.as_str()?;
        let sig_b64 = result.get("signature")?.as_str()?;
        let pk_bytes = base64::engine::general_purpose::STANDARD.decode(pk_b64).ok()?;
        let sig_bytes = base64::engine::general_purpose::STANDARD.decode(sig_b64).ok()?;

        Some(SignResult {
            public_key: hex::encode(pk_bytes),
            signature: hex::encode(sig_bytes),
        })
    }
}

fn signer_socket_name() -> String {
    let binary =
        cellmembrane_types::MembraneService::binary_for(cellmembrane_types::ServiceCapability::CryptoSigner);
    format!("{binary}.sock")
}

#[cfg(unix)]
fn uds_sign_request(socket_path: &Path, request: &str) -> Option<Vec<u8>> {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;

    let mut stream = UnixStream::connect(socket_path).ok()?;
    stream
        .set_write_timeout(Some(std::time::Duration::from_secs(2)))
        .ok()?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .ok()?;
    writeln!(stream, "{request}").ok()?;
    stream.shutdown(std::net::Shutdown::Write).ok()?;

    let mut buf = Vec::with_capacity(4096);
    stream.read_to_end(&mut buf).ok()?;
    Some(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_rejects_invalid_hex() {
        assert!(!verify_ed25519("hello", "not_hex", "also_not_hex"));
    }

    #[test]
    fn verify_rejects_wrong_length_key() {
        let short_key = hex::encode([0u8; 16]);
        let sig = hex::encode([0u8; 64]);
        assert!(!verify_ed25519("hello", &sig, &short_key));
    }

    #[test]
    fn verify_rejects_wrong_length_signature() {
        let key = hex::encode([0u8; 32]);
        let short_sig = hex::encode([0u8; 32]);
        assert!(!verify_ed25519("hello", &short_sig, &key));
    }

    #[test]
    fn verify_rejects_forged_signature() {
        use ed25519_dalek::SigningKey;

        let signing_key = SigningKey::from_bytes(&[42u8; 32]);
        let verifying_key = signing_key.verifying_key();

        let message = "deadbeefcafebabe";
        let pk_hex = hex::encode(verifying_key.as_bytes());
        let fake_sig = hex::encode([0u8; 64]);

        assert!(!verify_ed25519(message, &fake_sig, &pk_hex));
    }

    #[test]
    fn verify_accepts_valid_signature() {
        use ed25519_dalek::{Signer, SigningKey};

        let signing_key = SigningKey::from_bytes(&[42u8; 32]);
        let verifying_key = signing_key.verifying_key();

        let message = "deadbeefcafebabe";
        let signature = signing_key.sign(message.as_bytes());

        let pk_hex = hex::encode(verifying_key.as_bytes());
        let sig_hex = hex::encode(signature.to_bytes());

        assert!(verify_ed25519(message, &sig_hex, &pk_hex));
    }

    #[test]
    fn verify_rejects_wrong_message() {
        use ed25519_dalek::{Signer, SigningKey};

        let signing_key = SigningKey::from_bytes(&[42u8; 32]);
        let verifying_key = signing_key.verifying_key();

        let signature = signing_key.sign(b"original_message");

        let pk_hex = hex::encode(verifying_key.as_bytes());
        let sig_hex = hex::encode(signature.to_bytes());

        assert!(!verify_ed25519("tampered_message", &sig_hex, &pk_hex));
    }

    #[test]
    fn load_signatures_returns_default_for_missing() {
        let file = load_signatures(Path::new("/tmp/nonexistent_signatures_test_12345.toml"));
        assert!(file.signatures.is_empty());
    }

    #[test]
    fn sign_and_persist_without_beardog_returns_false() {
        let tmp = std::env::temp_dir().join("depot_sign_test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("checksums.toml"), "# empty").unwrap();

        let original = std::env::var(cellmembrane_types::service::ENV_SOCKET_BASE).ok();
        unsafe {
            std::env::set_var(cellmembrane_types::service::ENV_SOCKET_BASE, "/nonexistent");
        }

        let result = sign_and_persist(&tmp);

        unsafe {
            match &original {
                Some(v) => std::env::set_var(cellmembrane_types::service::ENV_SOCKET_BASE, v),
                None => std::env::remove_var(cellmembrane_types::service::ENV_SOCKET_BASE),
            }
        }
        let _ = std::fs::remove_dir_all(&tmp);

        assert!(!result, "should return false when bearDog socket is unreachable");
    }

    #[test]
    fn verify_depot_signature_end_to_end() {
        use ed25519_dalek::{Signer, SigningKey};

        let tmp = std::env::temp_dir().join("depot_verify_e2e_test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let checksums_content = "# test checksums\n[x86_64-unknown-linux-musl]\nbeardog = { blake3 = \"abc\", size = 1024 }\n";
        std::fs::write(tmp.join("checksums.toml"), checksums_content).unwrap();

        let checksums_blake3 = blake3::hash(checksums_content.as_bytes())
            .to_hex()
            .to_string();

        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let verifying_key = signing_key.verifying_key();
        let signature = signing_key.sign(checksums_blake3.as_bytes());

        let sig = DepotSignature {
            algorithm: SignatureAlgorithm::Ed25519,
            public_key: hex::encode(verifying_key.as_bytes()),
            checksums_blake3,
            signature: hex::encode(signature.to_bytes()),
            signer_gate: "testGate".into(),
            signed_at: "2026-07-10T21:38:00Z".into(),
        };

        assert!(verify_depot_signature(&tmp, &sig));

        std::fs::write(tmp.join("checksums.toml"), "# tampered").unwrap();
        assert!(!verify_depot_signature(&tmp, &sig));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn verify_policy_integrity_only_always_passes() {
        let tmp = std::env::temp_dir().join("depot_policy_integrity_test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("checksums.toml"), "# test").unwrap();

        assert!(verify_depot_with_policy(&tmp, DepotTrustPolicy::IntegrityOnly));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn verify_policy_require_signed_fails_without_signatures() {
        let tmp = std::env::temp_dir().join("depot_policy_require_test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("checksums.toml"), "# test").unwrap();

        assert!(!verify_depot_with_policy(
            &tmp,
            DepotTrustPolicy::RequireSigned
        ));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn verify_policy_verify_if_present_passes_without_signatures() {
        let tmp = std::env::temp_dir().join("depot_policy_optional_test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("checksums.toml"), "# test").unwrap();

        assert!(verify_depot_with_policy(
            &tmp,
            DepotTrustPolicy::VerifyIfPresent
        ));

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
