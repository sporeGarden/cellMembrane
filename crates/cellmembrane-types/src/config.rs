// SPDX-License-Identifier: AGPL-3.0-or-later

//! Top-level `membrane.toml` configuration.
//!
//! This is the user-facing configuration file. A third party writes a
//! `membrane.toml`, parses it with [`MembraneConfig::load`], and validates
//! it with [`MembraneConfig::validate`].

use crate::channels::ChannelConfig;
use crate::composition::MembraneComposition;
use crate::credentials::CredentialConfig;
use crate::firewall::FirewallRuleset;
use crate::identity::MembraneIdentity;
use crate::provider::ProviderConfig;
use crate::validation::Report;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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

    /// Composition tier: relay, rustdesk, tower, nest.
    pub composition: MembraneComposition,

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

    /// Hardening configuration.
    #[serde(default)]
    pub hardening: HardeningConfig,

    /// Telemetry and shadow validation configuration.
    #[serde(default)]
    pub telemetry: TelemetryConfig,

    /// Forward-compatible extension fields.
    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
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
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HardeningConfig {
    /// Install and enable fail2ban (MEM-02).
    #[serde(default = "default_true")]
    pub fail2ban: bool,
    /// Enable unattended security updates.
    #[serde(default = "default_true")]
    pub unattended_upgrades: bool,
    /// Remove mail transfer agent — exim4, postfix (MEM-06).
    #[serde(default = "default_true")]
    pub remove_mail_agent: bool,
    /// Remove provider-specific agents — droplet-agent, snapd (MEM-06).
    #[serde(default = "default_true")]
    pub remove_provider_agent: bool,
    /// Enable persistent journald storage at `/var/log/journal/` (MEM-07).
    #[serde(default = "default_true")]
    pub journald_persistent: bool,
}

impl Default for HardeningConfig {
    fn default() -> Self {
        Self {
            fail2ban: true,
            unattended_upgrades: true,
            remove_mail_agent: true,
            remove_provider_agent: true,
            journald_persistent: true,
        }
    }
}

impl HardeningConfig {
    /// Services that must NOT be running (MEM-06).
    pub fn prohibited_services() -> &'static [&'static str] {
        &["exim4", "droplet-agent", "snapd"]
    }
}

/// Telemetry and shadow validation from `[membrane.telemetry]`.
///
/// Aligns with Pillar 4 of `s_membrane_composition.rs`: shadow mode,
/// cutover gate days, and SkunkBat correlation requirements.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TelemetryConfig {
    /// Whether telemetry collection is active.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Shadow mode: "permanent" (always shadow), "cutover" (time-gated), "disabled".
    /// Matches `graph.telemetry.shadow_mode` in deploy graphs.
    #[serde(default = "default_shadow_mode")]
    pub shadow_mode: String,

    /// Minimum days of clean shadow data before sovereign cutover is allowed.
    /// Must be >= 7 per glacial readiness standard.
    #[serde(default = "default_cutover_days")]
    pub cutover_gate_days: u32,

    /// Whether SkunkBat audit correlation is required for this membrane.
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
            shadow_mode: "permanent".into(),
            cutover_gate_days: 7,
            skunkbat_correlation: false,
            extra: BTreeMap::new(),
        }
    }
}

fn default_shadow_mode() -> String {
    "permanent".into()
}

fn default_cutover_days() -> u32 {
    7
}

fn default_true() -> bool {
    true
}

impl MembraneConfig {
    /// Load a membrane configuration from a TOML file.
    pub fn load(path: &Path) -> Result<Self, String> {
        let contents =
            std::fs::read_to_string(path).map_err(|e| format!("Failed to read {path:?}: {e}"))?;
        let file: MembraneConfigFile =
            toml::from_str(&contents).map_err(|e| format!("Failed to parse {path:?}: {e}"))?;
        Ok(file.membrane)
    }

    /// Derive the firewall ruleset for this configuration.
    pub fn firewall(&self) -> FirewallRuleset {
        FirewallRuleset::for_composition(self.composition)
    }

