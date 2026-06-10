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
            let n_pos = if trimmed.contains("In sync:") { 2 } else { 1 };
            if let Some(n) = trimmed.split_whitespace().nth(n_pos) {
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

// ── Bootstrap ───────────────────────────────────────────────────────

/// Result of a single bootstrap phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapPhase {
    /// Phase identifier (e.g. "depot.fetch").
    pub name: String,
    /// Whether this phase succeeded.
    pub ok: bool,
    /// Human-readable outcome detail.
    pub detail: String,
}

/// Full result of a `gate.bootstrap` run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapResult {
    /// Name of the gate being enrolled.
    pub gate_name: String,
    /// Detected architecture triple.
    pub arch: String,
    /// Per-phase results.
    pub phases: Vec<BootstrapPhase>,
    /// Whether all phases passed (gate is enrolled).
    pub all_pass: bool,
}

/// Orchestrate full gate enrollment in one command.
///
/// Phases: detect arch → fetch depot → verify checksums → configure mesh → start NUCLEUS → health sweep.
pub async fn bootstrap(config: &ShadowConfig, gate_name: &str) -> Result<BootstrapResult> {
    use std::fmt::Write;
    let arch = crate::plasmid::detect_target_triple();
    let mut phases: Vec<BootstrapPhase> = Vec::new();

    // Phase 1: Detect architecture
    phases.push(BootstrapPhase {
        name: "arch.detect".into(),
        ok: true,
        detail: arch.clone(),
    });

    // Phase 2: Fetch all primals from WAN depot
    let fetch_args = crate::plasmid::FetchArgs {
        source: crate::plasmid::FetchSource::Wan,
        primal: None,
        release_tag: None,
        force: true,
        dry_run: false,
        dest: None,
    };
    let fetch_result = crate::plasmid::fetch(config, &fetch_args).await;
    let (fetch_ok, fetch_detail) = match &fetch_result {
        Ok(outcome) => {
            let downloaded = outcome
                .data
                .as_ref()
                .and_then(|d| d.get("downloaded"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            let failed = outcome
                .data
                .as_ref()
                .and_then(|d| d.get("failed"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            (
                failed == 0,
                format!("{downloaded} downloaded, {failed} failed"),
            )
        }
        Err(e) => (false, format!("fetch error: {e}")),
    };
    phases.push(BootstrapPhase {
        name: "depot.fetch".into(),
        ok: fetch_ok,
        detail: fetch_detail,
    });

    // Phase 3: Verify checksums (BLAKE3)
    let verify_result = verify_local_depot(&arch);
    phases.push(BootstrapPhase {
        name: "checksum.verify".into(),
        ok: verify_result.0,
        detail: verify_result.1,
    });

    // Phase 4: Configure mesh (songbird init to VPS relay)
    let mesh_result = configure_mesh(gate_name, &arch).await;
    phases.push(BootstrapPhase {
        name: "mesh.configure".into(),
        ok: mesh_result.0,
        detail: mesh_result.1,
    });

    // Phase 5: Start NUCLEUS primals
    let start_result = start_nucleus_primals(&arch);
    phases.push(BootstrapPhase {
        name: "nucleus.start".into(),
        ok: start_result.0,
        detail: start_result.1,
    });

    // Phase 6: Health sweep — verify processes running
    let health_result = health_sweep(&arch).await;
    phases.push(BootstrapPhase {
        name: "health.sweep".into(),
        ok: health_result.0,
        detail: health_result.1,
    });

    let all_pass = phases.iter().all(|p| p.ok);

    // Format human-readable report
    let mut report = format!("=== Gate Bootstrap: {gate_name} ({arch}) ===\n\n");
    for phase in &phases {
        let status = if phase.ok { "PASS" } else { "FAIL" };
        let _ = writeln!(report, "  [{status}] {}: {}", phase.name, phase.detail);
    }
    let passed = phases.iter().filter(|p| p.ok).count();
    let _ = write!(
        report,
        "\n  Result: {passed}/{} phases passed",
        phases.len()
    );
    if all_pass {
        let _ = write!(report, " — {gate_name} is ENROLLED");
    }

    Ok(BootstrapResult {
        gate_name: gate_name.to_string(),
        arch,
        phases,
        all_pass,
    })
}

/// Verify BLAKE3 checksums of local depot binaries against checksums.toml.
fn verify_local_depot(arch: &str) -> (bool, String) {
    #[derive(serde::Deserialize)]
    struct ChecksumFile {
        #[serde(flatten)]
        targets:
            std::collections::BTreeMap<String, std::collections::BTreeMap<String, ChecksumEntry>>,
    }
    #[derive(serde::Deserialize)]
    struct ChecksumEntry {
        blake3: String,
        #[serde(rename = "size")]
        _size: u64,
    }

    let dest_root = resolve_plasmidbin_dir();
    let bin_dir = dest_root.join("primals").join(arch);

    let checksums_path = if dest_root.join("checksums.toml").exists() {
        dest_root.join("checksums.toml")
    } else if let Ok(workspace) = crate::temporal::resolve_workspace_root() {
        let ws_path = workspace.join("infra/plasmidBin/checksums.toml");
        if ws_path.exists() {
            ws_path
        } else {
            return (
                false,
                "checksums.toml not found in depot or workspace".into(),
            );
        }
    } else {
        return (false, "checksums.toml not found".into());
    };

    let Ok(content) = std::fs::read_to_string(&checksums_path) else {
        return (false, "cannot read checksums.toml".into());
    };

    let parsed: ChecksumFile = match toml::from_str(&content) {
        Ok(p) => p,
        Err(e) => return (false, format!("parse error: {e}")),
    };

    let Some(entries) = parsed.targets.get(arch) else {
        return (false, format!("no [{arch}] section in checksums.toml"));
    };

    let mut verified = 0u32;
    let mut failed = 0u32;
    let mut missing = 0u32;

    for (name, entry) in entries {
        let bin_path = bin_dir.join(name);
        if !bin_path.exists() {
            missing += 1;
            continue;
        }
        let hash = crate::plasmid::compute_blake3_file(&bin_path);
        if hash == entry.blake3 {
            verified += 1;
        } else {
            failed += 1;
        }
    }

    let ok = failed == 0 && missing == 0;
    (
        ok,
        format!("{verified} verified, {failed} hash mismatch, {missing} missing"),
    )
}

/// Configure songbird mesh — init `node_id` and peer to VPS relay.
async fn configure_mesh(gate_name: &str, arch: &str) -> (bool, String) {
    let dest_root = resolve_plasmidbin_dir();
    let songbird_bin = dest_root.join("primals").join(arch).join("songbird");

    if !songbird_bin.exists() {
        return (false, "songbird binary not found".into());
    }

    let socket_dir = std::env::var("BIOMEOS_SOCKET_DIR").unwrap_or_else(|_| {
        let uid = std::env::var("UID")
            .or_else(|_| std::env::var("EUID"))
            .unwrap_or_else(|_| {
                std::fs::read_to_string("/proc/self/loginuid")
                    .unwrap_or_else(|_| "1000".into())
                    .trim()
                    .to_string()
            });
        format!("/run/user/{uid}/biomeos")
    });
    let socket_path = format!("{socket_dir}/songbird.sock");

    if !std::path::Path::new(&socket_path).exists() {
        return (
            false,
            format!("songbird socket not found at {socket_path} — start songbird first"),
        );
    }

    let vps_peer = std::env::var(cellmembrane_types::service::ENV_VPS_MESH_PEER)
        .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_VPS_MESH_PEER.into());

    let mesh_init = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "mesh.init",
        "params": {
            "node_id": gate_name,
            "peers": [vps_peer],
        },
        "id": 1
    });

    let output = tokio::process::Command::new("socat")
        .args(["-", &format!("UNIX-CONNECT:{socket_path}")])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();

    let Ok(mut child) = output else {
        return (false, "failed to spawn socat".into());
    };

    if let Some(stdin) = child.stdin.as_mut() {
        use tokio::io::AsyncWriteExt;
        let payload = mesh_init.to_string();
        if stdin.write_all(payload.as_bytes()).await.is_err() {
            return (false, "failed to write to socat stdin".into());
        }
        let _ = stdin.shutdown().await;
    }

    let result =
        tokio::time::timeout(std::time::Duration::from_secs(5), child.wait_with_output()).await;

    match result {
        Ok(Ok(out)) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            if stdout.contains("\"result\"") || stdout.contains("\"ok\"") {
                (true, format!("mesh.init sent to {vps_peer} as {gate_name}"))
            } else {
                (
                    true,
                    format!("mesh.init sent (response: {})", stdout.trim()),
                )
            }
        }
        Ok(Ok(out)) => (false, format!("socat exit {}", out.status)),
        Ok(Err(e)) => (false, format!("socat error: {e}")),
        Err(_) => (false, "mesh.init timed out after 5s".into()),
    }
}

