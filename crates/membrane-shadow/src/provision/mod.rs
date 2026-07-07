// SPDX-License-Identifier: AGPL-3.0-or-later
//! Cloud provisioning — create, monitor, and destroy fieldMouse droplets.
//!
//! Implements the "Glowplug" tier of the 3-tier diesel engine deployment model:
//! - Ember (sandbox): validates new builds before promotion
//! - Cylinder (main golgiBody): production, running HEAD
//! - Glowplug (canary droplet): warm standby, previous-good binaries
//!
//! Provider-agnostic interface backed by `DigitalOcean` (extensible to Hetzner).

#[cfg(feature = "http")]
pub mod digitalocean;

pub mod bootstrap;
pub mod verify;

use serde::{Deserialize, Serialize};

/// Supported cloud providers for automated provisioning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    /// `DigitalOcean` VPS (DO API v2).
    DigitalOcean,
    /// Hetzner Cloud (hcloud API) — reserved, not yet implemented.
    Hetzner,
}

impl std::fmt::Display for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DigitalOcean => write!(f, "digitalocean"),
            Self::Hetzner => write!(f, "hetzner"),
        }
    }
}

/// Error returned when parsing an unknown cloud provider string.
#[derive(Debug, Clone, thiserror::Error)]
#[error("unknown provider: {0}")]
pub struct ProviderParseError(pub String);

impl std::str::FromStr for Provider {
    type Err = ProviderParseError;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "digitalocean" | "do" => Ok(Self::DigitalOcean),
            "hetzner" => Ok(Self::Hetzner),
            _ => Err(ProviderParseError(s.to_string())),
        }
    }
}

/// Configuration for provisioning a new droplet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvisionRequest {
    /// Human-readable name for the droplet (becomes hostname).
    pub name: String,
    /// Cloud region (e.g. "nyc1", "sfo3").
    pub region: String,
    /// Droplet size slug (e.g. "s-1vcpu-2gb").
    pub size: String,
    /// OS image slug (e.g. "debian-12-x64").
    pub image: String,
    /// Gate profile to apply after provisioning.
    pub profile: String,
    /// SSH key fingerprints or IDs to inject.
    pub ssh_keys: Vec<String>,
    /// Tags for organization.
    pub tags: Vec<String>,
}

/// Environment variables for provision defaults.
const ENV_PROVISION_NAME: &str = "MEMBRANE_PROVISION_NAME";
const ENV_PROVISION_PROFILE: &str = "MEMBRANE_PROVISION_PROFILE";
const ENV_PROVISION_REGION: &str = "MEMBRANE_PROVISION_REGION";
const ENV_PROVISION_SIZE: &str = "MEMBRANE_PROVISION_SIZE";

impl Default for ProvisionRequest {
    fn default() -> Self {
        use cellmembrane_types::service::env_or;
        Self {
            name: env_or(ENV_PROVISION_NAME, "membrane-canary"),
            region: env_or(ENV_PROVISION_REGION, "nyc1"),
            size: env_or(ENV_PROVISION_SIZE, "s-1vcpu-2gb"),
            image: "debian-12-x64".into(),
            profile: env_or(ENV_PROVISION_PROFILE, "canary-fieldmouse"),
            ssh_keys: Vec::new(),
            tags: vec!["membrane".into(), "canary".into(), "ecoprimals".into()],
        }
    }
}

/// State of a provisioned droplet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DropletState {
    /// Provider-assigned droplet ID.
    pub id: u64,
    /// Droplet name/hostname.
    pub name: String,
    /// Current status (new, active, off, archive).
    pub status: String,
    /// Public IPv4 address (populated once active).
    pub ip: Option<String>,
    /// Region where deployed.
    pub region: String,
    /// Gate profile applied.
    pub profile: String,
    /// When provisioned (ISO 8601).
    pub created_at: String,
}

/// Outcome of a provision operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvisionOutcome {
    /// Whether the entire pipeline succeeded.
    pub success: bool,
    /// Droplet state at conclusion (if created).
    pub droplet: Option<DropletState>,
    /// Summary message.
    pub message: String,
    /// Phases completed during bootstrap.
    pub phases: Vec<String>,
}
