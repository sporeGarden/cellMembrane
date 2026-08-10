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

pub mod capability;
pub mod constants;
pub mod integrity;
pub mod ipc;
pub mod resolve;

pub use capability::{
    HealthCheckMethod, Protocol, ServerContract, ServiceCapability, TransportMode,
};
pub use constants::*;
pub use integrity::{
    BinaryIntegrity, HashAlgorithm, binary_integrity_for, binary_integrity_for_paths,
};
pub use ipc::{
    IpcProtocol, PROTOCOL_NEGOTIATION_PREFIX, PROTOCOL_NEGOTIATION_RESPONSE,
    PROTOCOL_NEGOTIATION_TIMEOUT_MS,
};
pub use resolve::*;

use crate::composition::MembraneComposition;
use std::fmt;

// ── ServicePaths — runtime path resolution ──────────────────────────────

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
    ///
    /// Prefers `api_socket` name when available (e.g. biomeOS → `neural-api-default.sock`),
    /// falling back to `{binary}.sock`.
    #[must_use]
    pub fn socket_path(&self, service: &MembraneService) -> Option<String> {
        if !service.has_socket {
            return None;
        }
        let name = service
            .api_socket
            .map_or_else(|| service.binary.to_owned(), |api| format!("{api}-default"));
        Some(format!("{}/{name}.sock", self.socket_base))
    }

    /// Resolve tarpc socket path for a dual-protocol service.
    ///
    /// Returns `None` if the service doesn't support tarpc.
    /// Path follows the Cephalization convention: `{socket_base}/{binary}.tarpc.sock`.
    #[must_use]
    pub fn tarpc_socket_path(&self, service: &MembraneService) -> Option<String> {
        if !service.has_tarpc() {
            return None;
        }
        Some(format!(
            "{}/{}.tarpc.sock",
            self.socket_base, service.binary
        ))
    }
}

impl Default for ServicePaths {
    fn default() -> Self {
        Self::from_env()
    }
}

/// Const-compatible string equality (byte-by-byte).
const fn const_str_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

// ── MembraneService — static service registry entry ─────────────────────

