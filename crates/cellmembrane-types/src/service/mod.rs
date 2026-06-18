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

pub mod constants;
pub mod integrity;

pub use constants::*;
pub use integrity::{
    BinaryIntegrity, HashAlgorithm, binary_integrity_for, binary_integrity_for_paths,
};

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

/// Server CLI contract — describes what args a primal's `server` subcommand accepts.
///
/// Each primal has evolved independently, resulting in CLI divergence. This enum
/// captures the actual capabilities so template systemd units can generate correct
/// `ExecStart` lines without trial and error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerContract {
    /// Full guideStone P4 contract: `server --socket <path> --security-socket <path> --pid-dir <path>`
    /// Used by: songbird, skunkbat
    Full,
    /// Socket + audit-dir: `server --socket <path> --audit-dir <path>`
    /// Used by: beardog (crypto spine)
    SocketAuditDir,
    /// Socket-only: `server --socket <path>`
    /// Used by: sweetgrass, nestgate, coralreef, squirrel, petaltongue, barracuda, toadstool
    SocketOnly,
    /// biomeOS-style: `api --socket <path>` or `neural-api --socket <path>`
    /// Used by: biomeos
    BiomeosApi,
    /// External binary with no `server` subcommand — started by systemd with args in the unit.
    /// Used by: hbbs, hbbr, caddy
    External,
    /// Tarpc server (non-JSON-RPC) — uses port binding, not socket.
    /// Used by: loamspine, rhizocrypt
    Tarpc,
}

impl ServerContract {
    /// Generate the `ExecStart` args for a primal given socket/security paths.
    ///
    /// Uses `install_base` to allow deployment to non-standard locations.
    #[must_use]
    pub fn exec_args_with_base(
        &self,
        install_base: &str,
        binary: &str,
        socket_path: &str,
        security_socket: &str,
    ) -> String {
        match self {
            Self::Full => format!(
                "{install_base}/{binary} server --socket {socket_path} --security-socket {security_socket} --pid-dir /run/membrane"
            ),
            Self::SocketAuditDir => format!(
                "{install_base}/{binary} server --socket {socket_path} --audit-dir /var/lib/membrane/{binary}"
            ),
            Self::SocketOnly | Self::Tarpc => {
                format!("{install_base}/{binary} server --socket {socket_path}")
            }
            Self::BiomeosApi => format!("{install_base}/{binary} api --socket {socket_path}"),
            Self::External => format!("{install_base}/{binary}"),
        }
    }

    /// Generate the `ExecStart` args using the default install base.
    #[must_use]
    pub fn exec_args(&self, binary: &str, socket_path: &str, security_socket: &str) -> String {
        self.exec_args_with_base(DEFAULT_INSTALL_BASE, binary, socket_path, security_socket)
    }
}

/// Capability tag for runtime discovery.
///
/// Instead of hardcoding binary names ("songbird", "beardog") in production
/// code, services declare capabilities and consumers discover providers
/// through the registry at compile time or runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceCapability {
    /// Mesh relay — provides peer-to-peer connectivity and message routing.
    MeshRelay,
    /// TURN server — NAT traversal for real-time connections.
    TurnServer,
    /// Cryptographic signing — ed25519 signatures, key management.
    CryptoSigner,
    /// Security enforcement — authentication, authorization, secrets.
    Security,
    /// Observability — metrics collection, health aggregation.
    Observability,
    /// Content serving — static file / API serving.
    ContentServing,
    /// Storage — persistent data management.
    Storage,
    /// Compute orchestration — job scheduling, pipeline execution.
    ComputeOrchestration,
    /// Identity — gate identity, certificate management.
    Identity,
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
            install_base: std::env::var(ENV_INSTALL_BASE)
                .unwrap_or_else(|_| DEFAULT_INSTALL_BASE.to_string()),
            socket_base: std::env::var(ENV_SOCKET_BASE)
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
    /// Declared capabilities — used for runtime discovery instead of name matching.
    pub capabilities: &'static [ServiceCapability],
    /// Server CLI contract — describes which args the primal's `server` subcommand accepts.
    /// Used by NUCLEUS template units to generate correct `ExecStart` lines per-primal.
    pub server_contract: ServerContract,
    /// Alternative socket name for JSON-RPC probing (e.g. `"neural-api"` for biomeOS).
    /// When `Some`, health probes prefer this over `{binary}.sock`.
    pub api_socket: Option<&'static str>,
    /// Capability socket aliases this primal exposes (in addition to `{binary}.sock`).
    ///
    /// Each primal may create additional sockets named by capability rather than
    /// binary. This registry allows bootstrap to predict the full socket set and
    /// health probes to verify capability presence.
    pub socket_aliases: &'static [&'static str],
}

mod registry;
use registry::ALL_SERVICES;

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

    /// Whether this service declares the given capability.
    #[must_use]
    pub fn has_capability(&self, cap: ServiceCapability) -> bool {
        self.capabilities.contains(&cap)
    }

    /// Discover the first service providing a given capability.
    ///
    /// Eliminates hardcoded binary-name lookups — consumers discover
    /// providers by what they do, not what they're called.
    #[must_use]
    pub fn with_capability(cap: ServiceCapability) -> Option<&'static Self> {
        ALL_SERVICES.iter().find(|s| s.has_capability(cap))
    }

    /// Resolve the binary name for a given capability.
    ///
    /// The registry is compile-time complete — every standard capability has
    /// exactly one canonical provider. This eliminates the need for `FALLBACK_*`
    /// constants and hardcoded primal names at call sites.
    #[must_use]
    pub fn binary_for(cap: ServiceCapability) -> &'static str {
        Self::with_capability(cap).map_or("unknown", |svc| svc.binary)
    }

    /// All services declaring a given capability (for multi-provider scenarios).
    #[must_use]
    pub fn all_with_capability(cap: ServiceCapability) -> Vec<&'static Self> {
        ALL_SERVICES
            .iter()
            .filter(|s| s.has_capability(cap))
            .collect()
    }

    /// Whether this service should be started after the mesh relay.
    ///
    /// Services providing `MeshRelay` are infrastructure — they must start
    /// before other primals that depend on connectivity.
    #[must_use]
    pub fn is_mesh_infrastructure(&self) -> bool {
        self.has_capability(ServiceCapability::MeshRelay)
            || self.has_capability(ServiceCapability::TurnServer)
    }
}
