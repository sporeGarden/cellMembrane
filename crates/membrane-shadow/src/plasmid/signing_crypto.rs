// SPDX-License-Identifier: AGPL-3.0-or-later

//! Ed25519 signature primitives and bearDog UDS signing client.
//!
//! Low-level crypto operations used by the depot signing pipeline.
//! Verify uses `ed25519-dalek` directly (no bearDog required).
//! Sign delegates to bearDog's `crypto.sign_ed25519` via centralized `sync_ipc`.

/// Pure ed25519 verification using `ed25519-dalek`.
pub(super) fn verify_ed25519(message: &str, signature_hex: &str, public_key_hex: &str) -> bool {
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

    verifying_key.verify(message.as_bytes(), &signature).is_ok()
}

pub(super) struct SignResult {
    pub(super) public_key: String,
    pub(super) signature: String,
}

/// Request an ed25519 signature from the `CryptoSigner` capability holder via UDS.
///
/// Discovers the signer socket at runtime via `MembraneService::binary_for`.
pub(super) fn request_signer_sign(data: &str) -> Option<SignResult> {
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
    let response_bytes = crate::sync_ipc::ipc_request(&socket_path, &request_str)?;
    let response: serde_json::Value = serde_json::from_slice(&response_bytes).ok()?;
    let result = response.get("result")?;

    let pk_b64 = result.get("public_key")?.as_str()?;
    let sig_b64 = result.get("signature")?.as_str()?;
    let pk_bytes = base64::engine::general_purpose::STANDARD
        .decode(pk_b64)
        .ok()?;
    let sig_bytes = base64::engine::general_purpose::STANDARD
        .decode(sig_b64)
        .ok()?;

    Some(SignResult {
        public_key: hex::encode(pk_bytes),
        signature: hex::encode(sig_bytes),
    })
}

fn signer_socket_name() -> String {
    crate::impulse::signer_socket_name()
}
