// SPDX-License-Identifier: AGPL-3.0-or-later

//! Local gate operations — shared helpers for bootstrap, health, and verify modules.

use std::path::PathBuf;

/// Resolve the local gate identity from env or filesystem.
///
/// Priority: `GATE_NAME` env → `/opt/ecoPrimals/.gate` → `~/.gate` → "unknown".
pub(super) fn resolve_local_gate_identity() -> String {
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
    match std::env::var(cellmembrane_types::service::ENV_HOME) {
        Ok(h) => PathBuf::from(h),
        Err(_) => PathBuf::from("/tmp"),
    }
}

/// Resolve the plasmidBin depot directory.
pub(super) fn resolve_plasmidbin_dir() -> PathBuf {
    crate::plasmid::resolve_path(None, "ECOPRIMALS_PLASMID_BIN", || {
        let data_home = std::env::var(cellmembrane_types::service::ENV_XDG_DATA_HOME)
            .unwrap_or_else(|_| {
                let home = std::env::var(cellmembrane_types::service::ENV_HOME)
                    .unwrap_or_else(|_| "/tmp".into());
                format!("{home}/.local/share")
            });
        PathBuf::from(format!("{data_home}/ecoPrimals/plasmidBin"))
    })
}
