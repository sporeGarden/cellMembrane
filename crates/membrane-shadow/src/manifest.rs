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

/// WaterFall sync configuration.
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
    /// What to do with diverged repos: `flag` or `skip`.
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
}

/// Gate profile — which repos a gate cares about.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateProfile {
    /// List of repo short names this gate syncs.
    pub repos: Vec<String>,
}

impl EcosystemManifest {
    /// Load manifest from a TOML file.
    ///
    /// # Errors
    /// Returns `ShadowError::Io` if the file can't be read, or
    /// `ShadowError::Parse` if the TOML is malformed.
    pub fn load(path: &Path) -> Result<Self> {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| ShadowError::Io(e))?;
        toml::from_str(&contents)
            .map_err(|e| ShadowError::Parse(format!("manifest parse: {e}")))
    }

    /// Find the manifest file relative to a workspace root.
    /// Looks at `infra/wateringHole/ecosystem_manifest.toml`.
    pub fn find_in_workspace(workspace_root: &Path) -> Option<PathBuf> {
        let path = workspace_root.join("infra/wateringHole/ecosystem_manifest.toml");
        path.exists().then_some(path)
    }

    /// Get repo entries for a specific gate, resolved to `RepoEntry` references.
    /// Returns entries in the order they appear in the gate profile.
    pub fn gate_repos(&self, gate: &str) -> Vec<(&str, &RepoEntry)> {
        let Some(profile) = self.gates.get(gate) else {
            return Vec::new();
        };
        profile
            .repos
            .iter()
            .filter_map(|name| {
                self.repos.get(name.as_str()).map(|entry| (name.as_str(), entry))
            })
            .collect()
    }

    /// Get local paths for a gate's repos (what cascade-pull iterates).
    pub fn gate_local_paths(&self, gate: &str) -> Vec<&str> {
        self.gate_repos(gate)
            .into_iter()
            .map(|(_, entry)| entry.local_path.as_str())
            .collect()
    }

    /// Get all distinct org names from repos.
    pub fn orgs(&self) -> Vec<&str> {
        let mut orgs: Vec<&str> = self
            .repos
            .values()
            .map(|r| r.org.as_str())
            .collect();
        orgs.sort_unstable();
        orgs.dedup();
        orgs
    }

    /// Get repos filtered by membrane type.
    pub fn repos_by_membrane(&self, membrane: &str) -> Vec<(&str, &RepoEntry)> {
        self.repos
            .iter()
            .filter(|(_, e)| e.membrane == membrane)
            .map(|(name, entry)| (name.as_str(), entry))
            .collect()
    }

    /// Build a GitHub clone URL for a repo.
    pub fn github_clone_url(entry: &RepoEntry) -> String {
        format!("https://github.com/{}.git", entry.github_repo)
    }

    /// Build a Forgejo SSH clone URL using the sync config.
    pub fn forgejo_clone_url(&self, entry: &RepoEntry) -> String {
        format!("{}/{}.git", self.sync.forgejo_ssh, entry.forgejo_repo)
    }
}

/// Convenience: load manifest from workspace root.
///
/// # Errors
/// Returns error if manifest not found or unparseable.
pub fn load_from_workspace(workspace_root: &Path) -> Result<EcosystemManifest> {
    let path = EcosystemManifest::find_in_workspace(workspace_root)
        .ok_or_else(|| ShadowError::Parse(
            format!("ecosystem_manifest.toml not found under {}", workspace_root.display())
        ))?;
    EcosystemManifest::load(&path)
}
