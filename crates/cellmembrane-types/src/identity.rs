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

/// Gate role classification for manifest `[gates.*.roles]`.
///
/// Each role maps to either a `ServiceCapability` (service roles) or an
/// infrastructure concern (infra roles like `WgHub`, `NatFirewall`).
/// Unknown role strings deserialize to `Other(String)` for forward compatibility.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GateRole {
    /// Mesh relay — message routing between gates.
    Relay,
    /// Mesh relay (alias).
    MeshRelay,
    /// TURN server for NAT traversal.
    Turn,
    /// STUN server for NAT traversal.
    Stun,
    /// Security services.
    Security,
    /// Cryptographic signing and key management.
    Crypto,
    /// Authentication and authorization.
    Auth,
    /// Content serving (static/API).
    Content,
    /// Forgejo sovereign git hosting.
    Forgejo,
    /// Binary depot serving.
    Depot,
    /// Observability and health aggregation.
    Observability,
    /// Metrics collection.
    Metrics,
    /// Compute orchestration.
    Compute,
    /// Build services.
    Build,
    /// Build hub (primary build authority).
    BuildHub,
    /// Persistent storage.
    Storage,
    /// nestGate data persistence.
    Nest,
    /// Ledger/audit trail.
    Ledger,
    /// Gate identity and certificate management.
    Identity,
    /// Caddy reverse proxy.
    Caddy,
    /// Caddy with TLS termination.
    CaddyTls,
    /// Generic TLS terminator.
    TlsTerminator,
    /// `WireGuard` hub (mesh relay point).
    WgHub,
    /// NAT/firewall gateway.
    NatFirewall,
    /// DHCP server.
    Dhcp,
    /// HTTP service host.
    Http,
    /// API gateway.
    Gateway,
    /// CI/CD pipeline.
    Ci,
    /// Primary DNS server.
    DnsPrimary,
    /// Forward-compatible catch-all for unknown role strings.
    Other(String),
}

impl Serialize for GateRole {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for GateRole {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(Self::from(s.as_str()))
    }
}

impl From<&str> for GateRole {
    fn from(s: &str) -> Self {
        match s {
            "relay" => Self::Relay,
            "mesh_relay" => Self::MeshRelay,
            "turn" => Self::Turn,
            "stun" => Self::Stun,
            "security" => Self::Security,
            "crypto" => Self::Crypto,
            "auth" => Self::Auth,
            "content" => Self::Content,
            "forgejo" => Self::Forgejo,
            "depot" => Self::Depot,
            "observability" => Self::Observability,
            "metrics" => Self::Metrics,
            "compute" => Self::Compute,
            "build" => Self::Build,
            "build_hub" => Self::BuildHub,
            "storage" => Self::Storage,
            "nest" => Self::Nest,
            "ledger" => Self::Ledger,
            "identity" => Self::Identity,
            "caddy" => Self::Caddy,
            "caddy_tls" => Self::CaddyTls,
            "tls_terminator" => Self::TlsTerminator,
            "wg_hub" => Self::WgHub,
            "nat_firewall" => Self::NatFirewall,
            "dhcp" => Self::Dhcp,
            "http" => Self::Http,
            "gateway" => Self::Gateway,
            "ci" => Self::Ci,
            "dns_primary" => Self::DnsPrimary,
            other => Self::Other(other.to_string()),
        }
    }
}

impl GateRole {
    /// Map this role to its service capability, if any.
    ///
    /// Infra-only roles (`WgHub`, `NatFirewall`, `Dhcp`, `Caddy*`, `DnsPrimary`)
    /// return `None` — they describe infrastructure, not primal services.
    #[must_use]
    pub const fn as_capability(&self) -> Option<super::ServiceCapability> {
        use super::ServiceCapability;
        match self {
            Self::Relay | Self::MeshRelay => Some(ServiceCapability::MeshRelay),
            Self::Turn | Self::Stun => Some(ServiceCapability::TurnServer),
            Self::Security | Self::Crypto | Self::Auth => Some(ServiceCapability::CryptoSigner),
            Self::Content | Self::Forgejo | Self::Depot | Self::Http | Self::Gateway => {
                Some(ServiceCapability::ContentServing)
            }
            Self::Observability | Self::Metrics => Some(ServiceCapability::Observability),
            Self::Compute | Self::Build | Self::BuildHub => {
                Some(ServiceCapability::ComputeOrchestration)
            }
            Self::Storage | Self::Nest | Self::Ledger => Some(ServiceCapability::Storage),
            Self::Identity => Some(ServiceCapability::Identity),
            _ => None,
        }
    }

    /// Whether this is an infrastructure-only role (no primal service mapping).
    #[must_use]
    pub const fn is_infra(&self) -> bool {
        matches!(
            self,
            Self::WgHub
                | Self::NatFirewall
                | Self::Dhcp
                | Self::Caddy
                | Self::CaddyTls
                | Self::TlsTerminator
                | Self::DnsPrimary
                | Self::Ci
        )
    }

    /// Whether this role indicates TLS termination responsibility.
    #[must_use]
    pub const fn is_tls(&self) -> bool {
        matches!(self, Self::Caddy | Self::CaddyTls | Self::TlsTerminator)
    }
}

impl fmt::Display for GateRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Relay => write!(f, "relay"),
            Self::MeshRelay => write!(f, "mesh_relay"),
            Self::Turn => write!(f, "turn"),
            Self::Stun => write!(f, "stun"),
            Self::Security => write!(f, "security"),
            Self::Crypto => write!(f, "crypto"),
            Self::Auth => write!(f, "auth"),
            Self::Content => write!(f, "content"),
            Self::Forgejo => write!(f, "forgejo"),
            Self::Depot => write!(f, "depot"),
            Self::Observability => write!(f, "observability"),
            Self::Metrics => write!(f, "metrics"),
            Self::Compute => write!(f, "compute"),
            Self::Build => write!(f, "build"),
            Self::BuildHub => write!(f, "build_hub"),
            Self::Storage => write!(f, "storage"),
            Self::Nest => write!(f, "nest"),
            Self::Ledger => write!(f, "ledger"),
            Self::Identity => write!(f, "identity"),
            Self::Caddy => write!(f, "caddy"),
            Self::CaddyTls => write!(f, "caddy_tls"),
            Self::TlsTerminator => write!(f, "tls_terminator"),
            Self::WgHub => write!(f, "wg_hub"),
            Self::NatFirewall => write!(f, "nat_firewall"),
            Self::Dhcp => write!(f, "dhcp"),
            Self::Http => write!(f, "http"),
            Self::Gateway => write!(f, "gateway"),
            Self::Ci => write!(f, "ci"),
            Self::DnsPrimary => write!(f, "dns_primary"),
            Self::Other(s) => write!(f, "{s}"),
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
