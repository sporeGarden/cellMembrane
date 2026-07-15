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
    BuildEntry, CompositionProfile, EcosystemManifest, GateProfile, ManifestBuildConfig,
    ManifestMeta, RepoEntry, SyncConfig, Topology, TopologyRoles,
};

impl EcosystemManifest {
    /// Load manifest from a TOML file.
    ///
    /// # Errors
    /// Returns `ShadowError::Io` if the file can't be read, or
    /// `ShadowError::Parse` if the TOML is malformed.
    pub fn load(path: &Path) -> Result<Self> {
        let contents = std::fs::read_to_string(path).map_err(ShadowError::Io)?;
        toml::from_str(&contents).map_err(ShadowError::Toml)
    }

    /// Async variant — reads the file on a blocking thread to avoid stalling
    /// the tokio runtime on file I/O.
    ///
    /// # Errors
    /// Returns `ShadowError::Io` if the file can't be read, or
    /// `ShadowError::Parse` if the TOML is malformed.
    pub async fn load_async(path: PathBuf) -> Result<Self> {
        let contents = tokio::fs::read_to_string(&path)
            .await
            .map_err(ShadowError::Io)?;
        toml::from_str(&contents).map_err(ShadowError::Toml)
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

    /// Get local paths for a gate's repos (what cascade-pull iterates).
    #[must_use]
    pub fn gate_local_paths(&self, gate: &str) -> Vec<&str> {
        self.gate_repos(gate)
            .into_iter()
            .map(|(_, entry)| entry.local_path.as_str())
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

    /// Get repos filtered by membrane sync mode.
    #[must_use]
    pub fn repos_by_membrane(
        &self,
        membrane: cellmembrane_types::MembraneSyncMode,
    ) -> Vec<(&str, &RepoEntry)> {
        self.repos
            .iter()
            .filter(|(_, e)| e.membrane == membrane)
            .map(|(name, entry)| (name.as_str(), entry))
            .collect()
    }

    /// Resolve divergence policy for a repo — per-repo override or global default.
    #[must_use]
    pub fn divergence_policy_for(&self, entry: &RepoEntry) -> DivergencePolicy {
        entry
            .divergence_policy
            .unwrap_or(self.sync.divergence_policy)
    }

    /// Build a GitHub clone URL for a repo.
    #[must_use]
    pub fn github_clone_url(entry: &RepoEntry) -> String {
        format!("https://github.com/{}.git", entry.github_repo)
    }

    /// Build a Forgejo SSH clone URL using the sync config.
    #[must_use]
    pub fn forgejo_clone_url(&self, entry: &RepoEntry) -> String {
        format!("{}/{}.git", self.sync.forgejo_ssh, entry.forgejo_repo)
    }

    /// Look up a build entry by primal slug (e.g. `"beardog"`).
    #[must_use]
    pub fn build_entry(&self, slug: &str) -> Option<&BuildEntry> {
        self.build.get(slug)
    }

    /// Get the `cargo build` package argument for a primal.
    /// Returns `Some("--package <pkg>")` for workspace primals, `None` otherwise.
    #[must_use]
    pub fn build_package_arg(&self, slug: &str) -> Option<&str> {
        self.build.get(slug).map(|b| b.package.as_str())
    }

    /// Return the ordered list of build-authority gates from `[topology.roles]`.
    /// Falls back to scanning `[gates.*]` for `build_authority = true`.
    #[must_use]
    pub fn build_authorities(&self) -> Vec<String> {
        if let Some(topo) = &self.topology {
            if let Some(roles) = &topo.roles {
                if !roles.build_authorities.is_empty() {
                    return roles.build_authorities.clone();
                }
            }
        }
        self.gates
            .iter()
            .filter(|(_, p)| p.build_authority)
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// Check whether a specific gate is a build authority.
    #[must_use]
    pub fn is_build_authority(&self, gate: &str) -> bool {
        self.build_authorities().iter().any(|g| g == gate)
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
mod wave;
pub use wave::{ExitCriterion, WaveState};

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
        ShadowError::Config(format!(
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
        ShadowError::Config(format!(
            "ecosystem_manifest.toml not found under {}",
            workspace_root.display()
        ))
    })?;
    EcosystemManifest::load_async(path).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellmembrane_types::{PushTarget, ZoneLabel};

    fn sample_manifest_toml() -> &'static str {
        r#"
[meta]
version = "2.5.0"
wave = 67
total_repos = 39

[sync]
default_source = "temporal"
push_target = "forgejo"
divergence_policy = "merge-ff"
forgejo_ssh = "ssh://git@git.primals.eco:2222"

[repos.bearDog]
org = "ecoPrimals"
local_path = "primals/bearDog"
github_repo = "ecoPrimals/bearDog"
forgejo_repo = "ecoPrimals/bearDog"
category = "primal"

[repos.cellMembrane]
org = "sporeGarden"
local_path = "gardens/cellMembrane"
github_repo = "sporeGarden/cellMembrane"
forgejo_repo = "sporeGarden/cellMembrane"
category = "garden"

[gates.eastGate]
repos = ["bearDog", "cellMembrane"]
"#
    }

    #[test]
    fn parse_manifest_meta() {
        let manifest: EcosystemManifest = toml::from_str(sample_manifest_toml()).unwrap();
        assert_eq!(manifest.meta.version, "2.5.0");
        assert_eq!(manifest.meta.wave, 67);
        assert_eq!(manifest.meta.total_repos, 39);
    }

    #[test]
    fn parse_manifest_sync_config() {
        let manifest: EcosystemManifest = toml::from_str(sample_manifest_toml()).unwrap();
        assert_eq!(manifest.sync.default_source, cellmembrane_types::CascadeSource::Temporal);
        assert_eq!(manifest.sync.push_target, PushTarget::Forgejo);
        assert_eq!(manifest.sync.divergence_policy, DivergencePolicy::MergeFf);
    }

    #[test]
    fn parse_manifest_repos() {
        let manifest: EcosystemManifest = toml::from_str(sample_manifest_toml()).unwrap();
        assert_eq!(manifest.repos.len(), 2);

        let bear = &manifest.repos["bearDog"];
        assert_eq!(bear.local_path, "primals/bearDog");
        assert_eq!(bear.category, cellmembrane_types::RepoCategory::Primal);
        assert_eq!(bear.org, "ecoPrimals");

        let cm = &manifest.repos["cellMembrane"];
        assert_eq!(cm.local_path, "gardens/cellMembrane");
        assert_eq!(cm.category, cellmembrane_types::RepoCategory::Garden);
    }

    #[test]
    fn forgejo_clone_url_format() {
        let manifest: EcosystemManifest = toml::from_str(sample_manifest_toml()).unwrap();
        let entry = &manifest.repos["cellMembrane"];
        assert_eq!(
            manifest.forgejo_clone_url(entry),
            "ssh://git@git.primals.eco:2222/sporeGarden/cellMembrane.git"
        );
    }

    #[test]
    fn gate_profiles_parsed() {
        let manifest: EcosystemManifest = toml::from_str(sample_manifest_toml()).unwrap();
        assert!(manifest.gates.contains_key("eastGate"));
        let gate = &manifest.gates["eastGate"];
        assert_eq!(gate.repos, vec!["bearDog", "cellMembrane"]);
    }

    #[test]
    fn gate_profile_zone_fields_parsed() {
        let toml_str = r#"
[meta]
version = "1.0.0"
[sync]

[gates.sporeGate]
repos = ["cellMembrane"]
zone = "backbone"
hub_port = "ether8"
link_speed_mbps = 2500

[gates.fieldGate]
repos = ["cellMembrane"]
zone = "house2"
hub_port = "2.5g"
link_speed_mbps = 2500

[gates.flockGate]
repos = ["cellMembrane"]
"#;
        let manifest: EcosystemManifest = toml::from_str(toml_str).unwrap();

        let sg = &manifest.gates["sporeGate"];
        assert_eq!(sg.zone, Some(ZoneLabel::Backbone));
        assert_eq!(sg.hub_port.as_deref(), Some("ether8"));
        assert_eq!(sg.link_speed_mbps, Some(2500));

        let fg = &manifest.gates["fieldGate"];
        assert_eq!(fg.zone, Some(ZoneLabel::House2));
        assert_eq!(fg.link_speed_mbps, Some(2500));

        let flock = &manifest.gates["flockGate"];
        assert_eq!(flock.zone, None);
        assert_eq!(flock.hub_port, None);
        assert_eq!(flock.link_speed_mbps, None);
    }

    #[test]
    fn gate_profile_lan_ip_parsed() {
        let toml_str = r#"
[meta]
version = "1.0.0"
[sync]

[gates.sporeGate]
repos = ["cellMembrane"]
wg_ip = "10.13.37.2"
lan_ip = "192.168.4.3"

[gates.eastGate]
repos = ["cellMembrane"]
wg_ip = "10.13.37.5"
"#;
        let manifest: EcosystemManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.lan_ip_for("sporeGate"), Some("192.168.4.3"));
        assert_eq!(manifest.lan_ip_for("eastGate"), None);
        assert_eq!(manifest.mesh_ip_for("sporeGate"), Some("10.13.37.2"));
        assert_eq!(manifest.lan_ip_for("unknown"), None);
    }

    #[test]
    fn sync_defaults_applied() {
        let toml_str = r#"
[meta]
version = "1.0.0"
[sync]
[gates]
"#;
        let manifest: EcosystemManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.sync.default_source, cellmembrane_types::CascadeSource::Temporal);
        assert_eq!(manifest.sync.default_branch, "main");
        assert_eq!(manifest.sync.divergence_policy, DivergencePolicy::Flag);
        assert_eq!(manifest.sync.push_target, PushTarget::All);
    }

    #[test]
    fn ssh_target_prefers_host_over_lan_over_wg() {
        let toml_str = r#"
[meta]
version = "1.0.0"
[sync]

[gates.golgiBody]
host = "157.230.3.183"
lan_ip = "10.116.0.2"
wg_ip = "10.13.37.1"

[gates.sporeGate]
lan_ip = "192.168.4.3"
wg_ip = "10.13.37.2"

[gates.flockGate]
wg_ip = "10.13.37.6"

[gates.southGate]
repos = []
"#;
        let m: EcosystemManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(m.ssh_target_for("golgiBody"), Some("157.230.3.183"));
        assert_eq!(m.ssh_target_for("sporeGate"), Some("192.168.4.3"));
        assert_eq!(m.ssh_target_for("flockGate"), Some("10.13.37.6"));
        assert_eq!(m.ssh_target_for("southGate"), None);
        assert_eq!(m.ssh_target_for("unknown"), None);
    }

    #[test]
    fn ssh_user_defaults_to_root() {
        let toml_str = r#"
[meta]
version = "1.0.0"
[sync]

[gates.golgiBody]
ssh_user = "deploy"

[gates.sporeGate]
repos = []
"#;
        let m: EcosystemManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(m.ssh_user_for("golgiBody"), "deploy");
        assert_eq!(m.ssh_user_for("sporeGate"), "root");
        assert_eq!(m.ssh_user_for("unknown"), "root");
    }

    #[test]
    fn gate_profile_parses_portable_fields() {
        let toml_str = r#"
[meta]
version = "1.0.0"
[sync]

[gates.grapheneGate]
gate_class = "portable_anchor"
bond_types = ["covalent"]
target = "aarch64-unknown-linux-musl"
mobility = "mobile"
bind_mode = "tcp_only"
composition = "tower"
transport = "adb"
tether_role = "usb_rndis"
nucleus_status = "Tower LIVE"
adb_ports = [9100, 9200, 9140]
notes = "Pixel 8a"
repos = ["wateringHole", "bearDog"]
"#;
        let m: EcosystemManifest = toml::from_str(toml_str).unwrap();
        let g = &m.gates["grapheneGate"];
        assert_eq!(g.gate_class.as_deref(), Some("portable_anchor"));
        assert_eq!(g.tether_role.as_deref(), Some("usb_rndis"));
        assert_eq!(g.adb_ports, vec![9100, 9200, 9140]);
        assert_eq!(g.nucleus_status.as_deref(), Some("Tower LIVE"));
        assert_eq!(g.bond_types, vec![cellmembrane_types::BondType::Covalent]);
        assert_eq!(g.mobility, Some(cellmembrane_types::GateMobility::Mobile));
        assert_eq!(g.repos.len(), 2);
    }

    #[test]
    fn build_entries_parsed() {
        let toml_str = r#"
[meta]
version = "2.8.0"
[sync]

[build.beardog]
binary_name = "beardog"
package = "beardog"
workspace = false
targets = ["x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl"]

[build.biomeos]
binary_name = "biomeos"
package = "biomeos-unibin"
workspace = true
gpu = false
targets = ["x86_64-unknown-linux-musl"]
notes = "CI-DIV-01: requires --package biomeos-unibin"

[build.barracuda]
binary_name = "barracuda"
package = "barracuda-core"
workspace = true
gpu = true
targets = ["x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl"]
"#;
        let m: EcosystemManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(m.build.len(), 3);

        let bear = m.build_entry("beardog").unwrap();
        assert_eq!(bear.binary_name, "beardog");
        assert_eq!(bear.package, "beardog");
        assert!(!bear.workspace);
        assert!(!bear.gpu);
        assert_eq!(bear.targets.len(), 2);

        let bio = m.build_entry("biomeos").unwrap();
        assert_eq!(bio.package, "biomeos-unibin");
        assert!(bio.workspace);
        assert!(!bio.gpu);
        assert!(bio.notes.as_deref().unwrap().contains("CI-DIV-01"));

        let barra = m.build_entry("barracuda").unwrap();
        assert!(barra.gpu);
        assert!(barra.workspace);
        assert_eq!(barra.package, "barracuda-core");

        assert!(m.build_entry("nonexistent").is_none());
        assert_eq!(m.build_package_arg("beardog"), Some("beardog"));
    }

    #[test]
    fn build_entries_default_when_absent() {
        let toml_str = r#"
[meta]
version = "1.0.0"
[sync]
"#;
        let m: EcosystemManifest = toml::from_str(toml_str).unwrap();
        assert!(m.build.is_empty());
    }

    #[test]
    fn gate_profile_defaults_new_fields_when_absent() {
        let toml_str = r#"
[meta]
version = "1.0.0"
[sync]

[gates.sporeGate]
repos = ["cellMembrane"]
"#;
        let m: EcosystemManifest = toml::from_str(toml_str).unwrap();
        let g = &m.gates["sporeGate"];
        assert!(g.gate_class.is_none());
        assert!(g.tether_role.is_none());
        assert!(g.adb_ports.is_empty());
        assert!(g.nucleus_status.is_none());
        assert!(g.bond_types.is_empty());
    }

    #[test]
    fn composition_profiles_parsed() {
        let toml_str = r#"
[meta]
version = "3.0.0"
[sync]

[compositions.full]
description = "Complete sovereign stack"
primals = ["beardog", "songbird", "skunkbat", "nestgate"]
services = ["mesh", "tls", "firewall", "cas"]
requires = ["rust", "musl-tools"]
examples = ["eastGate", "ironGate"]

[compositions.thin-relay]
description = "Relay + depot only"
primals = ["songbird"]
services = ["relay", "depot"]
repos = ["wateringHole"]
notes = "No Rust toolchain needed"
examples = ["golgiBody"]

[compositions.tower]
description = "Minimal secure mesh entry"
primals = ["beardog", "songbird", "skunkbat"]
services = ["tls", "mesh", "firewall"]
examples = ["grapheneGate"]
"#;
        let m: EcosystemManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(m.compositions.len(), 3);

        let full = m.composition("full").unwrap();
        assert_eq!(full.primals.len(), 4);
        assert_eq!(full.services.len(), 4);
        assert_eq!(full.requires.len(), 2);
        assert_eq!(full.examples, vec!["eastGate", "ironGate"]);

        let relay = m.composition("thin-relay").unwrap();
        assert_eq!(relay.primals, vec!["songbird"]);
        assert_eq!(relay.repos, vec!["wateringHole"]);
        assert!(relay.notes.as_deref().unwrap().contains("No Rust"));

        let tower = m.composition("tower").unwrap();
        assert_eq!(tower.primals.len(), 3);
        assert!(tower.requires.is_empty());
    }

    #[test]
    fn composition_names_returns_all() {
        let toml_str = r#"
[meta]
version = "1.0.0"
[sync]

[compositions.full]
description = "Full"
[compositions.tower]
description = "Tower"
[compositions.relay]
description = "Relay"
"#;
        let m: EcosystemManifest = toml::from_str(toml_str).unwrap();
        let names = m.composition_names();
        assert_eq!(names.len(), 3);
        assert!(names.contains(&"full"));
        assert!(names.contains(&"tower"));
        assert!(names.contains(&"relay"));
    }

    #[test]
    fn gate_composition_resolves_profile() {
        let toml_str = r#"
[meta]
version = "1.0.0"
[sync]

[compositions.tower]
description = "Minimal mesh entry"
primals = ["beardog", "songbird", "skunkbat"]

[compositions.thin-relay]
description = "Relay only"
primals = ["songbird"]

[gates.grapheneGate]
composition = "tower"
repos = []

[gates.golgiBody]
composition = "thin-relay"
repos = ["wateringHole"]

[gates.eastGate]
repos = ["bearDog"]
"#;
        let m: EcosystemManifest = toml::from_str(toml_str).unwrap();

        let g_comp = m.gate_composition("grapheneGate").unwrap();
        assert_eq!(g_comp.primals.len(), 3);
        assert_eq!(g_comp.description, "Minimal mesh entry");

        let golgi_comp = m.gate_composition("golgiBody").unwrap();
        assert_eq!(golgi_comp.primals, vec!["songbird"]);

        assert!(m.gate_composition("eastGate").is_none());
        assert!(m.gate_composition("unknown").is_none());
    }

    #[test]
    fn compositions_default_when_absent() {
        let toml_str = r#"
[meta]
version = "1.0.0"
[sync]
"#;
        let m: EcosystemManifest = toml::from_str(toml_str).unwrap();
        assert!(m.compositions.is_empty());
        assert!(m.composition_names().is_empty());
        assert!(m.composition("anything").is_none());
    }
}
