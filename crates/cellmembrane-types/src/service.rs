// SPDX-License-Identifier: AGPL-3.0-or-later

//! Membrane service definitions.
//!
//! Each running process on a membrane host is described by a [`MembraneService`].
//! Services map to systemd units and are derived from the composition.
//!
//! The service registry is static data — no allocations, no `Box::leak`.
//! Each service declares its own capabilities; the registry is the only
//! central knowledge. Binary integrity expectations are derived from the
//! registry rather than re-hardcoded.

use crate::composition::MembraneComposition;
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

/// Health check strategy for a service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthCheckMethod {
    /// JSON-RPC `health.liveness` probe.
    Liveness,
    /// Raw TCP connection probe.
    TcpConnect,
    /// HTTPS GET probe (200 OK).
    HttpsProbe,
    /// DNS query probe.
    DnsProbe,
}

impl fmt::Display for HealthCheckMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Liveness => write!(f, "health.liveness"),
            Self::TcpConnect => write!(f, "tcp_connect"),
            Self::HttpsProbe => write!(f, "https_probe"),
            Self::DnsProbe => write!(f, "dns_probe"),
        }
    }
}

/// Bind to all interfaces (externally reachable).
pub const BIND_ALL: &str = "0.0.0.0";
/// Bind to loopback only (not externally reachable).
pub const BIND_LOOPBACK: &str = "127.0.0.1";

/// A single membrane service (one running process).
///
/// All fields are `&'static str` — service definitions are compile-time
/// constants, not runtime-allocated data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MembraneService {
    /// Binary name (e.g. "songbird", "hbbs").
    pub binary: &'static str,
    /// Systemd unit name.
    pub systemd_unit: &'static str,
    /// Network port, if any (UDS-only services have `None`).
    pub port: Option<u16>,
    /// Protocol for the port.
    pub protocol: Protocol,
    /// Socket path for UDS-based services.
    pub socket_path: Option<&'static str>,
    /// Bind address.
    pub bind: &'static str,
    /// Health check strategy for this service.
    pub health_method: HealthCheckMethod,
    /// Whether this is an ecoPrimals primal (vs symbiotic partner).
    pub is_primal: bool,
    /// Install path on the membrane host.
    pub install_path: &'static str,
    /// Supplementary ports beyond the primary (e.g. hbbs ID server on 21115).
    /// Each entry is `(port, protocol, comment)`.
    pub extra_ports: &'static [(u16, Protocol, &'static str)],
    /// Minimum composition tier that includes this service.
    pub min_composition: MembraneComposition,
}

const BEARDOG: MembraneService = MembraneService {
    binary: "beardog",
    systemd_unit: "beardog-membrane.service",
    port: None,
    protocol: Protocol::Uds,
    socket_path: Some("/run/membrane/beardog.sock"),
    bind: "",
    health_method: HealthCheckMethod::Liveness,
    is_primal: true,
    install_path: "/opt/membrane/beardog",
    extra_ports: &[(8443, Protocol::Tcp, "beardog-tls-shadow")],
    min_composition: MembraneComposition::Tower,
};

const SONGBIRD: MembraneService = MembraneService {
    binary: "songbird",
    systemd_unit: "songbird-relay.service",
    port: Some(3478),
    protocol: Protocol::TcpAndUdp,
    socket_path: None,
    bind: BIND_ALL,
    health_method: HealthCheckMethod::Liveness,
    is_primal: true,
    install_path: "/opt/membrane/songbird",
    extra_ports: &[],
    min_composition: MembraneComposition::Relay,
};

const SKUNKBAT: MembraneService = MembraneService {
    binary: "skunkbat",
    systemd_unit: "skunkbat-membrane.service",
    port: Some(9140),
    protocol: Protocol::Tcp,
    socket_path: None,
    bind: BIND_LOOPBACK,
    health_method: HealthCheckMethod::Liveness,
    is_primal: true,
    install_path: "/opt/membrane/skunkbat",
    extra_ports: &[],
    min_composition: MembraneComposition::Tower,
};

const NESTGATE: MembraneService = MembraneService {
    binary: "nestgate",
    systemd_unit: "nestgate-membrane.service",
    port: Some(9500),
    protocol: Protocol::Tcp,
    socket_path: None,
    bind: BIND_ALL,
    health_method: HealthCheckMethod::Liveness,
    is_primal: true,
    install_path: "/opt/membrane/nestgate",
    extra_ports: &[],
    min_composition: MembraneComposition::Nest,
};

const RHIZOCRYPT: MembraneService = MembraneService {
    binary: "rhizocrypt",
    systemd_unit: "rhizocrypt-membrane.service",
    port: Some(9601),
    protocol: Protocol::Tcp,
    socket_path: None,
    bind: BIND_LOOPBACK,
    health_method: HealthCheckMethod::Liveness,
    is_primal: true,
    install_path: "/opt/membrane/rhizocrypt",
    extra_ports: &[(9602, Protocol::Tcp, "rhizocrypt-jsonrpc")],
    min_composition: MembraneComposition::Nest,
};

const LOAMSPINE: MembraneService = MembraneService {
    binary: "loamspine",
    systemd_unit: "loamspine-membrane.service",
    port: Some(9700),
    protocol: Protocol::Tcp,
    socket_path: None,
    bind: BIND_LOOPBACK,
    health_method: HealthCheckMethod::Liveness,
    is_primal: true,
    install_path: "/opt/membrane/loamspine",
    extra_ports: &[],
    min_composition: MembraneComposition::Nest,
};

