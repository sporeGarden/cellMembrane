// SPDX-License-Identifier: AGPL-3.0-or-later

//! Membrane service definitions.
//!
//! Each running process on a membrane host is described by a [`MembraneService`].
//! Services map to systemd units and are derived from the composition.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Transport protocol for a service port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    /// TCP only.
    Tcp,
    /// UDP only.
    Udp,
    /// Both TCP and UDP on the same port.
    TcpAndUdp,
    /// Unix domain socket (no port).
    Uds,
}

impl fmt::Display for Protocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tcp => write!(f, "tcp"),
            Self::Udp => write!(f, "udp"),
            Self::TcpAndUdp => write!(f, "tcp+udp"),
            Self::Uds => write!(f, "uds"),
        }
    }
}

/// A single membrane service (one running process).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MembraneService {
    /// Binary name (e.g. "songbird", "hbbs").
    pub binary: String,
    /// Systemd unit name.
    pub systemd_unit: String,
    /// Network port, if any (UDS-only services have `None`).
    pub port: Option<u16>,
    /// Protocol for the port.
    pub protocol: Protocol,
    /// Socket path for UDS-based services.
    pub socket_path: Option<String>,
    /// Bind address (empty = all interfaces, "127.0.0.1" = loopback only).
    pub bind: String,
    /// JSON-RPC health check method name.
    pub health_method: String,
    /// Whether this is an ecoPrimals primal (vs symbiotic partner).
    pub is_primal: bool,
    /// Install path on the membrane host.
    pub install_path: String,
}

impl MembraneService {
    /// Look up the canonical service definition for a binary name.
    pub fn for_binary(name: &str) -> Option<Self> {
        match name {
            "beardog" => Some(Self {
                binary: "beardog".into(),
                systemd_unit: "beardog-membrane.service".into(),
                port: None,
                protocol: Protocol::Uds,
                socket_path: Some("/run/membrane/beardog.sock".into()),
                bind: String::new(),
                health_method: "health.liveness".into(),
                is_primal: true,
                install_path: "/opt/membrane/beardog".into(),
            }),
            "songbird" => Some(Self {
                binary: "songbird".into(),
                systemd_unit: "songbird-relay.service".into(),
                port: Some(3478),
                protocol: Protocol::TcpAndUdp,
                socket_path: None,
                bind: "0.0.0.0".into(),
                health_method: "health.liveness".into(),
                is_primal: true,
                install_path: "/opt/membrane/songbird".into(),
            }),
            "skunkbat" => Some(Self {
                binary: "skunkbat".into(),
                systemd_unit: "skunkbat-membrane.service".into(),
                port: Some(9140),
                protocol: Protocol::Tcp,
                socket_path: None,
                bind: "127.0.0.1".into(),
                health_method: "health.liveness".into(),
                is_primal: true,
                install_path: "/opt/membrane/skunkbat".into(),
            }),
            "nestgate" => Some(Self {
                binary: "nestgate".into(),
                systemd_unit: "nestgate-membrane.service".into(),
                port: Some(9500),
                protocol: Protocol::Tcp,
                socket_path: None,
                bind: "0.0.0.0".into(),
                health_method: "health.liveness".into(),
                is_primal: true,
                install_path: "/opt/membrane/nestgate".into(),
            }),
            "rhizocrypt" => Some(Self {
                binary: "rhizocrypt".into(),
                systemd_unit: "rhizocrypt-membrane.service".into(),
                port: Some(9601),
                protocol: Protocol::Tcp,
                socket_path: None,
                bind: "127.0.0.1".into(),
                health_method: "health.liveness".into(),
                is_primal: true,
                install_path: "/opt/membrane/rhizocrypt".into(),
            }),
            "loamspine" => Some(Self {
                binary: "loamspine".into(),
                systemd_unit: "loamspine-membrane.service".into(),
                port: Some(9700),
                protocol: Protocol::Tcp,
                socket_path: None,
                bind: "127.0.0.1".into(),
                health_method: "health.liveness".into(),
                is_primal: true,
                install_path: "/opt/membrane/loamspine".into(),
            }),
            "sweetgrass" => Some(Self {
                binary: "sweetgrass".into(),
                systemd_unit: "sweetgrass-membrane.service".into(),
                port: Some(9850),
                protocol: Protocol::Tcp,
                socket_path: None,
                bind: "127.0.0.1".into(),
                health_method: "health.liveness".into(),
                is_primal: true,
                install_path: "/opt/membrane/sweetgrass".into(),
            }),
            "hbbs" => Some(Self {
                binary: "hbbs".into(),
                systemd_unit: "hbbs-membrane.service".into(),
                port: Some(21116),
                protocol: Protocol::TcpAndUdp,
                socket_path: None,
                bind: "0.0.0.0".into(),
                health_method: "tcp_connect".into(),
                is_primal: false,
                install_path: "/opt/membrane/hbbs".into(),
            }),
            "hbbr" => Some(Self {
                binary: "hbbr".into(),
                systemd_unit: "hbbr-membrane.service".into(),
                port: Some(21117),
                protocol: Protocol::Tcp,
                socket_path: None,
                bind: "0.0.0.0".into(),
                health_method: "tcp_connect".into(),
                is_primal: false,
                install_path: "/opt/membrane/hbbr".into(),
            }),
            "caddy" => Some(Self {
                binary: "caddy".into(),
                systemd_unit: "caddy-tls.service".into(),
                port: Some(443),
                protocol: Protocol::Tcp,
                socket_path: None,
                bind: "0.0.0.0".into(),
                health_method: "https_probe".into(),
                is_primal: false,
                install_path: "/usr/bin/caddy".into(),
            }),
            _ => None,
        }
    }

