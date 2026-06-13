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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsignalledPolicy {
    /// Log warning, fall through to legacy peek logic (Wave 111-112).
    Warn,
    /// Log error, fall through to legacy peek logic (Wave 112).
    Error,
    /// Reject with JSON-RPC error -32002 (Wave 113+).
    Reject,
}

impl Default for RiboCipherConfig {
    fn default() -> Self {
        Self {
            signal_tier: SignalTier::Clear,
            unsignalled_policy: UnsignalledPolicy::Warn,
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
            .map_or(UnsignalledPolicy::Warn, |s| match s {
                "error" => UnsignalledPolicy::Error,
                "reject" => UnsignalledPolicy::Reject,
                _ => UnsignalledPolicy::Warn,
            });

        Self {
            signal_tier,
            unsignalled_policy,
        }
    }

    /// Returns the wire prefix bytes for the configured tier and protocol.
    #[must_use]
    pub fn outbound_prefix(&self, protocol_type: u8) -> Vec<u8> {
        match self.signal_tier {
            SignalTier::Clear => vec![signal::CLEAR, protocol_type],
            SignalTier::Mito | SignalTier::Nuclear => {
                // Mito/Nuclear tiers require key material — not yet implemented.
                // Fall back to clear for now (safe: over-declaring is harmless).
                eprintln!(
                    "riboCipher: mito/nuclear tier configured but key derivation not yet \
                     implemented — falling back to clear signal"
                );
                vec![signal::CLEAR, protocol_type]
            }
        }
    }
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
    fn default_config_is_clear_warn() {
        let cfg = RiboCipherConfig::default();
        assert_eq!(cfg.signal_tier, SignalTier::Clear);
        assert_eq!(cfg.unsignalled_policy, UnsignalledPolicy::Warn);
    }

    #[test]
    fn from_toml_parses_section() {
        let toml_str = r#"
[transport.ribocipher]
signal_tier = "clear"
unsignalled_policy = "error"
"#;
        let parsed: toml::Table = toml_str.parse().unwrap();
        let cfg = RiboCipherConfig::from_toml(&parsed);
        assert_eq!(cfg.signal_tier, SignalTier::Clear);
        assert_eq!(cfg.unsignalled_policy, UnsignalledPolicy::Error);
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
        assert_eq!(cfg.unsignalled_policy, UnsignalledPolicy::Warn);
    }

    #[test]
    fn outbound_prefix_clear() {
        let cfg = RiboCipherConfig::default();
        let prefix = cfg.outbound_prefix(protocol::NDJSON_JSONRPC);
        assert_eq!(prefix, vec![0xEC, 0x01]);
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
