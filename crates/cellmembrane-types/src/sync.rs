// SPDX-License-Identifier: AGPL-3.0-or-later

//! Ecosystem sync and gate transport types.
//!
//! Typed enums for divergence resolution, push targets, and gate transport
//! modes declared in the ecosystem manifest (`ecosystem_manifest.toml`).

use serde::{Deserialize, Serialize};
use std::fmt;

/// How git divergence between local and upstream is resolved during sync.
///
/// Declared in the ecosystem manifest as `divergence_policy`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DivergencePolicy {
    /// Flag divergence for human review — no automatic resolution.
    #[default]
    Flag,
    /// Fast-forward merge when possible.
    MergeFf,
    /// Rebase local commits onto upstream before push.
    MergeRebase,
    /// Only sync impulse commits; block until reviewed.
    ImpulseOnly,
    /// Agent-driven resolution with automated merge/rebase decisions.
    Agentic,
}

impl DivergencePolicy {
    /// Whether this policy resolves divergence without human intervention.
    #[must_use]
    pub const fn resolves_automatically(&self) -> bool {
        matches!(self, Self::MergeFf | Self::MergeRebase | Self::Agentic)
    }

    /// Whether this policy blocks sync until a human reviews divergence.
    #[must_use]
    pub const fn requires_human_review(&self) -> bool {
        matches!(self, Self::ImpulseOnly | Self::Flag)
    }
}

impl fmt::Display for DivergencePolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Flag => write!(f, "flag"),
            Self::MergeFf => write!(f, "merge-ff"),
            Self::MergeRebase => write!(f, "merge-rebase"),
            Self::ImpulseOnly => write!(f, "impulse-only"),
            Self::Agentic => write!(f, "agentic"),
        }
    }
}

/// Which git remotes receive pushes after a successful sync.
///
/// Declared in the ecosystem manifest as `push_target`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PushTarget {
    /// Push to all configured remotes (Forgejo and GitHub).
    #[default]
    All,
    /// Push only to the self-hosted Forgejo instance.
    Forgejo,
    /// Push only to GitHub.
    Github,
}

impl PushTarget {
    /// Whether pushes include the Forgejo remote.
    #[must_use]
    pub const fn includes_forgejo(&self) -> bool {
        matches!(self, Self::All | Self::Forgejo)
    }

    /// Whether pushes include the GitHub remote.
    #[must_use]
    pub const fn includes_github(&self) -> bool {
        matches!(self, Self::All | Self::Github)
    }
}

impl fmt::Display for PushTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::All => write!(f, "all"),
            Self::Forgejo => write!(f, "forgejo"),
            Self::Github => write!(f, "github"),
        }
    }
}

/// Transport mode for reaching a gate during deploy or sync operations.
///
/// Declared per-gate in the ecosystem manifest as `transport`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateTransport {
    /// Reach gate over the public internet (WAN / VPS relay).
    #[default]
    Wan,
    /// Reach gate on the local area network.
    Lan,
    /// Reach gate via Android Debug Bridge (USB tethered device).
    Adb,
    /// Gate is the local machine — no remote transport needed.
    Local,
}

impl GateTransport {
    /// Whether this transport uses network connectivity (WAN or LAN).
    #[must_use]
    pub const fn is_network(&self) -> bool {
        matches!(self, Self::Wan | Self::Lan)
    }

    /// Whether this transport fetches artifacts from the HTTPS depot.
    #[must_use]
    pub const fn requires_depot(&self) -> bool {
        matches!(self, Self::Wan)
    }

    /// Whether this transport requires physical tethering (USB/ADB).
    #[must_use]
    pub const fn is_tethered(&self) -> bool {
        matches!(self, Self::Adb)
    }

    /// Whether this transport is the local machine (no remote hop needed).
    #[must_use]
    pub const fn is_local(&self) -> bool {
        matches!(self, Self::Local)
    }
}

impl fmt::Display for GateTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wan => write!(f, "wan"),
            Self::Lan => write!(f, "lan"),
            Self::Adb => write!(f, "adb"),
            Self::Local => write!(f, "local"),
        }
    }
}

/// Default cascade sync source preference.
///
/// Controls which git remote is preferred when `temporal.cascade` syncs repos.
/// Declared in the ecosystem manifest as `sync.default_source`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CascadeSource {
    /// Use the temporal matrix leader selection (default).
    #[default]
    Temporal,
    /// Prefer the sovereign Forgejo remote.
    Forgejo,
    /// Prefer the GitHub mirror remote (`origin`).
    Origin,
    /// Auto-detect best source per repo.
    Auto,
}

