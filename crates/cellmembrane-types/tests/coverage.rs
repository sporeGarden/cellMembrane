// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Coverage-expansion tests for modules below 90% line coverage:
// identity.rs, channels.rs, provider.rs, validation.rs, composition.rs

use cellmembrane_types::channels::{ChannelConfig, CryptoLayer, MembraneChannel, TrustLevel};
use cellmembrane_types::composition::MembraneComposition;
use cellmembrane_types::config::ShadowMode;
use cellmembrane_types::envelope::{BraidPolicy, EnvelopeLayer, EnvelopeTopology};
use cellmembrane_types::firewall::{FirewallProtocol, FirewallRule};
use cellmembrane_types::identity::{GateMobility, MembraneIdentity};
use cellmembrane_types::provider::{ProviderConfig, ProviderType, SubstrateProfile};
use cellmembrane_types::service::{HealthCheckMethod, MembraneService, Protocol, TransportMode};
use cellmembrane_types::validation::{Report, Severity};

// === identity.rs (was 0% coverage) ===

#[test]
fn identity_gate_id_returns_explicit() {
    let id: MembraneIdentity = toml::from_str(
        r#"
        family_id = "alpha"
        gate_id = "nyc-01"
        "#,
    )
    .unwrap();
    assert_eq!(id.gate_id_or_default(), "nyc-01");
}

#[test]
fn identity_gate_id_generates_default() {
    let id: MembraneIdentity = toml::from_str(
        r#"
        family_id = "membrane-alpha"
        "#,
    )
    .unwrap();
    assert_eq!(id.gate_id_or_default(), "membrane-alpha-membrane");
}

#[test]
fn identity_serde_roundtrip() {
    let id: MembraneIdentity = toml::from_str(
        r#"
        family_id = "eco-01"
        gate_id = "west-gate"
        "#,
    )
    .unwrap();
    let serialized = toml::to_string(&id).unwrap();
    let deserialized: MembraneIdentity = toml::from_str(&serialized).unwrap();
    assert_eq!(deserialized.family_id, "eco-01");
    assert_eq!(deserialized.gate_id.as_deref(), Some("west-gate"));
}

#[test]
fn identity_extra_fields_preserved() {
    let id: MembraneIdentity = toml::from_str(
        r#"
        family_id = "test"
        custom_field = "value"
        "#,
    )
    .unwrap();
    assert!(id.extra.contains_key("custom_field"));
}

#[test]
fn identity_mobility_default_is_fixed() {
    let id: MembraneIdentity = toml::from_str(
        r#"
        family_id = "eco"
        "#,
    )
    .unwrap();
    assert_eq!(id.mobility, GateMobility::Fixed);
}

#[test]
fn identity_mobility_mobile_parse() {
    let id: MembraneIdentity = toml::from_str(
        r#"
        family_id = "eco"
        gate_id = "golgiAlpha"
        mobility = "mobile"
        "#,
    )
    .unwrap();
    assert_eq!(id.mobility, GateMobility::Mobile);
    assert!(id.mobility.needs_reconnect_hook());
    assert!(!id.mobility.is_mesh_anchor());
}

#[test]
fn identity_mobility_fixed_attributes() {
    assert!(!GateMobility::Fixed.needs_reconnect_hook());
    assert!(GateMobility::Fixed.is_mesh_anchor());
    assert_eq!(GateMobility::Fixed.to_string(), "fixed");
    assert_eq!(GateMobility::Mobile.to_string(), "mobile");
}

// === channels.rs (was 37% coverage) ===

#[test]
fn channel_display_all_variants() {
    assert_eq!(format!("{}", MembraneChannel::Signal), "signal");
    assert_eq!(format!("{}", MembraneChannel::Relay), "relay");
    assert_eq!(format!("{}", MembraneChannel::Surface), "surface");
}

