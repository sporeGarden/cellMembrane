// SPDX-License-Identifier: AGPL-3.0-or-later

//! Local gate operations — shared helpers for bootstrap, health, and verify modules.

use std::path::PathBuf;

/// Resolve the local gate identity from env or filesystem.
///
/// Priority: `GATE_NAME` env → `/opt/ecoPrimals/.gate` → `~/.gate` → "unknown".
#[must_use]
pub fn resolve_local_gate_identity() -> String {
    if let Ok(name) = std::env::var(cellmembrane_types::service::ENV_GATE_NAME) {
        return name;
    }
    let ecoprimals_root = std::env::var(cellmembrane_types::service::ENV_ECOPRIMALS_ROOT)
        .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT.into());
    let candidates = [
        PathBuf::from(&ecoprimals_root).join(".gate"),
        dirs_home().join(".gate"),
    ];
    for path in &candidates {
        if let Ok(content) = std::fs::read_to_string(path) {
            let trimmed = content.trim().to_string();
            if !trimmed.is_empty() {
                return trimmed;
            }
        }
    }
    "unknown".into()
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
pub(super) fn resolve_plasmidbin_dir() -> PathBuf {
    if let Ok(val) = std::env::var(cellmembrane_types::service::ENV_PLASMIDBIN_DEPOT) {
        return PathBuf::from(val);
    }
    if let Ok(val) = std::env::var("ECOPRIMALS_PLASMID_BIN") {
        return PathBuf::from(val);
    }

    let eco_root = std::env::var(cellmembrane_types::service::ENV_ECOPRIMALS_ROOT)
        .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT.into());

    let infra_depot = PathBuf::from(&eco_root).join("infra").join("plasmidBin");
    if infra_depot.join("checksums.toml").exists() || infra_depot.join("primals").is_dir() {
        return infra_depot;
    }

    let flat_depot = PathBuf::from(&eco_root).join("plasmidBin");
    if flat_depot.join("checksums.toml").exists() || flat_depot.join("primals").is_dir() {
        return flat_depot;
    }

    if let Ok(root) = crate::resolve_workspace_root() {
        let ws_depot = root.join("plasmidBin");
        if ws_depot.join("checksums.toml").exists() {
            return ws_depot;
        }
    }

    crate::resolve_xdg_data_home()
        .join("ecoPrimals")
        .join("plasmidBin")
}

/// Resolve the membrane install base directory.
///
/// Priority: `MEMBRANE_INSTALL_BASE` env → default `/opt/membrane`.
pub(super) fn resolve_install_base() -> String {
    std::env::var(cellmembrane_types::service::ENV_INSTALL_BASE)
        .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_INSTALL_BASE.into())
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
