// SPDX-License-Identifier: AGPL-3.0-or-later

//! Mesh peer configuration — songbird federation and LAN peer discovery.
//!
//! Extracted from `bootstrap.rs` to isolate mesh topology concerns from the
//! bootstrap orchestrator. Handles transport resolution, UDS-based mesh.init
//! JSON-RPC calls, and multi-peer negotiation.

use super::BootstrapPhase;

/// Map a gate profile transport string to `FetchSource`.
///
/// `local` uses SSH/rsync (VPS layer). All remote transports currently
/// resolve to WAN HTTPS. As LAN rsync and ADB push mature, they will
/// diverge from the WAN fallback.
pub(super) fn transport_to_fetch_source(transport: &str) -> crate::plasmid::FetchSource {
    match transport {
        "local" => crate::plasmid::FetchSource::Vps,
        _ => crate::plasmid::FetchSource::Wan,
    }
}

/// Resolve the transport mode for a gate from the ecosystem manifest.
pub(super) fn resolve_gate_transport(gate_name: &str) -> String {
    let Ok(workspace_root) = crate::temporal::resolve_workspace_root() else {
        return "wan".into();
    };
    let Ok(manifest) = crate::manifest::load_from_workspace(&workspace_root) else {
        return "wan".into();
    };
    manifest
        .gates
        .get(gate_name)
        .and_then(|p| p.transport.clone())
        .unwrap_or_else(|| "wan".into())
}

/// Construct the mesh configuration phase.
pub(super) async fn mesh_phase(gate_name: &str, arch: &str, dry_run: bool) -> BootstrapPhase {
    if dry_run {
        let vps_peer = std::env::var(cellmembrane_types::service::ENV_VPS_MESH_PEER)
            .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_VPS_MESH_PEER.into());
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
    let (ok, detail) = configure_mesh(gate_name, arch).await;
    BootstrapPhase {
        name: "mesh.configure".into(),
        ok,
        detail,
    }
}

/// Configure mesh peering via songbird UDS JSON-RPC.
async fn configure_mesh(gate_name: &str, arch: &str) -> (bool, String) {
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

    let vps_peer = std::env::var(cellmembrane_types::service::ENV_VPS_MESH_PEER)
        .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_VPS_MESH_PEER.into());

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
        let source = transport_to_fetch_source("local");
        assert_eq!(source, crate::plasmid::FetchSource::Vps);
    }

    #[test]
    fn transport_wan_maps_to_wan() {
        let source = transport_to_fetch_source("wan");
        assert_eq!(source, crate::plasmid::FetchSource::Wan);
    }

    #[test]
    fn transport_unknown_maps_to_wan() {
        for input in ["https", "github", "rsync", "", "LAN", "adb"] {
            let source = transport_to_fetch_source(input);
            assert_eq!(
                source,
                crate::plasmid::FetchSource::Wan,
                "unknown transport '{input}' should map to Wan"
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
}
