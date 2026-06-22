// SPDX-License-Identifier: AGPL-3.0-or-later

//! Static service registry — compile-time service definitions for all NUCLEUS primals
//! and symbiotic partners.
//!
//! Each entry is a `const MembraneService` — zero allocation, zero runtime cost.
//! The registry is the single source of truth for binary names, ports, sockets,
//! capabilities, and composition tiers.

use super::{
    BIND_ALL, BIND_LOOPBACK, HealthCheckMethod, MembraneService, Protocol, ServerContract,
    ServiceCapability, TransportMode,
};
use crate::composition::MembraneComposition;

// ── Tower tier (security + mesh) ────────────────────────────────────────────

const BEARDOG: MembraneService = MembraneService {
    binary: "beardog",
    systemd_unit: "beardog-membrane.service",
    port: None,
    protocol: Protocol::Uds,
    has_socket: true,
    bind: "",
    health_method: HealthCheckMethod::Liveness,
    is_primal: true,
    system_install_path: None,
    extra_ports: &[(8443, Protocol::Tcp, "beardog-tls-shadow")],
    min_composition: MembraneComposition::Tower,
    vps_transport: TransportMode::UdsOnly,
    capabilities: &[ServiceCapability::CryptoSigner, ServiceCapability::Security],
    server_contract: ServerContract::SocketAuditDir,
    api_socket: None,
    socket_aliases: &["crypto", "security", "ed25519", "x25519", "btsp"],
};

const SONGBIRD: MembraneService = MembraneService {
    binary: "songbird",
    systemd_unit: "songbird-relay.service",
    port: Some(3478),
    protocol: Protocol::TcpAndUdp,
    has_socket: false,
    bind: BIND_ALL,
    health_method: HealthCheckMethod::Liveness,
    is_primal: true,
    system_install_path: None,
    extra_ports: &[],
    min_composition: MembraneComposition::Relay,
    vps_transport: TransportMode::TcpOptIn,
    capabilities: &[ServiceCapability::MeshRelay, ServiceCapability::TurnServer],
    server_contract: ServerContract::Full,
    api_socket: None,
    socket_aliases: &[],
};

const SKUNKBAT: MembraneService = MembraneService {
    binary: "skunkbat",
    systemd_unit: "skunkbat-membrane.service",
    port: Some(9140),
    protocol: Protocol::Tcp,
    has_socket: true,
    bind: BIND_LOOPBACK,
    health_method: HealthCheckMethod::Liveness,
    is_primal: true,
    system_install_path: None,
    extra_ports: &[],
    min_composition: MembraneComposition::Tower,
    vps_transport: TransportMode::UdsOnly,
    capabilities: &[ServiceCapability::Observability],
    server_contract: ServerContract::Full,
    api_socket: None,
    socket_aliases: &[],
};

// ── Nest tier (provenance + content) ────────────────────────────────────────

const NESTGATE: MembraneService = MembraneService {
    binary: "nestgate",
    systemd_unit: "nestgate-membrane.service",
    port: Some(9500),
    protocol: Protocol::Tcp,
    has_socket: true,
    bind: BIND_ALL,
    health_method: HealthCheckMethod::Liveness,
    is_primal: true,
    system_install_path: None,
    extra_ports: &[],
    min_composition: MembraneComposition::Nest,
    vps_transport: TransportMode::UdsOnly,
    capabilities: &[ServiceCapability::ContentServing],
    server_contract: ServerContract::SocketOnly,
    api_socket: None,
    socket_aliases: &[],
};

const RHIZOCRYPT: MembraneService = MembraneService {
    binary: "rhizocrypt",
    systemd_unit: "rhizocrypt-membrane.service",
    port: Some(9601),
    protocol: Protocol::Tcp,
    has_socket: true,
    bind: BIND_LOOPBACK,
    health_method: HealthCheckMethod::Liveness,
    is_primal: true,
    system_install_path: None,
    extra_ports: &[(9602, Protocol::Tcp, "rhizocrypt-jsonrpc")],
    min_composition: MembraneComposition::Nest,
    vps_transport: TransportMode::UdsOnly,
    capabilities: &[ServiceCapability::Storage],
    server_contract: ServerContract::Tarpc,
    api_socket: None,
    socket_aliases: &[],
};

