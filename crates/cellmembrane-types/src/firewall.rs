// SPDX-License-Identifier: AGPL-3.0-or-later

//! Firewall rule derivation from membrane composition.
//!
//! Given a [`MembraneComposition`], produces the exact set of UFW rules.
//! The firewall is composition-deterministic — no manual port management.

use crate::composition::{MembraneComposition, SSH_PORT};
use crate::service::Protocol;
use serde::{Deserialize, Serialize};
use std::fmt::{self, Write as _};

fn push_port_rules(
    rules: &mut Vec<FirewallRule>,
    port: u16,
    proto: Protocol,
    comment: &'static str,
) {
    match proto {
        Protocol::Tcp => rules.push(FirewallRule {
            port,
            protocol: FirewallProtocol::Tcp,
            comment,
        }),
        Protocol::Udp => rules.push(FirewallRule {
            port,
            protocol: FirewallProtocol::Udp,
            comment,
        }),
        Protocol::TcpAndUdp => {
            rules.push(FirewallRule {
                port,
                protocol: FirewallProtocol::Tcp,
                comment,
            });
            rules.push(FirewallRule {
                port,
                protocol: FirewallProtocol::Udp,
                comment,
            });
        }
        Protocol::Uds => {}
    }
}

/// A single firewall rule (one port + protocol combination).
///
/// Constructed programmatically by [`FirewallRuleset::for_composition`] —
/// never deserialized from external input.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct FirewallRule {
    /// Port number.
    pub port: u16,
    /// Protocol (tcp, udp, or both).
    pub protocol: FirewallProtocol,
    /// Human-readable comment for this rule.
    pub comment: &'static str,
}

/// Protocol specifier for a firewall rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FirewallProtocol {
    /// TCP only.
    Tcp,
    /// UDP only.
    Udp,
}

impl fmt::Display for FirewallProtocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tcp => write!(f, "tcp"),
            Self::Udp => write!(f, "udp"),
        }
    }
}

impl FirewallRule {
    /// Format as a `ufw allow` command.
    #[must_use]
    pub fn to_ufw_command(&self) -> String {
        format!(
            "ufw allow {}/{} comment '{}'",
            self.port, self.protocol, self.comment
        )
    }
}

impl fmt::Display for FirewallRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{} ({})", self.port, self.protocol, self.comment)
    }
}

/// Complete firewall ruleset for a membrane deployment.
#[derive(Debug, Clone, Serialize)]
pub struct FirewallRuleset {
    /// The composition this ruleset was derived from.
    pub composition: MembraneComposition,
    /// Individual firewall rules.
    pub rules: Vec<FirewallRule>,
}

impl FirewallRuleset {
    /// Derive the firewall ruleset for a given composition.
    #[must_use]
    pub fn for_composition(composition: MembraneComposition) -> Self {
        let spec = composition.spec();
        let mut rules = Vec::new();

        rules.push(FirewallRule {
            port: SSH_PORT,
            protocol: FirewallProtocol::Tcp,
            comment: "SSH",
        });

        for binary in spec.all_binaries() {
            if let Some(svc) = crate::service::MembraneService::for_binary(binary) {
                if !svc.is_externally_reachable() {
                    continue;
                }
                if let Some(port) = svc.port {
                    push_port_rules(&mut rules, port, svc.protocol, svc.binary);
                }
                for &(port, proto, comment) in svc.extra_ports {
                    push_port_rules(&mut rules, port, proto, comment);
                }
            }
        }

        rules.sort_by(|a, b| a.port.cmp(&b.port).then(a.protocol.cmp(&b.protocol)));
        rules.dedup();

        Self { composition, rules }
    }

    /// All unique ports in this ruleset.
    #[must_use]
    pub fn ports(&self) -> Vec<u16> {
        let mut ports: Vec<u16> = self.rules.iter().map(|r| r.port).collect();
        ports.sort_unstable();
        ports.dedup();
        ports
    }

