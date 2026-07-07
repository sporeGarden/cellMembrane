// SPDX-License-Identifier: AGPL-3.0-or-later

//! Manifest data types — serde structs for `ecosystem_manifest.toml`.
//!
//! Pure data definitions with no I/O or business logic. All query methods
//! live on `EcosystemManifest` in the parent module.

use cellmembrane_types::{DivergencePolicy, EnvelopeLayer, GateTransport, PushTarget, ZoneLabel};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
    #[serde(default)]
    pub divergence_policy: DivergencePolicy,
    /// Whether temporal sync should push to follower remotes.
    #[serde(default)]
    pub push_to_followers: bool,
    /// Push target: "forgejo" (sovereign mediator) or "all" (legacy dual-push).
    /// When "forgejo", temporal.sync pushes only to the forgejo remote;
    /// the VPS push mirror handles GitHub propagation.
    #[serde(default)]
    pub push_target: PushTarget,
    /// Auto-fire a SYNC impulse when divergence is detected.
    #[serde(default)]
    pub diverge_impulse: bool,
    /// Ordered list of remotes to push to (replaces hardcoded `PUSH_REMOTES`).
    /// Defaults to empty (falls back to `["forgejo", "origin"]`).
    #[serde(default)]
    pub push_remotes: Vec<String>,
}

fn default_source() -> String {
    "temporal".into()
}
fn default_branch() -> String {
    "main".into()
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
    pub divergence_policy: Option<DivergencePolicy>,
    /// Remotes to exclude from temporal matrix (e.g. `["upstream"]` for vendor forks).
    #[serde(default)]
    pub exclude_remotes: Vec<String>,

    // ── Build configuration (CI-DIV-01/02/03 absorption) ──────────

    /// Cargo package name when the workspace binary differs from the repo name.
    /// Passed as `cargo build -p <package>`. E.g. biomeOS → `"biomeos-unibin"`.
    #[serde(default)]
    pub package: Option<String>,
    /// Custom linker binary for this primal's build target.
    /// Injected as `CARGO_TARGET_{TARGET}_LINKER`. E.g. nestGate → `"ld.lld"`.
    #[serde(default)]
    pub linker: Option<String>,
    /// Whether this primal requires a glibc (gnu) build for GPU/dlopen access.
    /// When true, `plasmid.harvest` builds both musl and gnu targets.
    #[serde(default)]
    pub gpu: bool,
}

/// Resolved build configuration for a primal, extracted from the manifest.
#[derive(Debug, Clone, Default)]
pub struct ManifestBuildConfig {
    /// Cargo `-p` package override.
    pub package: Option<String>,
    /// Custom linker binary.
    pub linker: Option<String>,
    /// Whether this primal needs a glibc build for GPU workloads.
    pub gpu: bool,
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
    pub transport: Option<GateTransport>,
    /// Gate-specific notes for operators.
    #[serde(default)]
    pub notes: Option<String>,
    /// Cytoplasm zone this gate is in (e.g., `"backbone"`, `"house2"`).
    /// Defined in `TOPOLOGY_MAP.toml [cytoplasm.zones.*]`.
    #[serde(default)]
    pub zone: Option<ZoneLabel>,
    /// Physical port on the zone's hub switch (e.g., `"sfp+2"`, `"ether8"`).
    #[serde(default)]
    pub hub_port: Option<String>,
    /// Link speed to the hub switch in Mbps (e.g., `10000`, `2500`).
    #[serde(default)]
    pub link_speed_mbps: Option<u32>,
    /// K-Derm role: which envelope layer this gate operates at.
    /// E.g., `"plasma_membrane"`, `"periplasm"`, `"outer_membrane"`.
    #[serde(default)]
    pub kderm_role: Option<EnvelopeLayer>,
    /// Site topology annotation (e.g., `"triangle_3hub_backbone"`).
    #[serde(default)]
    pub site_topology: Option<String>,
    /// Functional roles this gate performs (e.g., `["build_hub", "depot", "firewall"]`).
    #[serde(default)]
    pub roles: Vec<String>,
    /// `WireGuard` mesh IP (e.g., `"10.13.37.2"`).
    #[serde(default)]
    pub wg_ip: Option<String>,
    /// `WireGuard` public key.
    #[serde(default)]
    pub wg_pubkey: Option<String>,
    /// Hostname or primary IP for SSH/direct access.
    #[serde(default)]
    pub host: Option<String>,
    /// WAN-facing interface name (e.g., `"enp1s0"`).
    #[serde(default)]
    pub wan_interface: Option<String>,
    /// LAN-facing interface name (e.g., `"eno1"`).
    #[serde(default)]
    pub lan_interface: Option<String>,
    /// LAN IP address (e.g., `"192.168.4.3"`) for direct LAN resolution.
    ///
    /// Enables DNS-agnostic LAN discovery: manifest → `lan_ip` → direct TCP
    /// without requiring dnsmasq entries. Used by `resolve_lan_tcp` when peer
    /// is on the same subnet.
    #[serde(default)]
    pub lan_ip: Option<String>,
    /// SSH user for this gate (defaults to `"root"`).
    #[serde(default)]
    pub ssh_user: Option<String>,
    /// LAN subnet this gate serves (e.g., `"192.168.4.0/22"`).
    #[serde(default)]
    pub lan_subnet: Option<String>,
    /// WAN endpoint for `WireGuard` peers to reach this gate.
    #[serde(default)]
    pub wan_endpoint: Option<String>,
    /// Gate class (e.g., `"portable_anchor"`, `"compute"`, `"relay"`).
    #[serde(default)]
    pub gate_class: Option<String>,
    /// USB/network tether role (e.g., `"usb_rndis"` — provides connectivity to another gate).
    #[serde(default)]
    pub tether_role: Option<String>,
    /// ADB port-forward ports for mobile gates (e.g., `[9100, 9200, 9140]`).
    #[serde(default)]
    pub adb_ports: Vec<u16>,
    /// NUCLEUS status annotation (free-text, e.g., `"Tower LIVE (bearDog+songBird+skunkBat)"`).
    #[serde(default)]
    pub nucleus_status: Option<String>,
    /// Bond types for topology affinity (e.g., `["covalent"]`).
    #[serde(default)]
    pub bond_types: Vec<String>,
}
