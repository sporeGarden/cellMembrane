// SPDX-License-Identifier: AGPL-3.0-or-later

//! Gate operations — VPS status, workspace sync, parity checks.
//!
//! Shadow domain mapping:
//!   - `gate.info`  → biomeOS gate.info
//!   - `gate.pull`  → biomeOS gate.pull (cascade-pull on golgiBody)
//!   - `gate.check` → biomeOS gate.check (parity check)

use crate::config::ShadowConfig;
use crate::error::Result;
use crate::ssh;
use serde::{Deserialize, Serialize};

/// VPS gate information — system health, identity, and workspace state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateInfo {
    /// System hostname (e.g. `membrane-relay`).
    pub hostname: String,
    /// Human-readable uptime (e.g. `up 2 weeks, 3 hours`).
    pub uptime: String,
    /// Gate identity from `{vps_root}/.gate`.
    pub gate_identity: String,
    /// Load average (1/5/15 min).
    pub load: String,
    /// Memory usage as `used/total`.
    pub memory: String,
    /// Disk usage as `used/total(percent)`.
    pub disk: String,
    /// Running membrane services.
    pub services: Vec<ServiceEntry>,
    /// Number of git repos in the VPS workspace.
    pub repo_count: u32,
}

/// A running systemd service entry from the VPS.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceEntry {
    /// Systemd unit name (e.g. `beardog-membrane.service`).
    pub unit: String,
    /// Sub-state (e.g. `running`).
    pub state: String,
}

/// Result of a cascade-pull or parity check on the VPS workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncResult {
    /// Gate that was synced.
    pub gate: String,
    /// Total repos in the gate profile.
    pub total: u32,
    /// Repos at parity or successfully pulled.
    pub synced: u32,
    /// Repos with commit drift between local and remote.
    pub drifted: u32,
    /// Repos not cloned on this gate.
    pub missing: u32,
    /// Full cascade-pull output for diagnostics.
    pub raw_output: String,
}

/// Get VPS system info, service list, and workspace state.
///
/// Shadow for: `biomeOS gate.info`
pub async fn info(config: &ShadowConfig) -> Result<GateInfo> {
    let root = &config.vps_root;
    let filter = &config.service_filter;
    let script = format!(
        r#"
echo "HOSTNAME:$(hostname)"
echo "UPTIME:$(uptime -p)"
echo "GATE:$(cat {root}/.gate 2>/dev/null || echo UNKNOWN)"
echo "LOAD:$(cat /proc/loadavg | cut -d' ' -f1-3)"
echo "MEMORY:$(free -h | awk '/Mem:/{{printf "%s/%s", $3, $2}}')"
echo "DISK:$(df -h / | awk 'NR==2{{printf "%s/%s(%s)", $3, $2, $5}}')"
echo "REPOS:$(find {root} -maxdepth 3 -name .git -type d 2>/dev/null | wc -l)"
echo "---SERVICES---"
systemctl list-units --type=service --state=running --no-pager --no-legend | \
    grep -E '{filter}' | \
    awk '{{print $1 ":" $4}}'
"#
    );

    let output = ssh::exec(config, &script).await?;

    let mut info = GateInfo {
        hostname: String::new(),
        uptime: String::new(),
        gate_identity: String::new(),
        load: String::new(),
        memory: String::new(),
        disk: String::new(),
        services: Vec::new(),
        repo_count: 0,
    };

    let mut in_services = false;
    for line in output.lines() {
        if line == "---SERVICES---" {
            in_services = true;
            continue;
        }

        if in_services {
            if let Some((unit, state)) = line.split_once(':') {
                info.services.push(ServiceEntry {
                    unit: unit.trim().to_string(),
                    state: state.trim().to_string(),
                });
            }
            continue;
        }

        if let Some((key, val)) = line.split_once(':') {
            match key {
                "HOSTNAME" => info.hostname = val.trim().to_string(),
                "UPTIME" => info.uptime = val.trim().to_string(),
                "GATE" => info.gate_identity = val.trim().to_string(),
                "LOAD" => info.load = val.trim().to_string(),
                "MEMORY" => info.memory = val.trim().to_string(),
                "DISK" => info.disk = val.trim().to_string(),
                "REPOS" => {
                    info.repo_count = val.trim().parse().unwrap_or(0);
                }
                _ => {}
            }
        }
    }

    Ok(info)
}

