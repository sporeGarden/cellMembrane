// SPDX-License-Identifier: AGPL-3.0-or-later

//! Gate pre-flight checks — validate local system readiness before deployment.
//!
//! Runs non-destructive checks to catch the common adhoc interventions
//! documented in the sporeGate Wave 115 AAR:
//!   - Interface detection (WAN/LAN by driver, speed, carrier)
//!   - IP conflict scanning (ARP probe for target gateway IP on detected LAN interface)
//!   - Port 53 listener check (systemd-resolved vs dnsmasq)
//!   - `NetworkManager` interference check
//!   - IPv6 forwarding state

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tracing::{info, warn};

/// Result of a single preflight check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreflightCheck {
    /// Machine-readable check identifier (e.g. `"port53.available"`).
    pub name: String,
    /// Whether this check passed.
    pub passed: bool,
    /// Human-readable explanation or remediation advice.
    pub detail: String,
}

/// Detected network interface with classification hints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedInterface {
    /// Kernel interface name (e.g. `"enp1s0"`).
    pub name: String,
    /// Kernel driver (from sysfs, e.g. `"r8169"`).
    pub driver: String,
    /// Negotiated link speed in Mbps (None if unavailable).
    pub speed_mbps: Option<u32>,
    /// Whether the interface has physical link.
    pub carrier: bool,
    /// MAC address.
    pub mac: String,
    /// Assigned IPv4 addresses.
    pub ipv4: Vec<String>,
    /// Heuristic role classification.
    pub role_hint: InterfaceRole,
}

/// Heuristic role assignment for an interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterfaceRole {
    /// Likely WAN — has default route or DHCP-assigned address.
    Wan,
    /// Likely LAN — second ethernet with no default route.
    Lan,
    /// Wireless interface.
    Wireless,
    /// Loopback or virtual.
    Virtual,
    /// Cannot determine.
    Unknown,
}

impl std::fmt::Display for InterfaceRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Wan => f.write_str("Wan"),
            Self::Lan => f.write_str("Lan"),
            Self::Wireless => f.write_str("Wireless"),
            Self::Virtual => f.write_str("Virtual"),
            Self::Unknown => f.write_str("Unknown"),
        }
    }
}

/// Full preflight report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreflightReport {
    /// Detected network interfaces with role classification.
    pub interfaces: Vec<DetectedInterface>,
    /// Individual check results.
    pub checks: Vec<PreflightCheck>,
    /// True only if every check passed.
    pub all_pass: bool,
}

/// Run all preflight checks on the local system.
pub async fn run_preflight(target_ip: Option<&str>) -> PreflightReport {
    let interfaces = detect_interfaces().await;
    let mut checks = Vec::new();

    checks.push(check_ethernet_count(&interfaces));
    checks.push(check_carrier(&interfaces));
    checks.push(check_port53().await);
    checks.push(check_networkmanager().await);

    if let Some(ip) = target_ip {
        checks.push(check_ip_conflict(ip, &interfaces).await);
    }

    checks.push(check_ipv6_forwarding().await);

    let all_pass = checks.iter().all(|c| c.passed);

    PreflightReport {
        interfaces,
        checks,
        all_pass,
    }
}

// ── Interface detection ─────────────────────────────────────────────

