// SPDX-License-Identifier: AGPL-3.0-or-later

//! Top-level `membrane.toml` configuration.
//!
//! This is the user-facing configuration file. A third party writes a
//! `membrane.toml`, parses it with [`MembraneConfig::load`], and validates
//! it with [`MembraneConfig::validate`].

mod validation;

use crate::channels::ChannelConfig;
use crate::composition::MembraneComposition;
use crate::credentials::CredentialConfig;
use crate::default_true;
use crate::envelope::EnvelopeTopology;
use crate::error::ConfigError;
use crate::firewall::FirewallRuleset;
use crate::identity::MembraneIdentity;
use crate::provider::ProviderConfig;
use crate::service::TransportMode;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

/// Root of a `membrane.toml` file.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MembraneConfigFile {
    /// The membrane configuration.
    pub membrane: MembraneConfig,
}

/// Core membrane configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MembraneConfig {
    /// Unique name for this membrane instance.
    pub name: String,

    /// Public domain for this membrane (if serving Channel 3 Surface).
    #[serde(default)]
    pub domain: Option<String>,

    /// Composition tier: relay, rustdesk, tower, nest, nucleus.
    pub composition: MembraneComposition,

    /// K-Derm envelope topology: monoderm (gate-only) or diderm (gate + VPS).
    /// Defaults to diderm for VPS providers, monoderm for gate-local.
    #[serde(default)]
    pub topology: Option<EnvelopeTopology>,

    /// VPS transport mode: `uds_only` (Wave 56 standard), `tcp_default`, or `tcp_opt_in`.
    /// Defaults to `uds_only` for VPS deployments.
    #[serde(default = "default_transport")]
    pub transport: TransportMode,

    /// Membrane identity (family ID, gate ID).
    #[serde(default)]
    pub identity: Option<MembraneIdentity>,

    /// Infrastructure provider configuration.
    #[serde(default)]
    pub provider: Option<ProviderConfig>,

    /// Per-channel overrides.
    #[serde(default)]
    pub channels: ChannelOverrides,

    /// Credential management.
    #[serde(default)]
    pub credentials: CredentialConfig,

    /// Deployment paths — substrate-agnostic base directories.
    #[serde(default)]
    pub paths: DeployPaths,

    /// Hardening configuration.
    #[serde(default)]
    pub hardening: HardeningConfig,

    /// Trust barrier configuration (peptidoglycan composition only).
    #[serde(default)]
    pub trust_barrier: Option<TrustBarrierConfig>,

    /// Telemetry and shadow validation configuration.
    #[serde(default)]
    pub telemetry: TelemetryConfig,

    /// Forward-compatible extension fields.
    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}

/// Deployment path configuration from `[membrane.paths]`.
///
/// Enables substrate-agnostic deployments by making base directories
/// configurable. Services discover their own paths at runtime by combining
/// the base with their binary name — no primal has knowledge of others' locations.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DeployPaths {
    /// Base directory for installed binaries (default: `/opt/membrane`).
    #[serde(default = "DeployPaths::default_install_base")]
    pub install_base: String,

    /// Base directory for UDS runtime sockets (default: `/run/membrane`).
    #[serde(default = "DeployPaths::default_socket_base")]
    pub socket_base: String,

    /// Base directory for credential files (default: `/opt/membrane`).
    #[serde(default = "DeployPaths::default_credential_base")]
    pub credential_base: String,

    /// Default transport endpoint for service injection (JSON format).
    /// When set, primals launched from this config receive `TRANSPORT_ENDPOINT`
    /// env var with this value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport_endpoint: Option<crate::transport::TransportEndpoint>,

    /// Forward-compatible extension fields.
    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}

impl Default for DeployPaths {
    fn default() -> Self {
        Self {
            install_base: Self::default_install_base(),
            socket_base: Self::default_socket_base(),
            credential_base: Self::default_credential_base(),
            transport_endpoint: None,
            extra: BTreeMap::new(),
        }
    }
}

impl DeployPaths {
    fn default_install_base() -> String {
        crate::service::DEFAULT_INSTALL_BASE.to_string()
    }
    fn default_socket_base() -> String {
        crate::service::DEFAULT_SOCKET_BASE.to_string()
    }
    fn default_credential_base() -> String {
        crate::service::DEFAULT_INSTALL_BASE.to_string()
    }