    /// Generate the full UFW setup script.
    #[must_use]
    pub fn to_ufw_script(&self) -> String {
        let mut lines = vec![
            "ufw --force reset".to_string(),
            "ufw default deny incoming".to_string(),
            "ufw default allow outgoing".to_string(),
        ];
        for rule in &self.rules {
            lines.push(rule.to_ufw_command());
        }
        lines.push("ufw --force enable".to_string());
        lines.join("\n")
    }
}

/// Configuration for nftables generation on plasma membrane gates.
///
/// Plasma membrane gates act as the LAN boundary — they perform NAT,
/// DHCP serving, and packet forwarding for interior gates. This config
/// parameterizes the nftables ruleset for that role.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "each flag is an independent firewall toggle"
)]
pub struct NftablesConfig {
    /// WAN-facing interface name (e.g., `"enp1s0"`, `"eth0"`).
    pub wan_interface: String,
    /// LAN-facing interface name (e.g., `"eno1"`, `"br0"`).
    pub lan_interface: String,
    /// LAN subnet in CIDR notation (e.g., `"192.168.4.0/22"`).
    pub lan_subnet: String,
    /// Gate hostname (used in ruleset comments).
    pub gate_name: String,
    /// Enable NAT masquerade on the WAN interface.
    pub enable_nat: bool,
    /// Enable DHCP server passthrough (UDP 67/68) on LAN interface.
    pub enable_dhcp: bool,
    /// Accept all input from LAN interface (gate is trusted by interior).
    #[serde(default)]
    pub trust_lan_input: bool,
    /// `WireGuard` overlay interface name (e.g., `"wg0"`). Enables overlay
    /// forwarding rules and accepts `WireGuard` UDP port in input chain.
    #[serde(default)]
    pub wireguard_interface: Option<String>,
    /// `WireGuard` listen port (default 51820).
    #[serde(default = "default_wg_port")]
    pub wireguard_port: u16,
    /// Drop all IPv6 forwarding (hardening for dual-stack LANs).
    #[serde(default = "default_true")]
    pub drop_ipv6_forward: bool,
}

/// Default `WireGuard` listen port (standard: 51820).
#[must_use]
pub const fn default_wg_port() -> u16 {
    51820
}
const fn default_true() -> bool {
    true
}

impl FirewallRule {
    fn to_nft_rule(&self) -> String {
        let proto = match self.protocol {
            FirewallProtocol::Tcp => "tcp",
            FirewallProtocol::Udp => "udp",
        };
        format!(
            "        {proto} dport {} accept comment \"{}\"",
            self.port, self.comment
        )
    }
}

impl FirewallRuleset {
    /// Generate a complete nftables ruleset script.
    ///
    /// For standalone gates (no NAT/forwarding), pass `None` for config.
    /// For plasma membrane gates (NAT/DHCP/forward), pass an [`NftablesConfig`].
    ///
    /// The generated script is idempotent: `flush ruleset` + full rebuild.
    #[must_use]
    pub fn to_nftables_script(&self, config: Option<&NftablesConfig>) -> String {
        let mut out = String::with_capacity(4096);

        let gate_label = config.map_or("standalone", |c| c.gate_name.as_str());
        let _ = write!(
            out,
            "#!/usr/sbin/nft -f\n\
             # Composition-deterministic firewall — generated by cellMembrane\n\
             # Composition: {}\n\
             # Gate: {gate_label}\n\
             # Channel proteins = nftables rules\n\n\
             flush ruleset\n\n",
            self.composition,
        );

        out.push_str("table inet membrane {\n");
        self.emit_input_chain(&mut out, config);
        Self::emit_forward_chain(&mut out, config);
        out.push_str("    chain output {\n");
        out.push_str("        type filter hook output priority 0; policy accept;\n");
        out.push_str("    }\n");
        out.push_str("}\n");
        Self::emit_nat_tables(&mut out, config);
        out
    }

