// SPDX-License-Identifier: AGPL-3.0-or-later

//! Core types for the impulsePotential system.

use cellmembrane_types::DivergencePolicy;
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
    pub repo_policy: DivergencePolicy,
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
    workspace_root
        .join(cellmembrane_types::service::INFRA_WATERING_HOLE)
        .join("impulses")
}

#[must_use]
pub fn active_dir(workspace_root: &Path) -> PathBuf {
    impulses_dir(workspace_root).join("active")
}

#[must_use]
pub fn current_wave(workspace_root: &Path) -> u32 {
    crate::freshness::current_wave(workspace_root)
}

#[must_use]
pub fn resolve_head_ref(workspace_root: &Path, project: &str) -> String {
    if project.is_empty() {
        return String::new();
    }
    crate::git_ops::resolve_head_ref(&workspace_root.join(project))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impulse_type_display() {
        assert_eq!(ImpulseType::Frago.to_string(), "FRAGO");
        assert_eq!(ImpulseType::Status.to_string(), "STATUS");
        assert_eq!(ImpulseType::Request.to_string(), "REQUEST");
        assert_eq!(ImpulseType::Announce.to_string(), "ANNOUNCE");
        assert_eq!(ImpulseType::Sync.to_string(), "SYNC");
    }

    #[test]
    fn priority_display() {
        assert_eq!(Priority::Routine.to_string(), "routine");
        assert_eq!(Priority::Priority.to_string(), "PRIORITY");
        assert_eq!(Priority::Flash.to_string(), "FLASH");
    }

    #[test]
    fn impulse_type_serde_roundtrip() {
        let json = serde_json::to_string(&ImpulseType::Frago).unwrap();
        assert_eq!(json, "\"frago\"");
        let parsed: ImpulseType = serde_json::from_str("\"announce\"").unwrap();
        assert_eq!(parsed, ImpulseType::Announce);
    }

    #[test]
    fn priority_serde_roundtrip() {
        let json = serde_json::to_string(&Priority::Flash).unwrap();
        assert_eq!(json, "\"flash\"");
        let parsed: Priority = serde_json::from_str("\"routine\"").unwrap();
        assert_eq!(parsed, Priority::Routine);
    }

    #[test]
    fn impulse_file_toml_roundtrip() {
        let impulse = ImpulseFile {
            impulse: ImpulseMeta {
                id: "IMP-089-001".into(),
                impulse_type: ImpulseType::Status,
                priority: Priority::Routine,
                wave: 89,
            },
            from: ImpulseFrom {
                gate: "eastGate".into(),
                team: "cellMembrane".into(),
                project: "gardens/cellMembrane".into(),
                git_ref: "ba71a81".into(),
            },
            to: ImpulseTo {
                gates: vec!["*".into()],
                teams: vec![],
            },
            content: ImpulseContent {
                subject: "Wave 88 sprint complete".into(),
                body: String::new(),
            },
            meta: ImpulseOpMeta {
                created: "2026-06-07T12:00:00-04:00".into(),
                expires: String::new(),
                ack_required: false,
            },
            signature: None,
            acks: vec![],
        };
        let serialized = toml::to_string_pretty(&impulse).unwrap();
        let deserialized: ImpulseFile = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.impulse.id, "IMP-089-001");
        assert_eq!(deserialized.impulse.impulse_type, ImpulseType::Status);
        assert_eq!(deserialized.from.gate, "eastGate");
    }

    #[test]
    fn sync_payload_serde() {
        let payload = SyncPayload {
            repo: "primals/toadStool".into(),
            diverge_type: "mutual".into(),
            merge_base: String::new(),
            remotes: [("forgejo".into(), "abc123".into())].into(),
            ahead: [("forgejo".into(), 13)].into(),
            repo_policy: "flag".into(),
            suggested_action: "human review".into(),
        };
        let json = serde_json::to_string(&payload).unwrap();
        let parsed: SyncPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.repo, "primals/toadStool");
        assert_eq!(parsed.ahead["forgejo"], 13);
    }

    #[test]
    fn impulses_dir_path() {
        let root = Path::new("/opt/eco");
        assert_eq!(
            impulses_dir(root),
            PathBuf::from("/opt/eco/infra/wateringHole/impulses")
        );
        assert_eq!(
            active_dir(root),
            PathBuf::from("/opt/eco/infra/wateringHole/impulses/active")
        );
    }

    #[test]
    fn resolve_head_ref_empty_project() {
        let result = resolve_head_ref(Path::new("/tmp"), "");
        assert_eq!(result, "");
    }
}
