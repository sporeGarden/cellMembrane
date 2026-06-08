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

/// Transport mode for VPS deployment (Wave 56 standard).
///
/// Determines whether a primal uses TCP ports or Unix domain sockets
/// for inter-primal communication. The VPS standard is `UdsOnly` —
/// zero TCP ports for all NUCLEUS primals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportMode {
    /// UDS-only: no TCP ports allocated. VPS deployment standard.
    /// Health checks via socket file existence.
    UdsOnly,
    /// TCP default: service binds to a TCP port (legacy / symbiotic).
    TcpDefault,
    /// TCP opt-in: UDS primary, TCP available via `TRANSPORT_ENDPOINT` injection.
    TcpOptIn,
}

impl fmt::Display for TransportMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UdsOnly => write!(f, "uds_only"),
            Self::TcpDefault => write!(f, "tcp_default"),
            Self::TcpOptIn => write!(f, "tcp_opt_in"),
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
    /// UDS socket file existence check (VPS standard).
    SocketExists,
}

impl fmt::Display for HealthCheckMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Liveness => write!(f, "health.liveness"),
            Self::TcpConnect => write!(f, "tcp_connect"),
            Self::HttpsProbe => write!(f, "https_probe"),
            Self::DnsProbe => write!(f, "dns_probe"),
            Self::SocketExists => write!(f, "socket_exists"),
        }
    }
}

/// Bind to all interfaces (externally reachable).
pub const BIND_ALL: &str = "0.0.0.0";
/// Bind to loopback only (not externally reachable).
pub const BIND_LOOPBACK: &str = "127.0.0.1";

/// Default base path for primal binary installations.
/// Override with `MEMBRANE_INSTALL_BASE` env var or membrane.toml config.
pub const DEFAULT_INSTALL_BASE: &str = "/opt/membrane";

/// Default base path for primal UDS sockets.
pub const DEFAULT_SOCKET_BASE: &str = "/run/membrane";

/// Default ecoPrimals workspace root on VPS deployments.
/// Override with `ECOPRIMALS_ROOT` env var.
pub const DEFAULT_ECOPRIMALS_ROOT: &str = "/opt/ecoPrimals";

// ── Standard deployment environment variables ────────────────────────

/// Environment variable for the plasmidBin depot directory.
pub const ENV_PLASMIDBIN_DEPOT: &str = "PLASMIDBIN_DEPOT";
/// Environment variable for the security provider socket path.
pub const ENV_SECURITY_PROVIDER: &str = "SONGBIRD_SECURITY_PROVIDER";
/// Environment variable for the membrane install base directory.
pub const ENV_INSTALL_BASE: &str = "MEMBRANE_INSTALL_BASE";
/// Environment variable for the membrane socket base directory.
pub const ENV_SOCKET_BASE: &str = "MEMBRANE_SOCKET_BASE";
/// Environment variable for the Forgejo SSH host.
pub const ENV_FORGEJO_SSH_HOST: &str = "FORGEJO_SSH_HOST";
/// Environment variable for the ecoPrimals workspace root.
pub const ENV_ECOPRIMALS_ROOT: &str = "ECOPRIMALS_ROOT";
/// Environment variable for the gate identity.
pub const ENV_GATE_NAME: &str = "GATE_NAME";
/// Environment variable for the songbird federation port.
pub const ENV_FEDERATION_PORT: &str = "SONGBIRD_FEDERATION_PORT";
/// Environment variable for the songbird production bind address.
pub const ENV_PRODUCTION_BIND: &str = "SONGBIRD_PRODUCTION_BIND_ADDRESS";
/// Environment variable for the webhook secret (HMAC-SHA256).
pub const ENV_WEBHOOK_SECRET: &str = "WEBHOOK_SECRET";
/// Environment variable for the `NeuralBridge` API socket path.
pub const ENV_NEURAL_API_SOCKET: &str = "NEURAL_API_SOCKET";
/// Environment variable for the peptidoglycan SSH host.
pub const ENV_PEPTI_SSH_HOST: &str = "PEPTI_SSH_HOST";
/// Environment variable for the Forgejo API token.
pub const ENV_FORGEJO_TOKEN: &str = "FORGEJO_TOKEN";
/// Environment variable for the Forgejo API URL.
pub const ENV_FORGEJO_API: &str = "FORGEJO_API";
/// Environment variable for the membrane SSH host (golgiBody).
pub const ENV_SSH_HOST: &str = "MEMBRANE_SSH_HOST";
/// Environment variable for the VPS ecoPrimals root directory.
pub const ENV_VPS_ECOPRIMALS_ROOT: &str = "VPS_ECOPRIMALS_ROOT";
/// Environment variable for NUCLEUS bind address.
pub const ENV_NUCLEUS_BIND: &str = "NUCLEUS_BIND_ADDRESS";

