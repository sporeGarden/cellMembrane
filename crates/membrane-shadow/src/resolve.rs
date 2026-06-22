// SPDX-License-Identifier: AGPL-3.0-or-later

//! Unified transport endpoint resolution — the membrane's channel routing layer.
//!
//! Resolves a `(gate, capability)` pair into a [`TransportEndpoint`] by examining
//! locality, topology, and available transport. This is the abstraction boundary
//! between physical and digital topology: primals call `resolve_endpoint` and get
//! back a transport-agnostic endpoint — UDS, TCP, or mesh relay — without knowing
//! or caring about the underlying path.
//!
//! Resolution order:
//! 1. **Local UDS** — if the capability provider is on this gate, return the socket path
//! 2. **Mesh TCP** — if the peer is reachable via `WireGuard` mesh, return mesh IP + port
//! 3. **Mesh relay** — if the peer is behind NAT or unreachable directly, return
//!    a songBird relay endpoint for multi-hop encrypted transport
//!
//! This implements Phase 3 of the Sovereign Transport Envelope (Wave 121):
//! "Wire `TransportEndpoint.mesh_relay` resolution through songBird mesh."

use cellmembrane_types::service::{MembraneService, ServiceCapability};
use cellmembrane_types::transport::TransportEndpoint;

/// Resolution context — what we know about the local gate and mesh topology.
pub struct ResolutionContext {
    /// Identity of the local gate (e.g. "sporeGate").
    pub local_gate: String,
    /// Socket base directory for local UDS paths.
    pub socket_base: String,
    /// XDG runtime directory for biomeOS socket paths.
    pub xdg_runtime: String,
}

impl ResolutionContext {
    /// Build context from environment.
    #[must_use]
    pub fn from_env() -> Self {
        let local_gate = crate::gate::resolve_local_gate_identity();
        let socket_base = std::env::var(cellmembrane_types::service::ENV_SOCKET_BASE)
            .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_SOCKET_BASE.into());
        let xdg_runtime = std::env::var(cellmembrane_types::service::ENV_XDG_RUNTIME_DIR)
            .unwrap_or_else(|_| format!("/run/user/{}", crate::gate::health::resolve_uid()));
        Self {
            local_gate,
            socket_base,
            xdg_runtime,
        }
    }
}

/// Resolve a transport endpoint for reaching a capability on a specific gate.
///
/// Returns the most direct transport available:
/// - `Uds` if the target is the local gate and has a socket
/// - `Tcp` if the target is reachable via `WireGuard` mesh
/// - `MeshRelay` if the target requires relay infrastructure
///
/// Returns `None` if no transport path can be determined.
#[must_use]
pub(crate) fn resolve_endpoint(
    ctx: &ResolutionContext,
    target_gate: &str,
    capability: ServiceCapability,
) -> Option<TransportEndpoint> {
    let svc = MembraneService::with_capability(capability)?;

    if is_local(ctx, target_gate) {
        return resolve_local_uds(ctx, svc);
    }

    if let Some(tcp) = resolve_mesh_tcp(target_gate, svc) {
        return Some(tcp);
    }

    Some(resolve_mesh_relay(target_gate, capability))
}

/// Resolve a transport endpoint for a capability by role name (string-based).
///
/// Looks up which gate provides the role in the manifest, then resolves
/// the transport to the first provider. Useful for service discovery:
/// `resolve_by_role(ctx, "forgejo")` → endpoint for the gate hosting Forgejo.
#[must_use]
pub(crate) fn resolve_by_role(ctx: &ResolutionContext, role: &str) -> Option<TransportEndpoint> {
    let manifest = load_manifest()?;
    let providers = manifest.gates_for_role(role);
    let (gate_name, profile) = providers.first()?;

    let capability = role_to_capability(role)?;
    let svc = MembraneService::with_capability(capability)?;

    if is_local(ctx, gate_name) {
        return resolve_local_uds(ctx, svc);
    }

    if let Some(ip) = &profile.wg_ip {
        if let Some(port) = svc.port {
            return Some(TransportEndpoint::Tcp {
                host: ip.clone(),
                port,
            });
        }
    }

    Some(resolve_mesh_relay(gate_name, capability))
}

/// Map well-known role names to service capabilities.
///
/// Covers manifest role strings (e.g. `roles = ["forgejo", "relay"]`)
/// and their semantic aliases.
#[must_use]
pub(crate) fn role_to_capability(role: &str) -> Option<ServiceCapability> {
    match role {
        "relay" | "mesh_relay" => Some(ServiceCapability::MeshRelay),
        "turn" | "stun" => Some(ServiceCapability::TurnServer),
        "security" | "crypto" | "auth" => Some(ServiceCapability::CryptoSigner),
        "content" | "forgejo" | "depot" => Some(ServiceCapability::ContentServing),
        "observability" | "metrics" => Some(ServiceCapability::Observability),
        "compute" | "build" | "build_hub" => Some(ServiceCapability::ComputeOrchestration),
        "storage" | "nest" | "ledger" => Some(ServiceCapability::Storage),
        "identity" | "biomeos" => Some(ServiceCapability::Identity),
        _ => None,
    }
}

