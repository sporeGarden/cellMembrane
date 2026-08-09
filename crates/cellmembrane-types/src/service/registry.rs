// SPDX-License-Identifier: AGPL-3.0-or-later

//! Static service registry — compile-time service definitions for all NUCLEUS primals
//! and symbiotic partners.
//!
//! Each entry is a `const MembraneService` — zero allocation, zero runtime cost.
//! The registry is the single source of truth for binary names, ports, sockets,
//! capabilities, and composition tiers.

use super::{
    BIND_ALL, BIND_LOOPBACK, DEFAULT_FEDERATION_PORT, HealthCheckMethod, IpcProtocol,
    MembraneService, Protocol, ServerContract, ServiceCapability, TransportMode,
};
use crate::composition::MembraneComposition;

/// All primals support both JSON-RPC and tarpc (C2 15/15 complete).
const DUAL_PROTOCOL: &[IpcProtocol] = &[IpcProtocol::JsonRpc, IpcProtocol::Tarpc];
/// External/symbiotic services use JSON-RPC only.
const JSONRPC_ONLY: &[IpcProtocol] = &[IpcProtocol::JsonRpc];

// ── Tower tier (security + mesh) ────────────────────────────────────────────

const BEARDOG: MembraneService = MembraneService {
    binary: "beardog",
    systemd_unit: "beardog-membrane.service",
    port: None,
    protocol: Protocol::Uds,
    has_socket: true,
    protocols: DUAL_PROTOCOL,
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
    requires_signed_lineage: true,
    gpu_required: false,
};

const SONGBIRD: MembraneService = MembraneService {
    binary: "songbird",
    systemd_unit: "songbird-relay.service",
    port: Some(3478),
    protocol: Protocol::TcpAndUdp,
    has_socket: false,
    protocols: DUAL_PROTOCOL,
    bind: BIND_ALL,
    health_method: HealthCheckMethod::Liveness,
    is_primal: true,
    system_install_path: None,
    extra_ports: &[(
        DEFAULT_FEDERATION_PORT,
        Protocol::Tcp,
        "songbird-federation",
    )],
    min_composition: MembraneComposition::Relay,
    vps_transport: TransportMode::TcpOptIn,
    capabilities: &[ServiceCapability::MeshRelay, ServiceCapability::TurnServer],
    server_contract: ServerContract::Full,
    api_socket: None,
    socket_aliases: &[],
    requires_signed_lineage: true,
    gpu_required: false,
};

const SKUNKBAT: MembraneService = MembraneService {
    binary: "skunkbat",
    systemd_unit: "skunkbat-membrane.service",
    port: Some(9140),
    protocol: Protocol::Tcp,
    has_socket: true,
    protocols: DUAL_PROTOCOL,
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
    requires_signed_lineage: true,
    gpu_required: false,
};

// ── Nest tier (provenance + content) ────────────────────────────────────────

const NESTGATE: MembraneService = MembraneService {
    binary: "nestgate",
    systemd_unit: "nestgate-membrane.service",
    port: Some(9500),
    protocol: Protocol::Tcp,
    has_socket: true,
    protocols: DUAL_PROTOCOL,
    bind: BIND_ALL,
    health_method: HealthCheckMethod::Liveness,
    is_primal: true,
    system_install_path: None,
    extra_ports: &[],
    min_composition: MembraneComposition::Nest,
    vps_transport: TransportMode::UdsOnly,
    capabilities: &[
        ServiceCapability::ContentServing,
        ServiceCapability::ContentAddressedStorage,
    ],
    server_contract: ServerContract::ServerNoSocket,
    api_socket: None,
    socket_aliases: &[],
    requires_signed_lineage: true,
    gpu_required: false,
};

const RHIZOCRYPT: MembraneService = MembraneService {
    binary: "rhizocrypt",
    systemd_unit: "rhizocrypt-membrane.service",
    port: Some(9601),
    protocol: Protocol::Tcp,
    has_socket: true,
    protocols: DUAL_PROTOCOL,
    bind: BIND_LOOPBACK,
    health_method: HealthCheckMethod::Liveness,
    is_primal: true,
    system_install_path: None,
    extra_ports: &[(9602, Protocol::Tcp, "rhizocrypt-jsonrpc")],
    min_composition: MembraneComposition::Nest,
    vps_transport: TransportMode::UdsOnly,
    capabilities: &[ServiceCapability::Storage],
    server_contract: ServerContract::SocketOnly,
    api_socket: None,
    socket_aliases: &[],
    requires_signed_lineage: false,
    gpu_required: false,
};

const LOAMSPINE: MembraneService = MembraneService {
    binary: "loamspine",
    systemd_unit: "loamspine-membrane.service",
    port: Some(9700),
    protocol: Protocol::Tcp,
    has_socket: true,
    protocols: DUAL_PROTOCOL,
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
    requires_signed_lineage: false,
    gpu_required: false,
};

const SWEETGRASS: MembraneService = MembraneService {
    binary: "sweetgrass",
    systemd_unit: "sweetgrass-membrane.service",
    port: Some(9850),
    protocol: Protocol::Tcp,
    has_socket: true,
    protocols: DUAL_PROTOCOL,
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
    requires_signed_lineage: false,
    gpu_required: false,
};

// ── Compute tier (Nucleus) ──────────────────────────────────────────────────

const TOADSTOOL: MembraneService = MembraneService {
    binary: "toadstool",
    systemd_unit: "toadstool-membrane.service",
    port: None,
    protocol: Protocol::Uds,
    has_socket: true,
    protocols: DUAL_PROTOCOL,
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
    requires_signed_lineage: false,
    gpu_required: false,
};