/// Runtime path resolver for membrane services.
///
/// Resolves install paths and socket paths from a configurable base,
/// eliminating hardcoded `/opt/membrane/` assumptions. Primals follow
/// the pattern `{base}/{binary}`, symbiotic partners use system paths.
#[derive(Debug, Clone)]
pub struct ServicePaths {
    install_base: String,
    socket_base: String,
}

impl ServicePaths {
    /// Create from environment, falling back to defaults.
    ///
    /// Reads `MEMBRANE_INSTALL_BASE` and `MEMBRANE_SOCKET_BASE` env vars.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            install_base: std::env::var("MEMBRANE_INSTALL_BASE")
                .unwrap_or_else(|_| DEFAULT_INSTALL_BASE.to_string()),
            socket_base: std::env::var("MEMBRANE_SOCKET_BASE")
                .unwrap_or_else(|_| DEFAULT_SOCKET_BASE.to_string()),
        }
    }

    /// Create with explicit base paths.
    #[must_use]
    pub fn new(install_base: impl Into<String>, socket_base: impl Into<String>) -> Self {
        Self {
            install_base: install_base.into(),
            socket_base: socket_base.into(),
        }
    }

    /// Create from a `DeployPaths` configuration (from `membrane.toml`).
    #[must_use]
    pub fn from_deploy_paths(paths: &crate::config::DeployPaths) -> Self {
        Self {
            install_base: paths.install_base.clone(),
            socket_base: paths.socket_base.clone(),
        }
    }

    /// Resolve install path for a service.
    ///
    /// Services with `system_install_path` use that (e.g. `/usr/bin/caddy`).
    /// All others derive from `{install_base}/{binary}`.
    #[must_use]
    pub fn install_path(&self, service: &MembraneService) -> String {
        service.system_install_path.map_or_else(
            || format!("{}/{}", self.install_base, service.binary),
            Into::into,
        )
    }

    /// Resolve socket path for a service.
    #[must_use]
    pub fn socket_path(&self, service: &MembraneService) -> Option<String> {
        service
            .has_socket
            .then(|| format!("{}/{}.sock", self.socket_base, service.binary))
    }
}

impl Default for ServicePaths {
    fn default() -> Self {
        Self::from_env()
    }
}

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
    /// Whether this service provides a UDS socket.
    /// The actual path is resolved via [`ServicePaths::socket_path`].
    pub has_socket: bool,
    /// Bind address.
    pub bind: &'static str,
    /// Health check strategy for this service.
    pub health_method: HealthCheckMethod,
    /// Whether this is an ecoPrimals primal (vs symbiotic partner).
    pub is_primal: bool,
    /// Static install path override for system-installed services (caddy, knotd).
    /// `None` means the path is derived at runtime from `ServicePaths::install_path()`.
    pub system_install_path: Option<&'static str>,
    /// Supplementary ports beyond the primary (e.g. hbbs ID server on 21115).
    /// Each entry is `(port, protocol, comment)`.
    pub extra_ports: &'static [(u16, Protocol, &'static str)],
    /// Minimum composition tier that includes this service.
    pub min_composition: MembraneComposition,
    /// VPS deployment transport mode (Wave 56 standard).
    pub vps_transport: TransportMode,
}

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
};

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
};

// ── Compute tier (Node composition) ──────────────────────────────────────────

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
};

