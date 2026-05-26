// SPDX-License-Identifier: AGPL-3.0-or-later

use cellmembrane_types::composition::MembraneComposition;
use cellmembrane_types::firewall::FirewallRuleset;

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
