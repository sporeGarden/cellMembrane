// SPDX-License-Identifier: AGPL-3.0-or-later

//! Ecosystem manifest reader — typed access to `ecosystem_manifest.toml`.
//!
//! Replaces the embedded Python `_py_read_manifest` in `cascade-pull.sh`
//! with a typed Rust reader. The manifest is the single source of truth
//! for repo metadata, gate profiles, sync configuration, and build config.
//!
//! Module layout:
//! - `types.rs`    — serde data structures (pure data, no I/O)
//! - `validate.rs` — post-parse schema validation
//! - `wave.rs`     — wave state + exit criteria

mod types;
mod validate;
mod wave;

pub use types::{
    EcosystemManifest, GateProfile, ManifestBuildConfig, ManifestMeta, RepoEntry, SyncConfig,
    Topology, TopologyRoles,
};
pub use wave::{ExitCriterion, WaveState};

use crate::error::{Result, ShadowError};
use cellmembrane_types::DivergencePolicy;
use std::path::{Path, PathBuf};

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

    /// Get repos filtered by membrane type.
    #[must_use]
    pub fn repos_by_membrane(&self, membrane: &str) -> Vec<(&str, &RepoEntry)> {
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

    /// Resolve build configuration for a primal by name.
    ///
    /// Performs case-insensitive matching against manifest repo keys
    /// (manifest uses `biomeOS`, harvest uses `biomeos`). Returns `None`
    /// if no matching repo entry exists or the entry has no build fields set.
    #[must_use]
    pub fn build_config_for(&self, primal: &str) -> Option<ManifestBuildConfig> {
        let lower = primal.to_lowercase();
        let entry = self.repos.values().find(|e| {
            let repo_key_lower = e
                .local_path
                .rsplit('/')
                .next()
                .unwrap_or("")
                .to_lowercase();
            repo_key_lower == lower
        })?;
        let cfg = ManifestBuildConfig {
            package: entry.package.clone(),
            linker: entry.linker.clone(),
            gpu: entry.gpu,
        };
        if cfg.package.is_none() && cfg.linker.is_none() && !cfg.gpu {
            return None;
        }
        Some(cfg)
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

    /// Find gates that have a specific role in their roles list.
    /// Returns `(gate_name, &GateProfile)` tuples.
    #[must_use]
    pub fn gates_for_role(&self, role: &str) -> Vec<(&str, &GateProfile)> {
        self.gates
            .iter()
            .filter(|(_, profile)| profile.roles.iter().any(|r| r == role))
            .map(|(name, profile)| (name.as_str(), profile))
            .collect()
    }

    /// Resolve the `WireGuard` mesh IP for a named gate.
    #[must_use]
    pub fn mesh_ip_for(&self, gate: &str) -> Option<String> {
        self.gates.get(gate).and_then(|p| p.wg_ip.clone())
    }

    /// Look up a gate's LAN IP from the manifest.
    ///
    /// Returns the `lan_ip` field if set, enabling direct TCP resolution on
    /// the local subnet without DNS or `WireGuard` overlay.
    #[must_use]
    pub fn lan_ip_for(&self, gate: &str) -> Option<String> {
        self.gates.get(gate).and_then(|p| p.lan_ip.clone())
    }

    /// Resolve the best SSH target for a gate from the manifest.
    ///
    /// Priority chain: `host` (explicit VPS hostname) → `lan_ip` (direct LAN
    /// peer) → `wg_ip` (mesh overlay). Returns `None` if the gate is not in
    /// the manifest or has no routable address.
    #[must_use]
    pub fn ssh_target_for(&self, gate: &str) -> Option<String> {
        let p = self.gates.get(gate)?;
        p.host
            .clone()
            .or_else(|| p.lan_ip.clone())
            .or_else(|| p.wg_ip.clone())
    }

    /// Resolve the SSH user for a gate (defaults to `"root"`).
    #[must_use]
    pub fn ssh_user_for(&self, gate: &str) -> &str {
        self.gates
            .get(gate)
            .and_then(|p| p.ssh_user.as_deref())
            .unwrap_or("root")
    }

    /// Resolve GPU primals from the manifest.
    ///
    /// Returns lowercase primal names for repos with `gpu = true`.
    /// Falls back to the compile-time `GPU_PRIMALS` constant when the
    /// manifest is unavailable.
    #[must_use]
    pub fn gpu_primals(&self) -> Vec<String> {
        self.repos
            .iter()
            .filter(|(_, e)| e.gpu && e.category == "primal")
            .map(|(name, _)| name.to_lowercase())
            .collect()
    }
}

/// Resolve the federation peer address from the manifest (golgi by default).
/// Falls back to `DEFAULT_VPS_MESH_PEER` if manifest is unavailable.
#[must_use]
pub fn resolve_federation_peer() -> String {
    let workspace = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_ECOPRIMALS_ROOT,
        cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT,
    );
    let port = cellmembrane_types::service::DEFAULT_FEDERATION_PORT;
    if let Ok(manifest) = load_from_workspace(std::path::Path::new(&workspace)) {
        let hub_gates = manifest.gates_for_role("wg_hub");
        if let Some((_, profile)) = hub_gates.first() {
            if let Some(ref host) = profile.host {
                return format!("{host}:{port}");
            }
            if let Some(ref ip) = profile.wg_ip {
                return format!("{ip}:{port}");
            }
        }
    }
    cellmembrane_types::service::DEFAULT_VPS_MESH_PEER.to_string()
}

