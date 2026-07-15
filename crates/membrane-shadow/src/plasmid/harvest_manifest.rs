// SPDX-License-Identifier: AGPL-3.0-or-later

//! Ecosystem manifest integration for the harvest pipeline.
//!
//! Loads build configs (package overrides, GPU flags, linker settings) from the
//! ecosystem manifest and applies them onto `SourceEntry` for per-primal builds.
//! Also resolves local workspace directories for `--local` builds.

use crate::error::{Result, ShadowError};
use crate::manifest::ManifestBuildConfig;
use std::collections::BTreeMap;
use std::path::PathBuf;
use tracing::info;

use super::harvest::SourceEntry;

/// Load build configs from the ecosystem manifest for all primals.
///
/// Returns a map keyed by lowercase primal name. If the manifest is unavailable,
/// returns an empty map (graceful fallback to `sources.toml` only).
pub(super) fn load_manifest_build_configs() -> BTreeMap<String, ManifestBuildConfig> {
    let workspace = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_ECOPRIMALS_ROOT,
        cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT,
    );
    let workspace_path = std::path::Path::new(&workspace);
    let manifest = match crate::manifest::load_from_workspace(workspace_path) {
        Ok(m) => m,
        Err(e) => {
            info!(error = %e, "manifest unavailable — using sources.toml only");
            return BTreeMap::new();
        }
    };

    let mut configs = BTreeMap::new();
    for (name, entry) in &manifest.repos {
        let cfg = ManifestBuildConfig {
            package: entry.package.clone(),
            linker: entry.linker.clone(),
            gpu: entry.gpu,
        };
        if cfg.package.is_some() || cfg.linker.is_some() || cfg.gpu {
            let lower = name.to_lowercase();
            configs.insert(lower, cfg);
        }
    }
    configs
}

/// Apply manifest build config overrides onto a `SourceEntry`.
///
/// Manifest `package` becomes `build_args = "-p <package>"` (overrides existing).
/// Manifest `gpu` overlays onto `source.gpu`.
pub(super) fn apply_manifest_overrides(source: &mut SourceEntry, cfg: &ManifestBuildConfig) {
    if let Some(pkg) = &cfg.package {
        source.build_args = Some(format!("-p {pkg}"));
    }
    if cfg.gpu {
        source.gpu = true;
    }
}

/// Resolve the local workspace directory for a primal.
///
/// Maps the lowercase primal slug (e.g. `beardog`) to the manifest's
/// `local_path` (e.g. `primals/bearDog`) relative to the workspace root.
pub(super) fn resolve_local_source_dir(primal: &str) -> Result<PathBuf> {
    let workspace = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_ECOPRIMALS_ROOT,
        cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT,
    );
    let workspace_path = std::path::Path::new(&workspace);

    if let Ok(manifest) = crate::manifest::load_from_workspace(workspace_path) {
        for (name, entry) in &manifest.repos {
            if name.to_lowercase() == primal {
                let dir = workspace_path.join(&entry.local_path);
                if dir.exists() {
                    return Ok(dir);
                }
                return Err(ShadowError::Config(format!(
                    "--local: workspace dir does not exist: {}",
                    dir.display()
                )));
            }
        }
    }

    Err(ShadowError::Config(format!(
        "--local: primal '{primal}' not found in ecosystem manifest"
    )))
}
