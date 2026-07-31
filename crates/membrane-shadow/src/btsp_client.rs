// SPDX-License-Identifier: AGPL-3.0-or-later

//! BTSP (`BearDog` Transport Security Protocol) client handshake.
//!
//! Implements the 4-step BTSP `ClientHello` handshake required for
//! authenticated communication with bearDog over UDS. All primals that
//! call bearDog crypto endpoints must perform this handshake before
//! sending JSON-RPC requests.
//!
//! # Protocol
//!
//! 1. Client → Server: `ClientHello { protocol, version, client_ephemeral_pub }`
//! 2. Server → Client: `ServerHello { challenge, session_id }`
//! 3. Client → Server: `ChallengeResponse { session_id, hmac }`
//! 4. Server → Client: `HandshakeComplete { cipher, session_id }`
//!
//! The HMAC is `HMAC-SHA256(FAMILY_SEED, challenge)`, proving the client
//! belongs to the same ecosystem family.

use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

/// BTSP protocol version.
const BTSP_VERSION: u32 = 1;

/// HKDF info parameter for BTSP challenge-response key derivation.
const HKDF_INFO_BTSP: &[u8] = b"btsp-challenge";

/// HKDF salt (shared with ribocipher for consistency).
const HKDF_SALT: &[u8] = b"ribocipher-v1";

// ── Handshake message types ───────────────────────────────────────────

#[derive(Debug, Serialize)]
struct ClientHello {
    protocol: &'static str,
    version: u32,
    client_ephemeral_pub: String,
}

#[derive(Debug, Deserialize)]
struct ServerHello {
    challenge: String,
    session_id: String,
}

#[derive(Debug, Serialize)]
struct ChallengeResponse {
    session_id: String,
    hmac: String,
}

#[derive(Debug, Deserialize)]
pub struct HandshakeComplete {
    /// Negotiated cipher suite (informational).
    #[serde(default)]
    pub cipher: Option<String>,
    /// Session identifier for this authenticated connection.
    pub session_id: String,
}

// ── Sync handshake (for signing.rs, impulse/primal.rs) ────────────────

/// Perform BTSP handshake over a connected sync `UnixStream`.
///
/// On success, the stream is authenticated and ready for JSON-RPC.
/// On failure, the stream should be dropped (bearDog will reject further traffic).
#[cfg(unix)]
pub fn handshake_sync(stream: &mut std::os::unix::net::UnixStream) -> Option<HandshakeComplete> {
    use std::io::{BufRead, BufReader, Write};

    let ephemeral_pub = generate_ephemeral_pub();
    let btsp_key = derive_btsp_key()?;

    // Step 1: Send ClientHello
    let hello = ClientHello {
        protocol: "btsp",
        version: BTSP_VERSION,
        client_ephemeral_pub: ephemeral_pub,
    };
    let hello_json = serde_json::to_string(&hello).ok()?;
    writeln!(stream, "{hello_json}").ok()?;

    // Step 2: Read ServerHello
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    if line.trim().is_empty() {
        warn!("BTSP: empty ServerHello response");
        return None;
    }
    let server_hello: ServerHello = serde_json::from_str(line.trim()).ok()?;

    // Step 3: Compute HMAC and send ChallengeResponse
    let hmac_hex = compute_challenge_hmac(&btsp_key, &server_hello.challenge);
    let response = ChallengeResponse {
        session_id: server_hello.session_id,
        hmac: hmac_hex,
    };
    let response_json = serde_json::to_string(&response).ok()?;
    writeln!(stream, "{response_json}").ok()?;

    // Step 4: Read HandshakeComplete
    let mut complete_line = String::new();
    reader.read_line(&mut complete_line).ok()?;
    if complete_line.trim().is_empty() {
        warn!("BTSP: empty HandshakeComplete response");
        return None;
    }
    let complete: HandshakeComplete = serde_json::from_str(complete_line.trim()).ok()?;

    debug!(
        session_id = %complete.session_id,
        cipher = ?complete.cipher,
        "BTSP handshake complete"
    );
    Some(complete)
}

// ── Async handshake (for jsonrpc.rs) ──────────────────────────────────