impl fmt::Display for CascadeSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Temporal => write!(f, "temporal"),
            Self::Forgejo => write!(f, "forgejo"),
            Self::Origin => write!(f, "origin"),
            Self::Auto => write!(f, "auto"),
        }
    }
}

/// How a repository relates to the inner/outer membrane boundary.
///
/// Declared per-repo in the ecosystem manifest as `membrane`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MembraneSyncMode {
    /// Only synced within the inner membrane (sovereign Forgejo).
    #[default]
    InnerOnly,
    /// Inner is source of truth; outer (GitHub) is a delayed mirror.
    TrailingMirror,
    /// Changes flow both directions (inner ↔ outer).
    Bidirectional,
    /// Outer-only (GitHub source, no Forgejo copy).
    OuterOnly,
}

impl fmt::Display for MembraneSyncMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InnerOnly => write!(f, "inner-only"),
            Self::TrailingMirror => write!(f, "trailing-mirror"),
            Self::Bidirectional => write!(f, "bidirectional"),
            Self::OuterOnly => write!(f, "outer-only"),
        }
    }
}

/// Sync priority for temporal cascade ordering.
///
/// Declared per-repo in the ecosystem manifest as `sync_priority`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncPriority {
    /// Synced first — critical infrastructure repos.
    High,
    /// Normal ordering (default).
    #[default]
    Standard,
    /// Synced last — non-critical or large repos.
    Low,
}

impl fmt::Display for SyncPriority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::High => write!(f, "high"),
            Self::Standard => write!(f, "standard"),
            Self::Low => write!(f, "low"),
        }
    }
}

/// Repository category in the ecosystem taxonomy.
///
/// Declared per-repo in the ecosystem manifest as `category`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepoCategory {
    /// Core primal binary (bearDog, songBird, etc.).
    #[default]
    Primal,
    /// Spring runtime/framework (primalSpring, groundSpring).
    Spring,
    /// Garden tooling (cellMembrane, peptiDog).
    Garden,
    /// Infrastructure and configuration (wateringHole).
    Infra,
    /// Monorepo root.
    Root,
    /// Composition target (footPrint, etc.).
    Protist,
}

