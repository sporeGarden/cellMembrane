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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
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
    /// WireGuard overlay interface name (e.g., `"wg0"`). Enables overlay
    /// forwarding rules and accepts WireGuard UDP port in input chain.
    #[serde(default)]
    pub wireguard_interface: Option<String>,
    /// WireGuard listen port (default 51820).
    #[serde(default = "default_wg_port")]
    pub wireguard_port: u16,
    /// Drop all IPv6 forwarding (hardening for dual-stack LANs).
    #[serde(default = "default_true")]
    pub drop_ipv6_forward: bool,
}

fn default_wg_port() -> u16 {
    51820
}
fn default_true() -> bool {
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
    #[allow(clippy::too_many_lines)]
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

        // ── input chain ──
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

        // ── forward chain ──
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

        // ── output chain ──
        out.push_str("    chain output {\n");
        out.push_str("        type filter hook output priority 0; policy accept;\n");
        out.push_str("    }\n");
        out.push_str("}\n");

        // ── NAT table (plasma membrane only) ──
        if let Some(cfg) = config {
            if cfg.enable_nat {
                out.push('\n');
                out.push_str("table ip membrane-nat {\n");
                out.push_str("    chain postrouting {\n");
                out.push_str("        type nat hook postrouting priority srcnat;\n");
                let _ = writeln!(
                    out,
                    "        oifname \"{}\" masquerade",
                    cfg.wan_interface
                );
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

        out
    }
}

impl PartialOrd for FirewallProtocol {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for FirewallProtocol {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (*self as u8).cmp(&(*other as u8))
    }
}
