// SPDX-License-Identifier: AGPL-3.0-or-later

//! Transport Endpoint — canonical type for structured service resolution.
//!
//! Wire-compatible with `songbird_types::TransportEndpoint` and
//! `sourdough_core::TransportEndpoint`. Tagged JSON serde format:
//!
//! ```json
//! { "transport": "uds", "path": "/run/membrane/beardog.sock" }
//! { "transport": "tcp", "host": "192.168.1.144", "port": 7700 }
//! { "transport": "mesh_relay", "peer_id": "strand-gate", "capability": "security" }
//! ```

use serde::{Deserialize, Serialize};
use std::fmt;

/// Structured transport endpoint — describes how to reach a service.
///
/// Returned by `ipc.resolve` / `capability.resolve`. Consumers match on the
/// variant to select the appropriate connection strategy.
///
/// Variants are ordered by locality (local > network > relay).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(tag = "transport")]
pub enum TransportEndpoint {
    /// Unix Domain Socket — same-host inter-primal communication (fastest).
    #[serde(rename = "uds")]
    Uds {
        /// Filesystem path to the socket.
        /// Prefix with `@` for Linux abstract namespace sockets.
        path: String,
    },

    /// TCP — direct network connection (cross-host, LAN, or VPS-to-VPS).
    #[serde(rename = "tcp")]
    Tcp {
        /// Host address (IPv4, IPv6, or hostname).
        host: String,
        /// TCP port number.
        port: u16,
    },

    /// Mesh relay — routes through Songbird's relay infrastructure.
    #[serde(rename = "mesh_relay")]
    MeshRelay {
        /// Mesh peer identifier (e.g. `"strand-gate"`, `"east-gate"`).
        peer_id: String,
        /// Capability being resolved on the remote peer.
        capability: String,
    },
}

impl TransportEndpoint {
    /// Whether this endpoint is local (same-host, no network hop).
    #[must_use]
    pub fn is_local(&self) -> bool {
        match self {
            Self::Uds { .. } => true,
            Self::Tcp { host, .. } => host == "127.0.0.1" || host == "::1" || host == "localhost",
            Self::MeshRelay { .. } => false,
        }
    }

    /// Whether this endpoint requires network access.
    #[must_use]
    #[allow(clippy::missing_const_for_fn)]
    pub fn is_network(&self) -> bool {
        !self.is_local()
    }

    /// Whether this endpoint uses relay infrastructure (higher latency).
    #[must_use]
    pub const fn is_relayed(&self) -> bool {
        matches!(self, Self::MeshRelay { .. })
    }

    /// Transport name as it appears in the wire format.
    #[must_use]
    pub const fn transport_name(&self) -> &'static str {
        match self {
            Self::Uds { .. } => "uds",
            Self::Tcp { .. } => "tcp",
            Self::MeshRelay { .. } => "mesh_relay",
        }
    }

    /// URI-style string for logging/diagnostics (not for parsing).
    #[must_use]
    pub fn display_uri(&self) -> String {
        match self {
            Self::Uds { path } => path.strip_prefix('@').map_or_else(
                || format!("unix://{path}"),
                |abstract_name| format!("unix-abstract://{abstract_name}"),
            ),
            Self::Tcp { host, port } => format!("tcp://{host}:{port}"),
            Self::MeshRelay {
                peer_id,
                capability,
            } => format!("mesh://{peer_id}/{capability}"),
        }
    }

    /// Parse a `TRANSPORT_ENDPOINT` env var or CLI value (JSON format).
    ///
    /// # Errors
    ///
    /// Returns `serde_json::Error` if the value is not valid JSON or does not
    /// match the `TransportEndpoint` tagged enum format.
    pub fn from_env_value(value: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(value)
    }
}

impl fmt::Display for TransportEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display_uri())
    }
}

/// Environment variable name for transport endpoint injection.
pub const ENV_TRANSPORT_ENDPOINT: &str = "TRANSPORT_ENDPOINT";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uds_serde_roundtrip() {
        let ep = TransportEndpoint::Uds {
            path: "/run/membrane/beardog.sock".into(),
        };
        let json = serde_json::to_string(&ep).unwrap();
        assert!(json.contains(r#""transport":"uds""#));
        let back: TransportEndpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(ep, back);
    }

    #[test]
    fn tcp_serde_roundtrip() {
        let ep = TransportEndpoint::Tcp {
            host: "192.168.1.144".into(),
            port: 7700,
        };
        let json = serde_json::to_string(&ep).unwrap();
        assert!(json.contains(r#""transport":"tcp""#));
        let back: TransportEndpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(ep, back);
    }

    #[test]
    fn mesh_relay_serde_roundtrip() {
        let ep = TransportEndpoint::MeshRelay {
            peer_id: "strand-gate".into(),
            capability: "security".into(),
        };
        let json = serde_json::to_string(&ep).unwrap();
        assert!(json.contains(r#""transport":"mesh_relay""#));
        let back: TransportEndpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(ep, back);
    }

    #[test]
    fn is_local_variants() {
        let uds = TransportEndpoint::Uds {
            path: "/tmp/test.sock".into(),
        };
        assert!(uds.is_local());

        let tcp_local = TransportEndpoint::Tcp {
            host: "127.0.0.1".into(),
            port: 8080,
        };
        assert!(tcp_local.is_local());

        let tcp_remote = TransportEndpoint::Tcp {
            host: "10.0.0.1".into(),
            port: 8080,
        };
        assert!(!tcp_remote.is_local());
        assert!(tcp_remote.is_network());

        let mesh = TransportEndpoint::MeshRelay {
            peer_id: "east".into(),
            capability: "crypto".into(),
        };
        assert!(!mesh.is_local());
        assert!(mesh.is_relayed());
    }

    #[test]
    fn display_uri_format() {
        let uds = TransportEndpoint::Uds {
            path: "/run/membrane/beardog.sock".into(),
        };
        assert_eq!(uds.display_uri(), "unix:///run/membrane/beardog.sock");

        let abstract_uds = TransportEndpoint::Uds {
            path: "@membrane-beardog".into(),
        };
        assert_eq!(
            abstract_uds.display_uri(),
            "unix-abstract://membrane-beardog"
        );

        let tcp = TransportEndpoint::Tcp {
            host: "10.0.0.1".into(),
            port: 9443,
        };
        assert_eq!(tcp.display_uri(), "tcp://10.0.0.1:9443");

        let mesh = TransportEndpoint::MeshRelay {
            peer_id: "strand-gate".into(),
            capability: "security".into(),
        };
        assert_eq!(mesh.display_uri(), "mesh://strand-gate/security");
    }

    #[test]
    fn from_env_value_parses_json() {
        let val = r#"{"transport":"tcp","host":"10.0.0.1","port":9443}"#;
        let ep = TransportEndpoint::from_env_value(val).unwrap();
        assert_eq!(
            ep,
            TransportEndpoint::Tcp {
                host: "10.0.0.1".into(),
                port: 9443,
            }
        );
    }

    #[test]
    fn transport_name_values() {
        let uds = TransportEndpoint::Uds {
            path: "/tmp/x.sock".into(),
        };
        assert_eq!(uds.transport_name(), "uds");

        let tcp = TransportEndpoint::Tcp {
            host: "h".into(),
            port: 1,
        };
        assert_eq!(tcp.transport_name(), "tcp");

        let mesh = TransportEndpoint::MeshRelay {
            peer_id: "p".into(),
            capability: "c".into(),
        };
        assert_eq!(mesh.transport_name(), "mesh_relay");
    }
}
