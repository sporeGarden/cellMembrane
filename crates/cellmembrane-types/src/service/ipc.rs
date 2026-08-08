// SPDX-License-Identifier: AGPL-3.0-or-later

//! IPC protocol types for G65 Protocol Negotiation Standard.
//!
//! Defines the protocol types, negotiation constants, and wire format
//! for single-socket protocol multiplexing. Under G65, a single UDS
//! socket negotiates the best mutual protocol at connection time.

use serde::{Deserialize, Serialize};
use std::fmt;

/// IPC protocol for UDS communication (G65 Protocol Negotiation Standard).
///
/// Each primal declares which protocols it supports. Under G65, a single
/// socket negotiates the best mutual protocol at connection time.
/// `JsonRpc` is always the fallback — if no negotiation occurs, the
/// connection proceeds as JSON-RPC (full backward compatibility).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IpcProtocol {
    /// JSON-RPC 2.0 — text-based, human-readable, default fallback.
    JsonRpc,
    /// tarpc — binary, type-safe, high-performance Rust RPC.
    Tarpc,
}

impl IpcProtocol {
    /// Wire name used in the G65 `PROTOCOLS:` negotiation line.
    #[must_use]
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::JsonRpc => "jsonrpc",
            Self::Tarpc => "tarpc",
        }
    }

    /// Parse from a G65 wire name.
    #[must_use]
    pub fn from_wire(s: &str) -> Option<Self> {
        match s.trim() {
            "jsonrpc" | "json-rpc" | "json_rpc" => Some(Self::JsonRpc),
            "tarpc" => Some(Self::Tarpc),
            _ => None,
        }
    }

    /// Select the best mutual protocol (client preference order wins).
    ///
    /// Returns `JsonRpc` as fallback if no mutual match is found.
    #[must_use]
    pub fn negotiate(client: &[Self], server: &[Self]) -> Self {
        for c in client {
            if server.contains(c) {
                return *c;
            }
        }
        Self::JsonRpc
    }
}

impl fmt::Display for IpcProtocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.wire_name())
    }
}

/// G65 protocol negotiation wire prefix.
pub const PROTOCOL_NEGOTIATION_PREFIX: &str = "PROTOCOLS: ";

/// G65 protocol negotiation response prefix.
pub const PROTOCOL_NEGOTIATION_RESPONSE: &str = "PROTOCOL: ";

/// Timeout for the first line read during negotiation (milliseconds).
///
/// If no `PROTOCOLS:` line arrives within this window, the connection
/// proceeds as JSON-RPC.
pub const PROTOCOL_NEGOTIATION_TIMEOUT_MS: u64 = 100;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipc_protocol_wire_roundtrip() {
        assert_eq!(
            IpcProtocol::from_wire("jsonrpc"),
            Some(IpcProtocol::JsonRpc)
        );
        assert_eq!(
            IpcProtocol::from_wire("json-rpc"),
            Some(IpcProtocol::JsonRpc)
        );
        assert_eq!(
            IpcProtocol::from_wire("json_rpc"),
            Some(IpcProtocol::JsonRpc)
        );
        assert_eq!(IpcProtocol::from_wire("tarpc"), Some(IpcProtocol::Tarpc));
        assert_eq!(IpcProtocol::from_wire("unknown"), None);
    }

    #[test]
    fn ipc_protocol_display() {
        assert_eq!(IpcProtocol::JsonRpc.to_string(), "jsonrpc");
        assert_eq!(IpcProtocol::Tarpc.to_string(), "tarpc");
    }

    #[test]
    fn negotiate_client_preference_wins() {
        let client = [IpcProtocol::Tarpc, IpcProtocol::JsonRpc];
        let server = [IpcProtocol::JsonRpc, IpcProtocol::Tarpc];
        assert_eq!(IpcProtocol::negotiate(&client, &server), IpcProtocol::Tarpc);
    }

    #[test]
    fn negotiate_fallback_to_jsonrpc() {
        let client = [IpcProtocol::Tarpc];
        let server = [IpcProtocol::JsonRpc];
        assert_eq!(
            IpcProtocol::negotiate(&client, &server),
            IpcProtocol::JsonRpc
        );
    }

    #[test]
    fn negotiate_empty_client_falls_back() {
        assert_eq!(
            IpcProtocol::negotiate(&[], &[IpcProtocol::Tarpc]),
            IpcProtocol::JsonRpc
        );
    }
}
