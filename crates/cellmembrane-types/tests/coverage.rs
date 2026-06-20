// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Coverage-expansion tests for domain modules.
// Split from the original monolithic file for 600L budget compliance.
// Identity → identity.rs, Provider → provider.rs, Validation → validation.rs,
// Config/Deploy/Validation-branches → config.rs

use cellmembrane_types::channels::{ChannelConfig, CryptoLayer, MembraneChannel, TrustLevel};
use cellmembrane_types::composition::MembraneComposition;
use cellmembrane_types::credentials::CredentialModel;
use cellmembrane_types::envelope::{BraidPolicy, EnvelopeLayer, EnvelopeTopology};
use cellmembrane_types::firewall::{FirewallProtocol, FirewallRule};
use cellmembrane_types::service::{HealthCheckMethod, MembraneService, Protocol, TransportMode};

// === channels.rs additional coverage ===

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
    assert_eq!(format!("{}", CredentialModel::Age), "age");
    assert_eq!(format!("{}", CredentialModel::BtspVault), "btsp_vault");
    assert_eq!(format!("{}", CredentialModel::Manual), "manual");
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