const LOAMSPINE: MembraneService = MembraneService {
    binary: "loamspine",
    systemd_unit: "loamspine-membrane.service",
    port: Some(9700),
    protocol: Protocol::Tcp,
    has_socket: true,
    bind: BIND_LOOPBACK,
    health_method: HealthCheckMethod::Liveness,
    is_primal: true,
    system_install_path: None,
    extra_ports: &[],
    min_composition: MembraneComposition::Nest,
    vps_transport: TransportMode::UdsOnly,
    capabilities: &[ServiceCapability::Storage],
    server_contract: ServerContract::Tarpc,
    api_socket: None,
    socket_aliases: &["ledger", "permanence"],
};

const SWEETGRASS: MembraneService = MembraneService {
    binary: "sweetgrass",
    systemd_unit: "sweetgrass-membrane.service",
    port: Some(9850),
    protocol: Protocol::Tcp,
    has_socket: true,
    bind: BIND_LOOPBACK,
    health_method: HealthCheckMethod::Liveness,
    is_primal: true,
    system_install_path: None,
    extra_ports: &[],
    min_composition: MembraneComposition::Nest,
    vps_transport: TransportMode::UdsOnly,
    capabilities: &[ServiceCapability::Storage],
    server_contract: ServerContract::SocketOnly,
    api_socket: None,
    socket_aliases: &["provenance"],
};

// ── Compute tier (Nucleus) ──────────────────────────────────────────────────

const TOADSTOOL: MembraneService = MembraneService {
    binary: "toadstool",
    systemd_unit: "toadstool-membrane.service",
    port: None,
    protocol: Protocol::Uds,
    has_socket: true,
    bind: "",
    health_method: HealthCheckMethod::Liveness,
    is_primal: true,
    system_install_path: None,
    extra_ports: &[],
    min_composition: MembraneComposition::Nucleus,
    vps_transport: TransportMode::UdsOnly,
    capabilities: &[ServiceCapability::ComputeOrchestration],
    server_contract: ServerContract::SocketOnly,
    api_socket: None,
    socket_aliases: &["compute-tarpc"],
};

const BARRACUDA: MembraneService = MembraneService {
    binary: "barracuda",
    systemd_unit: "barracuda-membrane.service",
    port: None,
    protocol: Protocol::Uds,
    has_socket: true,
    bind: "",
    health_method: HealthCheckMethod::Liveness,
    is_primal: true,
    system_install_path: None,
    extra_ports: &[],
    min_composition: MembraneComposition::Nucleus,
    vps_transport: TransportMode::UdsOnly,
    capabilities: &[ServiceCapability::ComputeOrchestration],
    server_contract: ServerContract::SocketOnly,
    api_socket: None,
    socket_aliases: &[],
};

const CORALREEF: MembraneService = MembraneService {
    binary: "coralreef",
    systemd_unit: "coralreef-membrane.service",
    port: None,
    protocol: Protocol::Uds,
    has_socket: true,
    bind: "",
    health_method: HealthCheckMethod::Liveness,
    is_primal: true,
    system_install_path: None,
    extra_ports: &[],
    min_composition: MembraneComposition::Nucleus,
    vps_transport: TransportMode::UdsOnly,
    capabilities: &[ServiceCapability::Storage],
    server_contract: ServerContract::SocketOnly,
    api_socket: None,
    socket_aliases: &["coralreef-tarpc"],
};

// ── Meta tier (orchestration) ────────────────────────────────────────────────

const BIOMEOS: MembraneService = MembraneService {
    binary: "biomeos",
    systemd_unit: "biomeos-membrane.service",
    port: None,
    protocol: Protocol::Uds,
    has_socket: true,
    bind: "",
    health_method: HealthCheckMethod::Liveness,
    is_primal: true,
    system_install_path: None,
    extra_ports: &[],
    min_composition: MembraneComposition::Nucleus,
    vps_transport: TransportMode::UdsOnly,
    capabilities: &[
        ServiceCapability::Identity,
        ServiceCapability::ComputeOrchestration,
    ],
    server_contract: ServerContract::BiomeosApi,
    api_socket: Some("neural-api"),
    socket_aliases: &["ai"],
};

const SQUIRREL: MembraneService = MembraneService {
    binary: "squirrel",
    systemd_unit: "squirrel-membrane.service",
    port: None,
    protocol: Protocol::Uds,
    has_socket: true,
    bind: "",
    health_method: HealthCheckMethod::Liveness,
    is_primal: true,
    system_install_path: None,
    extra_ports: &[],
    min_composition: MembraneComposition::Nucleus,
    vps_transport: TransportMode::UdsOnly,
    capabilities: &[ServiceCapability::Storage],
    server_contract: ServerContract::SocketOnly,
    api_socket: None,
    socket_aliases: &[],
};