/// Start nucleus primals in background.
fn start_nucleus_primals(arch: &str) -> (bool, String) {
    let dest_root = resolve_plasmidbin_dir();
    let bin_dir = dest_root.join("primals").join(arch);

    let primals = crate::plasmid::nucleus_primals();
    let mut started = 0u32;
    let mut failed = 0u32;
    let mut skipped = 0u32;

    for primal in &primals {
        let bin_path = bin_dir.join(primal);
        if !bin_path.exists() {
            failed += 1;
            continue;
        }

        if *primal == "songbird" {
            skipped += 1;
            continue;
        }

        let spawn_result = tokio::process::Command::new(&bin_path)
            .arg("server")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();

        match spawn_result {
            Ok(_) => started += 1,
            Err(_) => failed += 1,
        }
    }

    let ok = failed == 0;
    (
        ok,
        format!("{started} started, {skipped} skipped (pre-running), {failed} failed"),
    )
}

/// Health sweep — check which primals have a live process.
async fn health_sweep(arch: &str) -> (bool, String) {
    let dest_root = resolve_plasmidbin_dir();
    let bin_dir = dest_root.join("primals").join(arch);

    let primals = crate::plasmid::nucleus_primals();
    let mut alive = 0u32;
    let mut dead = 0u32;

    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    for primal in &primals {
        let bin_path = bin_dir.join(primal);
        if !bin_path.exists() {
            dead += 1;
            continue;
        }

        let output = tokio::process::Command::new("pgrep")
            .args(["-f", &format!("{primal}.*server")])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
            .await;

        match output {
            Ok(o) if o.status.success() => alive += 1,
            _ => dead += 1,
        }
    }

    let total = alive + dead;
    let ok = dead == 0;
    (ok, format!("{alive}/{total} primals alive"))
}

/// Resolve the local plasmidBin directory.
fn resolve_plasmidbin_dir() -> std::path::PathBuf {
    crate::plasmid::resolve_path(None, "ECOPRIMALS_PLASMID_BIN", || {
        let data_home = std::env::var(cellmembrane_types::service::ENV_XDG_DATA_HOME)
            .unwrap_or_else(|_| {
                let home = std::env::var(cellmembrane_types::service::ENV_HOME)
                    .unwrap_or_else(|_| "/tmp".into());
                format!("{home}/.local/share")
            });
        std::path::PathBuf::from(format!("{data_home}/ecoPrimals/plasmidBin"))
    })
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