/// Convenience: load manifest from workspace root.
///
/// # Errors
/// Returns error if manifest not found or unparseable.
pub fn load_from_workspace(workspace_root: &Path) -> Result<EcosystemManifest> {
    let path = EcosystemManifest::find_in_workspace(workspace_root).ok_or_else(|| {
        ShadowError::Parse(format!(
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
pub async fn load_from_workspace_async(workspace_root: &Path) -> Result<EcosystemManifest> {
    let path = EcosystemManifest::find_in_workspace(workspace_root).ok_or_else(|| {
        ShadowError::Parse(format!(
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
        assert_eq!(manifest.sync.default_source, "temporal");
        assert_eq!(manifest.sync.push_target, PushTarget::Forgejo);
        assert_eq!(manifest.sync.divergence_policy, DivergencePolicy::MergeFf);
    }

    #[test]
    fn parse_manifest_repos() {
        let manifest: EcosystemManifest = toml::from_str(sample_manifest_toml()).unwrap();
        assert_eq!(manifest.repos.len(), 2);

        let bear = &manifest.repos["bearDog"];
        assert_eq!(bear.local_path, "primals/bearDog");
        assert_eq!(bear.category, "primal");
        assert_eq!(bear.org, "ecoPrimals");

        let cm = &manifest.repos["cellMembrane"];
        assert_eq!(cm.local_path, "gardens/cellMembrane");
        assert_eq!(cm.category, "garden");
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
        assert_eq!(manifest.lan_ip_for("sporeGate"), Some("192.168.4.3".into()));
        assert_eq!(manifest.lan_ip_for("eastGate"), None);
        assert_eq!(manifest.mesh_ip_for("sporeGate"), Some("10.13.37.2".into()));
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
        assert_eq!(manifest.sync.default_source, "temporal");
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
        assert_eq!(m.ssh_target_for("golgiBody"), Some("157.230.3.183".into()));
        assert_eq!(m.ssh_target_for("sporeGate"), Some("192.168.4.3".into()));
        assert_eq!(m.ssh_target_for("flockGate"), Some("10.13.37.6".into()));
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
        assert_eq!(g.bond_types, vec!["covalent"]);
        assert_eq!(g.mobility.as_deref(), Some("mobile"));
        assert_eq!(g.repos.len(), 2);
    }

    #[test]
    fn build_config_fields_parsed() {
        let toml_str = r#"
[meta]
version = "1.0.0"
[sync]

[repos.biomeOS]
org = "ecoPrimals"
local_path = "primals/biomeOS"
package = "biomeos-unibin"
category = "primal"

[repos.nestGate]
org = "ecoPrimals"
local_path = "primals/nestGate"
linker = "ld.lld"
category = "primal"

[repos.barraCuda]
org = "ecoPrimals"
local_path = "primals/barraCuda"
gpu = true
category = "primal"

[repos.songBird]
org = "ecoPrimals"
local_path = "primals/songBird"
category = "primal"
"#;
        let m: EcosystemManifest = toml::from_str(toml_str).unwrap();

        let bio = &m.repos["biomeOS"];
        assert_eq!(bio.package.as_deref(), Some("biomeos-unibin"));
        assert!(bio.linker.is_none());
        assert!(!bio.gpu);

        let nest = &m.repos["nestGate"];
        assert!(nest.package.is_none());
        assert_eq!(nest.linker.as_deref(), Some("ld.lld"));

        let barra = &m.repos["barraCuda"];
        assert!(barra.gpu);

        let song = &m.repos["songBird"];
        assert!(song.package.is_none());
        assert!(song.linker.is_none());
        assert!(!song.gpu);
    }

    #[test]
    fn build_config_for_case_insensitive_lookup() {
        let toml_str = r#"
[meta]
version = "1.0.0"
[sync]

[repos.biomeOS]
org = "ecoPrimals"
local_path = "primals/biomeOS"
package = "biomeos-unibin"
category = "primal"

[repos.skunkBat]
org = "ecoPrimals"
local_path = "primals/skunkBat"
package = "skunk-bat-server"
category = "primal"

[repos.nestGate]
org = "ecoPrimals"
local_path = "primals/nestGate"
linker = "ld.lld"
category = "primal"

[repos.songBird]
org = "ecoPrimals"
local_path = "primals/songBird"
category = "primal"
"#;
        let m: EcosystemManifest = toml::from_str(toml_str).unwrap();

        let bio_cfg = m.build_config_for("biomeos").unwrap();
        assert_eq!(bio_cfg.package.as_deref(), Some("biomeos-unibin"));
        assert!(bio_cfg.linker.is_none());

        let skunk_cfg = m.build_config_for("skunkbat").unwrap();
        assert_eq!(skunk_cfg.package.as_deref(), Some("skunk-bat-server"));

        let nest_cfg = m.build_config_for("nestgate").unwrap();
        assert_eq!(nest_cfg.linker.as_deref(), Some("ld.lld"));

        assert!(
            m.build_config_for("songbird").is_none(),
            "no build fields set"
        );
        assert!(m.build_config_for("nonexistent").is_none());
    }

    #[test]
    fn build_config_defaults_when_absent() {
        let toml_str = r#"
[meta]
version = "1.0.0"
[sync]

[repos.bearDog]
org = "ecoPrimals"
local_path = "primals/bearDog"
category = "primal"
"#;
        let m: EcosystemManifest = toml::from_str(toml_str).unwrap();
        let bear = &m.repos["bearDog"];
        assert!(bear.package.is_none());
        assert!(bear.linker.is_none());
        assert!(!bear.gpu);
        assert!(m.build_config_for("beardog").is_none());
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
    fn gpu_primals_from_manifest() {
        let toml_str = r#"
[meta]
version = "1.0.0"
[sync]

[repos.barraCuda]
org = "ecoPrimals"
local_path = "primals/barraCuda"
category = "primal"
gpu = true

[repos.coralReef]
org = "ecoPrimals"
local_path = "primals/coralReef"
category = "primal"
gpu = true

[repos.songBird]
org = "ecoPrimals"
local_path = "primals/songBird"
category = "primal"

[repos.cellMembrane]
org = "sporeGarden"
local_path = "gardens/cellMembrane"
category = "garden"
gpu = true
"#;
        let m: EcosystemManifest = toml::from_str(toml_str).unwrap();
        let gpu = m.gpu_primals();
        assert_eq!(gpu.len(), 2);
        assert!(gpu.contains(&"barracuda".to_string()));
        assert!(gpu.contains(&"coralreef".to_string()));
        assert!(
            !gpu.contains(&"cellmembrane".to_string()),
            "garden with gpu=true should be excluded"
        );
    }
}