    /// Resolve the install path for a binary given the configured base.
    #[must_use]
    pub fn install_path(&self, binary: &str) -> String {
        format!("{}/{binary}", self.install_base)
    }

    /// Resolve the UDS socket path for a primal given the configured base.
    #[must_use]
    pub fn socket_path(&self, binary: &str) -> String {
        format!("{}/{binary}.sock", self.socket_base)
    }

    /// Produce the `TRANSPORT_ENDPOINT` env var value for a primal.
    ///
    /// If `transport_endpoint` is configured, returns its JSON serialization.
    /// Otherwise, returns a default UDS endpoint derived from `socket_base`.
    #[must_use]
    pub fn transport_env_value(&self, binary: &str) -> String {
        self.transport_endpoint.as_ref().map_or_else(
            || {
                let ep = crate::transport::TransportEndpoint::Uds {
                    path: self.socket_path(binary),
                };
                serde_json::to_string(&ep).unwrap_or_default()
            },
            |ep| serde_json::to_string(ep).unwrap_or_default(),
        )
    }
}

/// Per-channel configuration overrides from `[membrane.channels]`.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ChannelOverrides {
    /// Channel 1: Signal (DNS).
    #[serde(default)]
    pub signal: Option<ChannelConfig>,
    /// Channel 2: Relay (TURN).
    #[serde(default)]
    pub relay: Option<ChannelConfig>,
    /// Channel 3: Surface (TLS).
    #[serde(default)]
    pub surface: Option<ChannelConfig>,
}

/// System hardening configuration from `[membrane.hardening]`.
///
/// Maps to MEM-01 (SSH), MEM-02 (fail2ban), MEM-06 (services), MEM-07 (journald)
/// in `darkforest_membrane.sh`.
///
/// All steps are enabled by default. Disable individual steps via TOML:
/// ```toml
/// [hardening]
/// disabled_steps = ["remove_provider_agent"]
/// ```
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct HardeningConfig {
    /// Steps explicitly disabled by the operator. All others are active.
    #[serde(default)]
    pub disabled_steps: Vec<HardeningStep>,
}

/// Individual hardening steps that can be toggled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HardeningStep {
    /// Install and enable fail2ban (MEM-02).
    Fail2ban,
    /// Enable unattended security updates.
    UnattendedUpgrades,
    /// Remove mail transfer agent — exim4, postfix (MEM-06).
    RemoveMailAgent,
    /// Remove provider-specific agents — droplet-agent, snapd (MEM-06).
    RemoveProviderAgent,
    /// Enable persistent journald storage at `/var/log/journal/` (MEM-07).
    JournaldPersistent,
}

impl HardeningConfig {
    /// All hardening steps in canonical order.
    pub const ALL_STEPS: &[HardeningStep] = &[
        HardeningStep::Fail2ban,
        HardeningStep::UnattendedUpgrades,
        HardeningStep::RemoveMailAgent,
        HardeningStep::RemoveProviderAgent,
        HardeningStep::JournaldPersistent,
    ];

    /// Check if a hardening step is enabled (i.e., not in the disabled list).
    #[must_use]
    pub fn is_enabled(&self, step: HardeningStep) -> bool {
        !self.disabled_steps.contains(&step)
    }

    /// Returns the set of active hardening steps.
    #[must_use]
    pub fn active_steps(&self) -> Vec<HardeningStep> {
        Self::ALL_STEPS
            .iter()
            .copied()
            .filter(|s| self.is_enabled(*s))
            .collect()
    }

    /// Legacy compatibility: check fail2ban enabled.
    #[must_use]
    pub fn fail2ban(&self) -> bool {
        self.is_enabled(HardeningStep::Fail2ban)
    }

    /// Legacy compatibility: check `unattended_upgrades` enabled.
    #[must_use]
    pub fn unattended_upgrades(&self) -> bool {
        self.is_enabled(HardeningStep::UnattendedUpgrades)
    }

    /// Legacy compatibility: check `remove_mail_agent` enabled.
    #[must_use]
    pub fn remove_mail_agent(&self) -> bool {
        self.is_enabled(HardeningStep::RemoveMailAgent)
    }

    /// Legacy compatibility: check `remove_provider_agent` enabled.
    #[must_use]
    pub fn remove_provider_agent(&self) -> bool {
        self.is_enabled(HardeningStep::RemoveProviderAgent)
    }

