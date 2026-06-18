// SPDX-License-Identifier: AGPL-3.0-or-later

//! Network interface detection — sysfs, `ip link`, role classification.

use std::collections::BTreeMap;

use tracing::warn;

use super::preflight::{DetectedInterface, InterfaceRole};

pub(super) async fn detect_interfaces() -> Vec<DetectedInterface> {
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
