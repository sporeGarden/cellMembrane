// SPDX-License-Identifier: AGPL-3.0-or-later

//! Mesh peer configuration — songbird federation and LAN peer discovery.
//!
//! Extracted from `bootstrap.rs` to isolate mesh topology concerns from the
//! bootstrap orchestrator. Handles transport resolution, UDS-based mesh.init
//! JSON-RPC calls, and multi-peer negotiation.

use super::BootstrapPhase;
use cellmembrane_types::GateTransport;

/// Map a gate profile transport to `FetchSource`.
///
/// `local` uses SSH/rsync (VPS layer). All remote transports currently
/// resolve to WAN HTTPS. As LAN rsync and ADB push mature, they will
/// diverge from the WAN fallback.
pub(super) const fn transport_to_fetch_source(
    transport: GateTransport,
) -> crate::plasmid::FetchSource {
    match transport {
        GateTransport::Local => crate::plasmid::FetchSource::Vps,
        _ => crate::plasmid::FetchSource::Wan,
    }
}

/// Resolved gate profile fields from the ecosystem manifest.
///
/// Fields beyond `transport` and `mesh_peer` are staged for profile-driven
/// bootstrap evolution (Wave 117+: composition-aware NUCLEUS, manifest mobility).
#[derive(Default)]
pub(super) struct GateManifestProfile {
    pub transport: GateTransport,
    pub mesh_peer: Option<String>,
    #[allow(dead_code, reason = "staged for composition-aware NUCLEUS (Wave 117+)")]
    pub mobility: Option<String>,
    #[allow(dead_code, reason = "staged for composition-aware NUCLEUS (Wave 117+)")]
    pub composition: Option<String>,
}

/// Resolve gate profile fields from the ecosystem manifest.
pub(super) fn resolve_gate_profile(gate_name: &str) -> GateManifestProfile {
    let Ok(workspace_root) = crate::temporal::resolve_workspace_root() else {
        return GateManifestProfile::default();
    };
    let Ok(manifest) = crate::manifest::load_from_workspace(&workspace_root) else {
        return GateManifestProfile::default();
    };
    manifest
        .gates
        .get(gate_name)
        .map_or_else(GateManifestProfile::default, |p| GateManifestProfile {
            transport: p.transport.unwrap_or_default(),
            mesh_peer: p.mesh_peer.clone(),
            mobility: p.mobility.clone(),
            composition: p.composition.clone(),
        })
}

/// Resolve just the transport mode (backwards compat helper).
pub(super) fn resolve_gate_transport(gate_name: &str) -> GateTransport {
    resolve_gate_profile(gate_name).transport
}

/// Construct the mesh configuration phase.
///
/// If the gate's manifest profile specifies a `mesh_peer`, it takes priority over
/// the `MEMBRANE_VPS_MESH_PEER` env var and the compiled-in default.
pub(super) async fn mesh_phase(gate_name: &str, arch: &str, dry_run: bool) -> BootstrapPhase {
    let profile = resolve_gate_profile(gate_name);
    if dry_run {
        let vps_peer = resolve_primary_peer(profile.mesh_peer.as_deref());
        let extra = std::env::var(cellmembrane_types::service::ENV_MESH_PEERS).unwrap_or_default();
        let peer_info = if extra.is_empty() {
            format!("1 peer ({vps_peer})")
        } else {
            let count = 1 + extra.split(',').filter(|p| !p.trim().is_empty()).count();
            format!("{count} peers ({vps_peer} + LAN)")
        };
        return BootstrapPhase {
            name: "mesh.configure".into(),
            ok: true,
            detail: format!("dry-run: would mesh.init {peer_info} as {gate_name}"),
        };
    }
    let (ok, detail) = configure_mesh(gate_name, arch, profile.mesh_peer.as_deref()).await;
    BootstrapPhase {
        name: "mesh.configure".into(),
        ok,
        detail,
    }
}

/// Resolve the primary mesh peer: manifest profile > env var > compiled default.
fn resolve_primary_peer(manifest_peer: Option<&str>) -> String {
    manifest_peer.map_or_else(
        || {
            std::env::var(cellmembrane_types::service::ENV_VPS_MESH_PEER)
                .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_VPS_MESH_PEER.into())
        },
        String::from,
    )
}

