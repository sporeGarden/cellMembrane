// SPDX-License-Identifier: AGPL-3.0-or-later

//! riboCipher — Transport Signal Standard implementation for cellMembrane.
//!
//! Models the biological signal peptide system: connections declare their
//! intended protocol via an intentional signal envelope before any payload.
//! The accept loop reads the signal and routes deterministically.
//!
//! # Wire Format
//!
//! Tier 1 (Clear): `[0xEC][protocol_type: u8]` — 2 bytes, local UDS
//! Tier 2 (Mito):  `[0xED][hmac_tag: [u8; 4]]` — 5 bytes, cross-gate WAN
//! Tier 3 (Nuclear): `[0xEE][encrypted: [u8; 6]]` — 7 bytes, privileged
//!
//! cellMembrane uses Tier 1 (Clear) for all local UDS IPC, since all
//! connections are same-gate trusted paths over Unix domain sockets.

/// Signal tier prefix bytes.
pub mod signal {
    /// Clear signal — local same-gate IPC where the wire is trusted.
    pub const CLEAR: u8 = 0xEC;

    /// Mito-obfuscated — cross-gate WAN connections (family seed HMAC).
    #[allow(dead_code)]
    pub const MITO: u8 = 0xED;

    /// Nuclear-sealed — privileged protocol negotiation.
    #[allow(dead_code)]
    pub const NUCLEAR: u8 = 0xEE;
}

/// Protocol type identifiers (second byte after the signal prefix).
pub mod protocol {
    /// Lightweight health probe.
    #[allow(dead_code)]
    pub const PROBE: u8 = 0x00;

    /// NDJSON JSON-RPC — standard ecosystem IPC.
    pub const NDJSON_JSONRPC: u8 = 0x01;

    /// BTSP Binary — length-prefixed binary handshake.
    #[allow(dead_code)]
    pub const BTSP_BINARY: u8 = 0x02;

    /// BTSP JSON-line — JSON-line `ClientHello` handshake.
    #[allow(dead_code)]
    pub const BTSP_JSON_LINE: u8 = 0x03;

    /// HTTP/1.1 — axum/hyper over UDS.
    #[allow(dead_code)]
    pub const HTTP: u8 = 0x04;

    /// Encrypted Resume — post-BTSP session resume.
    #[allow(dead_code)]
    pub const ENCRYPTED_RESUME: u8 = 0x05;

    /// Dark Forest Beacon — birdsong beacon packet.
    #[allow(dead_code)]
    pub const DARK_FOREST_BEACON: u8 = 0x06;

    /// Mesh Relay — songBird relay-routed frame.
    #[allow(dead_code)]
    pub const MESH_RELAY: u8 = 0x07;
}

/// Clear signal prefix for NDJSON JSON-RPC over UDS.
///
/// This is the standard prefix cellMembrane prepends to all outbound
/// UDS JSON-RPC connections: `[0xEC, 0x01]`.
pub const CLEAR_JSONRPC_SIGNAL: [u8; 2] = [signal::CLEAR, protocol::NDJSON_JSONRPC];

/// Configuration for riboCipher transport behavior.
///
/// Maps to `[transport.ribocipher]` in `membrane.toml`.
#[derive(Debug, Clone)]
pub struct RiboCipherConfig {
    /// Signal tier for outbound connections ("clear", "mito", "nuclear").
    pub signal_tier: SignalTier,
    /// Policy for inbound connections without a riboCipher signal.
    pub unsignalled_policy: UnsignalledPolicy,
    /// Derived mito key (32 bytes from HKDF), if family seed is available.
    mito_key: Option<[u8; 32]>,
}

/// Signal tier for outbound connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalTier {
    /// Tier 1: clear signal, suitable for local UDS.
    Clear,
    /// Tier 2: mito-obfuscated, suitable for cross-gate WAN.
    Mito,
    /// Tier 3: nuclear-sealed, privileged.
    Nuclear,
}

/// Policy for unsignalled (legacy) inbound connections.
/// Deprecation schedule: WARN (111) → ERROR (112) → REJECT (113) → REMOVE (114).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsignalledPolicy {
    /// Log warning, fall through to legacy peek logic (deprecated — Wave 111 only).
    Warn,
    /// Log error, fall through to legacy peek logic (Wave 112 default).
    Error,
    /// Reject with JSON-RPC error -32002 (Wave 113+).
    Reject,
}