/// A single membrane service (one running process).
///
/// This struct is compile-time data. The service registry (in `registry.rs`)
/// is an array of const `MembraneService` values — zero allocations, zero
/// runtime cost.
///
/// All fields are `&'static str` — service definitions are compile-time
/// constants, not runtime-allocated data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools, reason = "registry entry — each bool is an independent service capability flag")]
pub struct MembraneService {
    /// Binary name on disk (e.g. `"beardog"`, `"songbird"`).
    pub binary: &'static str,
    /// Systemd unit name (e.g. `"beardog-membrane.service"`).
    pub systemd_unit: &'static str,
    /// Primary network port (`None` for UDS-only services).
    pub port: Option<u16>,
    /// Primary transport protocol.
    pub protocol: Protocol,
    /// Whether this service creates a UDS socket for IPC.
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
    /// IPC protocols this primal supports (G65 Cephalization).
    ///
    /// Primals that support protocol negotiation list all protocols they can
    /// serve. During the C2→G65 transition, `has_tarpc()` is derived from this.
    /// Under G65, a single socket negotiates the best protocol at connection
    /// time. All primals implicitly support `JsonRpc` (the fallback).
    pub protocols: &'static [IpcProtocol],
    /// Capability socket aliases this primal exposes (in addition to `{binary}.sock`).
    ///
    /// Each primal may create additional sockets named by capability rather than
    /// binary. This registry allows bootstrap to predict the full socket set and
    /// health probes to verify capability presence.
    pub socket_aliases: &'static [&'static str],
    /// Whether this primal requires signed depot lineage (post-primordial trust).
    ///
    /// Post-primordial primals cannot be locally built on consumer gates —
    /// binaries must chain back to a recognized build authority with valid
    /// BLAKE3 + provenance signatures.
    pub requires_signed_lineage: bool,
    /// Whether this primal needs a glibc build for GPU/dlopen access.
    ///
    /// GPU primals use `x86_64-unknown-linux-gnu` (glibc) instead of musl
    /// because GPU drivers require runtime `dlopen`. The manifest's `gpu = true`
    /// field is the runtime source of truth; this flag is the compile-time fallback.
    pub gpu_required: bool,
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
        !const_str_eq(self.bind, constants::BIND_LOOPBACK)
            && !matches!(self.protocol, Protocol::Uds)
    }

    /// Whether this service uses UDS-only transport on VPS (Wave 56 standard).
    #[must_use]
    pub const fn is_uds_only(&self) -> bool {
        matches!(self.vps_transport, TransportMode::UdsOnly)
    }

    /// Whether this primal supports tarpc (derived from `protocols` field).
    ///
    /// Backward-compatible accessor for the C2→G65 transition period.
    #[must_use]
    pub fn has_tarpc(&self) -> bool {
        self.protocols.contains(&IpcProtocol::Tarpc)
    }

    /// Whether this primal supports G65 protocol negotiation.
    ///
    /// A primal supports G65 if it declares more than just `JsonRpc` in its
    /// protocols list. G65 primals serve all protocols on a single socket.
    #[must_use]
    pub const fn supports_negotiation(&self) -> bool {
        self.protocols.len() > 1
    }

    /// Resolve install path using configurable `ServicePaths` (capability-based).
    ///
    /// Uses `system_install_path` for system services, otherwise derives
    /// from the configured install base. Removes the `/opt/membrane/` assumption.
    #[must_use]
    pub fn resolved_install_path(&self, paths: &ServicePaths) -> String {
        paths.install_path(self)
    }

    /// Resolve JSON-RPC socket path using configurable `ServicePaths`.
    #[must_use]
    pub fn resolved_socket_path(&self, paths: &ServicePaths) -> Option<String> {
        paths.socket_path(self)
    }

    /// Resolve tarpc socket path using configurable `ServicePaths`.
    ///
    /// Returns `None` if the primal doesn't support tarpc.
    /// Under G65, this path is deprecated — all protocols share one socket.
    #[must_use]
    pub fn resolved_tarpc_socket_path(&self, paths: &ServicePaths) -> Option<String> {
        paths.tarpc_socket_path(self)
    }

    /// Platform-appropriate default transport endpoint for this service (G66).
    ///
    /// On Unix: returns `Uds` at the resolved socket path.
    /// On Windows: returns `NamedPipe` or `Tcp` depending on service type.
    /// Callers should prefer `TRANSPORT_ENDPOINT` env var when available.
    #[must_use]
    pub fn default_endpoint(&self) -> crate::TransportEndpoint {
        if self.has_socket {
            let socket_base = resolve_socket_base();
            crate::TransportEndpoint::local_ipc(self.binary, &socket_base)
        } else if let Some(port) = self.port {
            crate::TransportEndpoint::Tcp {
                host: self.bind.to_string(),
                port,
            }
        } else {
            let socket_base = resolve_socket_base();
            crate::TransportEndpoint::local_ipc(self.binary, &socket_base)
        }
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

    /// Resolve the full service entry for a given capability.
    ///
    /// The registry is compile-time complete — every standard capability has
    /// exactly one canonical provider.
    ///
    /// # Panics
    ///
    /// Panics if no service is registered for the given capability,
    /// which indicates a registry definition gap.
    #[must_use]
    pub fn require_capability(cap: ServiceCapability) -> &'static Self {
        Self::with_capability(cap)
            .unwrap_or_else(|| panic!("no service registered for capability {cap:?}"))
    }

    /// Resolve the binary name for a given capability.
    ///
    /// Convenience wrapper around [`require_capability`](Self::require_capability).
    ///
    /// # Panics
    ///
    /// Panics if no service is registered for the given capability.
    #[must_use]
    pub fn binary_for(cap: ServiceCapability) -> &'static str {
        Self::require_capability(cap).binary
    }

    /// Look up a service by binary name, panicking if absent.
    ///
    /// Use for named-composition patterns where the binary name is an
    /// architectural constant (e.g. sporePrint roles).
    ///
    /// # Panics
    ///
    /// Panics if the binary is not in the registry.
    #[must_use]
    pub fn require_binary(name: &str) -> &'static Self {
        Self::for_binary(name)
            .unwrap_or_else(|| panic!("binary {name:?} must exist in service registry"))
    }

    /// All services declaring a given capability (for multi-provider scenarios).
    #[must_use]
    pub fn all_with_capability(cap: ServiceCapability) -> Vec<&'static Self> {
        ALL_SERVICES
            .iter()
            .filter(|s| s.has_capability(cap))
            .collect()
    }

    /// All primals requiring signed depot lineage (post-primordial trust).
    #[must_use]
    pub fn post_primordial_primals() -> Vec<&'static Self> {
        ALL_SERVICES
            .iter()
            .filter(|s| s.requires_signed_lineage)
            .collect()
    }

    /// All primal binary names requiring signed depot lineage.
    #[must_use]
    pub fn post_primordial_names() -> Vec<&'static str> {
        Self::post_primordial_primals()
            .into_iter()
            .map(|s| s.binary)
            .collect()
    }

    /// All primals that need glibc builds for GPU/dlopen access.
    #[must_use]
    pub fn gpu_primals() -> Vec<&'static Self> {
        ALL_SERVICES.iter().filter(|s| s.gpu_required).collect()
    }

    /// All GPU primal binary names (compile-time fallback).
    #[must_use]
    pub fn gpu_names() -> Vec<&'static str> {
        Self::gpu_primals().into_iter().map(|s| s.binary).collect()
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

    /// Core primals required for a gateway deployment (Tower composition).
    ///
    /// Returns registry entries for the security + mesh relay primals that
    /// every gateway needs. Replaces hardcoded binary name checks in
    /// deploy validation.
    #[must_use]
    pub fn gateway_primals() -> Vec<&'static Self> {
        Self::for_composition(MembraneComposition::Tower)
            .into_iter()
            .filter(|s| s.is_primal)
            .collect()
    }

    /// Build a systemd service filter regex (ERE) from the registry.
    ///
    /// Collects all unique binary names from the registry plus non-registry
    /// infrastructure services (forgejo, fail2ban). The result is suitable
    /// for `grep -E` filtering of `systemctl list-units`.
    #[must_use]
    pub fn build_service_filter() -> String {
        let mut parts: Vec<&str> = ALL_SERVICES.iter().map(|s| s.binary).collect();
        for extra in constants::INFRA_SERVICE_FILTER_EXTRAS {
            if !parts.contains(extra) {
                parts.push(extra);
            }
        }
        parts.dedup();
        parts.join("|")
    }
}