const SWEETGRASS: MembraneService = MembraneService {
    binary: "sweetgrass",
    systemd_unit: "sweetgrass-membrane.service",
    port: Some(9850),
    protocol: Protocol::Tcp,
    socket_path: None,
    bind: BIND_LOOPBACK,
    health_method: HealthCheckMethod::Liveness,
    is_primal: true,
    install_path: "/opt/membrane/sweetgrass",
    extra_ports: &[],
    min_composition: MembraneComposition::Nest,
};

const HBBS: MembraneService = MembraneService {
    binary: "hbbs",
    systemd_unit: "hbbs-membrane.service",
    port: Some(21116),
    protocol: Protocol::TcpAndUdp,
    socket_path: None,
    bind: BIND_ALL,
    health_method: HealthCheckMethod::TcpConnect,
    is_primal: false,
    install_path: "/opt/membrane/hbbs",
    extra_ports: &[(21115, Protocol::Tcp, "hbbs-id")],
    min_composition: MembraneComposition::RustDesk,
};

const HBBR: MembraneService = MembraneService {
    binary: "hbbr",
    systemd_unit: "hbbr-membrane.service",
    port: Some(21117),
    protocol: Protocol::Tcp,
    socket_path: None,
    bind: BIND_ALL,
    health_method: HealthCheckMethod::TcpConnect,
    is_primal: false,
    install_path: "/opt/membrane/hbbr",
    extra_ports: &[],
    min_composition: MembraneComposition::RustDesk,
};

const CADDY: MembraneService = MembraneService {
    binary: "caddy",
    systemd_unit: "caddy-tls.service",
    port: Some(443),
    protocol: Protocol::Tcp,
    socket_path: None,
    bind: BIND_ALL,
    health_method: HealthCheckMethod::HttpsProbe,
    is_primal: false,
    install_path: "/usr/bin/caddy",
    extra_ports: &[(80, Protocol::Tcp, "caddy-acme")],
    min_composition: MembraneComposition::Nest,
};

const KNOTDNS: MembraneService = MembraneService {
    binary: "knot-dns",
    systemd_unit: "knot.service",
    port: Some(53),
    protocol: Protocol::TcpAndUdp,
    socket_path: None,
    bind: BIND_ALL,
    health_method: HealthCheckMethod::DnsProbe,
    is_primal: false,
    install_path: "/usr/sbin/knotd",
    extra_ports: &[],
    min_composition: MembraneComposition::Nest,
};

/// All known membrane services. Runtime discovery starts here.
const ALL_SERVICES: &[MembraneService] = &[
    BEARDOG, SONGBIRD, SKUNKBAT, NESTGATE, RHIZOCRYPT,
    LOAMSPINE, SWEETGRASS, HBBS, HBBR, CADDY, KNOTDNS,
];

impl MembraneService {
    /// Look up the canonical service definition for a binary name.
    /// Returns a static reference — zero allocation.
    pub fn for_binary(name: &str) -> Option<&'static Self> {
        ALL_SERVICES.iter().find(|s| s.binary == name)
    }

    /// All known services in the registry.
    pub fn all() -> &'static [Self] {
        ALL_SERVICES
    }

    /// Services included in the given composition tier.
    pub fn for_composition(composition: MembraneComposition) -> Vec<&'static Self> {
        ALL_SERVICES
            .iter()
            .filter(|s| s.min_composition <= composition)
            .collect()
    }

    /// Whether this service is externally reachable (bind != loopback, not UDS).
    pub fn is_externally_reachable(&self) -> bool {
        self.bind != BIND_LOOPBACK && self.protocol != Protocol::Uds
    }
}

/// Binary integrity expectation for a membrane service.
///
/// Maps to MEM-09 (Songbird binary integrity) in `darkforest_membrane.sh`.
/// The BLAKE3 hash is verified against plasmidBin's `checksums.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BinaryIntegrity {
    /// Binary name.
    pub binary: &'static str,
    /// Absolute path on the membrane host — derived from service registry.
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
/// ecoPrimals binaries: static musl ELFs, BLAKE3 checksums.
/// Symbiotic binaries: SHA-256 from upstream releases.
///
/// Install paths are derived from the service registry — no duplication,
/// no `Box::leak`.
pub fn binary_integrity_for(
    composition: crate::composition::MembraneComposition,
) -> Vec<BinaryIntegrity> {
    let spec = composition.spec();
    let mut entries = Vec::new();

    for primal in &spec.primals {
        if let Some(svc) = MembraneService::for_binary(primal) {
            entries.push(BinaryIntegrity {
                binary: svc.binary,
                install_path: svc.install_path,
                hash_algorithm: HashAlgorithm::Blake3,
                require_static_musl: true,
            });
        }
    }

    for sym in &spec.symbiotic {
        if let Some(svc) = MembraneService::for_binary(sym) {
            entries.push(BinaryIntegrity {
                binary: svc.binary,
                install_path: svc.install_path,
                hash_algorithm: HashAlgorithm::Sha256,
                require_static_musl: false,
            });
        }
    }

    entries
}
