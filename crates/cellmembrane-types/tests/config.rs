// SPDX-License-Identifier: AGPL-3.0-or-later

use cellmembrane_types::config::ShadowMode;
use cellmembrane_types::validation::Severity;

#[test]
fn shadow_mode_display() {
    assert_eq!(format!("{}", ShadowMode::Permanent), "permanent");
    assert_eq!(format!("{}", ShadowMode::Cutover), "cutover");
    assert_eq!(format!("{}", ShadowMode::Disabled), "disabled");
}

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
