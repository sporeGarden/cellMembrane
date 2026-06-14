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
        PathBuf::from(format!("{ecoprimals_root}/.gate")),
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
/// Resolution: env `ECOPRIMALS_PLASMID_BIN` → workspace-relative → `/opt/ecoPrimals/plasmidBin`
/// → XDG data home fallback.
pub(super) fn resolve_plasmidbin_dir() -> PathBuf {
    if let Ok(val) = std::env::var("ECOPRIMALS_PLASMID_BIN") {
        return PathBuf::from(val);
    }

    if let Ok(root) = crate::resolve_workspace_root() {
        let ws_depot = root.join("plasmidBin");
        if ws_depot.join("checksums.toml").exists() {
            return ws_depot;
        }
    }

    let opt_depot = PathBuf::from("/opt/ecoPrimals/plasmidBin");
    if opt_depot.join("checksums.toml").exists() {
        return opt_depot;
    }

    let data_home = std::env::var(cellmembrane_types::service::ENV_XDG_DATA_HOME)
        .unwrap_or_else(|_| {
            let home = std::env::var(cellmembrane_types::service::ENV_HOME)
                .unwrap_or_else(|_| "/tmp".into());
            format!("{home}/.local/share")
        });
    PathBuf::from(format!("{data_home}/ecoPrimals/plasmidBin"))
}
