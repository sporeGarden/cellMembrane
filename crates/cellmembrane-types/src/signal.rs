// SPDX-License-Identifier: AGPL-3.0-or-later

//! riboCipher Signal Acceptance — centralized pattern for primal accept loops.
//!
//! Every primal that listens on a Unix Domain Socket must handle inbound
//! connections that begin with a riboCipher signal envelope. This module
//! provides the types and logic to detect, validate, and strip the signal
//! before handing the stream to the protocol handler.
//!
//! # Wire Format
//!
//! ```text
//! Tier 1 (Clear):   [0xEC][protocol_type: u8]        — 2 bytes
//! Tier 2 (Mito):    [0xED][hmac_tag: [u8; 4]]        — 5 bytes
//! Tier 3 (Nuclear): [0xEE][encrypted: [u8; 6]]       — 7 bytes
//! ```
//!
//! # Usage (in a primal's accept loop)
//!
//! ```rust,no_run
//! use cellmembrane_types::signal::{SignalResult, peek_signal};
//!
//! // After accepting a UDS connection, peek the first bytes:
//! let buf: [u8; 2] = [0xEC, 0x01]; // read from socket
//! match peek_signal(&buf) {
//!     SignalResult::Clear(proto) => {
//!         // Strip 2 bytes, handle protocol `proto`
//!     }
//!     SignalResult::Mito => {
//!         // Need 5 bytes total; validate HMAC tag
//!     }
//!     SignalResult::Nuclear => {
//!         // Need 7 bytes total; decrypt envelope
//!     }
//!     SignalResult::NotSignalled => {
//!         // Legacy connection — no signal prefix (apply unsignalled policy)
//!     }
//! }
//! ```

use serde::{Deserialize, Serialize};

/// Signal tier prefix bytes — the first byte of every riboCipher envelope.
pub mod prefix {
    /// Clear signal — trusted same-gate IPC.
    pub const CLEAR: u8 = 0xEC;
    /// Mito-obfuscated — family-seed HMAC'd cross-gate connection.
    pub const MITO: u8 = 0xED;
    /// Nuclear-sealed — privileged encrypted protocol.
    pub const NUCLEAR: u8 = 0xEE;
}

/// Protocol identifiers (second byte in a Clear signal envelope).
pub mod protocol {
    /// Lightweight health probe.
    pub const PROBE: u8 = 0x00;
    /// NDJSON JSON-RPC — standard ecosystem IPC.
    pub const JSONRPC: u8 = 0x01;
    /// BTSP Binary — length-prefixed binary handshake.
    pub const BTSP_BINARY: u8 = 0x02;
    /// BTSP JSON-line handshake.
    pub const BTSP_JSON_LINE: u8 = 0x03;
    /// HTTP/1.1 over UDS.
    pub const HTTP: u8 = 0x04;
    /// Encrypted resume.
    pub const ENCRYPTED_RESUME: u8 = 0x05;
    /// Dark Forest beacon.
    pub const DARK_FOREST_BEACON: u8 = 0x06;
    /// Mesh relay frame.
    pub const MESH_RELAY: u8 = 0x07;
}

/// Standard clear signal for JSON-RPC: `[0xEC, 0x01]`.
pub const CLEAR_JSONRPC: [u8; 2] = [prefix::CLEAR, protocol::JSONRPC];

/// Result of peeking the first byte(s) of an inbound connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalResult {
    /// Clear signal detected — contains the protocol identifier.
    /// Caller should strip 2 bytes from the stream.
    Clear(u8),
    /// Mito signal detected — caller needs 5 bytes total to validate.
    Mito,
    /// Nuclear signal detected — caller needs 7 bytes total to decrypt.
    Nuclear,
    /// No riboCipher signal — legacy or malformed connection.
    NotSignalled,
}

/// Peek the first byte(s) and determine the signal type.
///
/// This is the core accept-side logic. Call with the first 1-2 bytes
/// read from an incoming connection.
#[must_use]
pub fn peek_signal(buf: &[u8]) -> SignalResult {
    match buf.first() {
        Some(&prefix::CLEAR) => {
            let proto = buf.get(1).copied().unwrap_or(protocol::JSONRPC);
            SignalResult::Clear(proto)
        }
        Some(&prefix::MITO) => SignalResult::Mito,
        Some(&prefix::NUCLEAR) => SignalResult::Nuclear,
        _ => SignalResult::NotSignalled,
    }
}

/// Envelope length for a given signal tier prefix byte.
///
/// Returns `None` if the byte is not a valid signal prefix.
#[must_use]
pub const fn envelope_len(prefix_byte: u8) -> Option<usize> {
    match prefix_byte {
        prefix::CLEAR => Some(2),
        prefix::MITO => Some(5),
        prefix::NUCLEAR => Some(7),
        _ => None,
    }
}

