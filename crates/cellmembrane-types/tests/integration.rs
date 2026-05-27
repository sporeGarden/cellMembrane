// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Cross-module integration tests: config parsing, validation, provider
// inference, and topology integration. Domain-specific tests live in
// their own modules (channels.rs, composition.rs, envelope.rs, etc.).

use cellmembrane_types::composition::MembraneComposition;
use cellmembrane_types::config::MembraneConfig;
use cellmembrane_types::credentials::CredentialModel;
use cellmembrane_types::envelope::EnvelopeTopology;
use cellmembrane_types::provider::{ProviderType, SubstrateProfile};
use cellmembrane_types::validation::Severity;
use std::path::Path;

// --- membrane.toml parsing ---

#[test]
fn parse_reference_membrane_toml() {
    let config = MembraneConfig::load(Path::new("../../membrane.toml"))
        .expect("Failed to parse reference membrane.toml");

    assert_eq!(config.name, "membrane-relay");
    assert_eq!(config.domain.as_deref(), Some("membrane.primals.eco"));
    assert_eq!(config.composition, MembraneComposition::Nest);
}

#[test]
fn parse_identity_from_toml() {
    let config = MembraneConfig::load(Path::new("../../membrane.toml")).unwrap();
    let identity = config.identity.expect("identity should be present");
    assert_eq!(identity.family_id, "membrane-alpha");
    assert_eq!(identity.gate_id.as_deref(), Some("nyc-01"));
}

#[test]
fn parse_provider_from_toml() {
    let config = MembraneConfig::load(Path::new("../../membrane.toml")).unwrap();
    let provider = config.provider.expect("provider should be present");
    assert_eq!(provider.provider_type, ProviderType::DigitalOcean);
    assert_eq!(provider.region.as_deref(), Some("nyc1"));
    assert_eq!(provider.size.as_deref(), Some("s-1vcpu-2gb"));
}

#[test]
fn parse_channel_overrides() {
    let config = MembraneConfig::load(Path::new("../../membrane.toml")).unwrap();
    let signal = config.channels.signal.as_ref().expect("signal should exist");
    assert!(signal.enabled);

    let relay = config.channels.relay.as_ref().expect("relay should exist");
    assert!(relay.enabled);
    assert_eq!(relay.port, Some(3478));
}

#[test]
fn parse_minimal_membrane_toml() {
    let toml = r#"
    [membrane]
    name = "minimal"
    composition = "relay"
    "#;

    let file: cellmembrane_types::config::MembraneConfigFile =
        toml::from_str(toml).expect("Failed to parse minimal config");
    assert_eq!(file.membrane.name, "minimal");
    assert_eq!(file.membrane.composition, MembraneComposition::Relay);
    assert!(file.membrane.identity.is_none());
    assert!(file.membrane.provider.is_none());
}

// --- Provider types ---

#[test]
fn provider_substrate_profiles() {
    let toml = r#"type = "digitalocean""#;
    let p: cellmembrane_types::provider::ProviderConfig = toml::from_str(toml).unwrap();
    assert_eq!(p.substrate_profile(), SubstrateProfile::VpsFieldMouse);

    let toml = r#"type = "gate_local""#;
    let p: cellmembrane_types::provider::ProviderConfig = toml::from_str(toml).unwrap();
    assert_eq!(p.substrate_profile(), SubstrateProfile::GateLocal);
    assert!(!p.requires_ssh());
}

// --- Credential model ---

#[test]
fn credential_model_defaults_to_age() {
    assert_eq!(CredentialModel::default(), CredentialModel::Age);
}

// --- Hardening config ---

