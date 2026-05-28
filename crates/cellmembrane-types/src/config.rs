// SPDX-License-Identifier: AGPL-3.0-or-later

//! Top-level `membrane.toml` configuration.
//!
//! This is the user-facing configuration file. A third party writes a
//! `membrane.toml`, parses it with [`MembraneConfig::load`], and validates
//! it with [`MembraneConfig::validate`].

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
use crate::validation::Report;
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
            extra: BTreeMap::new(),
        }
    }
}

impl DeployPaths {
    fn default_install_base() -> String {
        "/opt/membrane".to_string()
    }
    fn default_socket_base() -> String {
        "/run/membrane".to_string()
    }
    fn default_credential_base() -> String {
        "/opt/membrane".to_string()
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
#[allow(clippy::struct_excessive_bools)]
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
    #[must_use]
    pub const fn prohibited_services() -> &'static [&'static str] {
        &["exim4", "droplet-agent", "snapd"]
    }
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

    /// Validate the configuration against the membrane spec.
    ///
    /// Delegates to focused sub-validators for each concern area.
    #[must_use]
    pub fn validate(&self) -> Report {
        let mut report = Report::new();
        self.validate_core(&mut report);
        self.validate_topology(&mut report);
        self.validate_identity(&mut report);
        self.validate_provider(&mut report);
        self.validate_channels(&mut report);
        self.validate_telemetry(&mut report);
        self.validate_hardening(&mut report);
        self.validate_inventory(&mut report);
        report
    }

    fn validate_core(&self, report: &mut Report) {
        if self.name.is_empty() {
            report.fail("config.name", "Membrane name must not be empty");
        } else {
            report.pass("config.name", format!("Name: {}", self.name));
        }
        report.pass(
            "config.composition",
            format!("Composition: {}", self.composition),
        );

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

        report.pass(
            "credentials.model",
            format!("Credential model: {}", self.credentials.model),
        );

        let spec = self.composition.spec();
        report.info(
            "composition.primals",
            format!(
                "{} primals + {} symbiotic required",
                spec.primals.len(),
                spec.symbiotic.len(),
            ),
        );

        report.pass(
            "transport.mode",
            format!("Transport mode: {}", self.transport),
        );
        if self.transport == TransportMode::UdsOnly {
            let uds_paths = spec.uds_socket_paths();
            let tcp_remain = spec.tcp_ports_uds_mode();
            report.info(
                "transport.uds_sockets",
                format!(
                    "{} primals on UDS, {} TCP ports still required (symbiotic/relay)",
                    uds_paths.len(),
                    tcp_remain.len(),
                ),
            );
        }
    }

    fn validate_topology(&self, report: &mut Report) {
        let topo = self.effective_topology();
        report.pass("topology.effective", format!("Envelope topology: {topo}"));

        if topo.has_periplasm() {
            let boundaries = topo.default_boundaries();
            report.info(
                "topology.boundaries",
                format!(
                    "{} boundary layers, {} periplasmic space(s)",
                    boundaries.len(),
                    topo.periplasm_count(),
                ),
            );
        }

        if topo == EnvelopeTopology::Monoderm {
            if let Some(ref p) = self.provider {
                if p.requires_ssh() {
                    report.warn(
                        "topology.monoderm_vps",
                        "Monoderm topology with remote provider \
                         — VPS acts as periplasm in diderm model",
                    );
                }
            }
        }
    }

    fn validate_identity(&self, report: &mut Report) {
        if self.composition.requires_tower_env() {
            if let Some(ref id) = self.identity {
                if id.family_id.is_empty() {
                    report.fail(
                        "identity.family_id",
                        "Family ID required for Tower+ but is empty",
                    );
                } else {
                    report.pass("identity.family_id", format!("Family ID: {}", id.family_id));
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
                format!("Identity is optional for {} composition", self.composition),
            );
        }
    }

    fn validate_provider(&self, report: &mut Report) {
        if let Some(ref provider) = self.provider {
            report.pass(
                "provider.present",
                format!("Provider: {}", provider.provider_type),
            );
            if provider.requires_ssh()
                && provider.host.is_none()
                && !provider.supports_provisioning()
            {
                report.fail("provider.host", "Bare metal provider requires host address");
            }
        } else {
            report.warn(
                "provider.missing",
                "No provider configured — deployment will require manual host specification",
            );
        }
    }

    fn validate_channels(&self, report: &mut Report) {
        if self.composition >= MembraneComposition::Nest {
            if let Some(ref domain) = self.domain {
                report.pass("surface.domain", format!("Domain: {domain}"));
            } else {
                report.warn(
                    "surface.domain",
                    "Nest composition typically needs a domain for Channel 3 Surface",
                );
            }
        }

        if let Some(ref signal) = self.channels.signal {
            if signal.enabled {
                if signal.dnssec == Some(true) {
                    report.pass(
                        "channel.signal",
                        "Signal channel enabled with DNSSEC zone signing",
                    );
                } else {
                    report.warn(
                        "channel.signal",
                        "Signal channel enabled without DNSSEC — consider enabling zone signing",
                    );
                }
            }
        }

        if let Some(ref relay) = self.channels.relay {
            if relay.enabled {
                if let Some(port) = relay.port {
                    report.info(
                        "channel.relay",
                        format!("Relay channel override: port {port}"),
                    );
                }
            }
        }
    }

    fn validate_telemetry(&self, report: &mut Report) {
        if self.telemetry.enabled {
            report.pass("telemetry.enabled", "Telemetry collection active");
        } else {
            report.warn(
                "telemetry.enabled",
                "Telemetry disabled — shadow validation will not run",
            );
        }

        if self.telemetry.cutover_gate_days >= MIN_CUTOVER_GATE_DAYS {
            report.pass(
                "telemetry.cutover_days",
                format!(
                    "Cutover gate: {} days (>= {} required)",
                    self.telemetry.cutover_gate_days, MIN_CUTOVER_GATE_DAYS,
                ),
            );
        } else {
            report.fail(
                "telemetry.cutover_days",
                format!(
                    "Cutover gate: {} days — glacial readiness requires >= {}",
                    self.telemetry.cutover_gate_days, MIN_CUTOVER_GATE_DAYS,
                ),
            );
        }

        if self.composition.has_btsp() && !self.telemetry.skunkbat_correlation {
            report.warn(
                "telemetry.skunkbat",
                "Tower+ composition should enable skunkbat_correlation for audit",
            );
        }
    }

    fn validate_hardening(&self, report: &mut Report) {
        if !self.hardening.journald_persistent {
            report.warn(
                "hardening.journald",
                "journald_persistent is false — volatile logging, no post-mortem forensics",
            );
        }
    }

    fn validate_inventory(&self, report: &mut Report) {
        let cred_files = crate::credentials::credential_files_for(self.composition);
        report.info(
            "credentials.files",
            format!(
                "{} credential files expected for {} composition",
                cred_files.len(),
                self.composition
            ),
        );

        let binaries = crate::service::binary_integrity_for(self.composition);
        report.info(
            "integrity.binaries",
            format!(
                "{} binaries to verify ({} BLAKE3, {} SHA-256)",
                binaries.len(),
                binaries
                    .iter()
                    .filter(|b| b.hash_algorithm == crate::service::HashAlgorithm::Blake3)
                    .count(),
                binaries
                    .iter()
                    .filter(|b| b.hash_algorithm == crate::service::HashAlgorithm::Sha256)
                    .count(),
            ),
        );

        let fw = self.firewall();
        report.info(
            "firewall.ports",
            format!("{} firewall rules derived", fw.rules.len()),
        );
    }
}