fn is_local(ctx: &ResolutionContext, target_gate: &str) -> bool {
    ctx.local_gate.eq_ignore_ascii_case(target_gate)
}

/// Resolve a local UDS endpoint — check socket existence in priority order.
///
/// Checks `api_socket` convention (e.g. `neural-api-default.sock` for biomeOS),
/// primary binary socket, and XDG fallbacks. Returns the first existing socket,
/// or the highest-priority candidate if none exist.
fn resolve_local_uds(ctx: &ResolutionContext, svc: &MembraneService) -> Option<TransportEndpoint> {
    if !svc.has_socket {
        return svc.port.map(|port| TransportEndpoint::Tcp {
            host: "127.0.0.1".into(),
            port,
        });
    }

    let ns = cellmembrane_types::service::NEURAL_API_NAMESPACE;
    let mut candidates: Vec<String> = Vec::with_capacity(6);

    if let Some(api) = svc.api_socket {
        candidates.push(format!("{}/{api}-default.sock", ctx.socket_base));
        candidates.push(format!("{}/{api}.sock", ctx.socket_base));
    }
    candidates.push(format!("{}/{}.sock", ctx.socket_base, svc.binary));
    candidates.push(format!("{}/{ns}/{}.sock", ctx.xdg_runtime, svc.binary));
    if let Some(api) = svc.api_socket {
        candidates.push(format!("{}/{ns}/{api}-default.sock", ctx.xdg_runtime));
    }

    for path in &candidates {
        if std::path::Path::new(path).exists() {
            return Some(TransportEndpoint::Uds { path: path.clone() });
        }
    }

    Some(TransportEndpoint::Uds {
        path: candidates[0].clone(),
    })
}

/// Resolve a mesh TCP endpoint via `WireGuard` IP from manifest.
fn resolve_mesh_tcp(target_gate: &str, svc: &MembraneService) -> Option<TransportEndpoint> {
    let port = svc.port?;
    let manifest = load_manifest()?;
    let ip = manifest.mesh_ip_for(target_gate)?;
    Some(TransportEndpoint::Tcp { host: ip, port })
}

/// Build a mesh relay endpoint — routes through songBird relay infrastructure.
fn resolve_mesh_relay(target_gate: &str, capability: ServiceCapability) -> TransportEndpoint {
    TransportEndpoint::MeshRelay {
        peer_id: target_gate.to_string(),
        capability: capability.wire_name().to_string(),
    }
}

