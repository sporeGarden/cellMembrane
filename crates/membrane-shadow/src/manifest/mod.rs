// SPDX-License-Identifier: AGPL-3.0-or-later

//! Ecosystem manifest reader — typed access to `ecosystem_manifest.toml`.
//!
//! Replaces the embedded Python `_py_read_manifest` in `cascade-pull.sh`
//! with a typed Rust reader. The manifest is the single source of truth
//! for repo metadata, gate profiles, and sync configuration.

mod types;

use crate::error::{Result, ShadowError};
use cellmembrane_types::DivergencePolicy;
use std::path::{Path, PathBuf};

pub use types::{
    BuildEntry, CompositionProfile, EcosystemManifest, GateProfile, ManifestBuildConfig, RepoEntry,
    SubBuilderEntry,
};

impl EcosystemManifest {
    /// Load manifest from a TOML file.
    ///
    /// # Errors
    /// Returns `ShadowError::Io` if the file can't be read, or
    /// `ShadowError::Parse` if the TOML is malformed.
    pub fn load(path: &Path) -> Result<Self> {
        let contents = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&contents)?)
    }

    /// Async variant — reads the file on a blocking thread to avoid stalling
    /// the tokio runtime on file I/O.
    ///
    /// # Errors
    /// Returns `ShadowError::Io` if the file can't be read, or
    /// `ShadowError::Parse` if the TOML is malformed.
    pub async fn load_async(path: PathBuf) -> Result<Self> {
        let contents = tokio::fs::read_to_string(&path).await?;
        Ok(toml::from_str(&contents)?)
    }

    /// Find the manifest file relative to a workspace root.
    /// Looks at `infra/wateringHole/ecosystem_manifest.toml`.
    #[must_use]
    pub fn find_in_workspace(workspace_root: &Path) -> Option<PathBuf> {
        let path = workspace_root
            .join(cellmembrane_types::service::INFRA_WATERING_HOLE)
            .join("ecosystem_manifest.toml");
        path.exists().then_some(path)
    }

    /// Get repo entries for a specific gate, resolved to `RepoEntry` references.
    /// Returns entries in the order they appear in the gate profile.
    #[must_use]
    pub fn gate_repos(&self, gate: &str) -> Vec<(&str, &RepoEntry)> {
        let Some(profile) = self.gates.get(gate) else {
            return Vec::new();
        };
        profile
            .repos
            .iter()
            .filter_map(|name| {
                self.repos
                    .get(name.as_str())
                    .map(|entry| (name.as_str(), entry))
            })
            .collect()
    }

    /// Get all distinct org names from repos.
    #[must_use]
    pub fn orgs(&self) -> Vec<&str> {
        let mut orgs: Vec<&str> = self.repos.values().map(|r| r.org.as_str()).collect();
        orgs.sort_unstable();
        orgs.dedup();
        orgs
    }

    /// Resolve divergence policy for a repo — per-repo override or global default.
    #[must_use]
    pub fn divergence_policy_for(&self, entry: &RepoEntry) -> DivergencePolicy {
        entry
            .divergence_policy
            .unwrap_or(self.sync.divergence_policy)
    }

    /// Build a Forgejo SSH clone URL using the sync config.
    #[must_use]
    pub fn forgejo_clone_url(&self, entry: &RepoEntry) -> String {
        format!("{}/{}.git", self.sync.forgejo_ssh, entry.forgejo_repo)
    }

    /// Look up a build entry by primal slug (e.g. `"beardog"`).
    #[must_use]
    #[allow(
        dead_code,
        reason = "manifest API — wired by tests, ready for consumers"
    )]
    pub fn build_entry(&self, slug: &str) -> Option<&BuildEntry> {
        self.build.get(slug)
    }

    /// Get the `cargo build` package argument for a primal.
    /// Returns `Some("--package <pkg>")` for workspace primals, `None` otherwise.
    #[must_use]
    #[allow(
        dead_code,
        reason = "manifest API — wired by tests, ready for consumers"
    )]
    pub fn build_package_arg(&self, slug: &str) -> Option<&str> {
        self.build.get(slug).map(|b| b.package.as_str())
    }

    /// Return the ordered list of build-authority gates from `[topology.roles]`.
    /// Falls back to scanning `[gates.*]` for `build_authority = true`.
    #[must_use]
    pub fn build_authorities(&self) -> Vec<String> {
        if let Some(topo) = &self.topology
            && let Some(roles) = &topo.roles
            && !roles.build_authorities.is_empty()
        {
            return roles.build_authorities.clone();
        }
        self.gates
            .iter()
            .filter(|(_, p)| p.build_authority)
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// Find gates that have a specific role in their roles list.
    /// Returns `(gate_name, &GateProfile)` tuples.
    #[must_use]
    pub fn gates_for_role(&self, role: &str) -> Vec<(&str, &GateProfile)> {
        let target = cellmembrane_types::GateRole::from(role);
        self.gates
            .iter()
            .filter(|(_, profile)| profile.roles.contains(&target))
            .map(|(name, profile)| (name.as_str(), profile))
            .collect()
    }

    /// Resolve the `WireGuard` mesh IP for a named gate.
    #[must_use]
    pub fn mesh_ip_for(&self, gate: &str) -> Option<&str> {
        self.gates.get(gate).and_then(|p| p.wg_ip.as_deref())
    }

    // validate() is provided by manifest/validate.rs — cross-field integrity checks.

    /// Look up a gate's LAN IP from the manifest.
    ///
    /// Returns the `lan_ip` field if set, enabling direct TCP resolution on
    /// the local subnet without DNS or `WireGuard` overlay.
    #[must_use]
    #[allow(
        dead_code,
        reason = "manifest API — wired by tests, ready for consumers"
    )]
    pub fn lan_ip_for(&self, gate: &str) -> Option<&str> {
        self.gates.get(gate).and_then(|p| p.lan_ip.as_deref())
    }

    /// Resolve the best SSH target for a gate from the manifest.
    ///
    /// Priority chain: `host` (explicit VPS hostname) → `lan_ip` (direct LAN
    /// peer) → `wg_ip` (mesh overlay). Returns `None` if the gate is not in
    /// the manifest or has no routable address.
    #[must_use]
    pub fn ssh_target_for(&self, gate: &str) -> Option<&str> {
        let p = self.gates.get(gate)?;
        p.host
            .as_deref()
            .or(p.lan_ip.as_deref())
            .or(p.wg_ip.as_deref())
    }

    /// Resolve the SSH user for a gate (defaults to `"root"`).
    #[must_use]
    #[allow(
        dead_code,
        reason = "manifest API — wired by tests, ready for consumers"
    )]
    pub fn ssh_user_for(&self, gate: &str) -> &str {
        self.gates
            .get(gate)
            .and_then(|p| p.ssh_user.as_deref())
            .unwrap_or("root")
    }

    /// Look up a composition profile by name.
    #[must_use]
    pub fn composition(&self, name: &str) -> Option<&CompositionProfile> {
        self.compositions.get(name)
    }

    /// Resolve the composition for a given gate, returning its profile.
    #[must_use]
    pub fn gate_composition(&self, gate: &str) -> Option<&CompositionProfile> {
        self.gates
            .get(gate)
            .and_then(|p| p.composition.as_deref())
            .and_then(|name| self.compositions.get(name))
    }

    /// List all defined composition profiles.
    #[must_use]
    pub fn composition_names(&self) -> Vec<&str> {
        self.compositions.keys().map(String::as_str).collect()
    }
}

