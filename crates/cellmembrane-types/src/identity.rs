// SPDX-License-Identifier: AGPL-3.0-or-later

//! Membrane identity types.
//!
//! A membrane's identity is its persistent state across redeploys:
//! family ID, gate ID, mobility class, domain, and host address.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// Whether a gate is physically fixed or mobile.
///
/// Mobile gates (NUCs, laptops) auto-mesh via VPS relay when on WAN and
/// discover LAN peers when plugged in locally. Fixed gates have stable
/// IPs and act as persistent mesh anchors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GateMobility {
    /// Permanently deployed at a fixed location with stable network.
    #[default]
    Fixed,
    /// Physically portable — meshes via VPS relay, LAN-peers when colocated.
    Mobile,
}

impl GateMobility {
    /// Whether this gate needs auto-reconnect on network change.
    #[must_use]
    pub const fn needs_reconnect_hook(&self) -> bool {
        matches!(self, Self::Mobile)
    }

    /// Whether this gate should be treated as a persistent mesh anchor.
    #[must_use]
    pub const fn is_mesh_anchor(&self) -> bool {
        matches!(self, Self::Fixed)
    }
}

impl fmt::Display for GateMobility {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fixed => write!(f, "fixed"),
            Self::Mobile => write!(f, "mobile"),
        }
    }
}

/// How primal processes bind their control sockets.
///
/// Determines the `PRIMAL_BIND_MODE` env var pushed to systemd units.
/// UDS is preferred for local-only gates; TCP is required for ADB/remote
/// gates where UDS paths don't cross host boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BindMode {
    /// Bind both UDS and TCP (default — auto-detect best path).
    #[default]
    Auto,
    /// TCP socket only (required for ADB-tethered or remote-only gates).
    TcpOnly,
    /// UDS with TCP fallback (prefer UDS, fall back to TCP if socket path fails).
    Fallback,
    /// UDS only (pure local, no network socket).
    Uds,
}

impl fmt::Display for BindMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auto => write!(f, "auto"),
            Self::TcpOnly => write!(f, "tcp_only"),
            Self::Fallback => write!(f, "fallback"),
            Self::Uds => write!(f, "uds"),
        }
    }
}

/// Persistent membrane identity from `[membrane.identity]` in `membrane.toml`.
///
/// The family ID ties this membrane to its ecosystem. The gate ID distinguishes
/// it from other membranes in the same family.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MembraneIdentity {
    /// Family identifier — shared across all membranes and gates in one ecosystem.
    /// Maps to `FAMILY_ID` in `tower.env`.
    pub family_id: String,

    /// Unique gate identifier for this membrane instance.
    /// Auto-generated from hostname if not specified.
    #[serde(default)]
    pub gate_id: Option<String>,

    /// Mobility class: fixed (stable location) or mobile (NUC/laptop).
    #[serde(default)]
    pub mobility: GateMobility,

    /// Forward-compatible extension fields.
    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}

impl MembraneIdentity {
    /// Gate ID, falling back to a default derived from the family ID.
    #[must_use]
    pub fn gate_id_or_default(&self) -> String {
        self.gate_id
            .as_ref()
            .map_or_else(|| format!("{}-membrane", self.family_id), Clone::clone)
    }
}
