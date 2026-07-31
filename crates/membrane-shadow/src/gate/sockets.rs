// SPDX-License-Identifier: AGPL-3.0-or-later

//! Socket resolution — shared infrastructure for discovering primal UDS paths.
//!
//! Resolves socket paths through a multi-tier strategy:
//!   1. `api_socket` aliases (e.g. `neural-api-default.sock`)
//!   2. `{socket_base}/{binary}.sock`
//!   3. `{XDG_RUNTIME_DIR}/{namespace}/{binary}.sock`
//!   4. Additional `socket_aliases` from the service registry

use std::path::Path;

const DEFAULT_FALLBACK_UID: &str = "1000";

/// Native UDS JSON-RPC call with riboCipher policy probe.
pub(crate) async fn uds_jsonrpc_call(socket_path: &str, request: &str) -> crate::Result<String> {
    let policy = crate::ribocipher::RiboCipherConfig::probe_policy();
    crate::jsonrpc::call_with_policy(Path::new(socket_path), request, &policy).await
}

/// Resolve the mesh relay UDS socket path via capability discovery.
pub(crate) fn resolve_mesh_relay_socket() -> String {
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
        let uid = resolve_uid();
        let ns = cellmembrane_types::service::NEURAL_API_NAMESPACE;
        format!("/run/user/{uid}/{ns}")
    })
}

/// Resolve the current user's UID from environment or `/proc`.
pub(crate) fn resolve_uid() -> String {
    std::env::var("UID")
        .or_else(|_| std::env::var("EUID"))
        .unwrap_or_else(|_| {
            std::fs::read_to_string("/proc/self/loginuid")
                .unwrap_or_else(|_| DEFAULT_FALLBACK_UID.into())
                .trim()
                .to_string()
        })
}

/// Build the candidate socket paths for a primal, ordered by priority.
///
/// Checks the service registry for `api_socket` aliases and `socket_aliases`,
/// then falls back to `{socket_base}/{binary}.sock` and XDG runtime directory.
pub(crate) fn resolve_primal_socket_paths(primal: &str) -> Vec<String> {
    let socket_base = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_SOCKET_BASE,
        cellmembrane_types::service::DEFAULT_SOCKET_BASE,
    );
    let xdg_runtime = std::env::var(cellmembrane_types::service::ENV_XDG_RUNTIME_DIR)
        .unwrap_or_else(|_| format!("/run/user/{}", resolve_uid()));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_primal_socket_paths_includes_socket_base() {
        let paths = resolve_primal_socket_paths("beardog");
        assert!(paths.iter().any(|p| p.contains("beardog.sock")));
        assert!(paths.len() >= 2);
    }

    #[test]
    fn resolve_uid_returns_non_empty() {
        let uid = resolve_uid();
        assert!(!uid.is_empty());
        assert!(uid.parse::<u32>().is_ok(), "UID should be numeric");
    }
}
