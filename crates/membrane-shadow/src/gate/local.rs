// SPDX-License-Identifier: AGPL-3.0-or-later

//! Local gate operations — shared helpers for bootstrap, health, and verify modules.

use std::path::PathBuf;

/// Resolve the local gate identity, delegating to the canonical `identity::resolve()`.
///
/// Falls back through workspace root candidates. Returns `"unknown"` only if
/// all resolution paths fail — callers should treat this as degraded state.
#[must_use]
pub fn resolve_local_gate_identity() -> String {
    let roots = candidate_workspace_roots();
    for root in &roots {
        if let Ok(id) = crate::identity::resolve(root) {
            return id.name;
        }
    }
    tracing::warn!(
        "gate identity unresolved — all candidate roots exhausted, returning \"unknown\""
    );
    "unknown".into()
}

fn candidate_workspace_roots() -> Vec<PathBuf> {
    let mut roots = Vec::with_capacity(3);
    if let Ok(root) = crate::resolve_workspace_root() {
        roots.push(root);
    }
    let ecoprimals_root = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_ECOPRIMALS_ROOT,
        cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT,
    );
    roots.push(PathBuf::from(ecoprimals_root));
    roots.push(dirs_home());
    roots
}

fn dirs_home() -> PathBuf {
    std::env::var(cellmembrane_types::service::ENV_HOME)
        .map_or_else(|_| PathBuf::from("/tmp"), PathBuf::from)
}

/// Resolve the plasmidBin depot directory.
///
/// Resolution priority (first existing wins):
/// 1. `PLASMIDBIN_DEPOT` env var (shared with harvest)
/// 2. `ECOPRIMALS_PLASMID_BIN` env var (gate-specific legacy)
/// 3. `{eco_root}/infra/plasmidBin` (canonical harvest output)
/// 4. `{eco_root}/plasmidBin` (backwards compat)
/// 5. workspace-relative `plasmidBin/`
/// 6. XDG data home fallback
pub fn resolve_plasmidbin_dir() -> PathBuf {
    if let Ok(val) = std::env::var(cellmembrane_types::service::ENV_PLASMIDBIN_DEPOT) {
        return PathBuf::from(val);
    }
    if let Ok(val) = std::env::var(cellmembrane_types::service::ENV_PLASMIDBIN_LEGACY) {
        return PathBuf::from(val);
    }

    let eco_root = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_ECOPRIMALS_ROOT,
        cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT,
    );

    let infra_depot = PathBuf::from(&eco_root).join(cellmembrane_types::service::INFRA_PLASMID_BIN);
    if infra_depot
        .join(cellmembrane_types::service::CHECKSUMS_FILE)
        .exists()
        || infra_depot.join("primals").is_dir()
    {
        return infra_depot;
    }

    let flat_depot = PathBuf::from(&eco_root).join(cellmembrane_types::service::PLASMID_BIN_DIR);
    if flat_depot
        .join(cellmembrane_types::service::CHECKSUMS_FILE)
        .exists()
        || flat_depot.join("primals").is_dir()
    {
        return flat_depot;
    }

    if let Ok(root) = crate::resolve_workspace_root() {
        let ws_depot = root.join(cellmembrane_types::service::PLASMID_BIN_DIR);
        if ws_depot
            .join(cellmembrane_types::service::CHECKSUMS_FILE)
            .exists()
        {
            return ws_depot;
        }
    }

    crate::resolve_xdg_data_home()
        .join("ecoPrimals")
        .join(cellmembrane_types::service::PLASMID_BIN_DIR)
}

/// Resolve the membrane install base directory.
///
/// Priority: `MEMBRANE_INSTALL_BASE` env → default `/opt/membrane`.
pub(super) fn resolve_install_base() -> String {
    cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_INSTALL_BASE,
        cellmembrane_types::service::DEFAULT_INSTALL_BASE,
    )
}

/// Resolve the gate name through a unified priority chain.
///
/// Resolution order:
///   1. `explicit` — caller-extracted name from CLI (`--gate`, positional, etc.)
///   2. `GATE_NAME` environment variable
///   3. `.gate` file at `root` (async read)
///   4. Multi-root `resolve_local_gate_identity()` (canonical fallback)
///
/// Callers are responsible for extracting the explicit name from their own
/// CLI conventions — this function handles everything after that.
pub async fn resolve_gate_name_async(
    explicit: Option<&str>,
    root: Option<&std::path::Path>,
) -> String {
    if let Some(name) = explicit {
        return name.to_string();
    }
    if let Ok(g) = std::env::var(cellmembrane_types::service::ENV_GATE_NAME) {
        if !g.is_empty() {
            return g;
        }
    }
    if let Some(r) = root {
        if let Ok(content) = tokio::fs::read_to_string(r.join(".gate")).await {
            let trimmed = content.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }
    resolve_local_gate_identity()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_identity_returns_non_empty() {
        let id = resolve_local_gate_identity();
        assert!(!id.is_empty(), "identity should never be empty");
    }

    #[test]
    fn dirs_home_returns_path() {
        let home = dirs_home();
        assert!(!home.as_os_str().is_empty());
    }

    #[test]
    fn resolve_install_base_returns_non_empty() {
        let base = resolve_install_base();
        assert!(!base.is_empty());
    }

    #[test]
    fn resolve_plasmidbin_dir_returns_path() {
        let dir = resolve_plasmidbin_dir();
        assert!(!dir.as_os_str().is_empty(), "depot dir should not be empty");
    }
}