    /// Legacy compatibility: check `journald_persistent` enabled.
    #[must_use]
    pub fn journald_persistent(&self) -> bool {
        self.is_enabled(HardeningStep::JournaldPersistent)
    }

    /// Services that must NOT be running (MEM-06).
    #[must_use]
    pub const fn prohibited_services() -> &'static [&'static str] {
        &["exim4", "droplet-agent", "snapd"]
    }
}

/// Trust barrier configuration for peptidoglycan composition.
///
/// Defines the relationship between outer and inner membranes,
/// and the opacity guarantees this relay provides.
///
/// ```toml
/// [membrane.trust_barrier]
/// inner_domain = "primal.eco"
/// outer_domain = "primals.eco"
/// opaque_relay = true
/// ```
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TrustBarrierConfig {
    /// Domain of the inner membrane (sovereign, full trust).
    pub inner_domain: String,
    /// Domain of the outer membrane (world-facing, untrusted by inner).
    pub outer_domain: String,
    /// Whether BTSP tokens must be opaque in transit through this barrier.
    #[serde(default = "default_true")]
    pub opaque_relay: bool,
    /// Content domain for CAS objects (optional, e.g. `nestgate.io`).
    #[serde(default)]
    pub content_domain: Option<String>,
}

/// Shadow validation mode — whether telemetry runs in permanent shadow,
/// counts down to cutover, or is disabled.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShadowMode {
    /// Always shadow — never cut over to sovereign telemetry automatically.
    #[default]
    Permanent,
    /// Time-gated — shadow until `cutover_gate_days` of clean data, then cut over.
    Cutover,
    /// No shadow validation. Telemetry may still collect but won't gate transitions.
    Disabled,
}

impl fmt::Display for ShadowMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Permanent => write!(f, "permanent"),
            Self::Cutover => write!(f, "cutover"),
            Self::Disabled => write!(f, "disabled"),
        }
    }
}

/// Telemetry and shadow validation from `[membrane.telemetry]`.
///
/// Aligns with Pillar 4 of `s_membrane_composition.rs`: shadow mode,
/// cutover gate days, and `SkunkBat` correlation requirements.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TelemetryConfig {
    /// Whether telemetry collection is active.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Shadow validation mode. Matches `graph.telemetry.shadow_mode` in deploy graphs.
    #[serde(default)]
    pub shadow_mode: ShadowMode,

    /// Minimum days of clean shadow data before sovereign cutover is allowed.
    /// Must be >= 7 per glacial readiness standard.
    #[serde(default = "default_cutover_days")]
    pub cutover_gate_days: u32,

    /// Whether `SkunkBat` audit correlation is required for this membrane.
    /// Tower+ compositions should always have this true.
    #[serde(default)]
    pub skunkbat_correlation: bool,

    /// Forward-compatible extension fields.
    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            shadow_mode: ShadowMode::Permanent,
            cutover_gate_days: 7,
            skunkbat_correlation: false,
            extra: BTreeMap::new(),
        }
    }
}

/// Minimum days of clean shadow data before cutover (glacial readiness standard).
pub const MIN_CUTOVER_GATE_DAYS: u32 = 7;

const fn default_cutover_days() -> u32 {
    MIN_CUTOVER_GATE_DAYS
}

const fn default_transport() -> TransportMode {
    TransportMode::UdsOnly
}

impl MembraneConfig {
    /// Load a membrane configuration from a TOML file.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Read`] if the file cannot be read, or
    /// [`ConfigError::Parse`] if the TOML content is invalid.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let contents = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        let file: MembraneConfigFile =
            toml::from_str(&contents).map_err(|source| ConfigError::Parse {
                path: path.to_path_buf(),
                source,
            })?;
        Ok(file.membrane)
    }

    /// Resolve the effective K-Derm envelope topology.
    ///
    /// If `topology` is explicitly set, use it. Otherwise, infer from the
    /// provider: gate-local providers default to monoderm, everything else
    /// defaults to diderm.
    #[must_use]
    pub const fn effective_topology(&self) -> EnvelopeTopology {
        if let Some(topo) = self.topology {
            return topo;
        }
        match &self.provider {
            Some(p) if !p.requires_ssh() => EnvelopeTopology::Monoderm,
            _ => EnvelopeTopology::Diderm,
        }
    }

    /// Derive the firewall ruleset for this configuration.
    #[must_use]
    pub fn firewall(&self) -> FirewallRuleset {
        FirewallRuleset::for_composition(self.composition)
    }
}