/// Configure mesh peering via songbird UDS JSON-RPC.
async fn configure_mesh(
    gate_name: &str,
    arch: &str,
    manifest_peer: Option<&str>,
) -> (bool, String) {
    let relay_binary = cellmembrane_types::MembraneService::binary_for(
        cellmembrane_types::ServiceCapability::MeshRelay,
    );

    let dest_root = super::resolve_plasmidbin_dir();
    let relay_bin = dest_root.join("primals").join(arch).join(relay_binary);

    if !relay_bin.exists() {
        return (false, format!("{relay_binary} binary not found"));
    }

    let socket_dir = super::health::resolve_biomeos_socket_dir();
    let socket_path = std::path::PathBuf::from(&socket_dir)
        .join(format!("{relay_binary}.sock"))
        .display()
        .to_string();

    for _ in 0..5 {
        if std::path::Path::new(&socket_path).exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }

    if !std::path::Path::new(&socket_path).exists() {
        return (
            false,
            format!(
                "{relay_binary} socket not found at {socket_path} — start {relay_binary} first"
            ),
        );
    }

    let vps_peer = resolve_primary_peer(manifest_peer);

    let mut peers: Vec<String> = vec![vps_peer.clone()];
    if let Ok(extra) = std::env::var(cellmembrane_types::service::ENV_MESH_PEERS) {
        for p in extra.split(',') {
            let trimmed = p.trim().to_string();
            if !trimmed.is_empty() && !peers.contains(&trimmed) {
                peers.push(trimmed);
            }
        }
    }

    let params = serde_json::json!({
        "node_id": gate_name,
        "peers": peers,
    });
    let request = crate::jsonrpc::request_with_params("mesh.init", &params, 1);

    match crate::jsonrpc::call(std::path::Path::new(&socket_path), &request).await {
        Ok(response) => {
            let peer_count = peers.len();
            if response.contains("\"result\"") || response.contains("\"ok\"") {
                (
                    true,
                    format!("mesh.init sent ({peer_count} peers) as {gate_name}"),
                )
            } else {
                (
                    true,
                    format!(
                        "mesh.init sent ({peer_count} peers, response: {})",
                        response.trim()
                    ),
                )
            }
        }
        Err(e) => (false, format!("mesh.init failed: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_local_maps_to_vps() {
        let source = transport_to_fetch_source(GateTransport::Local);
        assert_eq!(source, crate::plasmid::FetchSource::Vps);
    }

    #[test]
    fn transport_wan_maps_to_wan() {
        let source = transport_to_fetch_source(GateTransport::Wan);
        assert_eq!(source, crate::plasmid::FetchSource::Wan);
    }

    #[test]
    fn transport_non_local_maps_to_wan() {
        for transport in [GateTransport::Lan, GateTransport::Adb] {
            let source = transport_to_fetch_source(transport);
            assert_eq!(
                source,
                crate::plasmid::FetchSource::Wan,
                "transport '{transport}' should map to Wan"
            );
        }
    }

    #[tokio::test]
    async fn mesh_phase_dry_run_returns_ok() {
        let phase = mesh_phase("testGate", "x86_64-unknown-linux-musl", true).await;
        assert!(phase.ok, "dry-run should always succeed");
        assert_eq!(phase.name, "mesh.configure");
        assert!(phase.detail.contains("dry-run"));
        assert!(phase.detail.contains("testGate"));
    }

    #[tokio::test]
    async fn mesh_phase_dry_run_detail_mentions_peer_count() {
        let phase = mesh_phase("testGate", "x86_64", true).await;
        assert!(
            phase.detail.contains("peer"),
            "dry-run detail should mention peers, got: {}",
            phase.detail
        );
    }

    #[test]
    fn resolve_primary_peer_prefers_manifest() {
        let peer = resolve_primary_peer(Some("10.13.37.1:7700"));
        assert_eq!(peer, "10.13.37.1:7700");
    }

    #[test]
    fn resolve_primary_peer_falls_back_to_default() {
        let peer = resolve_primary_peer(None);
        assert!(
            !peer.is_empty(),
            "should fall back to env or compiled default"
        );
    }

    #[test]
    fn gate_manifest_profile_defaults() {
        let p = GateManifestProfile::default();
        assert_eq!(p.transport, GateTransport::Wan);
        assert!(p.mesh_peer.is_none());
        assert!(p.mobility.is_none());
        assert!(p.composition.is_none());
    }
}
