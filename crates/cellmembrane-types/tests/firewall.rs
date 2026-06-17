// SPDX-License-Identifier: AGPL-3.0-or-later

use cellmembrane_types::composition::MembraneComposition;
use cellmembrane_types::firewall::{FirewallRuleset, NftablesConfig};

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

#[test]
fn nftables_standalone_has_flush_and_chains() {
    let fw = FirewallRuleset::for_composition(MembraneComposition::Relay);
    let script = fw.to_nftables_script(None);
    assert!(script.contains("flush ruleset"), "must flush");
    assert!(script.contains("table inet membrane"), "inet table");
    assert!(script.contains("chain input"), "input chain");
    assert!(script.contains("chain forward"), "forward chain");
    assert!(script.contains("chain output"), "output chain");
    assert!(script.contains("policy drop"), "default drop");
    assert!(script.contains("tcp dport 22 accept"), "SSH rule");
    assert!(!script.contains("masquerade"), "standalone must not NAT");
    assert!(
        !script.contains("membrane-nat"),
        "standalone must not have NAT table"
    );
}

fn sporegate_config() -> NftablesConfig {
    NftablesConfig {
        wan_interface: "enp1s0".into(),
        lan_interface: "eno1".into(),
        lan_subnet: "192.168.4.0/22".into(),
        gate_name: "sporeGate".into(),
        enable_nat: true,
        enable_dhcp: true,
        trust_lan_input: true,
        wireguard_interface: Some("wg0".into()),
        wireguard_port: 51820,
        drop_ipv6_forward: true,
    }
}

#[test]
fn nftables_plasma_membrane_has_nat() {
    let fw = FirewallRuleset::for_composition(MembraneComposition::Nucleus);
    let script = fw.to_nftables_script(Some(&sporegate_config()));
    assert!(script.contains("table ip membrane-nat"), "NAT table");
    assert!(script.contains("masquerade"), "masquerade rule");
    assert!(
        script.contains("oifname \"enp1s0\" masquerade"),
        "NAT on WAN interface"
    );
}

#[test]
fn nftables_plasma_membrane_trusts_lan() {
    let fw = FirewallRuleset::for_composition(MembraneComposition::Nucleus);
    let script = fw.to_nftables_script(Some(&sporegate_config()));
    assert!(
        script.contains("iifname \"eno1\" accept"),
        "LAN trusted input"
    );
}

#[test]
fn nftables_plasma_membrane_has_wireguard() {
    let fw = FirewallRuleset::for_composition(MembraneComposition::Nucleus);
    let script = fw.to_nftables_script(Some(&sporegate_config()));
    assert!(script.contains("udp dport 51820 accept"), "WireGuard port");
    assert!(script.contains("iifname \"wg0\" accept"), "WireGuard input");
    assert!(
        script.contains("iifname \"wg0\" accept comment \"WireGuard overlay forward\""),
        "WireGuard forward"
    );
}

#[test]
fn nftables_plasma_membrane_forwards_lan_to_wan() {
    let fw = FirewallRuleset::for_composition(MembraneComposition::Nucleus);
    let script = fw.to_nftables_script(Some(&sporegate_config()));
    assert!(
        script.contains("iifname \"eno1\" oifname \"enp1s0\" accept"),
        "LAN → WAN forward"
    );
    assert!(
        script.contains("iifname \"eno1\" oifname \"eno1\" accept"),
        "LAN → LAN inter-gate forward"
    );
}

#[test]
fn nftables_plasma_membrane_drops_ipv6_forward() {
    let fw = FirewallRuleset::for_composition(MembraneComposition::Nucleus);
    let script = fw.to_nftables_script(Some(&sporegate_config()));
    assert!(script.contains("table ip6 membrane-v6"), "IPv6 table");
}

#[test]
fn nftables_idempotent_script() {
    let fw = FirewallRuleset::for_composition(MembraneComposition::Nucleus);
    let script = fw.to_nftables_script(Some(&sporegate_config()));
    assert!(script.starts_with("#!/usr/sbin/nft -f"));
    assert!(script.contains("# Composition: nucleus"));
    assert!(script.contains("# Gate: sporeGate"));
}

#[test]
fn nftables_all_compositions_generate() {
    for comp in MembraneComposition::all() {
        let fw = FirewallRuleset::for_composition(*comp);
        let standalone = fw.to_nftables_script(None);
        assert!(
            standalone.contains("flush ruleset"),
            "standalone {comp} must flush"
        );
        let plasma = fw.to_nftables_script(Some(&sporegate_config()));
        assert!(plasma.contains("masquerade"), "plasma {comp} must NAT");
    }
}

// ── nftables edge cases ──────────────────────────────────────────────

#[test]
fn nftables_dhcp_visible_without_trust_lan() {
    let fw = FirewallRuleset::for_composition(MembraneComposition::Nucleus);
    let mut cfg = sporegate_config();
    cfg.trust_lan_input = false;
    cfg.enable_dhcp = true;
    let script = fw.to_nftables_script(Some(&cfg));
    assert!(
        script.contains("DHCP server"),
        "DHCP rule visible when LAN not fully trusted"
    );
}

#[test]
fn nftables_dhcp_suppressed_by_trust_lan() {
    let fw = FirewallRuleset::for_composition(MembraneComposition::Nucleus);
    let cfg = sporegate_config();
    assert!(cfg.trust_lan_input);
    let script = fw.to_nftables_script(Some(&cfg));
    assert!(
        !script.contains("DHCP server"),
        "DHCP rule redundant when LAN trusted"
    );
}

#[test]
fn nftables_no_wireguard_when_absent() {
    let mut cfg = sporegate_config();
    cfg.wireguard_interface = None;
    let fw = FirewallRuleset::for_composition(MembraneComposition::Nucleus);
    let script = fw.to_nftables_script(Some(&cfg));
    assert!(!script.contains("wg0"));
    assert!(!script.contains("udp dport 51820"));
}

#[test]
fn nftables_no_ipv6_drop_when_disabled() {
    let mut cfg = sporegate_config();
    cfg.drop_ipv6_forward = false;
    let fw = FirewallRuleset::for_composition(MembraneComposition::Nucleus);
    let script = fw.to_nftables_script(Some(&cfg));
    assert!(!script.contains("membrane-v6"));
}

#[test]
fn nftables_config_serde_defaults() {
    let json = r#"{"wan_interface":"eth0","lan_interface":"br0","lan_subnet":"10.0.0.0/24","gate_name":"test","enable_nat":false,"enable_dhcp":false}"#;
    let parsed: NftablesConfig = serde_json::from_str(json).unwrap();
    assert!(!parsed.trust_lan_input, "trust_lan_input defaults false");
    assert!(parsed.drop_ipv6_forward, "drop_ipv6_forward defaults true");
    assert_eq!(parsed.wireguard_port, 51820, "wg port defaults 51820");
    assert!(
        parsed.wireguard_interface.is_none(),
        "wg interface defaults None"
    );
}

#[test]
fn nftables_standalone_has_log_drop() {
    let fw = FirewallRuleset::for_composition(MembraneComposition::Relay);
    let script = fw.to_nftables_script(None);
    assert!(script.contains("log prefix \"nft-drop: \" drop"));
}

#[test]
fn nftables_plasma_counter_drop_forward() {
    let fw = FirewallRuleset::for_composition(MembraneComposition::Nucleus);
    let script = fw.to_nftables_script(Some(&sporegate_config()));
    assert!(
        script.contains("counter drop"),
        "forward chain should count drops"
    );
}