/// Run cascade sync on the VPS via the `membrane` binary.
///
/// Shadow for: `biomeOS gate.pull`
/// Uses the Rust `membrane temporal.cascade` command if installed on the VPS,
/// falling back to `cascade-pull.sh` if the binary is not yet deployed.
/// Gate identity is resolved from `$VPS_ROOT/.gate` — no hardcoded gate names.
///
/// # Errors
/// Returns `ShadowError::Ssh` if the SSH connection or remote command fails.
pub async fn pull(config: &ShadowConfig) -> Result<SyncResult> {
    let root = &config.vps_root;
    let cmd = format!(
        "cd {root} && GATE=$(cat {root}/.gate 2>/dev/null || echo auto) && \
         if command -v membrane >/dev/null 2>&1; then \
           membrane temporal.cascade --source forgejo 2>&1; \
         else \
           infra/wateringHole/scripts/cascade-pull.sh --gate \"$GATE\" --source temporal; \
         fi",
    );
    let output = ssh::exec(config, &cmd).await?;
    Ok(parse_sync_output(&output))
}

/// Run parity check on the VPS workspace.
///
/// Shadow for: `biomeOS gate.check`
/// Uses `membrane temporal.check-all` if installed, falls back to bash.
/// Gate identity is resolved from `$VPS_ROOT/.gate` — no hardcoded gate names.
///
/// # Errors
/// Returns `ShadowError::Ssh` if the SSH connection or remote command fails.
pub async fn check(config: &ShadowConfig) -> Result<SyncResult> {
    let root = &config.vps_root;
    let cmd = format!(
        "cd {root} && GATE=$(cat {root}/.gate 2>/dev/null || echo auto) && \
         if command -v membrane >/dev/null 2>&1; then \
           membrane temporal.check 2>&1; \
         else \
           infra/wateringHole/scripts/cascade-pull.sh --gate \"$GATE\" --source temporal --check; \
         fi",
    );
    let output = ssh::exec(config, &cmd).await?;
    Ok(parse_sync_output(&output))
}

fn parse_sync_output(output: &str) -> SyncResult {
    let mut result = SyncResult {
        gate: String::new(),
        total: 0,
        synced: 0,
        drifted: 0,
        missing: 0,
        raw_output: output.to_string(),
    };

    for line in output.lines() {
        let trimmed = line.trim();

        if let Some(rest) = trimmed.strip_prefix("Gate:") {
            result.gate = rest.trim().to_string();
        }

        if trimmed.starts_with("Repos:") {
            if let Some(n) = trimmed.split_whitespace().nth(1) {
                result.total = n.parse().unwrap_or(0);
            }
        }
        if trimmed.contains("In sync:") || trimmed.contains("Parity:") {
            if let Some(n) = trimmed.split_whitespace().nth(2) {
                result.synced = n.parse().unwrap_or(0);
            }
        }
        if trimmed.starts_with("Drifted:") || trimmed.starts_with("Diverge:") {
            if let Some(n) = trimmed.split_whitespace().nth(1) {
                result.drifted = n.parse().unwrap_or(0);
            }
        }
        if trimmed.starts_with("Not cloned:") {
            if let Some(n) = trimmed.split_whitespace().nth(2) {
                result.missing = n.parse().unwrap_or(0);
            }
        }
        if trimmed.starts_with("Pulled:") || trimmed.starts_with("Synced:") {
            if let Some(n) = trimmed.split_whitespace().nth(1) {
                result.synced = n.parse().unwrap_or(0);
            }
        }
    }

    result
}