impl fmt::Display for RepoCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Primal => write!(f, "primal"),
            Self::Spring => write!(f, "spring"),
            Self::Garden => write!(f, "garden"),
            Self::Infra => write!(f, "infra"),
            Self::Root => write!(f, "root"),
            Self::Protist => write!(f, "protist"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- DivergencePolicy ---

    #[test]
    fn divergence_policy_serde_roundtrip() {
        for policy in [
            DivergencePolicy::Flag,
            DivergencePolicy::MergeFf,
            DivergencePolicy::MergeRebase,
            DivergencePolicy::ImpulseOnly,
            DivergencePolicy::Agentic,
        ] {
            let json = serde_json::to_string(&policy).unwrap();
            let parsed: DivergencePolicy = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, policy);
        }
    }

    #[test]
    fn divergence_policy_serde_kebab_case() {
        assert_eq!(
            serde_json::to_string(&DivergencePolicy::MergeFf).unwrap(),
            "\"merge-ff\""
        );
        assert_eq!(
            serde_json::to_string(&DivergencePolicy::MergeRebase).unwrap(),
            "\"merge-rebase\""
        );
        assert_eq!(
            serde_json::to_string(&DivergencePolicy::ImpulseOnly).unwrap(),
            "\"impulse-only\""
        );
    }

    #[test]
    fn divergence_policy_display() {
        assert_eq!(DivergencePolicy::Flag.to_string(), "flag");
        assert_eq!(DivergencePolicy::MergeFf.to_string(), "merge-ff");
        assert_eq!(DivergencePolicy::MergeRebase.to_string(), "merge-rebase");
        assert_eq!(DivergencePolicy::ImpulseOnly.to_string(), "impulse-only");
        assert_eq!(DivergencePolicy::Agentic.to_string(), "agentic");
    }

    #[test]
    fn divergence_policy_default_is_flag() {
        assert_eq!(DivergencePolicy::default(), DivergencePolicy::Flag);
    }

    #[test]
    fn divergence_policy_resolves_automatically() {
        assert!(!DivergencePolicy::Flag.resolves_automatically());
        assert!(DivergencePolicy::MergeFf.resolves_automatically());
        assert!(DivergencePolicy::MergeRebase.resolves_automatically());
        assert!(!DivergencePolicy::ImpulseOnly.resolves_automatically());
        assert!(DivergencePolicy::Agentic.resolves_automatically());
    }

    #[test]
    fn divergence_policy_requires_human_review() {
        assert!(DivergencePolicy::Flag.requires_human_review());
        assert!(!DivergencePolicy::MergeFf.requires_human_review());
        assert!(!DivergencePolicy::MergeRebase.requires_human_review());
        assert!(DivergencePolicy::ImpulseOnly.requires_human_review());
        assert!(!DivergencePolicy::Agentic.requires_human_review());
    }

    // --- PushTarget ---

    #[test]
    fn push_target_serde_roundtrip() {
        for target in [PushTarget::All, PushTarget::Forgejo, PushTarget::Github] {
            let json = serde_json::to_string(&target).unwrap();
            let parsed: PushTarget = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, target);
        }
    }

    #[test]
    fn push_target_serde_snake_case() {
        assert_eq!(serde_json::to_string(&PushTarget::All).unwrap(), "\"all\"");
        assert_eq!(
            serde_json::to_string(&PushTarget::Forgejo).unwrap(),
            "\"forgejo\""
        );
        assert_eq!(
            serde_json::to_string(&PushTarget::Github).unwrap(),
            "\"github\""
        );
    }

    #[test]
    fn push_target_display() {
        assert_eq!(PushTarget::All.to_string(), "all");
        assert_eq!(PushTarget::Forgejo.to_string(), "forgejo");
        assert_eq!(PushTarget::Github.to_string(), "github");
    }

    #[test]
    fn push_target_default_is_all() {
        assert_eq!(PushTarget::default(), PushTarget::All);
    }

    #[test]
    fn push_target_includes_forgejo() {
        assert!(PushTarget::All.includes_forgejo());
        assert!(PushTarget::Forgejo.includes_forgejo());
        assert!(!PushTarget::Github.includes_forgejo());
    }

    #[test]
    fn push_target_includes_github() {
        assert!(PushTarget::All.includes_github());
        assert!(!PushTarget::Forgejo.includes_github());
        assert!(PushTarget::Github.includes_github());
    }

    // --- GateTransport ---

    #[test]
    fn gate_transport_serde_roundtrip() {
        for transport in [
            GateTransport::Wan,
            GateTransport::Lan,
            GateTransport::Adb,
            GateTransport::Local,
        ] {
            let json = serde_json::to_string(&transport).unwrap();
            let parsed: GateTransport = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, transport);
        }
    }

    #[test]
    fn gate_transport_serde_snake_case() {
        assert_eq!(
            serde_json::to_string(&GateTransport::Wan).unwrap(),
            "\"wan\""
        );
        assert_eq!(
            serde_json::to_string(&GateTransport::Lan).unwrap(),
            "\"lan\""
        );
        assert_eq!(
            serde_json::to_string(&GateTransport::Adb).unwrap(),
            "\"adb\""
        );
        assert_eq!(
            serde_json::to_string(&GateTransport::Local).unwrap(),
            "\"local\""
        );
    }

    #[test]
    fn gate_transport_display() {
        assert_eq!(GateTransport::Wan.to_string(), "wan");
        assert_eq!(GateTransport::Lan.to_string(), "lan");
        assert_eq!(GateTransport::Adb.to_string(), "adb");
        assert_eq!(GateTransport::Local.to_string(), "local");
    }

    #[test]
    fn gate_transport_default_is_wan() {
        assert_eq!(GateTransport::default(), GateTransport::Wan);
    }

    #[test]
    fn gate_transport_is_network() {
        assert!(GateTransport::Wan.is_network());
        assert!(GateTransport::Lan.is_network());
        assert!(!GateTransport::Adb.is_network());
        assert!(!GateTransport::Local.is_network());
    }

    #[test]
    fn gate_transport_requires_depot() {
        assert!(GateTransport::Wan.requires_depot());
        assert!(!GateTransport::Lan.requires_depot());
        assert!(!GateTransport::Adb.requires_depot());
        assert!(!GateTransport::Local.requires_depot());
    }

    #[test]
    fn gate_transport_is_tethered() {
        assert!(GateTransport::Adb.is_tethered());
        assert!(!GateTransport::Wan.is_tethered());
        assert!(!GateTransport::Lan.is_tethered());
        assert!(!GateTransport::Local.is_tethered());
    }

    #[test]
    fn gate_transport_is_local() {
        assert!(GateTransport::Local.is_local());
        assert!(!GateTransport::Wan.is_local());
        assert!(!GateTransport::Lan.is_local());
        assert!(!GateTransport::Adb.is_local());
    }
}
