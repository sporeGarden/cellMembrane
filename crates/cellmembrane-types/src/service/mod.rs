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
    #[must_use]
    pub fn exec_args(&self, binary: &str, socket_path: &str, security_socket: &str) -> String {
        match self {
            Self::Full => format!(
                "/opt/membrane/{binary} server --socket {socket_path} --security-socket {security_socket} --pid-dir /run/membrane"
            ),
            Self::SocketAuditDir => format!(
                "/opt/membrane/{binary} server --socket {socket_path} --audit-dir /var/lib/membrane/{binary}"
            ),
            Self::SocketOnly | Self::Tarpc => format!(
                "/opt/membrane/{binary} server --socket {socket_path}"
            ),
            Self::BiomeosApi => format!(
                "/opt/membrane/{binary} api --socket {socket_path}"
            ),
            Self::External => format!("/opt/membrane/{binary}"),
        }
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

/// Bind to all interfaces (externally reachable).
pub const BIND_ALL: &str = "0.0.0.0";
/// Bind to loopback only (not externally reachable).
pub const BIND_LOOPBACK: &str = "127.0.0.1";

/// Default base path for primal binary installations.
/// Override with `MEMBRANE_INSTALL_BASE` env var or membrane.toml config.
pub const DEFAULT_INSTALL_BASE: &str = "/opt/membrane";

/// Default base path for primal UDS sockets.
pub const DEFAULT_SOCKET_BASE: &str = "/run/membrane";

/// Default configuration directory (system-wide config files).
pub const DEFAULT_CONFIG_DIR: &str = "/etc/membrane";

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
/// Environment variable for the membrane configuration directory.
pub const ENV_CONFIG_DIR: &str = "MEMBRANE_CONFIG_DIR";
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

/// Default socket filename for the biomeOS Neural API.
pub const NEURAL_API_SOCKET_NAME: &str = "neural-api-default.sock";

/// Namespace directory for biomeOS runtime sockets (under `XDG_RUNTIME_DIR` or /tmp).
pub const NEURAL_API_NAMESPACE: &str = "biomeos";
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
/// Environment variable for the membrane SSH external host (golgiBody-ext).
pub const ENV_SSH_HOST_EXT: &str = "MEMBRANE_SSH_HOST_EXT";
/// Environment variable for the golgiBody external host (relay target).
pub const ENV_GOLGI_EXT_HOST: &str = "GOLGI_EXT_HOST";
/// Environment variable for the Cloudflare API token.
pub const ENV_CLOUDFLARE_TOKEN: &str = "CLOUDFLARE_API_TOKEN";
/// Environment variable for the Cloudflare zone ID.
pub const ENV_CLOUDFLARE_ZONE: &str = "CLOUDFLARE_ZONE_ID";
/// Environment variable for the relay Forgejo remote name.
pub const ENV_RELAY_FORGEJO_REMOTE: &str = "RELAY_FORGEJO_REMOTE";
/// Environment variable for the `nestGate` content path.
pub const ENV_NESTGATE_CONTENT_PATH: &str = "NESTGATE_CONTENT_PATH";
/// Environment variable for the `nestGate` HTTP port.
pub const ENV_NESTGATE_PORT: &str = "NESTGATE_PORT";
/// Environment variable for the VPS membrane binary directory.
pub const ENV_VPS_BIN_DIR: &str = "VPS_MEMBRANE_BIN_DIR";
/// Environment variable for the songbird configuration path.
pub const ENV_SONGBIRD_CONFIG: &str = "SONGBIRD_CONFIG_PATH";
/// Environment variable for SSH connection timeout (seconds).
pub const ENV_SSH_TIMEOUT: &str = "SSH_TIMEOUT";
/// Environment variable for the Forgejo data directory path.
pub const ENV_FORGEJO_DATA_DIR: &str = "FORGEJO_DATA_DIR";
/// Environment variable for the Forgejo work directory path.
pub const ENV_FORGEJO_WORK_DIR: &str = "FORGEJO_WORK_DIR";
/// Environment variable for the Forgejo admin username.
pub const ENV_FORGEJO_ADMIN_USER: &str = "FORGEJO_ADMIN_USER";
/// Environment variable for the membrane service filter (systemd unit prefix).
pub const ENV_SERVICE_FILTER: &str = "MEMBRANE_SERVICE_FILTER";
/// Environment variable for the WAN depot base URL (outer membrane HTTPS endpoint).
pub const ENV_WAN_DEPOT_URL: &str = "WAN_DEPOT_URL";

/// Default WAN depot base URL served by Caddy on the sovereign membrane surface.
pub const DEFAULT_WAN_DEPOT_URL: &str = "https://membrane.primals.eco/depot";

/// Default VPS host (golgiBody sovereign surface).
pub const DEFAULT_VPS_HOST: &str = "157.230.3.183";

/// Default songbird federation port.
pub const DEFAULT_FEDERATION_PORT: u16 = 7700;

/// Default TURN relay port.
pub const DEFAULT_TURN_PORT: u16 = 3478;

/// RustDesk hbbs (ID/rendezvous server) port.
pub const RUSTDESK_HBBS_PORT: u16 = 21115;
/// RustDesk hbbr (relay server) port.
pub const RUSTDESK_HBBR_PORT: u16 = 21117;

/// Default VPS mesh peer address (golgiBody songbird federation endpoint).
pub const DEFAULT_VPS_MESH_PEER: &str = "157.230.3.183:7700";

/// Environment variable override for the VPS mesh peer address (host only).
pub const ENV_VPS_MESH_PEER: &str = "MEMBRANE_VPS_PEER";

// ── Standard system environment variables ────────────────────────────

/// XDG base directory for user data (fallback: `~/.local/share`).
pub const ENV_XDG_DATA_HOME: &str = "XDG_DATA_HOME";
/// XDG runtime directory (e.g. `/run/user/1000`).
pub const ENV_XDG_RUNTIME_DIR: &str = "XDG_RUNTIME_DIR";
/// XDG config directory (fallback: `~/.config`).
pub const ENV_XDG_CONFIG_HOME: &str = "XDG_CONFIG_HOME";
/// User home directory.
pub const ENV_HOME: &str = "HOME";
/// System hostname.
pub const ENV_HOSTNAME: &str = "HOSTNAME";
/// Alternate hostname variable (some systems use HOST instead of HOSTNAME).
pub const ENV_HOST: &str = "HOST";
/// Cloudflare API token (alternate alias used by `wrangler`/Cloudflare tooling).
pub const ENV_CF_API_TOKEN: &str = "CF_API_TOKEN";
/// Cloudflare zone ID (alternate alias used by `wrangler`/Cloudflare tooling).
pub const ENV_CF_ZONE_ID: &str = "CF_ZONE_ID";

/// Forgejo SSH git server address (host:port).
pub const ENV_FORGEJO_GIT_ADDR: &str = "FORGEJO_GIT_ADDR";
/// Default Forgejo SSH address for git operations.
pub const DEFAULT_FORGEJO_GIT_ADDR: &str = "git.primals.eco:2222";

/// GitHub organization name (for release artifact URLs).
pub const ENV_GITHUB_ORG: &str = "MEMBRANE_GITHUB_ORG";
/// Default GitHub organization.
pub const DEFAULT_GITHUB_ORG: &str = "ecoPrimals";

/// Forgejo organization name (for repo paths).
pub const ENV_FORGEJO_ORG: &str = "MEMBRANE_FORGEJO_ORG";
/// Default Forgejo organization.
pub const DEFAULT_FORGEJO_ORG: &str = "sporeGarden";

/// WAN depot hostname (used in Caddy config and depot URLs).
pub const ENV_DEPOT_HOSTNAME: &str = "MEMBRANE_DEPOT_HOSTNAME";
/// Default depot hostname served by Caddy.
pub const DEFAULT_DEPOT_HOSTNAME: &str = "membrane.primals.eco";

/// Sovereign git remote name — authority-first push target.
///
/// This is the canonical remote that the temporal sync system converges to
/// before pushing to mirror remotes. Override for non-standard deployments.
pub const ENV_SOVEREIGN_REMOTE: &str = "MEMBRANE_SOVEREIGN_REMOTE";
/// Default sovereign remote name.
pub const DEFAULT_SOVEREIGN_REMOTE: &str = "forgejo";

/// When set to `1`/`true`/`yes`, cascade auto-triggers harvest+sandbox+refresh
/// when depot staleness is detected (production gates only).
pub const ENV_AUTO_REBUILD: &str = "MEMBRANE_AUTO_REBUILD";

/// `DigitalOcean` API token for cloud provisioning (fieldMouse droplets).
/// Fallback: `DO_TOKEN` (doctl-compatible).
pub const ENV_DIGITALOCEAN_TOKEN: &str = "DIGITALOCEAN_TOKEN";

/// Fallback binary name for the crypto signer capability.
///
/// DEPRECATED: Use `MembraneService::binary_for(ServiceCapability::CryptoSigner)` instead.
/// Retained only for transitional compatibility — will be removed in Wave 114.
pub const FALLBACK_CRYPTO_SIGNER: &str = "beardog";

/// Fallback binary name for the mesh relay capability.
///
/// DEPRECATED: Use `MembraneService::binary_for(ServiceCapability::MeshRelay)` instead.
pub const FALLBACK_MESH_RELAY: &str = "songbird";

/// Fallback binary name for content serving.
///
/// DEPRECATED: Use `MembraneService::binary_for(ServiceCapability::ContentServing)` instead.
pub const FALLBACK_CONTENT_SERVING: &str = "nestgate";

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
        match Self::with_capability(cap) {
            Some(svc) => svc.binary,
            None => match cap {
                ServiceCapability::CryptoSigner => "beardog",
                ServiceCapability::MeshRelay => "songbird",
                ServiceCapability::ContentServing => "nestgate",
                _ => "unknown",
            },
        }
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
