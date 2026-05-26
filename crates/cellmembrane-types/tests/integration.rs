// SPDX-License-Identifier: AGPL-3.0-or-later

use cellmembrane_types::channels::{CryptoLayer, MembraneChannel};
use cellmembrane_types::composition::MembraneComposition;
use cellmembrane_types::config::MembraneConfig;
use cellmembrane_types::credentials::CredentialModel;
use cellmembrane_types::firewall::FirewallRuleset;
use cellmembrane_types::provider::{ProviderType, SubstrateProfile};
use cellmembrane_types::service::MembraneService;
use cellmembrane_types::validation::Severity;
use std::path::Path;

// --- membrane.toml parsing ---

#[test]
fn parse_reference_membrane_toml() {
    let config = MembraneConfig::load(Path::new("../../membrane.toml"))
        .expect("Failed to parse reference membrane.toml");

    assert_eq!(config.name, "membrane-relay");
    assert_eq!(config.domain.as_deref(), Some("membrane.primals.eco"));
    assert_eq!(config.composition, MembraneComposition::Tower);
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
    assert!(!signal.enabled);

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

// --- Composition model ---

#[test]
fn composition_ladder_ordering() {
    assert!(MembraneComposition::Relay < MembraneComposition::RustDesk);
    assert!(MembraneComposition::RustDesk < MembraneComposition::Tower);
    assert!(MembraneComposition::Tower < MembraneComposition::Nest);
}

#[test]
fn composition_btsp_requirements() {
    assert!(!MembraneComposition::Relay.has_btsp());
    assert!(!MembraneComposition::RustDesk.has_btsp());
    assert!(MembraneComposition::Tower.has_btsp());
    assert!(MembraneComposition::Nest.has_btsp());
}

#[test]
fn tower_composition_spec() {
    let spec = MembraneComposition::Tower.spec();
    assert_eq!(spec.primals, vec!["beardog", "songbird", "skunkbat"]);
    assert_eq!(spec.symbiotic, vec!["hbbs", "hbbr"]);
    assert!(spec.boot_order[0] == "beardog");
}

#[test]
fn nest_composition_includes_all_tower_primals() {
    let tower = MembraneComposition::Tower.spec();
    let nest = MembraneComposition::Nest.spec();
    for primal in &tower.primals {
        assert!(
            nest.primals.contains(primal),
            "Nest should include Tower primal: {primal}"
        );
    }
}

#[test]
fn nest_composition_adds_storage_primals() {
    let spec = MembraneComposition::Nest.spec();
    assert!(spec.primals.contains(&"nestgate"));
    assert!(spec.primals.contains(&"rhizocrypt"));
    assert!(spec.primals.contains(&"loamspine"));
    assert!(spec.primals.contains(&"sweetgrass"));
    assert!(spec.symbiotic.contains(&"caddy"));
}

// --- Channel types ---

#[test]
fn channel_trust_ordering() {
    assert!(MembraneChannel::Signal.trust_level() < MembraneChannel::Relay.trust_level());
    assert!(MembraneChannel::Relay.trust_level() < MembraneChannel::Surface.trust_level());
}

#[test]
fn channel_default_ports() {
    assert_eq!(MembraneChannel::Signal.default_ports(), &[53]);
    assert_eq!(MembraneChannel::Relay.default_ports(), &[3478]);
    assert_eq!(MembraneChannel::Surface.default_ports(), &[80, 443]);
}

#[test]
fn channel_crypto_layers() {
    assert_eq!(MembraneChannel::Signal.default_crypto(), CryptoLayer::None);
    assert_eq!(
        MembraneChannel::Relay.default_crypto(),
        CryptoLayer::TurnHmac
    );
    assert_eq!(MembraneChannel::Surface.default_crypto(), CryptoLayer::Tls);
}

// --- Service definitions ---

#[test]
fn service_lookup_all_primals() {
    for name in ["beardog", "songbird", "skunkbat", "nestgate", "rhizocrypt", "loamspine", "sweetgrass"] {
        assert!(
            MembraneService::for_binary(name).is_some(),
            "Service not found for primal: {name}"
        );
    }
}

#[test]
fn service_lookup_symbiotic() {
    for name in ["hbbs", "hbbr", "caddy"] {
        let svc = MembraneService::for_binary(name).expect(&format!("Service not found: {name}"));
        assert!(!svc.is_primal, "{name} should not be marked as primal");
    }
}

#[test]
fn beardog_is_uds_only() {
    let svc = MembraneService::for_binary("beardog").unwrap();
    assert!(svc.socket_path.is_some());
    assert!(svc.port.is_none());
    assert!(!svc.is_externally_reachable());
}

#[test]
fn skunkbat_is_loopback_only() {
    let svc = MembraneService::for_binary("skunkbat").unwrap();
    assert_eq!(svc.bind, "127.0.0.1");
    assert!(!svc.is_externally_reachable());
}

#[test]
fn songbird_is_externally_reachable() {
    let svc = MembraneService::for_binary("songbird").unwrap();
    assert!(svc.is_externally_reachable());
    assert_eq!(svc.port, Some(3478));
}

// --- Firewall derivation ---

#[test]
fn relay_firewall_minimal() {
    let fw = FirewallRuleset::for_composition(MembraneComposition::Relay);
    let ports = fw.ports();
    assert!(ports.contains(&22), "SSH must always be open");
    assert!(ports.contains(&3478), "TURN must be open");
    assert!(!ports.contains(&21115), "RustDesk should not be open");
    assert!(!ports.contains(&443), "TLS should not be open");
}

#[test]
fn tower_firewall_includes_rustdesk() {
    let fw = FirewallRuleset::for_composition(MembraneComposition::Tower);
    let ports = fw.ports();
    assert!(ports.contains(&3478));
    assert!(ports.contains(&21115));
    assert!(ports.contains(&21116));
    assert!(ports.contains(&21117));
}

#[test]
fn nest_firewall_includes_surface() {
    let fw = FirewallRuleset::for_composition(MembraneComposition::Nest);
    let ports = fw.ports();
    assert!(ports.contains(&80), "ACME port should be open");
    assert!(ports.contains(&443), "TLS port should be open");
    assert!(ports.contains(&9500), "NestGate should be open");
}

#[test]
fn firewall_ufw_script_format() {
    let fw = FirewallRuleset::for_composition(MembraneComposition::Relay);
    let script = fw.to_ufw_script();
    assert!(script.contains("ufw --force reset"));
    assert!(script.contains("ufw default deny incoming"));
    assert!(script.contains("ufw allow 22/tcp"));
    assert!(script.contains("ufw allow 3478/tcp"));
    assert!(script.contains("ufw --force enable"));
}

#[test]
fn firewall_rules_are_sorted() {
    for comp in MembraneComposition::all() {
        let fw = FirewallRuleset::for_composition(*comp);
        let ports: Vec<u16> = fw.rules.iter().map(|r| r.port).collect();
        for window in ports.windows(2) {
            assert!(
                window[0] <= window[1],
                "Firewall rules not sorted for {comp}: {} > {}",
                window[0],
                window[1]
            );
        }
    }
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

// --- Validation ---

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

// --- Credential model ---

#[test]
fn credential_model_defaults_to_age() {
    assert_eq!(CredentialModel::default(), CredentialModel::Age);
}

// --- Gap closure: journald persistence (MEM-07) ---

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

// --- Gap closure: credential file paths (MEM-08, MEM-12) ---

#[test]
fn relay_credential_files_include_turn() {
    let files = cellmembrane_types::credentials::credential_files_for(MembraneComposition::Relay);
    assert!(
        files.iter().any(|f| f.path.contains("songbird") || f.path.contains("relay-credentials")),
        "Relay must have TURN credential files"
    );
    for f in &files {
        assert_eq!(f.expected_owner, "root");
    }
}

#[test]
fn rustdesk_credential_files_include_key() {
    let files = cellmembrane_types::credentials::credential_files_for(MembraneComposition::RustDesk);
    assert!(
        files.iter().any(|f| f.path.contains("id_ed25519.pub")),
        "RustDesk must have public key file"
    );
    assert!(
        files.iter().any(|f| f.path.contains("id_ed25519") && !f.path.contains(".pub")),
        "RustDesk must have private key file"
    );
}

#[test]
fn tower_credential_files_include_tower_env() {
    let files = cellmembrane_types::credentials::credential_files_for(MembraneComposition::Tower);
    let tower_env = files.iter().find(|f| f.path.contains("tower.env"));
    assert!(tower_env.is_some(), "Tower must have tower.env");
    assert_eq!(tower_env.unwrap().expected_mode, "600");
}

#[test]
fn credential_files_grow_with_composition() {
    let relay = cellmembrane_types::credentials::credential_files_for(MembraneComposition::Relay);
    let rustdesk = cellmembrane_types::credentials::credential_files_for(MembraneComposition::RustDesk);
    let tower = cellmembrane_types::credentials::credential_files_for(MembraneComposition::Tower);
    assert!(rustdesk.len() > relay.len(), "RustDesk should have more credential files than Relay");
    assert!(tower.len() > rustdesk.len(), "Tower should have more credential files than RustDesk");
}

// --- Gap closure: binary integrity (MEM-09) ---

#[test]
fn binary_integrity_relay_has_songbird() {
    let bins = cellmembrane_types::service::binary_integrity_for(MembraneComposition::Relay);
    assert!(
        bins.iter().any(|b| b.binary == "songbird"),
        "Relay must verify songbird binary"
    );
    let songbird = bins.iter().find(|b| b.binary == "songbird").unwrap();
    assert_eq!(songbird.hash_algorithm, cellmembrane_types::service::HashAlgorithm::Blake3);
    assert!(songbird.require_static_musl);
}

#[test]
fn binary_integrity_tower_has_all_primals() {
    let bins = cellmembrane_types::service::binary_integrity_for(MembraneComposition::Tower);
    for primal in ["beardog", "songbird", "skunkbat"] {
        assert!(
            bins.iter().any(|b| b.binary == primal),
            "Tower must verify {primal}"
        );
    }
}

#[test]
fn binary_integrity_symbiotic_use_sha256() {
    let bins = cellmembrane_types::service::binary_integrity_for(MembraneComposition::Tower);
    for sym in ["hbbs", "hbbr"] {
        let entry = bins.iter().find(|b| b.binary == sym);
        assert!(entry.is_some(), "Tower must verify {sym}");
        assert_eq!(
            entry.unwrap().hash_algorithm,
            cellmembrane_types::service::HashAlgorithm::Sha256,
            "Symbiotic {sym} should use SHA-256"
        );
        assert!(!entry.unwrap().require_static_musl);
    }
}

#[test]
fn binary_integrity_grows_with_composition() {
    let relay = cellmembrane_types::service::binary_integrity_for(MembraneComposition::Relay);
    let tower = cellmembrane_types::service::binary_integrity_for(MembraneComposition::Tower);
    let nest = cellmembrane_types::service::binary_integrity_for(MembraneComposition::Nest);
    assert!(tower.len() > relay.len());
    assert!(nest.len() > tower.len());
}

// --- Gap closure: telemetry config (s_membrane_composition Pillar 4) ---

#[test]
fn telemetry_defaults_match_glacial_standard() {
    let config: cellmembrane_types::config::MembraneConfigFile = toml::from_str(r#"
        [membrane]
        name = "test"
        composition = "relay"
    "#).unwrap();
    let t = &config.membrane.telemetry;
    assert!(t.enabled);
    assert_eq!(t.shadow_mode, "permanent");
    assert!(t.cutover_gate_days >= 7);
}

#[test]
fn telemetry_parsed_from_reference_toml() {
    let config = MembraneConfig::load(Path::new("../../membrane.toml")).unwrap();
    assert!(config.telemetry.enabled);
    assert_eq!(config.telemetry.shadow_mode, "permanent");
    assert_eq!(config.telemetry.cutover_gate_days, 7);
    assert!(config.telemetry.skunkbat_correlation);
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

// --- Gap closure: validation report includes new audit categories ---

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

// --- Round-trip serde ---

#[test]
fn composition_serde_roundtrip() {
    #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
    struct Wrapper {
        c: MembraneComposition,
    }
    for comp in MembraneComposition::all() {
        let w = Wrapper { c: *comp };
        let serialized = toml::to_string(&w).unwrap();
        let deserialized: Wrapper = toml::from_str(&serialized).unwrap();
        assert_eq!(w, deserialized);
    }
}

#[test]
fn channel_serde_roundtrip() {
    #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
    struct Wrapper {
        c: MembraneChannel,
    }
    for ch in MembraneChannel::all() {
        let w = Wrapper { c: *ch };
        let serialized = toml::to_string(&w).unwrap();
        let deserialized: Wrapper = toml::from_str(&serialized).unwrap();
        assert_eq!(w, deserialized);
    }
}
