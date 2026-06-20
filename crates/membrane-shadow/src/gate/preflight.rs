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
use tracing::info;

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
    let interfaces = interface::detect_interfaces().await;
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

use super::interface;

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
    let port53_bound = is_tcp_port_bound(53).await || is_udp_port_bound(53).await;

    let has_resolved = std::path::Path::new("/run/systemd/resolve/stub-resolv.conf").exists();

    PreflightCheck {
        name: "port53.available".into(),
        passed: !port53_bound,
        detail: if !port53_bound {
            "port 53 is free".into()
        } else if has_resolved {
            "port 53 blocked by systemd-resolved — run: systemctl disable --now systemd-resolved"
                .into()
        } else {
            "port 53 in use by another process".into()
        },
    }
}

/// Check if a TCP port is bound by parsing `/proc/net/tcp` + `/proc/net/tcp6`.
async fn is_tcp_port_bound(port: u16) -> bool {
    is_port_bound_in_proc(port, "/proc/net/tcp").await
        || is_port_bound_in_proc(port, "/proc/net/tcp6").await
}

/// Check if a UDP port is bound by parsing `/proc/net/udp` + `/proc/net/udp6`.
async fn is_udp_port_bound(port: u16) -> bool {
    is_port_bound_in_proc(port, "/proc/net/udp").await
        || is_port_bound_in_proc(port, "/proc/net/udp6").await
}

/// Parse a `/proc/net/{tcp,udp}` file for a specific port in LISTEN state.
///
/// Format: `sl local_address rem_address st ...` where `local_address` is `HEX_IP:HEX_PORT`.
/// State `0A` = TCP LISTEN, `07` = UDP (unconditional).
async fn is_port_bound_in_proc(port: u16, path: &str) -> bool {
    let Ok(contents) = tokio::fs::read_to_string(path).await else {
        return false;
    };
    let hex_port = format!(":{port:04X}");
    contents.lines().skip(1).any(|line| {
        let mut fields = line.split_whitespace();
        let Some(local_addr) = fields.nth(1) else {
            return false;
        };
        let Some(state) = fields.nth(1) else {
            return false;
        };
        local_addr.ends_with(&hex_port) && (state == "0A" || state == "07")
    })
}

/// Detect whether a systemd unit is active via its cgroup presence.
///
/// Avoids the `systemctl is-active` shell-out by checking
/// `/sys/fs/cgroup/system.slice/{unit}/cgroup.procs`.
async fn is_systemd_unit_active(unit: &str) -> bool {
    let cgroup_procs = format!("/sys/fs/cgroup/system.slice/{unit}/cgroup.procs");
    tokio::fs::read_to_string(&cgroup_procs)
        .await
        .is_ok_and(|contents| !contents.trim().is_empty())
}

async fn check_networkmanager() -> PreflightCheck {
    let active = is_systemd_unit_active("NetworkManager.service").await;

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
    use super::interface::classify_role;
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
        let ifaces = [
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
        let ifaces = [DetectedInterface {
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

    #[test]
    fn proc_net_tcp_hex_port_formatting() {
        let port: u16 = 53;
        assert_eq!(format!(":{port:04X}"), ":0035");

        let port: u16 = 80;
        assert_eq!(format!(":{port:04X}"), ":0050");

        let port: u16 = 443;
        assert_eq!(format!(":{port:04X}"), ":01BB");
    }

    #[test]
    fn proc_net_tcp_line_parsing_identifies_listen() {
        let line = "   0: 00000000:0035 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 12345 1 0000000000000000 100 0 0 10 0";
        let hex_port = ":0035";
        let mut fields = line.split_whitespace();
        let local_addr = fields.nth(1).unwrap();
        let state = fields.nth(1).unwrap();
        assert!(local_addr.ends_with(hex_port));
        assert_eq!(state, "0A");
    }

    #[test]
    fn proc_net_tcp_line_parsing_rejects_non_listen() {
        let line = "   1: 0100007F:1F90 0100007F:D4E2 01 00000000:00000000 02:00000000 00000000     0        0 67890 2 0000000000000000 20 4 30 10 -1";
        let hex_port = ":1F90";
        let mut fields = line.split_whitespace();
        let local_addr = fields.nth(1).unwrap();
        let state = fields.nth(1).unwrap();
        assert!(local_addr.ends_with(hex_port));
        assert_ne!(state, "0A", "state 01=ESTABLISHED, not LISTEN");
    }

    #[test]
    fn proc_net_route_default_gateway_parsing() {
        let content = "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\n\
                       enp1s0\t00000000\t0101A8C0\t0003\t0\t0\t100\t00000000\n\
                       enp1s0\t0001A8C0\t00000000\t0001\t0\t0\t100\tFFFFFF00\n";
        let mut found = None;
        for line in content.lines().skip(1) {
            let mut fields = line.split_whitespace();
            let iface = fields.next().unwrap();
            let dest = fields.next().unwrap();
            if dest == "00000000" {
                found = Some(iface.to_string());
                break;
            }
        }
        assert_eq!(found.as_deref(), Some("enp1s0"));
    }
}