/// Perform BTSP handshake over a connected async `UnixStream`.
///
/// On success, returns the `HandshakeComplete` and the stream is ready
/// for JSON-RPC traffic. Caller must split the stream after this returns.
#[cfg(unix)]
pub async fn handshake_async(stream: &mut tokio::net::UnixStream) -> Option<HandshakeComplete> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

    let ephemeral_pub = generate_ephemeral_pub();
    let btsp_key = derive_btsp_key()?;

    // Step 1: Send ClientHello
    let hello = ClientHello {
        protocol: "btsp",
        version: BTSP_VERSION,
        client_ephemeral_pub: ephemeral_pub,
    };
    let hello_json = serde_json::to_string(&hello).ok()?;
    stream
        .write_all(format!("{hello_json}\n").as_bytes())
        .await
        .ok()?;

    // Step 2: Read ServerHello
    let server_hello: ServerHello = {
        let mut line = String::new();
        let mut buf = tokio::io::BufReader::new(&mut *stream);
        buf.read_line(&mut line).await.ok()?;
        if line.trim().is_empty() {
            warn!("BTSP: empty ServerHello response");
            return None;
        }
        serde_json::from_str(line.trim()).ok()?
    };

    // Step 3: Compute HMAC and send ChallengeResponse
    let hmac_hex = compute_challenge_hmac(&btsp_key, &server_hello.challenge);
    let response = ChallengeResponse {
        session_id: server_hello.session_id,
        hmac: hmac_hex,
    };
    let response_json = serde_json::to_string(&response).ok()?;
    stream
        .write_all(format!("{response_json}\n").as_bytes())
        .await
        .ok()?;

    // Step 4: Read HandshakeComplete
    let complete: HandshakeComplete = {
        let mut line = String::new();
        let mut buf = tokio::io::BufReader::new(&mut *stream);
        buf.read_line(&mut line).await.ok()?;
        if line.trim().is_empty() {
            warn!("BTSP: empty HandshakeComplete response");
            return None;
        }
        serde_json::from_str(line.trim()).ok()?
    };

    debug!(
        session_id = %complete.session_id,
        cipher = ?complete.cipher,
        "BTSP handshake complete (async)"
    );
    Some(complete)
}

// ── Crypto helpers ────────────────────────────────────────────────────

/// Generate a 32-byte random ephemeral public key (hex-encoded).
fn generate_ephemeral_pub() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("CSPRNG unavailable");
    hex::encode(bytes)
}

/// Derive the BTSP challenge-response key from the family seed.
///
/// Uses HKDF-SHA256 with a BTSP-specific info parameter, so the derived
/// key is distinct from riboCipher mito keys.
fn derive_btsp_key() -> Option<[u8; 32]> {
    let seed_source = cellmembrane_types::service::resolve_family_seed_env()?;
    let seed_bytes = if std::path::Path::new(&seed_source).exists() {
        std::fs::read(&seed_source).ok()?
    } else {
        seed_source.into_bytes()
    };
    Some(hkdf_sha256(&seed_bytes, HKDF_SALT, HKDF_INFO_BTSP))
}

/// Compute `HMAC-SHA256(btsp_key, challenge)` and return hex-encoded result.
fn compute_challenge_hmac(btsp_key: &[u8; 32], challenge: &str) -> String {
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;

    let mut mac = HmacSha256::new_from_slice(btsp_key).expect("HMAC-SHA256 accepts any key length");
    mac.update(challenge.as_bytes());
    let result = mac.finalize().into_bytes();
    hex::encode(result)
}

/// HKDF-SHA256 key derivation (extract-then-expand, single output block).
fn hkdf_sha256(ikm: &[u8], salt: &[u8], info: &[u8]) -> [u8; 32] {
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;

    let mut extract_mac =
        HmacSha256::new_from_slice(salt).expect("HMAC-SHA256 accepts any key length");
    extract_mac.update(ikm);
    let prk = extract_mac.finalize().into_bytes();

    let mut expand_mac =
        HmacSha256::new_from_slice(&prk).expect("HMAC-SHA256 accepts any key length");
    expand_mac.update(info);
    expand_mac.update(&[0x01]);
    let okm = expand_mac.finalize().into_bytes();

    let mut key = [0u8; 32];
    key.copy_from_slice(&okm);
    key
}