impl fmt::Display for MembraneService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.binary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_tarpc_derived_from_protocols() {
        let svc = MembraneService::for_binary("loamspine").expect("loamspine in registry");
        assert!(svc.has_tarpc());

        let ext = MembraneService::for_binary("caddy").expect("caddy in registry");
        assert!(!ext.has_tarpc());
    }

    #[test]
    fn supports_negotiation_dual_protocol() {
        let svc = MembraneService::for_binary("beardog").expect("beardog in registry");
        assert!(svc.supports_negotiation());

        let ext = MembraneService::for_binary("hbbs").expect("hbbs in registry");
        assert!(!ext.supports_negotiation());
    }

    #[test]
    fn all_primals_support_dual_protocol() {
        for svc in MembraneService::all() {
            if svc.is_primal {
                assert!(
                    svc.has_tarpc(),
                    "{} is a primal but doesn't support tarpc",
                    svc.binary
                );
                assert!(
                    svc.protocols.contains(&IpcProtocol::JsonRpc),
                    "{} missing JsonRpc",
                    svc.binary
                );
            }
        }
    }

    #[test]
    fn g65_shipped_primals_use_socket_only_contract() {
        for name in ["squirrel", "beardog", "sweetgrass", "rhizocrypt"] {
            let svc = MembraneService::for_binary(name).expect(name);
            assert_ne!(
                svc.server_contract,
                ServerContract::Tarpc,
                "{name} shipped G65 but still has Tarpc contract"
            );
        }
    }

    #[test]
    fn default_endpoint_uds_for_socket_primal() {
        let svc = MembraneService::for_binary("beardog").expect("beardog in registry");
        let ep = svc.default_endpoint();
        if cfg!(unix) {
            assert!(
                matches!(ep, crate::TransportEndpoint::Uds { .. }),
                "beardog should get UDS on Unix: {ep:?}"
            );
        }
    }

    #[test]
    fn default_endpoint_tcp_for_non_socket_service() {
        let svc = MembraneService::for_binary("songbird").expect("songbird in registry");
        if !svc.has_socket {
            let ep = svc.default_endpoint();
            assert!(
                matches!(ep, crate::TransportEndpoint::Tcp { .. }),
                "non-socket service should get TCP: {ep:?}"
            );
        }
    }

    #[test]
    fn post_primordial_registry_coverage() {
        let names = MembraneService::post_primordial_names();
        assert!(names.contains(&"beardog"));
        assert!(names.contains(&"songbird"));
        assert!(names.contains(&"skunkbat"));
        assert!(names.contains(&"nestgate"));
        assert!(names.contains(&"biomeos"));
        assert!(!names.contains(&"squirrel"));
    }

    #[test]
    fn gpu_registry_coverage() {
        let names = MembraneService::gpu_names();
        assert!(names.contains(&"barracuda"));
        assert!(names.contains(&"coralreef"));
        assert!(!names.contains(&"beardog"));
    }
}
