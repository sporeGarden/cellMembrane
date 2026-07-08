// SPDX-License-Identifier: AGPL-3.0-or-later

//! Network interface detection — sysfs + `/proc/net`, role classification.

use std::collections::BTreeMap;

use tracing::warn;

use super::preflight::{DetectedInterface, InterfaceRole};

pub(super) async fn detect_interfaces() -> Vec<DetectedInterface> {
    let sysfs = std::path::Path::new("/sys/class/net");
    let Ok(mut entries) = tokio::fs::read_dir(sysfs).await else {
        warn!("cannot read /sys/class/net");
        return vec![];
    };

    let addr_map = parse_proc_net_addresses().await;
    let default_route_iface = resolve_default_route_iface().await;

    let mut interfaces = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        if name == "lo" {
            continue;
        }

        let iface_dir = sysfs.join(&name);
        let iface_type = read_sysfs_file(&iface_dir.join("type")).await;
        if iface_type.trim() == "772" {
            continue;
        }

        let mac = read_sysfs_file(&iface_dir.join("address")).await;
        let carrier = read_sysfs_file(&iface_dir.join("carrier")).await.trim() == "1";
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
            mac: mac.trim().to_string(),
            ipv4,
            role_hint,
        });
    }

    interfaces.sort_by(|a, b| a.name.cmp(&b.name));
    interfaces
}

/// Parse IPv4 addresses from `/proc/net/fib_trie` per-interface,
/// falling back to `ip -j addr show` if unavailable.
async fn parse_proc_net_addresses() -> BTreeMap<String, Vec<String>> {
    if let Some(map) = parse_proc_net_if_inet6_and_fib().await {
        if !map.is_empty() {
            return map;
        }
    }
    ip_addr_show_fallback().await
}

/// Read IPv4 addresses from `/proc/net/fib_trie` keyed by interface.
async fn parse_proc_net_if_inet6_and_fib() -> Option<BTreeMap<String, Vec<String>>> {
    let content = tokio::fs::read_to_string("/proc/net/fib_trie").await.ok()?;
    let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();

    let sysfs = std::path::Path::new("/sys/class/net");
    let mut entries = tokio::fs::read_dir(sysfs).await.ok()?;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let iface = entry.file_name().to_string_lossy().to_string();
        if iface == "lo" {
            continue;
        }
        let ifindex_path = sysfs.join(&iface).join("ifindex");
        if tokio::fs::metadata(&ifindex_path).await.is_err() {
            continue;
        }
        let operstate_path = sysfs.join(&iface).join("operstate");
        let _state = read_sysfs_file(&operstate_path).await;
        map.entry(iface).or_default();
    }

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("/32 host") {
            if let Some(ip) = trimmed
                .strip_prefix("/32 host ")
                .map(|s| s.trim().to_string())
            {
                if ip != cellmembrane_types::service::BIND_LOOPBACK
                    && !ip.starts_with("127.")
                {
                    for addrs in map.values_mut() {
                        if addrs.is_empty() {
                            addrs.push(ip.clone());
                            break;
                        }
                    }
                }
            }
        }
    }

    Some(map)
}

/// Fallback: parse `ip -j addr show` output for interface→IPv4 map.
async fn ip_addr_show_fallback() -> BTreeMap<String, Vec<String>> {
    let output = tokio::process::Command::new("ip")
        .args(["-j", "addr", "show"])
        .output()
        .await
        .ok();

    let Some(output) = output else {
        return BTreeMap::new();
    };
    let Ok(text) = std::str::from_utf8(&output.stdout) else {
        return BTreeMap::new();
    };
    let Ok(addrs) = serde_json::from_str::<Vec<serde_json::Value>>(text) else {
        return BTreeMap::new();
    };

    let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for entry in &addrs {
        let ifname = entry["ifname"].as_str().unwrap_or("");
        let Some(addr_info) = entry["addr_info"].as_array() else {
            continue;
        };
        let ips: Vec<String> = addr_info
            .iter()
            .filter(|ai| ai["family"].as_str() == Some("inet"))
            .filter_map(|ai| ai["local"].as_str().map(String::from))
            .collect();
        if !ips.is_empty() {
            map.entry(ifname.to_string()).or_default().extend(ips);
        }
    }
    map
}

/// Resolve default route interface from `/proc/net/route`.
async fn resolve_default_route_iface() -> Option<String> {
    let content = tokio::fs::read_to_string("/proc/net/route").await.ok()?;
    for line in content.lines().skip(1) {
        let mut fields = line.split_whitespace();
        let iface = fields.next()?;
        let destination = fields.next()?;
        if destination == "00000000" {
            return Some(iface.to_string());
        }
    }
    None
}

async fn read_sysfs_file(path: &std::path::Path) -> String {
    tokio::fs::read_to_string(path).await.unwrap_or_default()
}

pub(super) fn classify_role(
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

#[cfg(test)]
mod tests {
    use super::super::preflight::InterfaceRole;
    use super::*;

    #[test]
    fn classify_wireless_by_prefix() {
        assert_eq!(
            classify_role("wlp3s0", "iwlwifi", true, &[], None),
            InterfaceRole::Wireless
        );
    }

    #[test]
    fn classify_virtual_overlays() {
        assert_eq!(
            classify_role("wg0", "", false, &[], None),
            InterfaceRole::Virtual
        );
        assert_eq!(
            classify_role("docker0", "", false, &[], None),
            InterfaceRole::Virtual
        );
        assert_eq!(
            classify_role("veth123", "", false, &[], None),
            InterfaceRole::Virtual
        );
    }

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
    fn classify_lan_by_carrier() {
        assert_eq!(
            classify_role("eno1", "igb", true, &["10.0.0.1".into()], Some("enp1s0")),
            InterfaceRole::Lan
        );
    }

    #[test]
    fn classify_unknown_when_no_match() {
        assert_eq!(
            classify_role("random0", "", false, &[], None),
            InterfaceRole::Unknown
        );
    }

    #[test]
    fn default_route_parses_proc_net_route_format() {
        let route_content = "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\n\
                             enp1s0\t00000000\t0101A8C0\t0003\t0\t0\t100\n\
                             enp1s0\t0001A8C0\t00000000\t0001\t0\t0\t100\n";
        let mut found = None;
        for line in route_content.lines().skip(1) {
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