impl Default for RiboCipherConfig {
    fn default() -> Self {
        Self {
            signal_tier: SignalTier::Clear,
            unsignalled_policy: UnsignalledPolicy::Reject,
            mito_key: None,
        }
    }
}

impl RiboCipherConfig {
    /// Load from a parsed TOML table (from `membrane.toml`).
    #[must_use]
    pub fn from_toml(table: &toml::Table) -> Self {
        let section = table
            .get("transport")
            .and_then(|t| t.as_table())
            .and_then(|t| t.get("ribocipher"))
            .and_then(|r| r.as_table());

        let Some(rc) = section else {
            return Self::default();
        };

        let signal_tier =
            rc.get("signal_tier")
                .and_then(|v| v.as_str())
                .map_or(SignalTier::Clear, |s| match s {
                    "mito" => SignalTier::Mito,
                    "nuclear" => SignalTier::Nuclear,
                    _ => SignalTier::Clear,
                });

        let unsignalled_policy = rc
            .get("unsignalled_policy")
            .and_then(|v| v.as_str())
            .map_or(UnsignalledPolicy::Reject, |s| match s {
                "warn" => UnsignalledPolicy::Warn,
                "error" => UnsignalledPolicy::Error,
                _ => UnsignalledPolicy::Reject,
            });

        let mito_key = derive_mito_key_from_env();

        Self {
            signal_tier,
            unsignalled_policy,
            mito_key,
        }
    }

    /// Construct with an explicit mito key (for testing or pre-derived contexts).
    #[must_use]
    pub const fn with_mito_key(mut self, key: [u8; 32]) -> Self {
        self.mito_key = Some(key);
        self
    }

    /// Whether legacy fallback (raw JSON without signal) should be attempted.
    ///
    /// In `Reject` mode (Wave 113+), no fallback — if the peer doesn't respond
    /// to the riboCipher signal, the connection fails immediately.
    #[must_use]
    pub const fn allows_fallback(&self) -> bool {
        matches!(
            self.unsignalled_policy,
            UnsignalledPolicy::Warn | UnsignalledPolicy::Error
        )
    }

    /// JSON-RPC error code for rejected unsignalled connections.
    pub const REJECT_ERROR_CODE: i32 = -32002;

    /// Returns the wire prefix bytes for the configured tier and protocol.
    #[must_use]
    pub fn outbound_prefix(&self, protocol_type: u8) -> Vec<u8> {
        match self.signal_tier {
            SignalTier::Clear => vec![signal::CLEAR, protocol_type],
            SignalTier::Mito => {
                if let Some(key) = &self.mito_key {
                    let tag = mito_hmac_tag(key, protocol_type);
                    let mut prefix = Vec::with_capacity(5);
                    prefix.push(signal::MITO);
                    prefix.extend_from_slice(&tag);
                    prefix
                } else {
                    vec![signal::CLEAR, protocol_type]
                }
            }
            SignalTier::Nuclear => {
                // Nuclear tier requires per-peer lineage keys (deferred).
                // Fall back to mito if key material available, otherwise clear.
                if let Some(key) = &self.mito_key {
                    let tag = mito_hmac_tag(key, protocol_type);
                    let mut prefix = Vec::with_capacity(5);
                    prefix.push(signal::MITO);
                    prefix.extend_from_slice(&tag);
                    prefix
                } else {
                    vec![signal::CLEAR, protocol_type]
                }
            }
        }
    }

    /// Verify a mito-tier signal tag against known protocol types.
    ///
    /// Returns the protocol type if the tag matches any known type, or `None`.
    #[must_use]
    pub fn verify_mito_tag(&self, tag: &[u8; 4]) -> Option<u8> {
        let key = self.mito_key.as_ref()?;
        (0x00..=0x07).find(|&proto| mito_hmac_tag(key, proto) == *tag)
    }
}

// ── Key derivation ─────────────────────────────────────────────────────

/// HKDF-SHA256 salt for riboCipher key derivation.
const HKDF_SALT: &[u8] = b"ribocipher-v1";

/// HKDF info parameter for mito-tier signal key.
const HKDF_INFO_MITO: &[u8] = b"mito-signal";

/// Derive the mito key from the `FAMILY_SEED` environment variable.
///
/// Reads family seed from:
/// 1. `FAMILY_SEED` env var (may be a path to a key file, or inline seed)
/// 2. Falls back gracefully to `None` if unavailable.
fn derive_mito_key_from_env() -> Option<[u8; 32]> {
    let seed_source = std::env::var("FAMILY_SEED").ok()?;
    let seed_bytes = if std::path::Path::new(&seed_source).exists() {
        std::fs::read(&seed_source).ok()?
    } else {
        seed_source.into_bytes()
    };
    Some(hkdf_sha256(&seed_bytes, HKDF_SALT, HKDF_INFO_MITO))
}

