// SPDX-License-Identifier: AGPL-3.0-or-later

//! Firewall rule derivation from membrane composition.
//!
//! Given a [`MembraneComposition`], produces the exact set of UFW rules.
//! The firewall is composition-deterministic — no manual port management.

use crate::composition::MembraneComposition;
use crate::service::Protocol;
use serde::{Deserialize, Serialize};
use std::fmt;

/// A single firewall rule (one port + protocol combination).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FirewallRule {
    /// Port number.
    pub port: u16,
    /// Protocol (tcp, udp, or both).
    pub protocol: FirewallProtocol,
    /// Human-readable comment for this rule.
    pub comment: String,
}

/// Protocol specifier for a firewall rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirewallRuleset {
    /// The composition this ruleset was derived from.
    pub composition: MembraneComposition,
    /// Individual firewall rules.
    pub rules: Vec<FirewallRule>,
}

impl FirewallRuleset {
    /// Derive the firewall ruleset for a given composition.
    pub fn for_composition(composition: MembraneComposition) -> Self {
        let spec = composition.spec();
        let mut rules = Vec::new();

        // SSH is always open
        rules.push(FirewallRule {
            port: 22,
            protocol: FirewallProtocol::Tcp,
            comment: "SSH".into(),
        });

        // Derive rules from composition services
        for binary in spec.all_binaries() {
            if let Some(svc) = crate::service::MembraneService::for_binary(binary) {
                if !svc.is_externally_reachable() {
                    continue;
                }
                if let Some(port) = svc.port {
                    let name = svc.binary.to_owned();
                    match svc.protocol {
                        Protocol::Tcp => {
                            rules.push(FirewallRule {
                                port,
                                protocol: FirewallProtocol::Tcp,
                                comment: name,
                            });
                        }
                        Protocol::Udp => {
                            rules.push(FirewallRule {
                                port,
                                protocol: FirewallProtocol::Udp,
                                comment: name,
                            });
                        }
                        Protocol::TcpAndUdp => {
                            rules.push(FirewallRule {
                                port,
                                protocol: FirewallProtocol::Tcp,
                                comment: name.clone(),
                            });
                            rules.push(FirewallRule {
                                port,
                                protocol: FirewallProtocol::Udp,
                                comment: name,
                            });
                        }
                        Protocol::Uds => {}
                    }
                }
            }
        }

        // hbbs also listens on 21115 (TCP) and 21116 (TCP+UDP), hbbr on 21117.
        // The service registry covers 21116/21117 but 21115 is the ID server port.
        if spec.symbiotic.contains(&"hbbs")
            && !rules.iter().any(|r| r.port == 21115)
        {
            rules.push(FirewallRule {
                port: 21115,
                protocol: FirewallProtocol::Tcp,
                comment: "hbbs-id".into(),
            });
        }

        if spec.symbiotic.contains(&"caddy")
            && !rules.iter().any(|r| r.port == 80)
        {
            rules.push(FirewallRule {
                port: 80,
                protocol: FirewallProtocol::Tcp,
                comment: "caddy-acme".into(),
            });
        }

        // Sort for deterministic output
        rules.sort_by(|a, b| a.port.cmp(&b.port).then(a.protocol.cmp(&b.protocol)));
        rules.dedup();

        Self {
            composition,
            rules,
        }
    }

    /// All unique ports in this ruleset.
    pub fn ports(&self) -> Vec<u16> {
        let mut ports: Vec<u16> = self.rules.iter().map(|r| r.port).collect();
        ports.sort();
        ports.dedup();
        ports
    }

    /// Generate the full UFW setup script.
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
