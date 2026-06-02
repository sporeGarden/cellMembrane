// SPDX-License-Identifier: AGPL-3.0-or-later

//! Core types for the impulsePotential system.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Impulse types following the `IMPULSE_POTENTIAL_STANDARD`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ImpulseType {
    /// Amends a standing order — action required.
    Frago,
    /// Informational state update — no action required.
    Status,
    /// Asks for something from target gate(s).
    Request,
    /// Broadcast ecosystem-wide notice.
    Announce,
    /// Divergence detected — merge coordination needed (auto-fired by cascade).
    Sync,
}

impl std::fmt::Display for ImpulseType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Frago => write!(f, "FRAGO"),
            Self::Status => write!(f, "STATUS"),
            Self::Request => write!(f, "REQUEST"),
            Self::Announce => write!(f, "ANNOUNCE"),
            Self::Sync => write!(f, "SYNC"),
        }
    }
}

/// Priority levels for impulses.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    /// Normal workflow coordination.
    Routine,
    /// Time-sensitive, blocking other work.
    Priority,
    /// Critical — requires immediate attention.
    Flash,
}

impl std::fmt::Display for Priority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Routine => write!(f, "routine"),
            Self::Priority => write!(f, "PRIORITY"),
            Self::Flash => write!(f, "FLASH"),
        }
    }
}

/// Top-level impulse file structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpulseFile {
    /// Impulse metadata.
    pub impulse: ImpulseMeta,
    /// Origin information.
    pub from: ImpulseFrom,
    /// Target information.
    pub to: ImpulseTo,
    /// Message content.
    pub content: ImpulseContent,
    /// Operational metadata.
    pub meta: ImpulseOpMeta,
    /// Ed25519 signature (optional — present when bearDog is available).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<ImpulseSignature>,
    /// Acknowledgments from receiving gates.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub acks: Vec<ImpulseAck>,
}

/// The `[impulse]` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpulseMeta {
    /// Unique impulse ID.
    pub id: String,
    /// Impulse type.
    #[serde(rename = "type")]
    pub impulse_type: ImpulseType,
    /// Priority level.
    pub priority: Priority,
    /// Wave number when created.
    pub wave: u32,
}

/// The `[from]` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpulseFrom {
    /// Originating gate.
    pub gate: String,
    /// Team name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub team: String,
    /// Project path (e.g. "springs/hotSpring").
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub project: String,
    /// Commit ref — rootPulse DAG provenance.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    #[serde(rename = "ref")]
    pub git_ref: String,
}

/// The `[to]` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpulseTo {
    /// Target gates (`["*"]` for broadcast).
    pub gates: Vec<String>,
    /// Target teams (informational).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub teams: Vec<String>,
}

/// The `[content]` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpulseContent {
    /// Short subject line (max 80 chars).
    pub subject: String,
    /// Optional extended body.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body: String,
}

/// The `[meta]` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpulseOpMeta {
    /// Creation timestamp.
    pub created: String,
    /// Expiration timestamp (optional).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub expires: String,
    /// Whether acknowledgment is required.
    #[serde(default)]
    pub ack_required: bool,
}

/// Optional Ed25519 signature (bearDog signing).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpulseSignature {
    /// Signing algorithm.
    pub algorithm: String,
    /// Hex-encoded Ed25519 public key of the signing gate.
    pub public_key: String,
    /// Hex-encoded signature over the canonical impulse payload.
    pub value: String,
    /// When the signature was created.
    pub signed_at: String,
}

/// An acknowledgment entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpulseAck {
    /// Gate that acknowledged.
    pub gate: String,
    /// When it was acknowledged.
    pub timestamp: String,
    /// Optional note.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// Arguments for firing a new impulse (rootPulse ACTION).
pub struct PostArgs<'a> {
    /// Target gate names.
    pub to_gates: Vec<&'a str>,
    /// Impulse type (frago, status, request, announce).
    pub impulse_type: ImpulseType,
    /// Priority level.
    pub priority: Priority,
    /// Short subject line.
    pub subject: &'a str,
    /// Optional extended body text.
    pub body: &'a str,
    /// Project path (e.g. "springs/hotSpring").
    pub project: &'a str,
    /// Team name.
    pub team: &'a str,
}

/// Structured payload for SYNC impulses — carries divergence context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPayload {
    /// Repo local path (e.g. `infra/plasmidBin`).
    pub repo: String,
    /// Divergence type classification.
    pub diverge_type: String,
    /// Merge base (common ancestor SHA, if known).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub merge_base: String,
    /// Per-remote HEAD SHAs.
    pub remotes: std::collections::BTreeMap<String, String>,
    /// Per-remote ahead counts.
    pub ahead: std::collections::BTreeMap<String, u32>,
    /// Repo-level divergence policy from manifest.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub repo_policy: String,
    /// Suggested resolution action.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub suggested_action: String,
}

/// Top-level structure for SYNC impulse files (extends `ImpulseFile` with payload).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncImpulseFile {
    /// Standard impulse metadata.
    pub impulse: ImpulseMeta,
    /// Origin information.
    pub from: ImpulseFrom,
    /// Target information.
    pub to: ImpulseTo,
    /// Message content.
    pub content: ImpulseContent,
    /// Divergence payload.
    pub payload: SyncPayload,
    /// Operational metadata.
    pub meta: ImpulseOpMeta,
    /// Ed25519 signature (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<ImpulseSignature>,
    /// Acknowledgments from receiving gates.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub acks: Vec<ImpulseAck>,
}

/// Arguments for auto-firing a SYNC divergence impulse from cascade.
pub struct SyncDivergeArgs {
    /// Repo local path.
    pub repo_path: String,
    /// Per-remote position data `(remote, ahead, behind)`.
    pub positions: Vec<(String, u32, u32)>,
    /// Per-repo divergence policy from manifest.
    pub repo_policy: String,
}

/// Result of `potential.check` — membrane gradient health.
#[derive(Debug, Serialize)]
pub struct PotentialHealth {
    /// Total active impulses.
    pub total: usize,
    /// Impulses needing ack.
    pub needs_ack: usize,
    /// Expired but unarchived.
    pub expired: usize,
    /// Impulses per wave.
    pub by_wave: std::collections::BTreeMap<u32, usize>,
    /// Current wave.
    pub current_wave: u32,
}

#[must_use]
pub fn impulses_dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join("infra/wateringHole/impulses")
}

#[must_use]
pub fn active_dir(workspace_root: &Path) -> PathBuf {
    impulses_dir(workspace_root).join("active")
}

#[must_use]
pub fn current_wave(workspace_root: &Path) -> u32 {
    let freshness = workspace_root.join("infra/wateringHole/freshness.toml");
    if let Ok(contents) = std::fs::read_to_string(&freshness) {
        if let Ok(val) = contents.parse::<toml::Table>() {
            if let Some(wave) = val.get("wave").and_then(|w| w.as_table()) {
                if let Some(id) = wave.get("id").and_then(toml::Value::as_integer) {
                    return id as u32;
                }
            }
        }
    }
    0
}

#[must_use]
pub fn resolve_head_ref(workspace_root: &Path, project: &str) -> String {
    if project.is_empty() {
        return String::new();
    }
    crate::git_ops::resolve_head_ref(&workspace_root.join(project))
}
