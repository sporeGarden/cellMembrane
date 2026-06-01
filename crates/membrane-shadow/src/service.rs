// SPDX-License-Identifier: AGPL-3.0-or-later

//! Systemd service management on the VPS.
//!
//! Shadow domain mapping:
//!   - `gate.service.list`    → biomeOS gate.service.list
//!   - `gate.service.status`  → biomeOS gate.service.status
//!   - `gate.service.restart` → biomeOS gate.service.restart
//!   - `gate.service.logs`    → biomeOS gate.service.logs

use crate::config::ShadowConfig;
use crate::error::Result;
use crate::ssh;
use serde::{Deserialize, Serialize};

/// Detailed service status from systemd on the VPS.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceStatus {
    /// Systemd unit name (e.g. `beardog-membrane.service`).
    pub unit: String,
    /// Whether `ActiveState` is `active`.
    pub active: bool,
    /// Systemd sub-state (e.g. `running`, `dead`, `failed`).
    pub sub_state: String,
    /// Unit description from the service file.
    pub description: String,
    /// Main process ID, if running.
    pub pid: Option<u32>,
    /// Current memory usage, formatted (e.g. `12.3Mi`).
    pub memory: Option<String>,
    /// Timestamp when the service entered active state.
    pub uptime: Option<String>,
}

/// List running membrane services.
///
/// Shadow for: `biomeOS gate.service.list`
pub async fn list(config: &ShadowConfig) -> Result<Vec<ServiceStatus>> {
    let cmd = format!(
        "systemctl list-units --type=service --state=running --no-pager --no-legend | \
         grep -E '{}'",
        config.service_filter,
    );
    let output = ssh::exec(config, &cmd).await?;

    let mut services = Vec::new();
    for line in output.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 4 {
            services.push(ServiceStatus {
                unit: parts[0].to_string(),
                active: parts[2] == "active",
                sub_state: parts[3].to_string(),
                description: parts[4..].join(" "),
                pid: None,
                memory: None,
                uptime: None,
            });
        }
    }

    Ok(services)
}

/// Get detailed status for a specific service.
///
/// Shadow for: `biomeOS gate.service.status`
pub async fn status(config: &ShadowConfig, unit: &str) -> Result<ServiceStatus> {
    let cmd = format!(
        "systemctl show {unit} --no-pager \
         --property=ActiveState,SubState,Description,MainPID,MemoryCurrent,ActiveEnterTimestamp"
    );
    let output = ssh::exec(config, &cmd).await?;

    let mut svc = ServiceStatus {
        unit: unit.to_string(),
        active: false,
        sub_state: String::new(),
        description: String::new(),
        pid: None,
        memory: None,
        uptime: None,
    };

    for line in output.lines() {
        if let Some((key, val)) = line.split_once('=') {
            match key {
                "ActiveState" => svc.active = val == "active",
                "SubState" => svc.sub_state = val.to_string(),
                "Description" => svc.description = val.to_string(),
                "MainPID" => svc.pid = val.parse().ok().filter(|&p: &u32| p > 0),
                "MemoryCurrent" => {
                    if let Ok(bytes) = val.parse::<u64>() {
                        svc.memory = Some(format_bytes(bytes));
                    }
                }
                "ActiveEnterTimestamp" if !val.is_empty() => {
                    svc.uptime = Some(val.to_string());
                }
                _ => {}
            }
        }
    }

    Ok(svc)
}

/// Restart a service.
///
/// Shadow for: `biomeOS gate.service.restart`
pub async fn restart(config: &ShadowConfig, unit: &str) -> Result<ServiceStatus> {
    ssh::exec(config, &format!("systemctl restart {unit}")).await?;

    status(config, unit).await
}

/// Get recent logs for a service.
///
/// Shadow for: `biomeOS gate.service.logs`
pub async fn logs(config: &ShadowConfig, unit: &str, lines: u32) -> Result<String> {
    ssh::exec(
        config,
        &format!("journalctl -u {unit} --no-pager -n {lines}"),
    )
    .await
}

fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    const GIB: u64 = MIB * 1024;

    if bytes >= GIB {
        format!("{:.1}Gi", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.1}Mi", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1}Ki", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes}B")
    }
}