const PETALTONGUE: MembraneService = MembraneService {
    binary: "petaltongue",
    systemd_unit: "petaltongue-membrane.service",
    port: Some(8080),
    protocol: Protocol::Tcp,
    has_socket: true,
    bind: BIND_LOOPBACK,
    health_method: HealthCheckMethod::Liveness,
    is_primal: true,
    system_install_path: None,
    extra_ports: &[],
    min_composition: MembraneComposition::Nucleus,
    vps_transport: TransportMode::UdsOnly,
    capabilities: &[ServiceCapability::ContentServing],
    server_contract: ServerContract::SocketOnly,
    api_socket: None,
    socket_aliases: &["visualization"],
};

// ── Symbiotic partners (not ecoPrimals) ──────────────────────────────────────

const HBBS: MembraneService = MembraneService {
    binary: "hbbs",
    systemd_unit: "hbbs-membrane.service",
    port: Some(21116),
    protocol: Protocol::TcpAndUdp,
    has_socket: false,
    bind: BIND_ALL,
    health_method: HealthCheckMethod::TcpConnect,
    is_primal: false,
    system_install_path: None,
    extra_ports: &[(21115, Protocol::Tcp, "hbbs-id")],
    min_composition: MembraneComposition::RustDesk,
    vps_transport: TransportMode::TcpDefault,
    capabilities: &[],
    server_contract: ServerContract::External,
    api_socket: None,
    socket_aliases: &[],
};

const HBBR: MembraneService = MembraneService {
    binary: "hbbr",
    systemd_unit: "hbbr-membrane.service",
    port: Some(21117),
    protocol: Protocol::Tcp,
    has_socket: false,
    bind: BIND_ALL,
    health_method: HealthCheckMethod::TcpConnect,
    is_primal: false,
    system_install_path: None,
    extra_ports: &[],
    min_composition: MembraneComposition::RustDesk,
    vps_transport: TransportMode::TcpDefault,
    capabilities: &[],
    server_contract: ServerContract::External,
    api_socket: None,
    socket_aliases: &[],
};

const CADDY: MembraneService = MembraneService {
    binary: "caddy",
    systemd_unit: "caddy-tls.service",
    port: Some(443),
    protocol: Protocol::Tcp,
    has_socket: false,
    bind: BIND_ALL,
    health_method: HealthCheckMethod::HttpsProbe,
    is_primal: false,
    system_install_path: Some("/usr/bin/caddy"),
    extra_ports: &[(80, Protocol::Tcp, "caddy-acme")],
    min_composition: MembraneComposition::Nest,
    vps_transport: TransportMode::TcpDefault,
    capabilities: &[],
    server_contract: ServerContract::External,
    api_socket: None,
    socket_aliases: &[],
};

const KNOTDNS: MembraneService = MembraneService {
    binary: "knot-dns",
    systemd_unit: "knot.service",
    port: Some(53),
    protocol: Protocol::TcpAndUdp,
    has_socket: false,
    bind: BIND_ALL,
    health_method: HealthCheckMethod::DnsProbe,
    is_primal: false,
    system_install_path: Some("/usr/sbin/knotd"),
    extra_ports: &[],
    min_composition: MembraneComposition::Nest,
    vps_transport: TransportMode::TcpDefault,
    capabilities: &[],
    server_contract: ServerContract::External,
    api_socket: None,
    socket_aliases: &[],
};

/// All known membrane services. Runtime discovery starts here.
///
/// Order: Tower (3) → Nest provenance (4) → Nucleus compute (3) → Nucleus meta (3) → Symbiotic (4).
pub(super) const ALL_SERVICES: &[MembraneService] = &[
    BEARDOG,
    SONGBIRD,
    SKUNKBAT,
    NESTGATE,
    RHIZOCRYPT,
    LOAMSPINE,
    SWEETGRASS,
    TOADSTOOL,
    BARRACUDA,
    CORALREEF,
    BIOMEOS,
    SQUIRREL,
    PETALTONGUE,
    HBBS,
    HBBR,
    CADDY,
    KNOTDNS,
];
