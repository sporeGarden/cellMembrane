// SPDX-License-Identifier: AGPL-3.0-or-later

//! Shared cryptographic primitives — HKDF-SHA256, HMAC-SHA256.
//!
//! Consolidates identical implementations previously duplicated across
//! `btsp_client`, `ribocipher`, and `enroll_crypto`.

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// HKDF-SHA256 key derivation (extract-then-expand, single output block).
///
/// Implements RFC 5869 for a single 32-byte output key:
/// ```text
/// PRK = HMAC-SHA256(salt, IKM)
/// OKM = HMAC-SHA256(PRK, info || 0x01)
/// ```
///
/// HMAC-SHA256 accepts keys of any length, so construction is infallible.
pub(crate) fn hkdf_sha256(ikm: &[u8], salt: &[u8], info: &[u8]) -> [u8; 32] {
    let prk = hmac_sha256(salt, ikm);
    let mut expand_mac =
        HmacSha256::new_from_slice(&prk).expect("HMAC-SHA256 accepts any key length");
    expand_mac.update(info);
    expand_mac.update(&[0x01]);
    expand_mac.finalize().into_bytes().into()
}

/// Compute `HMAC-SHA256(key, message)` and return the 32-byte digest.
pub(crate) fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC-SHA256 accepts any key length");
    mac.update(message);
    mac.finalize().into_bytes().into()
}

/// Compute `HMAC-SHA256(key, message)` and return hex-encoded string.
pub(crate) fn hmac_sha256_hex(key: &[u8], message: &[u8]) -> String {
    hex::encode(hmac_sha256(key, message))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hkdf_produces_deterministic_output() {
        let k1 = hkdf_sha256(b"seed", b"salt", b"info");
        let k2 = hkdf_sha256(b"seed", b"salt", b"info");
        assert_eq!(k1, k2);
    }

    #[test]
    fn hkdf_different_info_produces_different_keys() {
        let k1 = hkdf_sha256(b"seed", b"salt", b"btsp-handshake");
        let k2 = hkdf_sha256(b"seed", b"salt", b"mito-signal");
        assert_ne!(k1, k2);
    }

    #[test]
    fn hmac_sha256_is_deterministic() {
        let h1 = hmac_sha256(b"key", b"message");
        let h2 = hmac_sha256(b"key", b"message");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 32);
    }

    #[test]
    fn hmac_sha256_hex_matches_raw() {
        let raw = hmac_sha256(b"key", b"msg");
        let hex_str = hmac_sha256_hex(b"key", b"msg");
        assert_eq!(hex_str, hex::encode(raw));
    }
}