#[test]
fn trust_level_display() {
    assert_eq!(format!("{}", TrustLevel::Public), "public");
    assert_eq!(format!("{}", TrustLevel::Medium), "medium");
    assert_eq!(format!("{}", TrustLevel::High), "high");
}

#[test]
fn crypto_layer_display() {
    assert_eq!(format!("{}", CryptoLayer::None), "none");
    assert_eq!(format!("{}", CryptoLayer::Dnssec), "dnssec");
    assert_eq!(format!("{}", CryptoLayer::TurnHmac), "turn_hmac");
    assert_eq!(format!("{}", CryptoLayer::Tls), "tls");
}

#[test]
fn channel_config_default() {
    let cfg = ChannelConfig::default();
    assert!(cfg.enabled);
    assert!(cfg.port.is_none());
    assert!(cfg.primal.is_none());
    assert!(cfg.dnssec.is_none());
    assert!(cfg.tls_domain.is_none());
    assert!(cfg.acme_email.is_none());
    assert!(cfg.extra.is_empty());
}

#[test]
fn channel_config_serde() {
    let toml_str = r#"
        enabled = false
        port = 5353
        primal = "custom-dns"
        dnssec = true
    "#;
    let cfg: ChannelConfig = toml::from_str(toml_str).unwrap();
    assert!(!cfg.enabled);
    assert_eq!(cfg.port, Some(5353));
    assert_eq!(cfg.primal.as_deref(), Some("custom-dns"));
    assert_eq!(cfg.dnssec, Some(true));
}

#[test]
fn channel_all_returns_three() {
    assert_eq!(MembraneChannel::all().len(), 3);
}

#[test]
fn channel_default_primal_names() {
    assert_eq!(MembraneChannel::Signal.default_primal(), "knot-dns");
    assert_eq!(MembraneChannel::Relay.default_primal(), "songbird");
    assert_eq!(MembraneChannel::Surface.default_primal(), "caddy");
}

// === provider.rs (was 38% coverage) ===

#[test]
fn provider_type_display_all() {
    assert_eq!(format!("{}", ProviderType::DigitalOcean), "digitalocean");
    assert_eq!(format!("{}", ProviderType::Hetzner), "hetzner");
    assert_eq!(format!("{}", ProviderType::BareMetal), "bare_metal");
    assert_eq!(format!("{}", ProviderType::GateLocal), "gate_local");
    assert_eq!(format!("{}", ProviderType::Custom), "custom");
}