    fn emit_input_chain(&self, out: &mut String, config: Option<&NftablesConfig>) {
        out.push_str("    chain input {\n");
        out.push_str("        type filter hook input priority 0; policy drop;\n\n");
        out.push_str("        ct state established,related accept\n");
        out.push_str("        ct state invalid drop\n\n");
        out.push_str("        iif lo accept\n\n");
        out.push_str("        meta l4proto icmp accept\n");
        out.push_str("        meta l4proto icmpv6 accept\n\n");

        if let Some(cfg) = config {
            if cfg.trust_lan_input {
                let _ = writeln!(
                    out,
                    "        iifname \"{}\" accept comment \"LAN trusted (plasma membrane interior)\"",
                    cfg.lan_interface
                );
            }
            if cfg.enable_dhcp && !cfg.trust_lan_input {
                let _ = writeln!(
                    out,
                    "        iifname \"{}\" udp dport {{ 67, 68 }} accept comment \"DHCP server\"",
                    cfg.lan_interface
                );
            }
            if let Some(ref wg_iface) = cfg.wireguard_interface {
                let _ = writeln!(
                    out,
                    "        udp dport {} accept comment \"WireGuard mesh overlay\"",
                    cfg.wireguard_port
                );
                let _ = writeln!(
                    out,
                    "        iifname \"{wg_iface}\" accept comment \"WireGuard overlay input\""
                );
            }
        }
        for rule in &self.rules {
            let _ = writeln!(out, "{}", rule.to_nft_rule());
        }
        out.push_str("\n        log prefix \"nft-drop: \" drop\n");
        out.push_str("    }\n\n");
    }

    fn emit_forward_chain(out: &mut String, config: Option<&NftablesConfig>) {
        out.push_str("    chain forward {\n");
        if let Some(cfg) = config {
            out.push_str("        type filter hook forward priority 0; policy drop;\n\n");
            out.push_str("        ct state established,related accept\n");
            let _ = writeln!(
                out,
                "        iifname \"{}\" oifname \"{}\" accept comment \"LAN → WAN\"",
                cfg.lan_interface, cfg.wan_interface
            );
            let _ = writeln!(
                out,
                "        iifname \"{}\" oifname \"{}\" accept comment \"LAN → LAN (inter-gate)\"",
                cfg.lan_interface, cfg.lan_interface
            );
            if let Some(ref wg_iface) = cfg.wireguard_interface {
                let _ = writeln!(
                    out,
                    "        iifname \"{wg_iface}\" accept comment \"WireGuard overlay forward\""
                );
            }
            out.push_str("\n        counter drop\n");
        } else {
            out.push_str("        type filter hook forward priority 0; policy drop;\n");
            out.push_str("        ct state established,related accept\n");
        }
        out.push_str("    }\n\n");
    }