/// Policy for handling connections that arrive WITHOUT a signal prefix.
///
/// As of Wave 114, the default is `Reject`. Primals should adopt this
/// via the centralized accept pattern.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnsignalledPolicy {
    /// Accept with warning log (deprecated — migration only).
    Warn,
    /// Accept with error log (transitional).
    Error,
    /// Reject immediately (Wave 114+ default).
    #[default]
    Reject,
}

impl UnsignalledPolicy {
    /// Whether this policy allows the connection to proceed.
    #[must_use]
    pub const fn allows_connection(self) -> bool {
        matches!(self, Self::Warn | Self::Error)
    }
}

/// Acceptance decision after signal inspection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceptDecision {
    /// Connection is signalled and valid — proceed with `skip_bytes` consumed.
    Accept {
        /// Number of bytes to skip (envelope length).
        skip_bytes: usize,
        /// Declared protocol type.
        protocol: u8,
    },
    /// Connection has no signal — proceed per `UnsignalledPolicy` (legacy fallback).
    Unsignalled,
    /// Connection should be rejected (unknown signal or policy denial).
    Reject,
}

/// Make an acceptance decision given a peek buffer and policy.
///
/// This is the recommended function for primal accept loops:
///
/// ```rust
/// use cellmembrane_types::signal::{accept_decision, UnsignalledPolicy, AcceptDecision};
///
/// let first_bytes = [0xEC, 0x01]; // peeked from socket
/// let decision = accept_decision(&first_bytes, UnsignalledPolicy::Reject);
/// assert_eq!(decision, AcceptDecision::Accept { skip_bytes: 2, protocol: 0x01 });
/// ```
#[must_use]
pub fn accept_decision(peek_buf: &[u8], policy: UnsignalledPolicy) -> AcceptDecision {
    match peek_signal(peek_buf) {
        SignalResult::Clear(proto) => AcceptDecision::Accept {
            skip_bytes: 2,
            protocol: proto,
        },
        SignalResult::Mito => AcceptDecision::Accept {
            skip_bytes: 5,
            protocol: protocol::JSONRPC,
        },
        SignalResult::Nuclear => AcceptDecision::Accept {
            skip_bytes: 7,
            protocol: protocol::JSONRPC,
        },
        SignalResult::NotSignalled => {
            if policy.allows_connection() {
                AcceptDecision::Unsignalled
            } else {
                AcceptDecision::Reject
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peek_clear_jsonrpc() {
        assert_eq!(peek_signal(&[0xEC, 0x01]), SignalResult::Clear(0x01));
    }

    #[test]
    fn peek_clear_probe() {
        assert_eq!(peek_signal(&[0xEC, 0x00]), SignalResult::Clear(0x00));
    }

    #[test]
    fn peek_mito() {
        assert_eq!(peek_signal(&[0xED, 0xAA, 0xBB]), SignalResult::Mito);
    }

    #[test]
    fn peek_nuclear() {
        assert_eq!(peek_signal(&[0xEE, 0x00]), SignalResult::Nuclear);
    }

    #[test]
    fn peek_not_signalled() {
        assert_eq!(peek_signal(&[0x7B]), SignalResult::NotSignalled); // '{' — raw JSON
        assert_eq!(peek_signal(&[]), SignalResult::NotSignalled);
    }

    #[test]
    fn envelope_lengths() {
        assert_eq!(envelope_len(prefix::CLEAR), Some(2));
        assert_eq!(envelope_len(prefix::MITO), Some(5));
        assert_eq!(envelope_len(prefix::NUCLEAR), Some(7));
        assert_eq!(envelope_len(0x00), None);
    }

    #[test]
    fn accept_decision_clear() {
        let d = accept_decision(&CLEAR_JSONRPC, UnsignalledPolicy::Reject);
        assert_eq!(
            d,
            AcceptDecision::Accept {
                skip_bytes: 2,
                protocol: 0x01
            }
        );
    }

    #[test]
    fn accept_decision_unsignalled_reject() {
        let d = accept_decision(&[0x7B, 0x22], UnsignalledPolicy::Reject);
        assert_eq!(d, AcceptDecision::Reject);
    }

    #[test]
    fn accept_decision_unsignalled_warn() {
        let d = accept_decision(&[0x7B, 0x22], UnsignalledPolicy::Warn);
        assert_eq!(d, AcceptDecision::Unsignalled);
    }

    #[test]
    fn default_policy_is_reject() {
        assert_eq!(UnsignalledPolicy::default(), UnsignalledPolicy::Reject);
    }
}