/// BTSP signal prefix: `[0xEC, 0x03]` (Clear tier, BTSP JSON-line protocol).
pub const BTSP_JSONLINE_SIGNAL: [u8; 2] = [
    crate::ribocipher::signal::CLEAR,
    crate::ribocipher::protocol::BTSP_JSON_LINE,
];

/// Whether BTSP is available (family seed is configured).
#[must_use]
pub fn is_available() -> bool {
    derive_btsp_key().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ephemeral_pub_is_64_hex_chars() {
        let pub_key = generate_ephemeral_pub();
        assert_eq!(pub_key.len(), 64);
        assert!(pub_key.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn ephemeral_pub_is_random() {
        let k1 = generate_ephemeral_pub();
        let k2 = generate_ephemeral_pub();
        assert_ne!(k1, k2);
    }

    #[test]
    fn btsp_key_derivation_is_deterministic() {
        let seed = b"test-seed-123";
        let k1 = hkdf_sha256(seed, HKDF_SALT, HKDF_INFO_BTSP);
        let k2 = hkdf_sha256(seed, HKDF_SALT, HKDF_INFO_BTSP);
        assert_eq!(k1, k2);
        assert_ne!(k1, [0u8; 32]);
    }

    #[test]
    fn btsp_key_differs_from_mito_key() {
        let seed = b"same-family-seed";
        let btsp = hkdf_sha256(seed, HKDF_SALT, HKDF_INFO_BTSP);
        let mito = hkdf_sha256(seed, HKDF_SALT, b"mito-signal");
        assert_ne!(btsp, mito);
    }

    #[test]
    fn challenge_hmac_is_deterministic() {
        let key = hkdf_sha256(b"test-seed", HKDF_SALT, HKDF_INFO_BTSP);
        let h1 = compute_challenge_hmac(&key, "challenge-abc");
        let h2 = compute_challenge_hmac(&key, "challenge-abc");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);
    }

    #[test]
    fn challenge_hmac_differs_by_challenge() {
        let key = hkdf_sha256(b"test-seed", HKDF_SALT, HKDF_INFO_BTSP);
        let h1 = compute_challenge_hmac(&key, "challenge-1");
        let h2 = compute_challenge_hmac(&key, "challenge-2");
        assert_ne!(h1, h2);
    }

    #[test]
    fn btsp_signal_prefix() {
        assert_eq!(BTSP_JSONLINE_SIGNAL, [0xEC, 0x03]);
    }

    #[test]
    fn client_hello_serializes() {
        let hello = ClientHello {
            protocol: "btsp",
            version: BTSP_VERSION,
            client_ephemeral_pub: "aa".repeat(32),
        };
        let json = serde_json::to_string(&hello).unwrap();
        assert!(json.contains("\"protocol\":\"btsp\""));
        assert!(json.contains("\"version\":1"));
        assert!(json.contains("\"client_ephemeral_pub\""));
    }

    #[test]
    fn server_hello_deserializes() {
        let json = r#"{"challenge":"abc123","session_id":"sess-1"}"#;
        let hello: ServerHello = serde_json::from_str(json).unwrap();
        assert_eq!(hello.challenge, "abc123");
        assert_eq!(hello.session_id, "sess-1");
    }

    #[test]
    fn handshake_complete_deserializes() {
        let json = r#"{"cipher":"aes-256-gcm","session_id":"sess-1"}"#;
        let complete: HandshakeComplete = serde_json::from_str(json).unwrap();
        assert_eq!(complete.cipher.as_deref(), Some("aes-256-gcm"));
        assert_eq!(complete.session_id, "sess-1");
    }

    #[test]
    fn handshake_complete_optional_cipher() {
        let json = r#"{"session_id":"sess-2"}"#;
        let complete: HandshakeComplete = serde_json::from_str(json).unwrap();
        assert!(complete.cipher.is_none());
        assert_eq!(complete.session_id, "sess-2");
    }
}
