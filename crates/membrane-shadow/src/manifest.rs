// SPDX-License-Identifier: AGPL-3.0-or-later

//! Ecosystem manifest reader — typed access to `ecosystem_manifest.toml`.
//!
//! Replaces the embedded Python `_py_read_manifest` in `cascade-pull.sh`
//! with a typed Rust reader. The manifest is the single source of truth
//! for repo metadata, gate profiles, and sync configuration.

use crate::error::{Result, ShadowError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Top-level manifest structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EcosystemManifest {
    /// Metadata — version, wave, counts.
    pub meta: ManifestMeta,
    /// Sync configuration — default source, divergence policy.
    pub sync: SyncConfig,
    /// K-Derm diderm topology — node placement and roles.
    #[serde(default)]
    pub topology: Option<Topology>,
    /// Repository definitions keyed by short name (e.g. `biomeOS`).
    #[serde(default)]
    pub repos: BTreeMap<String, RepoEntry>,
    /// Gate profiles keyed by gate name (e.g. `eastGate`).
    #[serde(default)]
    pub gates: BTreeMap<String, GateProfile>,
}

/// K-Derm diderm topology configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Topology {
    /// Envelope model: `monoderm` or `diderm`.
    #[serde(default)]
    pub model: String,
    /// Inner membrane node name.
    #[serde(default)]
    pub inner_membrane: String,
    /// Peptidoglycan (structural) node name.
    #[serde(default)]
    pub peptidoglycan: String,
    /// Outer membrane node name.
    #[serde(default)]
    pub outer_membrane: String,
    /// Per-host IP addresses.
    #[serde(default)]
    pub hosts: BTreeMap<String, String>,
    /// Layer-specific functional roles in the waterFall relay chain.
    #[serde(default)]
    pub roles: Option<TopologyRoles>,
}

/// Functional role assignments per K-Derm layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopologyRoles {
    /// Cis face: receives gate pushes (Forgejo sovereign store).
    #[serde(default)]
    pub push_receiver: String,
    /// Structural: sync hub + impulse cascade mediator.
    #[serde(default)]
    pub sync_mediator: String,
    /// Trans face: ships to extracellular (GitHub SSH push).
    #[serde(default)]
    pub external_publisher: String,
}

/// Manifest metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestMeta {
    /// Semantic version of the manifest format.
    pub version: String,
    /// Date the manifest was last generated.
    #[serde(default)]
    pub generated: String,
    /// Current wave number.
    #[serde(default)]
    pub wave: u32,
    /// Total repo count.
    #[serde(default)]
    pub total_repos: u32,
}

/// `WaterFall` sync configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConfig {
    /// Forgejo HTTPS base URL.
    #[serde(default)]
    pub forgejo_base_url: String,
    /// Forgejo SSH URL for git operations.
    #[serde(default)]
    pub forgejo_ssh: String,
    /// Default sync source (`origin`, `forgejo`, `auto`, `temporal`).
    #[serde(default = "default_source")]
    pub default_source: String,
    /// Default git branch.
    #[serde(default = "default_branch")]
    pub default_branch: String,
    /// Global divergence policy: `flag`, `merge-ff`, `merge-rebase`,
    /// `impulse-only`, `agentic`. Per-repo overrides in `RepoEntry`.
    #[serde(default = "default_divergence_policy")]
    pub divergence_policy: String,
    /// Whether temporal sync should push to follower remotes.
    #[serde(default)]
    pub push_to_followers: bool,
    /// Push target: "forgejo" (sovereign mediator) or "all" (legacy dual-push).
    /// When "forgejo", temporal.sync pushes only to the forgejo remote;
    /// the VPS push mirror handles GitHub propagation.
    #[serde(default = "default_push_target")]
    pub push_target: String,
    /// Auto-fire a SYNC impulse when divergence is detected.
    #[serde(default)]
    pub diverge_impulse: bool,
    /// Ordered list of remotes to push to (replaces hardcoded `PUSH_REMOTES`).
    /// Defaults to empty (falls back to `["forgejo", "origin"]`).
    #[serde(default)]
    pub push_remotes: Vec<String>,
}

fn default_push_target() -> String {
    "all".into()
}

fn default_source() -> String {
    "temporal".into()
}
fn default_branch() -> String {
    "main".into()
}
fn default_divergence_policy() -> String {
    "flag".into()
}

/// A single repository entry from `[repos.*]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoEntry {
    /// GitHub org (e.g. `ecoPrimals`).
    pub org: String,
    /// Local path relative to workspace root.
    pub local_path: String,
    /// Membrane type: `inner-only`, `trailing-mirror`, `bidirectional`, `outer-only`.
    #[serde(default)]
    pub membrane: String,
    /// Sync priority: `high`, `standard`, `low`.
    #[serde(default)]
    pub sync_priority: String,
    /// Category: `primal`, `spring`, `garden`, `infra`, `root`.
    #[serde(default)]
    pub category: String,
    /// Human description.
    #[serde(default)]
    pub description: String,
    /// Full GitHub repo path (e.g. `ecoPrimals/biomeOS`).
    #[serde(default)]
    pub github_repo: String,
    /// Full Forgejo repo path.
    #[serde(default)]
    pub forgejo_repo: String,
    /// Default branch override.
    #[serde(default)]
    pub default_branch: Option<String>,
    /// Per-repo divergence policy override (falls back to `sync.divergence_policy`).
    /// Values: `flag`, `merge-ff`, `merge-rebase`, `impulse-only`, `agentic`.
    #[serde(default)]
    pub divergence_policy: Option<String>,
    /// Remotes to exclude from temporal matrix (e.g. `["upstream"]` for vendor forks).
    #[serde(default)]
    pub exclude_remotes: Vec<String>,
}