const BARRACUDA: MembraneService = MembraneService {
    binary: "barracuda",
    systemd_unit: "barracuda-membrane.service",
    port: None,
    protocol: Protocol::Uds,
    has_socket: true,
    protocols: DUAL_PROTOCOL,
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
    requires_signed_lineage: false,
    gpu_required: true,
};

const CORALREEF: MembraneService = MembraneService {
    binary: "coralreef",
    systemd_unit: "coralreef-membrane.service",
    port: None,
    protocol: Protocol::Uds,
    has_socket: true,
    protocols: DUAL_PROTOCOL,
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
    requires_signed_lineage: false,
    gpu_required: true,
};

// ── Meta tier (orchestration) ────────────────────────────────────────────────

const BIOMEOS: MembraneService = MembraneService {
    binary: "biomeos",
    systemd_unit: "biomeos-membrane.service",
    port: None,
    protocol: Protocol::Uds,
    has_socket: true,
    protocols: DUAL_PROTOCOL,
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
    requires_signed_lineage: true,
    gpu_required: false,
};

const SQUIRREL: MembraneService = MembraneService {
    binary: "squirrel",
    systemd_unit: "squirrel-membrane.service",
    port: None,
    protocol: Protocol::Uds,
    has_socket: true,
    protocols: DUAL_PROTOCOL,
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
    requires_signed_lineage: false,
    gpu_required: false,
};

const PETALTONGUE: MembraneService = MembraneService {
    binary: "petaltongue",
    systemd_unit: "petaltongue-membrane.service",
    port: Some(8080),
    protocol: Protocol::Tcp,
    has_socket: true,
    protocols: DUAL_PROTOCOL,
    bind: BIND_LOOPBACK,
    health_method: HealthCheckMethod::Liveness,
    is_primal: true,
    system_install_path: None,
    extra_ports: &[],
    min_composition: MembraneComposition::Nucleus,
    vps_transport: TransportMode::UdsOnly,
    capabilities: &[
        ServiceCapability::ContentServing,
        ServiceCapability::Visualization,
    ],
    server_contract: ServerContract::SocketOnly,
    api_socket: None,
    socket_aliases: &["visualization"],
    requires_signed_lineage: false,
    gpu_required: false,
};

// ── Symbiotic partners (not ecoPrimals) ──────────────────────────────────────

const HBBS: MembraneService = MembraneService {
    binary: "hbbs",
    systemd_unit: "hbbs-membrane.service",
    port: Some(21116),
    protocol: Protocol::TcpAndUdp,
    has_socket: false,
    protocols: JSONRPC_ONLY,
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
    requires_signed_lineage: false,
    gpu_required: false,
};

const HBBR: MembraneService = MembraneService {
    binary: "hbbr",
    systemd_unit: "hbbr-membrane.service",
    port: Some(21117),
    protocol: Protocol::Tcp,
    has_socket: false,
    protocols: JSONRPC_ONLY,
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
    requires_signed_lineage: false,
    gpu_required: false,
};

const CADDY: MembraneService = MembraneService {
    binary: "caddy",
    systemd_unit: "caddy-tls.service",
    port: Some(443),
    protocol: Protocol::Tcp,
    has_socket: false,
    protocols: JSONRPC_ONLY,
    bind: BIND_ALL,
    health_method: HealthCheckMethod::HttpsProbe,
    is_primal: false,
    system_install_path: Some("/usr/bin/caddy"),
    extra_ports: &[(80, Protocol::Tcp, "caddy-acme")],
    min_composition: MembraneComposition::Nest,
    vps_transport: TransportMode::TcpDefault,
    capabilities: &[ServiceCapability::ReverseProxy],
    server_contract: ServerContract::External,
    api_socket: None,
    socket_aliases: &[],
    requires_signed_lineage: false,
    gpu_required: false,
};

const KNOTDNS: MembraneService = MembraneService {
    binary: "knot-dns",
    systemd_unit: "knot.service",
    port: Some(53),
    protocol: Protocol::TcpAndUdp,
    has_socket: false,
    protocols: JSONRPC_ONLY,
    bind: BIND_ALL,
    health_method: HealthCheckMethod::DnsProbe,
    is_primal: false,
    system_install_path: Some("/usr/sbin/knotd"),
    extra_ports: &[],
    min_composition: MembraneComposition::Nest,
    vps_transport: TransportMode::TcpDefault,
    capabilities: &[ServiceCapability::DnsAuthority],
    server_contract: ServerContract::External,
    api_socket: None,
    socket_aliases: &[],
    requires_signed_lineage: false,
    gpu_required: false,
};

// ── Infrastructure (build + deploy) ──────────────────────────────────────────

const MEMBRANE_BUILDER: MembraneService = MembraneService {
    binary: "membrane",
    systemd_unit: "membrane-builder.service",
    port: None,
    protocol: Protocol::Uds,
    has_socket: true,
    protocols: JSONRPC_ONLY,
    bind: "",
    health_method: HealthCheckMethod::Liveness,
    is_primal: false,
    system_install_path: None,
    extra_ports: &[],
    min_composition: MembraneComposition::Nucleus,
    vps_transport: TransportMode::TcpOptIn,
    capabilities: &[ServiceCapability::Build],
    server_contract: ServerContract::External,
    api_socket: None,
    socket_aliases: &["builder", "harvest"],
    requires_signed_lineage: false,
    gpu_required: false,
};

/// All known membrane services. Runtime discovery starts here.
///
/// Order: Tower (3) → Nest provenance (4) → Nucleus compute (3) → Nucleus meta (3) → Infra (1) → Symbiotic (4).
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
    MEMBRANE_BUILDER,
    HBBS,
    HBBR,
    CADDY,
    KNOTDNS,
];