/// All known membrane services. Runtime discovery starts here.
///
/// Order: Tower (3) → Nest provenance (4) → Nucleus compute (3) → Nucleus meta (3) → Symbiotic (4).
const ALL_SERVICES: &[MembraneService] = &[
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

impl MembraneService {
    /// Look up the canonical service definition for a binary name.
    /// Returns a static reference — zero allocation.
    #[must_use]
    pub fn for_binary(name: &str) -> Option<&'static Self> {
        ALL_SERVICES.iter().find(|s| s.binary == name)
    }

    /// All known services in the registry.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        ALL_SERVICES
    }

    /// Services included in the given composition tier.
    #[must_use]
    pub fn for_composition(composition: MembraneComposition) -> Vec<&'static Self> {
        ALL_SERVICES
            .iter()
            .filter(|s| s.min_composition <= composition)
            .collect()
    }

    /// Whether this service is externally reachable (bind != loopback, not UDS).
    #[must_use]
    pub const fn is_externally_reachable(&self) -> bool {
        !matches!(self.bind.as_bytes(), b"127.0.0.1") && !matches!(self.protocol, Protocol::Uds)
    }

    /// Whether this service uses UDS-only transport on VPS (Wave 56 standard).
    #[must_use]
    pub const fn is_uds_only(&self) -> bool {
        matches!(self.vps_transport, TransportMode::UdsOnly)
    }

    /// Resolve install path using configurable `ServicePaths` (capability-based).
    ///
    /// Uses `system_install_path` for system services, otherwise derives
    /// from the configured install base. Removes the `/opt/membrane/` assumption.
    #[must_use]
    pub fn resolved_install_path(&self, paths: &ServicePaths) -> String {
        paths.install_path(self)
    }

    /// Resolve socket path using configurable `ServicePaths`.
    #[must_use]
    pub fn resolved_socket_path(&self, paths: &ServicePaths) -> Option<String> {
        paths.socket_path(self)
    }

    /// Health check method to use in UDS-only mode.
    /// Primals with UDS-only transport use socket existence checks instead of TCP probes.
    #[must_use]
    pub const fn uds_health_check(&self) -> HealthCheckMethod {
        if self.is_uds_only() && self.has_socket {
            return HealthCheckMethod::SocketExists;
        }
        self.health_method
    }

    /// Services that require TCP ports even in UDS-only deployments
    /// (symbiotic partners and relay services with external surface).
    #[must_use]
    pub const fn requires_tcp_in_uds_mode(&self) -> bool {
        matches!(self.vps_transport, TransportMode::TcpDefault)
    }
}

/// Binary integrity expectation for a membrane service.
///
/// Maps to MEM-09 (Songbird binary integrity) in `darkforest_membrane.sh`.
/// The BLAKE3 hash is verified against `plasmidBin`'s `checksums.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryIntegrity {
    /// Binary name.
    pub binary: &'static str,
    /// Resolved install path (runtime-configurable).
    pub install_path: String,
    /// Hash algorithm used for verification.
    pub hash_algorithm: HashAlgorithm,
    /// Whether the binary must be a static musl ELF (stripped).
    pub require_static_musl: bool,
}

/// Hash algorithm for binary verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashAlgorithm {
    /// BLAKE3 — used by `plasmidBin` `checksums.toml`.
    Blake3,
    /// SHA-256 — fallback when b3sum is not installed.
    Sha256,
}

/// Returns binary integrity expectations using default paths (backward compat).
#[must_use]
pub fn binary_integrity_for(
    composition: crate::composition::MembraneComposition,
) -> Vec<BinaryIntegrity> {
    binary_integrity_for_paths(composition, &ServicePaths::from_env())
}

/// Returns binary integrity expectations using configurable `ServicePaths`.
///
/// ecoPrimals binaries: static musl ELFs, BLAKE3 checksums.
/// Symbiotic binaries: SHA-256 from upstream releases.
///
/// Install paths are resolved from `ServicePaths` — no hardcoded assumptions.
#[must_use]
pub fn binary_integrity_for_paths(
    composition: crate::composition::MembraneComposition,
    paths: &ServicePaths,
) -> Vec<BinaryIntegrity> {
    let spec = composition.spec();
    let mut entries = Vec::new();

    for primal in &spec.primals {
        if let Some(svc) = MembraneService::for_binary(primal) {
            entries.push(BinaryIntegrity {
                binary: svc.binary,
                install_path: paths.install_path(svc),
                hash_algorithm: HashAlgorithm::Blake3,
                require_static_musl: true,
            });
        }
    }

    for sym in &spec.symbiotic {
        if let Some(svc) = MembraneService::for_binary(sym) {
            entries.push(BinaryIntegrity {
                binary: svc.binary,
                install_path: paths.install_path(svc),
                hash_algorithm: HashAlgorithm::Sha256,
                require_static_musl: false,
            });
        }
    }

    entries
}