    /// Whether this service is externally reachable (bind != loopback, not UDS).
    pub fn is_externally_reachable(&self) -> bool {
        self.bind != "127.0.0.1" && self.protocol != Protocol::Uds
    }
}

/// Binary integrity expectation for a membrane service.
///
/// Maps to MEM-09 (Songbird binary integrity) in `darkforest_membrane.sh`.
/// The BLAKE3 hash is verified against plasmidBin's `checksums.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryIntegrity {
    /// Binary name.
    pub binary: &'static str,
    /// Absolute path on the membrane host.
    pub install_path: &'static str,
    /// Hash algorithm used for verification.
    pub hash_algorithm: HashAlgorithm,
    /// Whether the binary must be a static musl ELF (stripped).
    pub require_static_musl: bool,
}

/// Hash algorithm for binary verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashAlgorithm {
    /// BLAKE3 — used by plasmidBin checksums.toml.
    Blake3,
    /// SHA-256 — fallback when b3sum is not installed.
    Sha256,
}

/// Returns the binary integrity expectations for a given composition.
///
/// All ecoPrimals binaries must be static musl ELFs with BLAKE3 checksums.
/// Symbiotic binaries (hbbs/hbbr, caddy) use SHA-256 from upstream releases.
pub fn binary_integrity_for(
    composition: crate::composition::MembraneComposition,
) -> Vec<BinaryIntegrity> {
    let spec = composition.spec();
    let mut entries = Vec::new();

    for primal in &spec.primals {
        if let Some(svc) = MembraneService::for_binary(primal) {
            entries.push(BinaryIntegrity {
                binary: primal,
                install_path: match *primal {
                    "beardog" => "/opt/membrane/beardog",
                    "songbird" => "/opt/membrane/songbird",
                    "skunkbat" => "/opt/membrane/skunkbat",
                    "nestgate" => "/opt/membrane/nestgate",
                    "rhizocrypt" => "/opt/membrane/rhizocrypt",
                    "loamspine" => "/opt/membrane/loamspine",
                    "sweetgrass" => "/opt/membrane/sweetgrass",
                    _ => Box::leak(svc.install_path.into_boxed_str()),
                },
                hash_algorithm: HashAlgorithm::Blake3,
                require_static_musl: true,
            });
        }
    }

    for sym in &spec.symbiotic {
        if let Some(svc) = MembraneService::for_binary(sym) {
            entries.push(BinaryIntegrity {
                binary: sym,
                install_path: match *sym {
                    "hbbs" => "/opt/membrane/hbbs",
                    "hbbr" => "/opt/membrane/hbbr",
                    "caddy" => "/usr/bin/caddy",
                    _ => Box::leak(svc.install_path.into_boxed_str()),
                },
                hash_algorithm: HashAlgorithm::Sha256,
                require_static_musl: false,
            });
        }
    }

    entries
}
