// SPDX-License-Identifier: AGPL-3.0-or-later

//! Gate operations — VPS status, workspace sync, parity checks, local bootstrap/status.
//!
//! Shadow domain mapping:
//!   - `gate.info`      → VPS workspace info
//!   - `gate.pull`      → `membrane temporal.cascade` on VPS
//!   - `gate.check`     → `membrane temporal.check` on VPS
//!   - `gate.bootstrap` → local gate enrollment
//!   - `gate.status`    → local gate health probe
//!   - `gate.provision` → cloud droplet provisioning (fieldMouse canary)

pub mod bootstrap;
pub(crate) mod enroll;
pub mod health;
mod interface;
mod local;
mod mesh;
pub(crate) mod nucleus;
pub mod preflight;
pub(crate) mod systemd_units;
pub(crate) mod sporeprint;
pub(crate) mod sovereignty;
pub mod verify;

pub use bootstrap::{BootstrapPhase, BootstrapResult, bootstrap};
pub use enroll::{EnrollResult, enroll};
pub use health::{GateStatus, StatusProbe, status};

/// Outcome of a gate subsystem probe — typed replacement for `(bool, String)` tuples.
pub(crate) struct ProbeResult {
    pub ok: bool,
    pub detail: String,
}

impl ProbeResult {
    pub fn pass(detail: impl Into<String>) -> Self {
        Self { ok: true, detail: detail.into() }
    }

    pub fn fail(detail: impl Into<String>) -> Self {
        Self { ok: false, detail: detail.into() }
    }
}

use local::resolve_install_base;
pub use local::resolve_local_gate_identity;
pub(crate) use local::resolve_plasmidbin_dir;

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
/// Uses the manifest `sync.default_source` if available, falling back to "forgejo".
///
/// Shadow for: `biomeOS gate.pull`
pub async fn pull(config: &ShadowConfig) -> Result<SyncResult> {
    let source = crate::temporal::resolve_workspace_root()
        .ok()
        .and_then(|r| crate::manifest::load_from_workspace(&r).ok())
        .map_or_else(
            || cellmembrane_types::CascadeSource::Forgejo,
            |m| m.sync.default_source,
        );
    let root = &config.vps_root;
    let cmd = format!("cd {root} && membrane temporal.cascade --source {source} 2>&1");
    let output = ssh::exec(config, &cmd).await?;
    Ok(parse_sync_output(&output))
}

/// Run parity check on the VPS workspace.
///
/// Shadow for: `biomeOS gate.check`
pub async fn check(config: &ShadowConfig) -> Result<SyncResult> {
    let root = &config.vps_root;
    let cmd = format!("cd {root} && membrane temporal.check 2>&1");
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

        if trimmed.starts_with("Repos:")
            && let Some(n) = trimmed.split_whitespace().nth(1)
        {
            result.total = n.parse().unwrap_or(0);
        }
        if (trimmed.contains("In sync:") || trimmed.contains("Parity:"))
            && let Some(n) = trimmed
                .split_whitespace()
                .nth(if trimmed.contains("In sync:") { 2 } else { 1 })
        {
            result.synced = n.parse().unwrap_or(0);
        }
        if (trimmed.starts_with("Drifted:") || trimmed.starts_with("Diverge:"))
            && let Some(n) = trimmed.split_whitespace().nth(1)
        {
            result.drifted = n.parse().unwrap_or(0);
        }
        if trimmed.starts_with("Not cloned:")
            && let Some(n) = trimmed.split_whitespace().nth(2)
        {
            result.missing = n.parse().unwrap_or(0);
        }
        if (trimmed.starts_with("Pulled:") || trimmed.starts_with("Synced:"))
            && let Some(n) = trimmed.split_whitespace().nth(1)
        {
            result.synced = n.parse().unwrap_or(0);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sync_output_full() {
        let output = "Gate: eastGate\nRepos: 22\nIn sync: 20\nDrifted: 1\nNot cloned: 1\n";
        let r = parse_sync_output(output);
        assert_eq!(r.gate, "eastGate");
        assert_eq!(r.total, 22);
        assert_eq!(r.synced, 20);
        assert_eq!(r.drifted, 1);
        assert_eq!(r.missing, 1);
    }

    #[test]
    fn parse_sync_output_parity_variant() {
        let output = "Gate: golgiBody\nRepos: 38\nParity: 38\nDrifted: 0\n";
        let r = parse_sync_output(output);
        assert_eq!(r.gate, "golgiBody");
        assert_eq!(r.total, 38);
        assert_eq!(r.synced, 38);
        assert_eq!(r.drifted, 0);
    }

    #[test]
    fn parse_sync_output_pulled_variant() {
        let output = "Gate: strandGate\nRepos: 10\nPulled: 8\nDrifted: 2\n";
        let r = parse_sync_output(output);
        assert_eq!(r.gate, "strandGate");
        assert_eq!(r.synced, 8);
        assert_eq!(r.drifted, 2);
    }

    #[test]
    fn parse_sync_output_empty() {
        let r = parse_sync_output("");
        assert_eq!(r.gate, "");
        assert_eq!(r.total, 0);
        assert_eq!(r.synced, 0);
    }

    #[test]
    fn gate_info_serializes() {
        let info = GateInfo {
            hostname: "membrane-relay".into(),
            uptime: "up 2 weeks".into(),
            gate_identity: "golgiBody".into(),
            load: "0.15 0.10 0.05".into(),
            memory: "1.2G/4.0G".into(),
            disk: "12G/50G(24%)".into(),
            services: vec![ServiceEntry {
                unit: "beardog-membrane.service".into(),
                state: "running".into(),
            }],
            repo_count: 38,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("membrane-relay"));
        assert!(json.contains("beardog-membrane"));
    }

    #[test]
    fn sync_result_serializes() {
        let r = SyncResult {
            gate: "eastGate".into(),
            total: 22,
            synced: 20,
            drifted: 1,
            missing: 1,
            raw_output: "...".into(),
        };
        let json = serde_json::to_string(&r).unwrap();
        let deser: SyncResult = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.gate, "eastGate");
        assert_eq!(deser.synced, 20);
    }
}