fn load_manifest() -> Option<crate::manifest::EcosystemManifest> {
    let root = crate::temporal::resolve_workspace_root().ok()?;
    crate::manifest::load_from_workspace(&root).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ctx(gate: &str) -> ResolutionContext {
        ResolutionContext {
            local_gate: gate.into(),
            socket_base: "/run/membrane".into(),
            xdg_runtime: "/tmp/test-runtime".into(),
        }
    }

    #[test]
    fn local_gate_resolves_uds() {
        let ctx = test_ctx("sporeGate");
        let ep = resolve_endpoint(&ctx, "sporeGate", ServiceCapability::CryptoSigner);
        assert!(ep.is_some());
        let ep = ep.unwrap();
        assert!(ep.is_local() || matches!(ep, TransportEndpoint::Uds { .. }));
    }

    #[test]
    fn remote_gate_resolves_non_local() {
        let ctx = test_ctx("sporeGate");
        let ep = resolve_endpoint(&ctx, "golgi", ServiceCapability::MeshRelay);
        assert!(ep.is_some());
        let ep = ep.unwrap();
        assert!(!ep.is_local() || ep.is_relayed());
    }

    #[test]
    fn mesh_relay_has_peer_and_capability() {
        let ep = resolve_mesh_relay("eastGate", ServiceCapability::CryptoSigner);
        match ep {
            TransportEndpoint::MeshRelay {
                peer_id,
                capability,
            } => {
                assert_eq!(peer_id, "eastGate");
                assert_eq!(capability, "crypto_signer", "wire name must be snake_case");
            }
            _ => panic!("expected MeshRelay"),
        }
    }

    #[test]
    fn mesh_relay_wire_names_are_snake_case() {
        let cases = [
            (ServiceCapability::MeshRelay, "mesh_relay"),
            (ServiceCapability::TurnServer, "turn_server"),
            (ServiceCapability::CryptoSigner, "crypto_signer"),
            (ServiceCapability::Security, "security"),
            (ServiceCapability::Observability, "observability"),
            (ServiceCapability::ContentServing, "content_serving"),
            (ServiceCapability::Storage, "storage"),
            (
                ServiceCapability::ComputeOrchestration,
                "compute_orchestration",
            ),
            (ServiceCapability::Identity, "identity"),
        ];
        for (cap, expected) in cases {
            let ep = resolve_mesh_relay("test", cap);
            let TransportEndpoint::MeshRelay { capability, .. } = ep else {
                panic!("expected MeshRelay");
            };
            assert_eq!(capability, expected, "wire_name mismatch for {cap:?}");
        }
    }

    #[test]
    fn is_local_case_insensitive() {
        let ctx = test_ctx("sporeGate");
        assert!(is_local(&ctx, "sporeGate"));
        assert!(is_local(&ctx, "SPOREGATE"));
        assert!(is_local(&ctx, "SporeGate"));
        assert!(!is_local(&ctx, "eastGate"));
    }

    #[test]
    fn role_mapping_covers_standard_roles() {
        assert!(role_to_capability("relay").is_some());
        assert!(role_to_capability("security").is_some());
        assert!(role_to_capability("content").is_some());
        assert!(role_to_capability("compute").is_some());
        assert!(role_to_capability("storage").is_some());
        assert!(role_to_capability("identity").is_some());
        assert!(role_to_capability("unknown_role").is_none());
    }

    #[test]
    fn role_mapping_covers_manifest_aliases() {
        assert!(role_to_capability("forgejo").is_some());
        assert!(role_to_capability("depot").is_some());
        assert!(role_to_capability("build").is_some());
        assert!(role_to_capability("build_hub").is_some());
        assert!(role_to_capability("turn").is_some());
        assert!(role_to_capability("auth").is_some());
        assert!(role_to_capability("nest").is_some());
        assert!(role_to_capability("biomeos").is_some());
        assert!(role_to_capability("metrics").is_some());
        assert!(
            role_to_capability("dns_primary").is_none(),
            "DNS roles are infra, not primal capabilities"
        );
    }

    #[test]
    fn display_uri_for_all_variants() {
        let uds = TransportEndpoint::Uds {
            path: "/run/membrane/beardog.sock".into(),
        };
        assert!(uds.display_uri().starts_with("unix://"));

        let tcp = TransportEndpoint::Tcp {
            host: "10.13.37.2".into(),
            port: 3478,
        };
        assert!(tcp.display_uri().starts_with("tcp://"));

        let relay = TransportEndpoint::MeshRelay {
            peer_id: "golgi".into(),
            capability: "meshrelay".into(),
        };
        assert!(relay.display_uri().starts_with("mesh://"));
    }

    #[test]
    fn resolve_by_role_returns_none_for_unknown() {
        let ctx = test_ctx("sporeGate");
        let ep = resolve_by_role(&ctx, "nonexistent_role");
        assert!(ep.is_none());
    }

    #[test]
    fn local_uds_for_socket_service() {
        let ctx = test_ctx("test");
        let svc = MembraneService::with_capability(ServiceCapability::CryptoSigner).unwrap();
        let ep = resolve_local_uds(&ctx, svc);
        assert!(ep.is_some());
        match ep.unwrap() {
            TransportEndpoint::Uds { path } => {
                assert!(path.contains("beardog"));
                assert!(path.ends_with(".sock"));
            }
            _ => panic!("expected UDS"),
        }
    }

    #[test]
    fn local_uds_resolves_biomeos_for_identity() {
        let ctx = test_ctx("test");
        let svc = MembraneService::with_capability(ServiceCapability::Identity).unwrap();
        assert_eq!(
            svc.binary, "biomeos",
            "Identity capability should resolve to biomeOS"
        );
        assert_eq!(svc.api_socket, Some("neural-api"));
        let ep = resolve_local_uds(&ctx, svc);
        match ep.unwrap() {
            TransportEndpoint::Uds { path } => {
                assert!(
                    path.contains("neural-api") || path.contains("biomeos"),
                    "biomeOS UDS should resolve to neural-api or biomeos socket, got: {path}"
                );
            }
            _ => panic!("expected UDS for biomeOS"),
        }
    }

    #[test]
    fn local_uds_candidates_include_api_and_binary() {
        let ctx = test_ctx("test");
        let svc = MembraneService::with_capability(ServiceCapability::Identity).unwrap();
        let ep = resolve_local_uds(&ctx, svc).unwrap();
        if let TransportEndpoint::Uds { path } = ep {
            assert!(
                path.contains("neural-api") || path.contains("biomeos"),
                "expected neural-api or biomeos in path, got: {path}"
            );
        }
    }

    #[test]
    fn identity_resolves_to_biomeos_not_sweetgrass() {
        let svc = MembraneService::with_capability(ServiceCapability::Identity).unwrap();
        assert_eq!(svc.binary, "biomeos");
        assert_ne!(svc.binary, "sweetgrass");
    }

    #[test]
    fn resolve_endpoint_identity_on_remote_gate() {
        let ctx = test_ctx("sporeGate");
        let ep = resolve_endpoint(&ctx, "golgi", ServiceCapability::Identity);
        assert!(ep.is_some());
        let ep = ep.unwrap();
        assert!(
            !ep.is_local() || ep.is_relayed(),
            "remote gate should resolve to TCP or relay"
        );
    }
}
