// SPDX-License-Identifier: AGPL-3.0-or-later

//! Membrane channel types.
//!
//! A membrane exposes exactly three channel types — Signal, Relay, and Surface —
//! each with distinct trust levels, crypto layers, and port policies. See
//! `specs/CELLMEMBRANE_ARCHITECTURE.md` for the full specification.

use serde::{Deserialize, Serialize};
use std::fmt;

/// The three membrane channel types.
///
/// Every external connection to a membrane enters through exactly one channel.
/// Channels are process-isolated: separate systemd units, no shared state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MembraneChannel {
    /// Channel 1: DNS resolution (knot-dns, port 53).
    Signal,
    /// Channel 2: NAT traversal / TURN relay (Songbird, port 3478).
    Relay,
    /// Channel 3: HTTPS / content delivery (Caddy + NestGate, ports 80/443).
    Surface,
}

impl MembraneChannel {
    /// Returns all channel variants.
    pub fn all() -> &'static [Self] {
        &[Self::Signal, Self::Relay, Self::Surface]
    }

    /// Default port(s) for this channel.
    pub fn default_ports(&self) -> &'static [u16] {
        match self {
            Self::Signal => &[53],
            Self::Relay => &[3478],
            Self::Surface => &[80, 443],
        }
    }

    /// Default primal name for this channel.
    pub fn default_primal(&self) -> &'static str {
        match self {
            Self::Signal => "knot-dns",
            Self::Relay => "songbird",
            Self::Surface => "caddy",
        }
    }

    /// Trust level for this channel.
    pub fn trust_level(&self) -> TrustLevel {
        match self {
            Self::Signal => TrustLevel::Public,
            Self::Relay => TrustLevel::Medium,
            Self::Surface => TrustLevel::High,
        }
    }

    /// Default crypto layer for this channel.
    pub fn default_crypto(&self) -> CryptoLayer {
        match self {
            Self::Signal => CryptoLayer::None,
            Self::Relay => CryptoLayer::TurnHmac,
            Self::Surface => CryptoLayer::Tls,
        }
    }
}

impl fmt::Display for MembraneChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Signal => write!(f, "signal"),
            Self::Relay => write!(f, "relay"),
            Self::Surface => write!(f, "surface"),
        }
    }
}

/// Trust level assigned to a channel based on its exposure profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustLevel {
    /// Public data, no authentication (DNS).
    Public,
    /// Metadata visible, content encrypted (TURN relay).
    Medium,
    /// TLS private keys, session state (HTTPS surface).
    High,
}

impl fmt::Display for TrustLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Public => write!(f, "public"),
            Self::Medium => write!(f, "medium"),
            Self::High => write!(f, "high"),
        }
    }
}

/// Extracellular crypto layer protecting traffic between the membrane and the internet.
///
/// Intracellular crypto (BTSP) is orthogonal and always present when `FAMILY_ID` is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CryptoLayer {
    /// No extracellular encryption (DNS without DNSSEC).
    None,
    /// DNSSEC zone signing.
    Dnssec,
    /// TURN HMAC shared credential.
    TurnHmac,
    /// TLS 1.3 (Let's Encrypt or custom CA).
    Tls,
}

impl fmt::Display for CryptoLayer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => write!(f, "none"),
            Self::Dnssec => write!(f, "dnssec"),
            Self::TurnHmac => write!(f, "turn_hmac"),
            Self::Tls => write!(f, "tls"),
        }
    }
}

/// Configuration for a single membrane channel, as specified in `membrane.toml`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChannelConfig {
    /// Whether this channel is active.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Override the default port for this channel.
    #[serde(default)]
    pub port: Option<u16>,

    /// Override the default primal for this channel.
    #[serde(default)]
    pub primal: Option<String>,

    /// Whether DNSSEC zone signing is enabled (Signal channel only).
    #[serde(default)]
    pub dnssec: Option<bool>,

    /// TLS domain (Surface channel only).
    #[serde(default)]
    pub tls_domain: Option<String>,

    /// ACME email for certificate issuance (Surface channel only).
    #[serde(default)]
    pub acme_email: Option<String>,

    /// Forward-compatible extension fields.
    #[serde(flatten)]
    pub extra: std::collections::BTreeMap<String, toml::Value>,
}

impl Default for ChannelConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            port: None,
            primal: None,
            dnssec: None,
            tls_domain: None,
            acme_email: None,
            extra: std::collections::BTreeMap::new(),
        }
    }
}

use crate::default_true;