async fn detect_interfaces() -> Vec<DetectedInterface> {
    let output = tokio::process::Command::new("ip")
        .args(["-j", "link", "show"])
        .output()
        .await;

    let Ok(output) = output else {
        warn!("ip link show failed");
        return vec![];
    };

    let Ok(text) = std::str::from_utf8(&output.stdout) else {
        return vec![];
    };

    let Ok(links) = serde_json::from_str::<Vec<serde_json::Value>>(text) else {
        return vec![];
    };

    let addr_output = tokio::process::Command::new("ip")
        .args(["-j", "addr", "show"])
        .output()
        .await
        .ok();

    let addr_map = addr_output
        .as_ref()
        .and_then(|o| std::str::from_utf8(&o.stdout).ok())
        .and_then(|s| serde_json::from_str::<Vec<serde_json::Value>>(s).ok())
        .map(|addrs| build_addr_map(&addrs))
        .unwrap_or_default();

    let default_route_iface = resolve_default_route_iface().await;

    let mut interfaces = Vec::new();
    for link in &links {
        let name = link["ifname"].as_str().unwrap_or("").to_string();
        if name.is_empty() || name == "lo" {
            continue;
        }

        let link_type = link["link_type"].as_str().unwrap_or("");
        if link_type == "loopback" {
            continue;
        }

        let mac = link["address"].as_str().unwrap_or("").to_string();
        let flags = link["flags"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
            .unwrap_or_default();

        let carrier = flags.contains(&"LOWER_UP");

        let driver = read_sysfs_driver(&name).await;
        let speed_mbps = read_sysfs_speed(&name).await;
        let ipv4 = addr_map.get(&name).cloned().unwrap_or_default();

        let role_hint = classify_role(
            &name,
            &driver,
            carrier,
            &ipv4,
            default_route_iface.as_deref(),
        );

        interfaces.push(DetectedInterface {
            name,
            driver,
            speed_mbps,
            carrier,
            mac,
            ipv4,
            role_hint,
        });
    }

    interfaces
}

fn build_addr_map(addrs: &[serde_json::Value]) -> BTreeMap<String, Vec<String>> {
    let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for entry in addrs {
        let ifname = entry["ifname"].as_str().unwrap_or("").to_string();
        if let Some(addr_info) = entry["addr_info"].as_array() {
            for ai in addr_info {
                if ai["family"].as_str() == Some("inet") {
                    if let Some(local) = ai["local"].as_str() {
                        map.entry(ifname.clone())
                            .or_default()
                            .push(local.to_string());
                    }
                }
            }
        }
    }
    map
}

async fn resolve_default_route_iface() -> Option<String> {
    let output = tokio::process::Command::new("ip")
        .args(["-j", "route", "show", "default"])
        .output()
        .await
        .ok()?;
    let text = std::str::from_utf8(&output.stdout).ok()?;
    let routes: Vec<serde_json::Value> = serde_json::from_str(text).ok()?;
    routes
        .first()
        .and_then(|r| r["dev"].as_str())
        .map(String::from)
}

fn classify_role(
    name: &str,
    driver: &str,
    carrier: bool,
    ipv4: &[String],
    default_route_iface: Option<&str>,
) -> InterfaceRole {
    if name.starts_with("wl") || driver.contains("wifi") || driver.contains("iwl") {
        return InterfaceRole::Wireless;
    }
    if name.starts_with("veth")
        || name.starts_with("br-")
        || name.starts_with("docker")
        || name.starts_with("virbr")
        || name.starts_with("wg")
        || name.starts_with("tun")
    {
        return InterfaceRole::Virtual;
    }

    if Some(name) == default_route_iface {
        return InterfaceRole::Wan;
    }

    if carrier && !ipv4.is_empty() && name.starts_with("en") {
        return InterfaceRole::Lan;
    }

    if carrier && name.starts_with("en") {
        return InterfaceRole::Lan;
    }

    InterfaceRole::Unknown
}

async fn read_sysfs_driver(iface: &str) -> String {
    let path = format!("/sys/class/net/{iface}/device/driver");
    tokio::fs::read_link(&path)
        .await
        .ok()
        .and_then(|p| p.file_name().map(|f| f.to_string_lossy().to_string()))
        .unwrap_or_default()
}

async fn read_sysfs_speed(iface: &str) -> Option<u32> {
    let path = format!("/sys/class/net/{iface}/speed");
    tokio::fs::read_to_string(&path)
        .await
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .filter(|&s: &u32| s > 0 && s < 1_000_000)
}

// ── Individual checks ───────────────────────────────────────────────

fn check_ethernet_count(interfaces: &[DetectedInterface]) -> PreflightCheck {
    let eth_count = interfaces
        .iter()
        .filter(|i| {
            matches!(
                i.role_hint,
                InterfaceRole::Wan | InterfaceRole::Lan | InterfaceRole::Unknown
            ) && i.name.starts_with("en")
        })
        .count();

    PreflightCheck {
        name: "ethernet.count".into(),
        passed: eth_count >= 2,
        detail: format!("{eth_count} ethernet interfaces detected (need >= 2 for router)"),
    }
}

fn check_carrier(interfaces: &[DetectedInterface]) -> PreflightCheck {
    let without_carrier: Vec<&str> = interfaces
        .iter()
        .filter(|i| i.name.starts_with("en") && !i.carrier)
        .map(|i| i.name.as_str())
        .collect();

    PreflightCheck {
        name: "ethernet.carrier".into(),
        passed: without_carrier.is_empty(),
        detail: if without_carrier.is_empty() {
            "all ethernet interfaces have carrier".into()
        } else {
            format!("no carrier: {} (check cable)", without_carrier.join(", "))
        },
    }
}

async fn check_port53() -> PreflightCheck {
    let output = tokio::process::Command::new("ss")
        .args(["-tlnp"])
        .output()
        .await;

    let listeners = output
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();

    let port53_lines: Vec<&str> = listeners
        .lines()
        .filter(|l| l.contains(":53 ") || l.contains(":53\t"))
        .collect();

    let has_resolved = port53_lines
        .iter()
        .any(|l| l.contains("systemd-resolve") || l.contains("resolved"));

    PreflightCheck {
        name: "port53.available".into(),
        passed: port53_lines.is_empty(),
        detail: if port53_lines.is_empty() {
            "port 53 is free".into()
        } else if has_resolved {
            "port 53 blocked by systemd-resolved — run: systemctl disable --now systemd-resolved"
                .into()
        } else {
            format!(
                "port 53 in use by: {}",
                port53_lines.first().unwrap_or(&"unknown")
            )
        },
    }
}

async fn check_networkmanager() -> PreflightCheck {
    let output = tokio::process::Command::new("systemctl")
        .args(["is-active", "NetworkManager"])
        .output()
        .await;

    let active = output
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .is_some_and(|s| s.trim() == "active");

    if !active {
        return PreflightCheck {
            name: "networkmanager.absent".into(),
            passed: true,
            detail: "NetworkManager not active — no interference".into(),
        };
    }

    let unmanage_path = "/etc/NetworkManager/conf.d/99-unmanage-wired.conf";
    let has_unmanage = tokio::fs::metadata(unmanage_path).await.is_ok();

    PreflightCheck {
        name: "networkmanager.unmanaged".into(),
        passed: has_unmanage,
        detail: if has_unmanage {
            "NetworkManager active but wired exclusion configured".into()
        } else {
            "NetworkManager active — wired interfaces may conflict. Create /etc/NetworkManager/conf.d/99-unmanage-wired.conf".into()
        },
    }
}

async fn check_ip_conflict(target_ip: &str, interfaces: &[DetectedInterface]) -> PreflightCheck {
    let existing_owners: Vec<String> = interfaces
        .iter()
        .filter(|i| i.ipv4.contains(&target_ip.to_string()))
        .map(|i| i.name.clone())
        .collect();

    if !existing_owners.is_empty() {
        return PreflightCheck {
            name: "ip.conflict".into(),
            passed: true,
            detail: format!(
                "{target_ip} already assigned to {} (this host owns it)",
                existing_owners.join(", ")
            ),
        };
    }

    let probe_iface = interfaces
        .iter()
        .find(|i| i.role_hint == InterfaceRole::Lan && i.carrier)
        .or_else(|| {
            interfaces
                .iter()
                .find(|i| i.name.starts_with("en") && i.carrier)
        });

    let Some(iface) = probe_iface else {
        return PreflightCheck {
            name: "ip.conflict".into(),
            passed: true,
            detail: format!(
                "{target_ip} — skipped ARP probe (no active ethernet interface for probing)"
            ),
        };
    };

    let arping = tokio::process::Command::new("arping")
        .args(["-c", "2", "-w", "3", "-I", &iface.name, target_ip])
        .output()
        .await;

    let conflict = arping
        .ok()
        .is_some_and(|o| o.status.success() && !o.stdout.is_empty());

    if conflict {
        info!(ip = target_ip, iface = %iface.name, "IP conflict detected via ARP");
    }

    PreflightCheck {
        name: "ip.conflict".into(),
        passed: !conflict,
        detail: if conflict {
            format!(
                "{target_ip} already claimed by another device on the network (probed via {})",
                iface.name
            )
        } else {
            format!(
                "{target_ip} appears available (no ARP response on {})",
                iface.name
            )
        },
    }
}

async fn check_ipv6_forwarding() -> PreflightCheck {
    let val = tokio::fs::read_to_string("/proc/sys/net/ipv6/conf/all/forwarding")
        .await
        .unwrap_or_default();
    let enabled = val.trim() == "1";

    PreflightCheck {
        name: "ipv6.forwarding".into(),
        passed: !enabled,
        detail: if enabled {
            "IPv6 forwarding enabled — will cause iPhone stalls without NAT66/PD. Disable: sysctl net.ipv6.conf.all.forwarding=0".into()
        } else {
            "IPv6 forwarding disabled (safe default)".into()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_wan_by_default_route() {
        assert_eq!(
            classify_role(
                "enp1s0",
                "r8169",
                true,
                &["192.168.1.2".into()],
                Some("enp1s0")
            ),
            InterfaceRole::Wan
        );
    }

    #[test]
    fn classify_lan_by_carrier_and_ip() {
        assert_eq!(
            classify_role(
                "eno1",
                "r8169",
                true,
                &["192.168.4.1".into()],
                Some("enp1s0")
            ),
            InterfaceRole::Lan
        );
    }

    #[test]
    fn classify_wireless() {
        assert_eq!(
            classify_role("wlp3s0", "iwlwifi", true, &[], None),
            InterfaceRole::Wireless
        );
    }

    #[test]
    fn classify_virtual() {
        assert_eq!(
            classify_role("wg0", "", false, &[], None),
            InterfaceRole::Virtual
        );
        assert_eq!(
            classify_role("docker0", "", false, &[], None),
            InterfaceRole::Virtual
        );
    }

    #[test]
    fn ethernet_count_check_needs_two() {
        let ifaces = vec![
            DetectedInterface {
                name: "enp1s0".into(),
                driver: "r8169".into(),
                speed_mbps: Some(1000),
                carrier: true,
                mac: "aa:bb:cc:dd:ee:ff".into(),
                ipv4: vec!["192.168.1.2".into()],
                role_hint: InterfaceRole::Wan,
            },
            DetectedInterface {
                name: "eno1".into(),
                driver: "r8169".into(),
                speed_mbps: Some(1000),
                carrier: true,
                mac: "11:22:33:44:55:66".into(),
                ipv4: vec![],
                role_hint: InterfaceRole::Lan,
            },
        ];
        assert!(check_ethernet_count(&ifaces).passed);
    }

    #[test]
    fn ethernet_count_fails_with_one() {
        let ifaces = vec![DetectedInterface {
            name: "enp1s0".into(),
            driver: "r8169".into(),
            speed_mbps: Some(1000),
            carrier: true,
            mac: "aa:bb:cc:dd:ee:ff".into(),
            ipv4: vec![],
            role_hint: InterfaceRole::Wan,
        }];
        assert!(!check_ethernet_count(&ifaces).passed);
    }

    #[test]
    fn carrier_check_flags_no_link() {
        let ifaces = vec![DetectedInterface {
            name: "eno1".into(),
            driver: "r8169".into(),
            speed_mbps: None,
            carrier: false,
            mac: "11:22:33:44:55:66".into(),
            ipv4: vec![],
            role_hint: InterfaceRole::Lan,
        }];
        let check = check_carrier(&ifaces);
        assert!(!check.passed);
        assert!(check.detail.contains("eno1"));
    }

    #[test]
    fn preflight_report_serializes() {
        let report = PreflightReport {
            interfaces: vec![],
            checks: vec![PreflightCheck {
                name: "test".into(),
                passed: true,
                detail: "ok".into(),
            }],
            all_pass: true,
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"all_pass\":true"));
    }

    #[test]
    fn interface_role_display() {
        assert_eq!(InterfaceRole::Wan.to_string(), "Wan");
        assert_eq!(InterfaceRole::Lan.to_string(), "Lan");
        assert_eq!(InterfaceRole::Wireless.to_string(), "Wireless");
        assert_eq!(InterfaceRole::Virtual.to_string(), "Virtual");
        assert_eq!(InterfaceRole::Unknown.to_string(), "Unknown");
    }

    #[test]
    fn ip_conflict_prefers_lan_interface() {
        let ifaces = vec![
            DetectedInterface {
                name: "enp1s0".into(),
                driver: "r8169".into(),
                speed_mbps: Some(1000),
                carrier: true,
                mac: "aa:bb:cc:dd:ee:ff".into(),
                ipv4: vec!["192.168.1.2".into()],
                role_hint: InterfaceRole::Wan,
            },
            DetectedInterface {
                name: "eno1".into(),
                driver: "igc".into(),
                speed_mbps: Some(1000),
                carrier: true,
                mac: "11:22:33:44:55:66".into(),
                ipv4: vec!["192.168.4.1".into()],
                role_hint: InterfaceRole::Lan,
            },
        ];

        let probe_iface = ifaces
            .iter()
            .find(|i| i.role_hint == InterfaceRole::Lan && i.carrier)
            .or_else(|| {
                ifaces
                    .iter()
                    .find(|i| i.name.starts_with("en") && i.carrier)
            });

        assert_eq!(probe_iface.map(|i| i.name.as_str()), Some("eno1"));
    }

    #[test]
    fn ip_conflict_falls_back_to_any_ethernet() {
        let ifaces = vec![DetectedInterface {
            name: "enp5s0".into(),
            driver: "ixgbe".into(),
            speed_mbps: Some(10000),
            carrier: true,
            mac: "aa:bb:cc:dd:ee:ff".into(),
            ipv4: vec![],
            role_hint: InterfaceRole::Unknown,
        }];

        let probe_iface = ifaces
            .iter()
            .find(|i| i.role_hint == InterfaceRole::Lan && i.carrier)
            .or_else(|| {
                ifaces
                    .iter()
                    .find(|i| i.name.starts_with("en") && i.carrier)
            });

        assert_eq!(probe_iface.map(|i| i.name.as_str()), Some("enp5s0"));
    }
}