    fn emit_nat_tables(out: &mut String, config: Option<&NftablesConfig>) {
        let Some(cfg) = config else { return };

        if cfg.enable_nat {
            out.push('\n');
            out.push_str("table ip membrane-nat {\n");
            out.push_str("    chain postrouting {\n");
            out.push_str("        type nat hook postrouting priority srcnat;\n");
            let _ = writeln!(out, "        oifname \"{}\" masquerade", cfg.wan_interface);
            out.push_str("    }\n");
            out.push_str("}\n");
        }

        if cfg.drop_ipv6_forward {
            out.push('\n');
            out.push_str("table ip6 membrane-v6 {\n");
            out.push_str("    chain forward {\n");
            out.push_str("        type filter hook forward priority 0; policy drop;\n");
            out.push_str("        ct state established,related accept\n");
            out.push_str("        drop\n");
            out.push_str("    }\n");
            out.push_str("}\n");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_ruleset_always_includes_ssh() {
        let ruleset = FirewallRuleset::for_composition(MembraneComposition::Relay);
        assert!(
            ruleset
                .rules
                .iter()
                .any(|r| r.port == SSH_PORT && r.protocol == FirewallProtocol::Tcp),
            "Relay composition must include SSH"
        );
    }

    #[test]
    fn rules_are_sorted_and_deduped() {
        let ruleset = FirewallRuleset::for_composition(MembraneComposition::Tower);
        for window in ruleset.rules.windows(2) {
            assert!(
                (window[0].port, window[0].protocol) <= (window[1].port, window[1].protocol),
                "rules must be sorted: {:?} should come before {:?}",
                window[0],
                window[1]
            );
        }
    }

    #[test]
    fn ufw_command_format() {
        let rule = FirewallRule {
            port: 443,
            protocol: FirewallProtocol::Tcp,
            comment: "HTTPS",
        };
        let cmd = rule.to_ufw_command();
        assert!(cmd.contains("ufw allow 443/tcp"));
        assert!(cmd.contains("comment 'HTTPS'"));
    }

    #[test]
    fn nft_rule_format() {
        let rule = FirewallRule {
            port: 51820,
            protocol: FirewallProtocol::Udp,
            comment: "WireGuard",
        };
        let nft = rule.to_nft_rule();
        assert!(nft.contains("udp dport 51820 accept"));
        assert!(nft.contains("WireGuard"));
    }

    #[test]
    fn ufw_script_has_reset_and_enable() {
        let ruleset = FirewallRuleset::for_composition(MembraneComposition::Relay);
        let script = ruleset.to_ufw_script();
        assert!(script.starts_with("ufw --force reset"));
        assert!(script.ends_with("ufw --force enable"));
    }

    #[test]
    fn ports_returns_unique_sorted() {
        let ruleset = FirewallRuleset::for_composition(MembraneComposition::Nest);
        let ports = ruleset.ports();
        for window in ports.windows(2) {
            assert!(window[0] < window[1], "ports must be unique and sorted");
        }
    }

    #[test]
    fn higher_composition_has_more_rules() {
        let relay = FirewallRuleset::for_composition(MembraneComposition::Relay);
        let tower = FirewallRuleset::for_composition(MembraneComposition::Tower);
        assert!(
            tower.rules.len() >= relay.rules.len(),
            "Tower ({}) should have >= Relay ({}) rules",
            tower.rules.len(),
            relay.rules.len()
        );
    }

    #[test]
    fn nftables_standalone_no_nat() {
        let ruleset = FirewallRuleset::for_composition(MembraneComposition::Relay);
        let script = ruleset.to_nftables_script(None);
        assert!(script.contains("flush ruleset"));
        assert!(script.contains("table inet membrane"));
        assert!(!script.contains("masquerade"));
        assert!(!script.contains("membrane-nat"));
    }

    #[test]
    fn nftables_plasma_membrane_has_nat() {
        let ruleset = FirewallRuleset::for_composition(MembraneComposition::Relay);
        let config = NftablesConfig {
            wan_interface: "enp1s0".into(),
            lan_interface: "eno1".into(),
            lan_subnet: "192.168.4.0/22".into(),
            gate_name: "sporeGate".into(),
            enable_nat: true,
            enable_dhcp: true,
            trust_lan_input: false,
            wireguard_interface: Some("wg0".into()),
            wireguard_port: 51820,
            drop_ipv6_forward: true,
        };
        let script = ruleset.to_nftables_script(Some(&config));
        assert!(script.contains("masquerade"));
        assert!(script.contains("membrane-nat"));
        assert!(script.contains("DHCP server"));
        assert!(script.contains("WireGuard mesh overlay"));
        assert!(script.contains("membrane-v6"));
    }

    #[test]
    fn firewall_protocol_display() {
        assert_eq!(FirewallProtocol::Tcp.to_string(), "tcp");
        assert_eq!(FirewallProtocol::Udp.to_string(), "udp");
    }

    #[test]
    fn firewall_rule_display() {
        let rule = FirewallRule {
            port: 22,
            protocol: FirewallProtocol::Tcp,
            comment: "SSH",
        };
        assert_eq!(rule.to_string(), "22/tcp (SSH)");
    }
}
