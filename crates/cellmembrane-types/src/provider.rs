// SPDX-License-Identifier: AGPL-3.0-or-later

//! Infrastructure provider configuration.
//!
//! Abstracts the substrate where a membrane runs — VPS cloud, bare metal,
//! or LAN gate. See `specs/MULTI_MEMBRANE_DEPLOYMENT.md`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// Provider type identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderType {
    /// `DigitalOcean` VPS (provisioned via `doctl`).
    #[serde(alias = "digitalocean")]
    DigitalOcean,
    /// Hetzner Cloud (provisioned via `hcloud`).
    Hetzner,
    /// Physical or virtual machine with SSH access, no cloud API.
    BareMetal,
    /// Local LAN gate — deploys to localhost, no SSH.
    GateLocal,
    /// User-provided provisioning; membrane only handles deploy.
    Custom,
}

impl fmt::Display for ProviderType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DigitalOcean => write!(f, "digitalocean"),
            Self::Hetzner => write!(f, "hetzner"),
            Self::BareMetal => write!(f, "bare_metal"),
            Self::GateLocal => write!(f, "gate_local"),
            Self::Custom => write!(f, "custom"),
        }
    }
}

/// Provider configuration from `[membrane.provider]` in `membrane.toml`.
///
/// Common fields are typed; provider-specific fields land in `extra`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProviderConfig {
    /// Provider type.
    #[serde(rename = "type")]
    pub provider_type: ProviderType,

    /// Cloud region or datacenter location.
    #[serde(default)]
    pub region: Option<String>,

    /// VM size / server type.
    #[serde(default)]
    pub size: Option<String>,

    /// OS image identifier.
    #[serde(default)]
    pub image: Option<String>,

    /// Remote host for bare-metal or SSH-based deploys.
    #[serde(default)]
    pub host: Option<String>,

    /// SSH user (defaults to "root").
    #[serde(default)]
    pub ssh_user: Option<String>,

    /// SSH port (defaults to 22).
    #[serde(default)]
    pub ssh_port: Option<u16>,

    /// Provider-specific extension fields.
    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}

impl ProviderConfig {
    /// SSH user, defaulting to "root".
    #[must_use]
    pub fn ssh_user_or_default(&self) -> &str {
        self.ssh_user.as_deref().unwrap_or("root")
    }

    /// SSH port, defaulting to 22.
    #[must_use]
    pub fn ssh_port_or_default(&self) -> u16 {
        self.ssh_port.unwrap_or(crate::composition::SSH_PORT)
    }

    /// Whether this provider requires remote SSH access for deployment.
    #[must_use]
    pub const fn requires_ssh(&self) -> bool {
        !matches!(self.provider_type, ProviderType::GateLocal)
    }

    /// Whether this provider supports API-based provisioning.
    #[must_use]
    pub const fn supports_provisioning(&self) -> bool {
        matches!(
            self.provider_type,
            ProviderType::DigitalOcean | ProviderType::Hetzner
        )
    }

    /// Derive the substrate profile from the provider type.
    #[must_use]
    pub const fn substrate_profile(&self) -> SubstrateProfile {
        match self.provider_type {
            ProviderType::DigitalOcean | ProviderType::Hetzner | ProviderType::Custom => {
                SubstrateProfile::VpsFieldMouse
            }
            ProviderType::BareMetal => SubstrateProfile::RemoteCovalent,
            ProviderType::GateLocal => SubstrateProfile::GateLocal,
        }
    }
}

/// Deployment context constraints derived from the substrate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubstrateProfile {
    /// External VPS — full hardening, provider is adversary.
    VpsFieldMouse,
    /// Remote bare metal — SSH hardening, partial trust.
    RemoteCovalent,
    /// Owned LAN hardware — physical trust, local deploy.
    GateLocal,
}

impl SubstrateProfile {
    /// Whether `biomeOS` integration is available on this substrate.
    #[must_use]
    pub const fn has_biomeos(&self) -> bool {
        matches!(self, Self::GateLocal)
    }

    /// Whether full Dark Forest hardening is required.
    #[must_use]
    pub const fn requires_full_hardening(&self) -> bool {
        matches!(self, Self::VpsFieldMouse)
    }
}

impl fmt::Display for SubstrateProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::VpsFieldMouse => write!(f, "vps_fieldmouse"),
            Self::RemoteCovalent => write!(f, "remote_covalent"),
            Self::GateLocal => write!(f, "gate_local"),
        }
    }
}
