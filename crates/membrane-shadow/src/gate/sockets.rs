// SPDX-License-Identifier: AGPL-3.0-or-later

//! Socket resolution — shared infrastructure for discovering primal UDS paths.
//!
//! Resolves socket paths through a multi-tier strategy:
//!   1. `api_socket` aliases (e.g. `neural-api-default.sock`)
//!   2. `{socket_base}/{binary}.sock`
//!   3. `{XDG_RUNTIME_DIR}/{namespace}/{binary}.sock`
//!   4. Additional `socket_aliases` from the service registry

use std::path::Path;

/// Native UDS JSON-RPC call with riboCipher policy probe.
pub(crate) async fn uds_jsonrpc_call(socket_path: &str, request: &str) -> crate::Result<String> {
    let policy = crate::ribocipher::RiboCipherConfig::probe_policy();
    crate::jsonrpc::call_with_policy(Path::new(socket_path), request, &policy).await
}

/// Resolve the mesh relay UDS socket path via capability discovery.
///
/// Honors `MEMBRANE_MESH_RELAY_SOCKET` env override first, then probes
/// the candidate list from `resolve_primal_socket_paths` using the binary
/// registered for `MeshRelay` capability.
pub(crate) fn resolve_mesh_relay_socket() -> String {
    if let Ok(env_path) = std::env::var(cellmembrane_types::service::ENV_SONGBIRD_SOCKET) {
        if Path::new(&env_path).exists() {
            return env_path;
        }
    }
    let binary_name = cellmembrane_types::MembraneService::binary_for(
        cellmembrane_types::ServiceCapability::MeshRelay,
    );
    let paths = resolve_primal_socket_paths(binary_name);
    paths
        .into_iter()
        .find(|p| Path::new(p).exists())
        .unwrap_or_else(|| {
            let socket_dir = resolve_biomeos_socket_dir();
            format!("{socket_dir}/{binary_name}.sock")
        })
}

/// Resolve the biomeOS neural-api socket directory.
pub(crate) fn resolve_biomeos_socket_dir() -> String {
    std::env::var(cellmembrane_types::service::ENV_BIOMEOS_SOCKET_DIR).unwrap_or_else(|_| {
        let xdg = cellmembrane_types::service::resolve_xdg_runtime_dir();
        let ns = cellmembrane_types::service::NEURAL_API_NAMESPACE;
        format!("{xdg}/{ns}")
    })
}

/// Build the candidate JSON-RPC socket paths for a primal, ordered by priority.
///
/// Uses `resolve_socket_base()` which auto-adapts to init scope (system vs
/// user-space deploy), then adds XDG/`biomeos` namespace paths for user
/// session discovery. Checks the service registry for `api_socket` aliases
/// and `socket_aliases`.
///
/// Returns only JSON-RPC sockets — use [`resolve_primal_tarpc_socket_paths`]
/// for tarpc binary-protocol sockets.
pub(crate) fn resolve_primal_socket_paths(primal: &str) -> Vec<String> {
    let socket_base = cellmembrane_types::service::resolve_socket_base();
    let xdg_runtime = cellmembrane_types::service::resolve_xdg_runtime_dir();
    let ns = cellmembrane_types::service::NEURAL_API_NAMESPACE;
    let mut paths = vec![
        format!("{socket_base}/{primal}.sock"),
        format!("{xdg_runtime}/{ns}/{primal}.sock"),
    ];
    if let Some(svc) = cellmembrane_types::MembraneService::for_binary(primal) {
        if let Some(api) = svc.api_socket {
            paths.insert(0, format!("{socket_base}/{api}.sock"));
            paths.insert(0, format!("{socket_base}/{api}-default.sock"));
            paths.push(format!("{xdg_runtime}/{ns}/{api}-default.sock"));
        }
        for alias in svc.socket_aliases {
            paths.push(format!("{socket_base}/{alias}.sock"));
        }
    }
    paths
}

/// Build candidate tarpc socket paths for a dual-protocol primal (G64).
///
/// Returns an empty vec if the primal has `has_tarpc: false` or is unknown
/// to the registry. Probes `{socket_base}/{binary}.tarpc.sock` and
/// `{xdg}/{namespace}/{binary}.tarpc.sock`.
///
/// Not yet called from production code — primals are shipping dual-socket
/// incrementally. Will be wired once tarpc health probing is added.
#[allow(dead_code)]
pub(crate) fn resolve_primal_tarpc_socket_paths(primal: &str) -> Vec<String> {
    let svc = match cellmembrane_types::MembraneService::for_binary(primal) {
        Some(s) if s.has_tarpc => s,
        _ => return Vec::new(),
    };
    let socket_base = cellmembrane_types::service::resolve_socket_base();
    let xdg_runtime = cellmembrane_types::service::resolve_xdg_runtime_dir();
    let ns = cellmembrane_types::service::NEURAL_API_NAMESPACE;
    vec![
        format!("{socket_base}/{}.tarpc.sock", svc.binary),
        format!("{xdg_runtime}/{ns}/{}.tarpc.sock", svc.binary),
    ]
}

/// Check whether a socket path is a tarpc socket (by file extension).
pub(crate) fn is_tarpc_socket(path: &str) -> bool {
    path.ends_with(cellmembrane_types::service::TARPC_SOCKET_SUFFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_primal_socket_paths_includes_socket_base() {
        let paths = resolve_primal_socket_paths("beardog");
        assert!(paths.iter().any(|p| p.contains("beardog.sock")));
        assert!(paths.len() >= 2);
        assert!(
            paths.iter().all(|p| !p.contains(".tarpc.sock")),
            "JSON-RPC resolver must not return tarpc sockets"
        );
    }

    #[test]
    fn tarpc_socket_paths_for_serving_primal() {
        let paths = resolve_primal_tarpc_socket_paths("loamspine");
        assert!(
            !paths.is_empty(),
            "loamspine has tarpc — should return candidates"
        );
        assert!(paths.iter().all(|p| p.ends_with(".tarpc.sock")));
    }

    #[test]
    fn tarpc_socket_paths_empty_for_non_serving() {
        let paths = resolve_primal_tarpc_socket_paths("beardog");
        assert!(
            paths.is_empty(),
            "beardog has_tarpc=false — should return empty"
        );
    }

    #[test]
    fn is_tarpc_socket_filter() {
        assert!(is_tarpc_socket("/run/membrane/loamspine.tarpc.sock"));
        assert!(!is_tarpc_socket("/run/membrane/loamspine.sock"));
        assert!(!is_tarpc_socket("/run/membrane/security.sock"));
    }

    #[test]
    fn xdg_runtime_dir_returns_non_empty() {
        let dir = cellmembrane_types::service::resolve_xdg_runtime_dir();
        assert!(!dir.is_empty());
        assert!(
            dir.starts_with('/'),
            "XDG runtime dir should be absolute: {dir}"
        );
    }
}