#[test]
fn hardening_defaults_include_journald() {
    let config: cellmembrane_types::config::MembraneConfigFile = toml::from_str(r#"
        [membrane]
        name = "test"
        composition = "relay"
    "#).unwrap();
    assert!(config.membrane.hardening.journald_persistent);
}

#[test]
fn hardening_prohibited_services() {
    let prohibited = cellmembrane_types::config::HardeningConfig::prohibited_services();
    assert!(prohibited.contains(&"exim4"));
    assert!(prohibited.contains(&"droplet-agent"));
    assert!(prohibited.contains(&"snapd"));
}

// --- Telemetry config ---

#[test]
fn telemetry_defaults_match_glacial_standard() {
    let config: cellmembrane_types::config::MembraneConfigFile = toml::from_str(r#"
        [membrane]
        name = "test"
        composition = "relay"
    "#).unwrap();
    let t = &config.membrane.telemetry;
    assert!(t.enabled);
    assert_eq!(t.shadow_mode, cellmembrane_types::config::ShadowMode::Permanent);
    assert!(t.cutover_gate_days >= 7);
}

#[test]
fn telemetry_parsed_from_reference_toml() {
    let config = MembraneConfig::load(Path::new("../../membrane.toml")).unwrap();
    assert!(config.telemetry.enabled);
    assert_eq!(config.telemetry.shadow_mode, cellmembrane_types::config::ShadowMode::Permanent);
    assert_eq!(config.telemetry.cutover_gate_days, 7);
    assert!(config.telemetry.skunkbat_correlation);
}

// --- Topology integration ---

#[test]
fn reference_toml_has_diderm_topology() {
    let config = MembraneConfig::load(Path::new("../../membrane.toml")).unwrap();
    assert_eq!(config.topology, Some(EnvelopeTopology::Diderm));
    assert_eq!(config.effective_topology(), EnvelopeTopology::Diderm);
}

#[test]
fn topology_defaults_to_diderm_for_vps() {
    let toml = r#"
    [membrane]
    name = "test"
    composition = "relay"

    [membrane.provider]
    type = "digitalocean"
    "#;
    let file: cellmembrane_types::config::MembraneConfigFile = toml::from_str(toml).unwrap();
    assert!(file.membrane.topology.is_none());
    assert_eq!(file.membrane.effective_topology(), EnvelopeTopology::Diderm);
}

#[test]
fn topology_defaults_to_monoderm_for_gate_local() {
    let toml = r#"
    [membrane]
    name = "gate"
    composition = "tower"

    [membrane.identity]
    family_id = "test"

    [membrane.provider]
    type = "gate_local"
    "#;
    let file: cellmembrane_types::config::MembraneConfigFile = toml::from_str(toml).unwrap();
    assert!(file.membrane.topology.is_none());
    assert_eq!(file.membrane.effective_topology(), EnvelopeTopology::Monoderm);
}

// --- Cross-module validation ---

#[test]
fn validate_reference_config() {
    let config = MembraneConfig::load(Path::new("../../membrane.toml")).unwrap();
    let report = config.validate();
    assert!(
        report.is_ok(),
        "Reference config should validate:\n{report}"
    );
    assert!(report.count(Severity::Fail) == 0);
    assert!(report.count(Severity::Pass) > 0);
}

#[test]
fn validate_tower_without_identity_fails() {
    let toml = r#"
    [membrane]
    name = "no-identity"
    composition = "tower"
    "#;

    let file: cellmembrane_types::config::MembraneConfigFile = toml::from_str(toml).unwrap();
    let report = file.membrane.validate();
    assert!(!report.is_ok(), "Tower without identity should fail");
    assert!(report.count(Severity::Fail) > 0);
}

#[test]
fn validate_relay_without_identity_passes() {
    let toml = r#"
    [membrane]
    name = "relay-only"
    composition = "relay"
    "#;

    let file: cellmembrane_types::config::MembraneConfigFile = toml::from_str(toml).unwrap();
    let report = file.membrane.validate();
    assert!(report.is_ok(), "Relay without identity should pass:\n{report}");
}

#[test]
fn validate_empty_name_fails() {
    let toml = r#"
    [membrane]
    name = ""
    composition = "relay"
    "#;

    let file: cellmembrane_types::config::MembraneConfigFile = toml::from_str(toml).unwrap();
    let report = file.membrane.validate();
    assert!(!report.is_ok());
}

#[test]
fn validate_low_cutover_days_fails() {
    let toml = r#"
    [membrane]
    name = "bad-cutover"
    composition = "relay"

    [membrane.telemetry]
    cutover_gate_days = 3
    "#;

    let file: cellmembrane_types::config::MembraneConfigFile = toml::from_str(toml).unwrap();
    let report = file.membrane.validate();
    assert!(!report.is_ok(), "cutover_gate_days < 7 should fail:\n{report}");
}

#[test]
fn validate_tower_without_skunkbat_correlation_warns() {
    let toml = r#"
    [membrane]
    name = "no-skunkbat"
    composition = "tower"

    [membrane.identity]
    family_id = "test-family"

    [membrane.telemetry]
    skunkbat_correlation = false
    "#;

    let file: cellmembrane_types::config::MembraneConfigFile = toml::from_str(toml).unwrap();
    let report = file.membrane.validate();
    assert!(
        report.count(Severity::Warn) > 0,
        "Tower without skunkbat_correlation should warn"
    );
}

#[test]
fn validate_reference_includes_credential_and_integrity_info() {
    let config = MembraneConfig::load(Path::new("../../membrane.toml")).unwrap();
    let report = config.validate();
    let checks: Vec<&str> = report.entries.iter().map(|e| e.check.as_str()).collect();
    assert!(checks.contains(&"credentials.files"), "Should report credential file count");
    assert!(checks.contains(&"integrity.binaries"), "Should report binary integrity count");
    assert!(checks.contains(&"telemetry.enabled"), "Should report telemetry status");
    assert!(checks.contains(&"telemetry.cutover_days"), "Should report cutover gate");
}

#[test]
fn validate_reference_includes_topology_info() {
    let config = MembraneConfig::load(Path::new("../../membrane.toml")).unwrap();
    let report = config.validate();
    let checks: Vec<&str> = report.entries.iter().map(|e| e.check.as_str()).collect();
    assert!(checks.contains(&"topology.effective"), "Should report topology");
    assert!(checks.contains(&"topology.boundaries"), "Should report boundary count for diderm");
}

#[test]
fn validate_monoderm_vps_warns() {
    let toml = r#"
    [membrane]
    name = "odd-setup"
    composition = "relay"
    topology = "monoderm"

    [membrane.provider]
    type = "digitalocean"
    "#;
    let file: cellmembrane_types::config::MembraneConfigFile = toml::from_str(toml).unwrap();
    let report = file.membrane.validate();
    assert!(
        report.entries.iter().any(|e| e.check == "topology.monoderm_vps"),
        "Monoderm with VPS should warn"
    );
}