#[test]
fn provider_ssh_defaults() {
    let cfg: ProviderConfig = toml::from_str(r#"type = "bare_metal""#).unwrap();
    assert_eq!(cfg.ssh_user_or_default(), "root");
    assert_eq!(cfg.ssh_port_or_default(), 22);
}

#[test]
fn provider_ssh_overrides() {
    let cfg: ProviderConfig = toml::from_str(
        r#"
        type = "bare_metal"
        ssh_user = "deploy"
        ssh_port = 2222
        "#,
    )
    .unwrap();
    assert_eq!(cfg.ssh_user_or_default(), "deploy");
    assert_eq!(cfg.ssh_port_or_default(), 2222);
}

#[test]
fn provider_requires_ssh() {
    let do_cfg: ProviderConfig = toml::from_str(r#"type = "digitalocean""#).unwrap();
    let gate_cfg: ProviderConfig = toml::from_str(r#"type = "gate_local""#).unwrap();
    let bare_cfg: ProviderConfig = toml::from_str(r#"type = "bare_metal""#).unwrap();
    assert!(do_cfg.requires_ssh());
    assert!(!gate_cfg.requires_ssh());
    assert!(bare_cfg.requires_ssh());
}

#[test]
fn provider_supports_provisioning() {
    let do_cfg: ProviderConfig = toml::from_str(r#"type = "digitalocean""#).unwrap();
    let hz_cfg: ProviderConfig = toml::from_str(r#"type = "hetzner""#).unwrap();
    let bare_cfg: ProviderConfig = toml::from_str(r#"type = "bare_metal""#).unwrap();
    let custom: ProviderConfig = toml::from_str(r#"type = "custom""#).unwrap();
    assert!(do_cfg.supports_provisioning());
    assert!(hz_cfg.supports_provisioning());
    assert!(!bare_cfg.supports_provisioning());
    assert!(!custom.supports_provisioning());
}

#[test]
fn substrate_profile_all_variants() {
    let do_cfg: ProviderConfig = toml::from_str(r#"type = "digitalocean""#).unwrap();
    let bare_cfg: ProviderConfig = toml::from_str(r#"type = "bare_metal""#).unwrap();
    let gate_cfg: ProviderConfig = toml::from_str(r#"type = "gate_local""#).unwrap();
    assert_eq!(do_cfg.substrate_profile(), SubstrateProfile::VpsFieldMouse);
    assert_eq!(
        bare_cfg.substrate_profile(),
        SubstrateProfile::RemoteCovalent
    );
    assert_eq!(gate_cfg.substrate_profile(), SubstrateProfile::GateLocal);
}

#[test]
fn substrate_profile_display() {
    assert_eq!(
        format!("{}", SubstrateProfile::VpsFieldMouse),
        "vps_fieldmouse"
    );
    assert_eq!(
        format!("{}", SubstrateProfile::RemoteCovalent),
        "remote_covalent"
    );
    assert_eq!(format!("{}", SubstrateProfile::GateLocal), "gate_local");
}

#[test]
fn substrate_biomeos_and_hardening() {
    assert!(!SubstrateProfile::VpsFieldMouse.has_biomeos());
    assert!(SubstrateProfile::VpsFieldMouse.requires_full_hardening());
    assert!(SubstrateProfile::GateLocal.has_biomeos());
    assert!(!SubstrateProfile::GateLocal.requires_full_hardening());
    assert!(!SubstrateProfile::RemoteCovalent.has_biomeos());
    assert!(!SubstrateProfile::RemoteCovalent.requires_full_hardening());
}

#[test]
fn provider_extra_fields() {
    let cfg: ProviderConfig = toml::from_str(
        r#"
        type = "digitalocean"
        region = "nyc1"
        size = "s-1vcpu-2gb"
        image = "debian-12-x64"
        custom_tag = "test"
        "#,
    )
    .unwrap();
    assert_eq!(cfg.region.as_deref(), Some("nyc1"));
    assert_eq!(cfg.size.as_deref(), Some("s-1vcpu-2gb"));
    assert_eq!(cfg.image.as_deref(), Some("debian-12-x64"));
    assert!(cfg.extra.contains_key("custom_tag"));
}

// === validation.rs (was 59% coverage) ===

#[test]
fn report_display_format() {
    let mut report = Report::new();
    report.pass("test.check", "passed");
    report.fail("test.fail", "something broke");
    let output = format!("{report}");
    assert!(output.contains("[PASS] test.check: passed"));
    assert!(output.contains("[FAIL] test.fail: something broke"));
    assert!(output.contains("--- 1 passed, 1 failed, 0 warnings"));
}

#[test]
fn report_total_checks() {
    let mut report = Report::new();
    report.pass("a", "ok");
    report.pass("b", "ok");
    report.fail("c", "fail");
    report.warn("d", "maybe");
    report.info("e", "fyi");
    assert_eq!(report.total_checks(), 3);
}

#[test]
fn report_merge() {
    let mut r1 = Report::new();
    r1.pass("a", "ok");
    let mut r2 = Report::new();
    r2.fail("b", "not ok");
    r2.warn("c", "meh");
    r1.merge(r2);
    assert_eq!(r1.entries.len(), 3);
    assert!(!r1.is_ok());
}

#[test]
fn report_summary() {
    let mut report = Report::new();
    report.pass("a", "ok");
    report.pass("b", "ok");
    report.fail("c", "nope");
    report.warn("d", "hmm");
    assert_eq!(report.summary(), "2 passed, 1 failed, 1 warnings");
}

#[test]
fn severity_display() {
    assert_eq!(format!("{}", Severity::Info), "INFO");
    assert_eq!(format!("{}", Severity::Warn), "WARN");
    assert_eq!(format!("{}", Severity::Fail), "FAIL");
    assert_eq!(format!("{}", Severity::Pass), "PASS");
}

#[test]
fn report_entry_display() {
    let entry = cellmembrane_types::validation::ReportEntry {
        severity: Severity::Warn,
        check: "net.port".to_string(),
        message: "port conflict detected".to_string(),
    };
    assert_eq!(
        format!("{entry}"),
        "[WARN] net.port: port conflict detected"
    );
}

#[test]
fn report_count_empty() {
    let report = Report::new();
    assert_eq!(report.count(Severity::Pass), 0);
    assert_eq!(report.count(Severity::Fail), 0);
    assert!(report.is_ok());
}

// === composition.rs additional coverage ===

#[test]
fn composition_display() {
    assert_eq!(format!("{}", MembraneComposition::Relay), "relay");
    assert_eq!(format!("{}", MembraneComposition::RustDesk), "rustdesk");
    assert_eq!(format!("{}", MembraneComposition::Tower), "tower");
    assert_eq!(format!("{}", MembraneComposition::Nest), "nest");
}

#[test]
fn composition_active_channels() {
    let relay_ch = MembraneComposition::Relay.active_channels();
    assert_eq!(relay_ch.len(), 1);
    assert_eq!(relay_ch[0], MembraneChannel::Relay);

    let nest_ch = MembraneComposition::Nest.active_channels();
    assert_eq!(nest_ch.len(), 3);
}

#[test]
fn composition_spec_all_binaries() {
    let spec = MembraneComposition::Nest.spec();
    let all = spec.all_binaries();
    assert!(all.contains(&"beardog"));
    assert!(all.contains(&"caddy"));
}

#[test]
fn composition_spec_all_ports() {
    let spec = MembraneComposition::Relay.spec();
    let ports = spec.all_ports();
    assert!(ports.contains(&22));
    assert!(ports.contains(&3478));
}

#[test]
fn composition_spec_service_for() {
    let spec = MembraneComposition::Tower.spec();
    assert!(spec.service_for("beardog").is_some());
    assert!(spec.service_for("nonexistent").is_none());
}

// === service.rs additional coverage ===

#[test]
fn protocol_display() {
    assert_eq!(format!("{}", Protocol::Tcp), "tcp");
    assert_eq!(format!("{}", Protocol::Udp), "udp");
    assert_eq!(format!("{}", Protocol::TcpAndUdp), "tcp+udp");
    assert_eq!(format!("{}", Protocol::Uds), "uds");
}

#[test]
fn transport_mode_display() {
    assert_eq!(format!("{}", TransportMode::UdsOnly), "uds_only");
    assert_eq!(format!("{}", TransportMode::TcpDefault), "tcp_default");
    assert_eq!(format!("{}", TransportMode::TcpOptIn), "tcp_opt_in");
}

#[test]
fn health_check_display() {
    assert_eq!(
        format!("{}", HealthCheckMethod::Liveness),
        "health.liveness"
    );
    assert_eq!(format!("{}", HealthCheckMethod::TcpConnect), "tcp_connect");
    assert_eq!(format!("{}", HealthCheckMethod::HttpsProbe), "https_probe");
    assert_eq!(format!("{}", HealthCheckMethod::DnsProbe), "dns_probe");
    assert_eq!(
        format!("{}", HealthCheckMethod::SocketExists),
        "socket_exists"
    );
}

#[test]
fn service_all_returns_seventeen() {
    assert_eq!(MembraneService::all().len(), 17);
}

#[test]
fn service_unknown_binary_returns_none() {
    assert!(MembraneService::for_binary("nonexistent").is_none());
}

// === firewall.rs additional coverage ===

#[test]
fn firewall_rule_display() {
    let rule = FirewallRule {
        port: 443,
        protocol: FirewallProtocol::Tcp,
        comment: "HTTPS",
    };
    assert_eq!(format!("{rule}"), "443/tcp (HTTPS)");
}

#[test]
fn firewall_rule_ufw_command() {
    let rule = FirewallRule {
        port: 3478,
        protocol: FirewallProtocol::Udp,
        comment: "TURN",
    };
    assert_eq!(rule.to_ufw_command(), "ufw allow 3478/udp comment 'TURN'");
}

#[test]
fn firewall_protocol_display() {
    assert_eq!(format!("{}", FirewallProtocol::Tcp), "tcp");
    assert_eq!(format!("{}", FirewallProtocol::Udp), "udp");
}

#[test]
fn firewall_protocol_ordering() {
    assert!(FirewallProtocol::Tcp < FirewallProtocol::Udp);
}

// === config.rs additional coverage ===

#[test]
fn shadow_mode_display() {
    assert_eq!(format!("{}", ShadowMode::Permanent), "permanent");
    assert_eq!(format!("{}", ShadowMode::Cutover), "cutover");
    assert_eq!(format!("{}", ShadowMode::Disabled), "disabled");
}

// === envelope.rs additional coverage ===

#[test]
fn envelope_topology_display() {
    assert_eq!(format!("{}", EnvelopeTopology::Monoderm), "monoderm");
    assert_eq!(format!("{}", EnvelopeTopology::Diderm), "diderm");
}

#[test]
fn envelope_layer_display() {
    assert_eq!(format!("{}", EnvelopeLayer::Cytoplasm), "cytoplasm");
    assert_eq!(
        format!("{}", EnvelopeLayer::PlasmaMembrane),
        "plasma_membrane"
    );
    assert_eq!(format!("{}", EnvelopeLayer::Periplasm), "periplasm");
    assert_eq!(
        format!("{}", EnvelopeLayer::OuterMembrane),
        "outer_membrane"
    );
    assert_eq!(format!("{}", EnvelopeLayer::Extracellular), "extracellular");
}

#[test]
fn braid_policy_display() {
    assert_eq!(format!("{}", BraidPolicy::PassThrough), "pass_through");
    assert_eq!(format!("{}", BraidPolicy::Verify), "verify");
    assert_eq!(format!("{}", BraidPolicy::Block), "block");
}

// === credentials.rs additional coverage ===

#[test]
fn credential_model_display() {
    use cellmembrane_types::credentials::CredentialModel;
    assert_eq!(format!("{}", CredentialModel::Age), "age");
    assert_eq!(format!("{}", CredentialModel::BtspVault), "btsp_vault");
    assert_eq!(format!("{}", CredentialModel::Manual), "manual");
}

// === DeployPaths (configurable path evolution) ===

#[test]
fn deploy_paths_defaults() {
    use cellmembrane_types::config::DeployPaths;
    let paths = DeployPaths::default();
    assert_eq!(paths.install_base, "/opt/membrane");
    assert_eq!(paths.socket_base, "/run/membrane");
    assert_eq!(paths.credential_base, "/opt/membrane");
}

#[test]
fn deploy_paths_resolve_install() {
    use cellmembrane_types::config::DeployPaths;
    let paths = DeployPaths::default();
    assert_eq!(paths.install_path("beardog"), "/opt/membrane/beardog");
    assert_eq!(paths.install_path("songbird"), "/opt/membrane/songbird");
}

#[test]
fn deploy_paths_resolve_socket() {
    use cellmembrane_types::config::DeployPaths;
    let paths = DeployPaths::default();
    assert_eq!(paths.socket_path("beardog"), "/run/membrane/beardog.sock");
}

#[test]
fn deploy_paths_custom_base() {
    use cellmembrane_types::config::DeployPaths;
    let toml_str = r#"
        install_base = "/usr/local/primals"
        socket_base = "/var/run/eco"
        credential_base = "/etc/eco/creds"
    "#;
    let paths: DeployPaths = toml::from_str(toml_str).unwrap();
    assert_eq!(
        paths.install_path("songbird"),
        "/usr/local/primals/songbird"
    );
    assert_eq!(paths.socket_path("skunkbat"), "/var/run/eco/skunkbat.sock");
    assert_eq!(paths.credential_base, "/etc/eco/creds");
}

#[test]
fn deploy_paths_transport_env_default_uds() {
    use cellmembrane_types::TransportEndpoint;
    use cellmembrane_types::config::DeployPaths;
    let paths = DeployPaths::default();
    let val = paths.transport_env_value("beardog");
    let ep: TransportEndpoint = serde_json::from_str(&val).unwrap();
    assert_eq!(
        ep,
        TransportEndpoint::Uds {
            path: "/run/membrane/beardog.sock".into()
        }
    );
}

#[test]
fn deploy_paths_transport_env_custom_override() {
    use cellmembrane_types::TransportEndpoint;
    use cellmembrane_types::config::DeployPaths;
    let toml_str = r#"
        install_base = "/opt/membrane"
        socket_base = "/run/membrane"
        credential_base = "/opt/membrane"
        [transport_endpoint]
        transport = "tcp"
        host = "10.0.0.5"
        port = 9443
    "#;
    let paths: DeployPaths = toml::from_str(toml_str).unwrap();
    let val = paths.transport_env_value("anything");
    let ep: TransportEndpoint = serde_json::from_str(&val).unwrap();
    assert_eq!(
        ep,
        TransportEndpoint::Tcp {
            host: "10.0.0.5".into(),
            port: 9443
        }
    );
}

#[test]
fn config_with_custom_paths() {
    use cellmembrane_types::config::MembraneConfig;
    use std::path::Path;
    let config = MembraneConfig::load(Path::new("../../membrane.toml")).unwrap();
    assert_eq!(config.paths.install_base, "/opt/membrane");
    assert_eq!(config.paths.socket_base, "/run/membrane");
}

// === Config error handling (typed errors) ===

#[test]
fn config_load_nonexistent_returns_read_error() {
    use cellmembrane_types::config::MembraneConfig;
    use cellmembrane_types::error::ConfigError;
    use std::path::Path;
    let result = MembraneConfig::load(Path::new("/nonexistent/membrane.toml"));
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(matches!(err, ConfigError::Read { .. }));
    let msg = format!("{err}");
    assert!(msg.contains("/nonexistent/membrane.toml"));
}

#[test]
fn config_load_invalid_toml_returns_parse_error() {
    use cellmembrane_types::config::MembraneConfig;
    use cellmembrane_types::error::ConfigError;
    use std::io::Write;
    let dir = std::env::temp_dir();
    let path = dir.join("cellmembrane_test_bad.toml");
    {
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "[membrane]\nname = ").unwrap();
    }
    let result = MembraneConfig::load(&path);
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), ConfigError::Parse { .. }));
    let _ = std::fs::remove_file(&path);
}

// === Config validation branches (coverage push) ===

#[test]
fn validate_nest_without_domain_warns() {
    let toml = r#"
        [membrane]
        name = "test"
        composition = "nest"
        [membrane.identity]
        family_id = "test-family"
    "#;
    let file: cellmembrane_types::config::MembraneConfigFile = toml::from_str(toml).unwrap();
    let report = file.membrane.validate();
    assert!(
        report
            .entries
            .iter()
            .any(|e| e.check == "surface.domain" && e.severity == Severity::Warn),
        "Nest without domain should warn"
    );
}

#[test]
fn validate_signal_without_dnssec_warns() {
    let toml = r#"
        [membrane]
        name = "test"
        composition = "nest"
        [membrane.identity]
        family_id = "test-family"
        [membrane.channels.signal]
        enabled = true
    "#;
    let file: cellmembrane_types::config::MembraneConfigFile = toml::from_str(toml).unwrap();
    let report = file.membrane.validate();
    assert!(
        report
            .entries
            .iter()
            .any(|e| e.check == "channel.signal" && e.severity == Severity::Warn),
        "Signal without DNSSEC should warn"
    );
}

#[test]
fn validate_signal_with_dnssec_passes() {
    let toml = r#"
        [membrane]
        name = "test"
        composition = "nest"
        [membrane.identity]
        family_id = "test-family"
        [membrane.channels.signal]
        enabled = true
        dnssec = true
    "#;
    let file: cellmembrane_types::config::MembraneConfigFile = toml::from_str(toml).unwrap();
    let report = file.membrane.validate();
    assert!(
        report
            .entries
            .iter()
            .any(|e| e.check == "channel.signal" && e.severity == Severity::Pass),
        "Signal with DNSSEC should pass"
    );
}

#[test]
fn validate_relay_port_override_info() {
    let toml = r#"
        [membrane]
        name = "test"
        composition = "relay"
        [membrane.channels.relay]
        enabled = true
        port = 9999
    "#;
    let file: cellmembrane_types::config::MembraneConfigFile = toml::from_str(toml).unwrap();
    let report = file.membrane.validate();
    assert!(
        report
            .entries
            .iter()
            .any(|e| e.check == "channel.relay" && e.message.contains("9999")),
        "Relay port override should be reported"
    );
}

#[test]
fn validate_telemetry_disabled_warns() {
    let toml = r#"
        [membrane]
        name = "test"
        composition = "relay"
        [membrane.telemetry]
        enabled = false
    "#;
    let file: cellmembrane_types::config::MembraneConfigFile = toml::from_str(toml).unwrap();
    let report = file.membrane.validate();
    assert!(
        report
            .entries
            .iter()
            .any(|e| e.check == "telemetry.enabled" && e.severity == Severity::Warn),
    );
}

#[test]
fn validate_journald_disabled_warns() {
    let toml = r#"
        [membrane]
        name = "test"
        composition = "relay"
        [membrane.hardening]
        disabled_steps = ["journald_persistent"]
    "#;
    let file: cellmembrane_types::config::MembraneConfigFile = toml::from_str(toml).unwrap();
    let report = file.membrane.validate();
    assert!(
        report
            .entries
            .iter()
            .any(|e| e.check == "hardening.journald" && e.severity == Severity::Warn),
    );
}

#[test]
fn validate_bare_metal_without_host_fails() {
    let toml = r#"
        [membrane]
        name = "test"
        composition = "relay"
        [membrane.provider]
        type = "bare_metal"
    "#;
    let file: cellmembrane_types::config::MembraneConfigFile = toml::from_str(toml).unwrap();
    let report = file.membrane.validate();
    assert!(
        report
            .entries
            .iter()
            .any(|e| e.check == "provider.host" && e.severity == Severity::Fail),
        "Bare metal without host should fail"
    );
}

#[test]
fn validate_no_provider_warns() {
    let toml = r#"
        [membrane]
        name = "test"
        composition = "relay"
    "#;
    let file: cellmembrane_types::config::MembraneConfigFile = toml::from_str(toml).unwrap();
    let report = file.membrane.validate();
    assert!(
        report
            .entries
            .iter()
            .any(|e| e.check == "provider.missing" && e.severity == Severity::Warn),
    );
}

// === Iterator evolution tests ===

#[test]
fn iter_binaries_matches_all_binaries() {
    let spec = MembraneComposition::Nest.spec();
    let collected: Vec<&str> = spec.iter_binaries().collect();
    assert_eq!(collected, spec.all_binaries());
}

#[test]
fn iter_binaries_is_lazy() {
    let spec = MembraneComposition::Tower.spec();
    let count = spec.iter_binaries().count();
    assert!(count > 0);
    assert!(spec.iter_binaries().any(|b| b == "beardog"));
}