    /// Validate the configuration against the membrane spec.
    pub fn validate(&self) -> Report {
        let mut report = Report::new();
        let spec = self.composition.spec();

        // Name must be non-empty
        if self.name.is_empty() {
            report.fail("config.name", "Membrane name must not be empty");
        } else {
            report.pass("config.name", format!("Name: {}", self.name));
        }

        // Composition is valid (already parsed, so always valid here)
        report.pass(
            "config.composition",
            format!("Composition: {}", self.composition),
        );

        // Identity required for Tower+
        if self.composition.requires_tower_env() {
            if let Some(ref id) = self.identity {
                if id.family_id.is_empty() {
                    report.fail(
                        "identity.family_id",
                        "Family ID required for Tower+ but is empty",
                    );
                } else {
                    report.pass(
                        "identity.family_id",
                        format!("Family ID: {}", id.family_id),
                    );
                }
            } else {
                report.fail(
                    "identity.required",
                    format!(
                        "{} composition requires [membrane.identity] with family_id",
                        self.composition,
                    ),
                );
            }
        } else {
            report.info(
                "identity.optional",
                format!(
                    "Identity is optional for {} composition",
                    self.composition,
                ),
            );
        }

        // Provider should be present for deployment
        if let Some(ref provider) = self.provider {
            report.pass(
                "provider.present",
                format!("Provider: {}", provider.provider_type),
            );

            if provider.requires_ssh() && provider.host.is_none() {
                if !provider.supports_provisioning() {
                    report.fail(
                        "provider.host",
                        "Bare metal provider requires host address",
                    );
                }
            }
        } else {
            report.warn(
                "provider.missing",
                "No provider configured — deployment will require manual host specification",
            );
        }

        // Domain required for Surface channel
        if self.composition >= MembraneComposition::Nest {
            if self.domain.is_some() {
                report.pass(
                    "surface.domain",
                    format!("Domain: {}", self.domain.as_deref().unwrap_or("")),
                );
            } else {
                report.warn(
                    "surface.domain",
                    "Nest composition typically needs a domain for Channel 3 Surface",
                );
            }
        }

        // Channel overrides validation
        if let Some(ref signal) = self.channels.signal {
            if signal.enabled {
                report.warn(
                    "channel.signal",
                    "Signal channel (knot-dns) enabled — ensure DNS zone is configured",
                );
            }
        }

        // Dark Forest compliance
        if self.composition.dark_forest_compliant() {
            report.pass(
                "dark_forest.composition",
                "Composition supports full Dark Forest compliance",
            );
        } else {
            report.info(
                "dark_forest.composition",
                format!(
                    "{} composition does not enforce BTSP — Dark Forest partial only",
                    self.composition,
                ),
            );
        }

        // Credential model
        report.pass(
            "credentials.model",
            format!("Credential model: {}", self.credentials.model),
        );

        // Primal count
        report.info(
            "composition.primals",
            format!(
                "{} primals + {} symbiotic required",
                spec.primals.len(),
                spec.symbiotic.len(),
            ),
        );

        // Telemetry contract (aligns with s_membrane_composition.rs Pillar 4)
        if self.telemetry.enabled {
            report.pass("telemetry.enabled", "Telemetry collection active");
        } else {
            report.warn("telemetry.enabled", "Telemetry disabled — shadow validation will not run");
        }

        if self.telemetry.cutover_gate_days >= 7 {
            report.pass(
                "telemetry.cutover_days",
                format!("Cutover gate: {} days (>= 7 required)", self.telemetry.cutover_gate_days),
            );
        } else {
            report.fail(
                "telemetry.cutover_days",
                format!(
                    "Cutover gate: {} days — glacial readiness requires >= 7",
                    self.telemetry.cutover_gate_days,
                ),
            );
        }

        if self.composition.has_btsp() && !self.telemetry.skunkbat_correlation {
            report.warn(
                "telemetry.skunkbat",
                "Tower+ composition should enable skunkbat_correlation for audit",
            );
        }

        // Hardening: journald persistence
        if !self.hardening.journald_persistent {
            report.warn(
                "hardening.journald",
                "journald_persistent is false — volatile logging, no post-mortem forensics",
            );
        }

        // Credential file inventory
        let cred_files = crate::credentials::credential_files_for(self.composition);
        report.info(
            "credentials.files",
            format!("{} credential files expected for {} composition", cred_files.len(), self.composition),
        );

        // Binary integrity inventory
        let binaries = crate::service::binary_integrity_for(self.composition);
        report.info(
            "integrity.binaries",
            format!("{} binaries to verify ({} BLAKE3, {} SHA-256)",
                binaries.len(),
                binaries.iter().filter(|b| b.hash_algorithm == crate::service::HashAlgorithm::Blake3).count(),
                binaries.iter().filter(|b| b.hash_algorithm == crate::service::HashAlgorithm::Sha256).count(),
            ),
        );

        // Firewall summary
        let fw = self.firewall();
        report.info(
            "firewall.ports",
            format!("{} firewall rules derived", fw.rules.len()),
        );

        report
    }
}