/// Gate profile — topology-aware configuration for deterministic deployment.
///
/// Each gate in the ecosystem has a profile that describes its architecture,
/// transport, composition, and behavior. `gate.bootstrap` reads this profile
/// to configure the gate without operator memory (guideStone P1: Deterministic).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateProfile {
    /// List of repo short names this gate syncs.
    #[serde(default)]
    pub repos: Vec<String>,
    /// Target architecture (e.g. `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`).
    #[serde(default)]
    pub target: Option<String>,
    /// Mobility classification: `fixed` or `mobile`.
    #[serde(default)]
    pub mobility: Option<String>,
    /// Mesh relay peer address for federation (e.g. `157.230.3.183:7700`).
    #[serde(default)]
    pub mesh_peer: Option<String>,
    /// `PRIMAL_BIND_MODE` for this gate (e.g. `tcp_only`, `fallback`, `uds`).
    #[serde(default)]
    pub bind_mode: Option<String>,
    /// Composition: which primals to start (e.g. `tower`, `compute`, `full`).
    #[serde(default)]
    pub composition: Option<String>,
    /// Transport: how binaries reach this gate (`wan`, `lan`, `adb`, `local`).
    #[serde(default)]
    pub transport: Option<String>,
    /// Gate-specific notes for operators.
    #[serde(default)]
    pub notes: Option<String>,
}

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

    /// Find the manifest file relative to a workspace root.
    /// Looks at `infra/wateringHole/ecosystem_manifest.toml`.
    #[must_use]
    pub fn find_in_workspace(workspace_root: &Path) -> Option<PathBuf> {
        let path = workspace_root.join("infra/wateringHole/ecosystem_manifest.toml");
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
    pub fn divergence_policy_for<'a>(&'a self, entry: &'a RepoEntry) -> &'a str {
        entry
            .divergence_policy
            .as_deref()
            .unwrap_or(&self.sync.divergence_policy)
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
}

// ── Wave Lifecycle ───────────────────────────────────────────────────

/// Typed representation of a wave's lifecycle state.
///
/// Evolves the raw `meta.wave` u32 into a domain object that can track
/// lifecycle progression. Freshness and cascade become derived views of
/// the wave state rather than hand-crafted TOML fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaveState {
    /// Wave numeric identifier.
    pub id: u32,
    /// ISO-8601 date when the wave was opened (first cascade at this ID).
    #[serde(default)]
    pub opened: Option<String>,
    /// ISO-8601 date when exit criteria were met and wave was closed.
    #[serde(default)]
    pub closed: Option<String>,
    /// Exit criteria with their satisfaction state.
    #[serde(default)]
    pub exit_criteria: Vec<ExitCriterion>,
    /// Last rootPulse session committed for this wave (sovereignty proof).
    #[serde(default)]
    pub last_rootpulse_session: Option<String>,
}

/// A single exit criterion for wave closure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExitCriterion {
    /// Human-readable description of the criterion.
    pub description: String,
    /// Whether this criterion has been satisfied.
    #[serde(default)]
    pub satisfied: bool,
}

impl WaveState {
    /// Create a new open wave.
    #[must_use]
    pub fn open(id: u32) -> Self {
        Self {
            id,
            opened: Some(chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()),
            closed: None,
            exit_criteria: Vec::new(),
            last_rootpulse_session: None,
        }
    }

    /// Construct from manifest meta (backward compatible with raw wave ID).
    #[must_use]
    pub const fn from_manifest(meta: &ManifestMeta) -> Self {
        Self {
            id: meta.wave,
            opened: None,
            closed: None,
            exit_criteria: Vec::new(),
            last_rootpulse_session: None,
        }
    }

    /// Whether all exit criteria are satisfied.
    #[must_use]
    pub fn is_closeable(&self) -> bool {
        !self.exit_criteria.is_empty() && self.exit_criteria.iter().all(|c| c.satisfied)
    }

    /// Mark the wave as closed with the current timestamp.
    pub fn close(&mut self) {
        self.closed = Some(chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string());
    }

    /// Record a rootpulse session.
    pub fn record_rootpulse(&mut self, session_id: &str) {
        self.last_rootpulse_session = Some(session_id.to_string());
    }
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(manifest.sync.push_target, "forgejo");
        assert_eq!(manifest.sync.divergence_policy, "merge-ff");
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
        assert_eq!(manifest.sync.divergence_policy, "flag");
        assert_eq!(manifest.sync.push_target, "all");
    }
}
