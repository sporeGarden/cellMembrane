// SPDX-License-Identifier: AGPL-3.0-or-later

//! `MembraneConfig` validation — sub-validators for each concern area.

use super::{MembraneConfig, MIN_CUTOVER_GATE_DAYS};
use crate::composition::MembraneComposition;
use crate::envelope::EnvelopeTopology;
use crate::service::TransportMode;
use crate::validation::Report;

impl MembraneConfig {
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
        if !self.hardening.journald_persistent() {
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