mod validate;

/// Resolve the federation peer address from the manifest (golgi by default).
///
/// Prefers `wg_ip` over `host` — mesh traffic should traverse the encrypted
/// `WireGuard` overlay when available, falling back to public IP only when no
/// overlay address is configured.
///
/// Falls back to `DEFAULT_VPS_MESH_PEER` if manifest is unavailable.
#[must_use]
pub(crate) fn resolve_federation_peer() -> String {
    let workspace = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_ECOPRIMALS_ROOT,
        cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT,
    );
    let port = cellmembrane_types::service::DEFAULT_FEDERATION_PORT;
    if let Ok(manifest) = load_from_workspace(std::path::Path::new(&workspace)) {
        let hub_gates = manifest.gates_for_role("wg_hub");
        if let Some((_, profile)) = hub_gates.first() {
            if let Some(ref ip) = profile.wg_ip {
                return format!("{ip}:{port}");
            }
            if let Some(ref host) = profile.host {
                return format!("{host}:{port}");
            }
        }
    }
    cellmembrane_types::service::DEFAULT_VPS_MESH_PEER.to_string()
}

/// Convenience: load manifest from workspace root.
///
/// # Errors
/// Returns error if manifest not found or unparseable.
pub(crate) fn load_from_workspace(workspace_root: &Path) -> Result<EcosystemManifest> {
    let path = EcosystemManifest::find_in_workspace(workspace_root).ok_or_else(|| {
        ShadowError::config(format!(
            "ecosystem_manifest.toml not found under {}",
            workspace_root.display()
        ))
    })?;
    EcosystemManifest::load(&path)
}

/// Async convenience: load manifest from workspace root without blocking the runtime.
///
/// # Errors
/// Returns error if manifest not found or unparseable.
pub(crate) async fn load_from_workspace_async(workspace_root: &Path) -> Result<EcosystemManifest> {
    let path = EcosystemManifest::find_in_workspace(workspace_root).ok_or_else(|| {
        ShadowError::config(format!(
            "ecosystem_manifest.toml not found under {}",
            workspace_root.display()
        ))
    })?;
    EcosystemManifest::load_async(path).await
}

#[cfg(test)]
mod tests;