/// HKDF-SHA256 key derivation (extract-then-expand, single output block).
///
/// Produces a 32-byte derived key. Uses HMAC-SHA256 internally per RFC 5869.
fn hkdf_sha256(ikm: &[u8], salt: &[u8], info: &[u8]) -> [u8; 32] {
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;

    // Extract: PRK = HMAC-SHA256(salt, IKM)
    let mut extract_mac = HmacSha256::new_from_slice(salt).expect("HMAC can take key of any size");
    extract_mac.update(ikm);
    let prk = extract_mac.finalize().into_bytes();

    // Expand: OKM = HMAC-SHA256(PRK, info || 0x01)  [single block, 32 bytes]
    let mut expand_mac = HmacSha256::new_from_slice(&prk).expect("HMAC can take key of any size");
    expand_mac.update(info);
    expand_mac.update(&[0x01]);
    let okm = expand_mac.finalize().into_bytes();

    let mut key = [0u8; 32];
    key.copy_from_slice(&okm);
    key
}

/// Compute the 4-byte HMAC tag for a mito-tier signal.
///
/// `tag = HMAC-SHA256(mito_key, [protocol_type])[0..4]`
fn mito_hmac_tag(mito_key: &[u8; 32], protocol_type: u8) -> [u8; 4] {
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;

    let mut mac = HmacSha256::new_from_slice(mito_key).expect("HMAC can take key of any size");
    mac.update(&[protocol_type]);
    let result = mac.finalize().into_bytes();

    let mut tag = [0u8; 4];
    tag.copy_from_slice(&result[..4]);
    tag
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_signal_is_two_bytes() {
        assert_eq!(CLEAR_JSONRPC_SIGNAL, [0xEC, 0x01]);
    }

    #[test]
    fn signal_bytes_avoid_json_and_http() {
        assert!(signal::CLEAR > 0x7F, "must not be ASCII printable");
        assert!(signal::MITO > 0x7F);
        assert!(signal::NUCLEAR > 0x7F);
        assert_ne!(signal::CLEAR, b'{');
        assert_ne!(signal::CLEAR, b'[');
    }

    #[test]
    fn default_config_is_clear_reject() {
        let cfg = RiboCipherConfig::default();
        assert_eq!(cfg.signal_tier, SignalTier::Clear);
        assert_eq!(cfg.unsignalled_policy, UnsignalledPolicy::Reject);
    }

    #[test]
    fn from_toml_parses_section() {
        let toml_str = r#"
[transport.ribocipher]
signal_tier = "clear"
unsignalled_policy = "reject"
"#;
        let parsed: toml::Table = toml_str.parse().unwrap();
        let cfg = RiboCipherConfig::from_toml(&parsed);
        assert_eq!(cfg.signal_tier, SignalTier::Clear);
        assert_eq!(cfg.unsignalled_policy, UnsignalledPolicy::Reject);
    }

    #[test]
    fn from_toml_handles_missing_section() {
        let toml_str = r#"
[membrane]
name = "test"
"#;
        let parsed: toml::Table = toml_str.parse().unwrap();
        let cfg = RiboCipherConfig::from_toml(&parsed);
        assert_eq!(cfg.signal_tier, SignalTier::Clear);
        assert_eq!(cfg.unsignalled_policy, UnsignalledPolicy::Reject);
    }

    #[test]
    fn from_toml_explicit_warn_still_valid() {
        let toml_str = r#"
[transport.ribocipher]
signal_tier = "clear"
unsignalled_policy = "warn"
"#;
        let parsed: toml::Table = toml_str.parse().unwrap();
        let cfg = RiboCipherConfig::from_toml(&parsed);
        assert_eq!(cfg.unsignalled_policy, UnsignalledPolicy::Warn);
    }

    #[test]
    fn reject_disallows_fallback() {
        let cfg = RiboCipherConfig::default();
        assert_eq!(cfg.unsignalled_policy, UnsignalledPolicy::Reject);
        assert!(!cfg.allows_fallback());
    }

    #[test]
    fn error_allows_fallback() {
        let toml_str = r#"
[transport.ribocipher]
unsignalled_policy = "error"
"#;
        let parsed: toml::Table = toml_str.parse().unwrap();
        let cfg = RiboCipherConfig::from_toml(&parsed);
        assert!(cfg.allows_fallback());
    }

    #[test]
    fn outbound_prefix_clear() {
        let cfg = RiboCipherConfig::default();
        let prefix = cfg.outbound_prefix(protocol::NDJSON_JSONRPC);
        assert_eq!(prefix, vec![0xEC, 0x01]);
    }

    #[test]
    fn outbound_prefix_mito_with_key() {
        let key = hkdf_sha256(b"test-family-seed", HKDF_SALT, HKDF_INFO_MITO);
        let cfg = RiboCipherConfig {
            signal_tier: SignalTier::Mito,
            unsignalled_policy: UnsignalledPolicy::Warn,
            mito_key: Some(key),
        };
        let prefix = cfg.outbound_prefix(protocol::NDJSON_JSONRPC);
        assert_eq!(prefix.len(), 5);
        assert_eq!(prefix[0], signal::MITO);
    }

    #[test]
    fn outbound_prefix_mito_without_key_falls_back() {
        let cfg = RiboCipherConfig {
            signal_tier: SignalTier::Mito,
            unsignalled_policy: UnsignalledPolicy::Warn,
            mito_key: None,
        };
        let prefix = cfg.outbound_prefix(protocol::NDJSON_JSONRPC);
        assert_eq!(prefix, vec![signal::CLEAR, protocol::NDJSON_JSONRPC]);
    }

    #[test]
    fn mito_tag_is_deterministic() {
        let key = hkdf_sha256(b"determinism-test", HKDF_SALT, HKDF_INFO_MITO);
        let tag1 = mito_hmac_tag(&key, protocol::NDJSON_JSONRPC);
        let tag2 = mito_hmac_tag(&key, protocol::NDJSON_JSONRPC);
        assert_eq!(tag1, tag2);
    }

    #[test]
    fn mito_tags_differ_by_protocol() {
        let key = hkdf_sha256(b"protocol-diff", HKDF_SALT, HKDF_INFO_MITO);
        let tag_json = mito_hmac_tag(&key, protocol::NDJSON_JSONRPC);
        let tag_http = mito_hmac_tag(&key, protocol::HTTP);
        assert_ne!(tag_json, tag_http);
    }

    #[test]
    fn verify_mito_tag_roundtrip() {
        let key = hkdf_sha256(b"verify-roundtrip", HKDF_SALT, HKDF_INFO_MITO);
        let cfg = RiboCipherConfig {
            signal_tier: SignalTier::Mito,
            unsignalled_policy: UnsignalledPolicy::Warn,
            mito_key: Some(key),
        };
        let tag = mito_hmac_tag(&key, protocol::BTSP_BINARY);
        let verified = cfg.verify_mito_tag(&tag);
        assert_eq!(verified, Some(protocol::BTSP_BINARY));
    }

    #[test]
    fn verify_mito_tag_rejects_invalid() {
        let key = hkdf_sha256(b"reject-test", HKDF_SALT, HKDF_INFO_MITO);
        let cfg = RiboCipherConfig {
            signal_tier: SignalTier::Mito,
            unsignalled_policy: UnsignalledPolicy::Warn,
            mito_key: Some(key),
        };
        let bad_tag = [0xFF, 0xFF, 0xFF, 0xFF];
        assert_eq!(cfg.verify_mito_tag(&bad_tag), None);
    }

    #[test]
    fn hkdf_produces_32_bytes() {
        let key = hkdf_sha256(b"input-material", b"salt", b"info");
        assert_eq!(key.len(), 32);
        assert_ne!(key, [0u8; 32]);
    }

    #[test]
    fn hkdf_different_info_different_key() {
        let k1 = hkdf_sha256(b"same-ikm", b"same-salt", b"info-a");
        let k2 = hkdf_sha256(b"same-ikm", b"same-salt", b"info-b");
        assert_ne!(k1, k2);
    }

    #[test]
    fn protocol_types_are_distinct() {
        let types = [
            protocol::PROBE,
            protocol::NDJSON_JSONRPC,
            protocol::BTSP_BINARY,
            protocol::BTSP_JSON_LINE,
            protocol::HTTP,
            protocol::ENCRYPTED_RESUME,
            protocol::DARK_FOREST_BEACON,
            protocol::MESH_RELAY,
        ];
        let mut seen = std::collections::HashSet::new();
        for t in types {
            assert!(seen.insert(t), "duplicate protocol type: {t:#x}");
        }
    }
}
