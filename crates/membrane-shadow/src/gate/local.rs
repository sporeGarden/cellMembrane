// SPDX-License-Identifier: AGPL-3.0-or-later

//! Local gate operations — bootstrap, status, and health probes.
//!
//! These operations run on the local machine (unlike `info`/`pull`/`check`
//! which operate on the VPS via SSH).

use crate::config::ShadowConfig;
use crate::error::Result;
use serde::{Deserialize, Serialize};

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
/// With `dry_run = true`, reports what would happen without executing side effects.
pub async fn bootstrap(
    config: &ShadowConfig,
    gate_name: &str,
    dry_run: bool,
) -> Result<BootstrapResult> {
    let arch = crate::plasmid::detect_target_triple();
    let mut phases: Vec<BootstrapPhase> = Vec::new();

    phases.push(BootstrapPhase {
        name: "arch.detect".into(),
        ok: true,
        detail: arch.clone(),
    });

    phases.push(bootstrap_fetch_phase(config, dry_run).await);

    let verify_result = verify_local_depot(&arch);
    phases.push(BootstrapPhase {
        name: "checksum.verify".into(),
        ok: verify_result.0,
        detail: if dry_run {
            format!("dry-run: would verify — current: {}", verify_result.1)
        } else {
            verify_result.1
        },
    });

    phases.push(bootstrap_mesh_phase(gate_name, &arch, dry_run).await);
    phases.push(bootstrap_nucleus_phase(&arch, dry_run));
    phases.push(bootstrap_health_phase(&arch, dry_run).await);

    let all_pass = phases.iter().all(|p| p.ok);

    Ok(BootstrapResult {
        gate_name: gate_name.to_string(),
        arch,
        phases,
        all_pass,
    })
}

async fn bootstrap_fetch_phase(config: &ShadowConfig, dry_run: bool) -> BootstrapPhase {
    if dry_run {
        return BootstrapPhase {
            name: "depot.fetch".into(),
            ok: true,
            detail: "dry-run: would fetch all primals from WAN depot".into(),
        };
    }
    let fetch_args = crate::plasmid::FetchArgs {
        source: crate::plasmid::FetchSource::Wan,
        primal: None,
        release_tag: None,
        force: true,
        dry_run: false,
        dest: None,
    };
    let (ok, detail) = match crate::plasmid::fetch(config, &fetch_args).await {
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
    BootstrapPhase {
        name: "depot.fetch".into(),
        ok,
        detail,
    }
}

async fn bootstrap_mesh_phase(gate_name: &str, arch: &str, dry_run: bool) -> BootstrapPhase {
    if dry_run {
        let vps_peer = std::env::var(cellmembrane_types::service::ENV_VPS_MESH_PEER)
            .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_VPS_MESH_PEER.into());
        return BootstrapPhase {
            name: "mesh.configure".into(),
            ok: true,
            detail: format!("dry-run: would mesh.init as {gate_name} → {vps_peer}"),
        };
    }
    let (ok, detail) = configure_mesh(gate_name, arch).await;
    BootstrapPhase {
        name: "mesh.configure".into(),
        ok,
        detail,
    }
}

fn bootstrap_nucleus_phase(arch: &str, dry_run: bool) -> BootstrapPhase {
    if dry_run {
        let count = crate::plasmid::nucleus_primals().len();
        return BootstrapPhase {
            name: "nucleus.start".into(),
            ok: true,
            detail: format!("dry-run: would start {count} primals"),
        };
    }
    let (ok, detail) = start_nucleus_primals(arch);
    BootstrapPhase {
        name: "nucleus.start".into(),
        ok,
        detail,
    }
}

async fn bootstrap_health_phase(arch: &str, dry_run: bool) -> BootstrapPhase {
    if dry_run {
        return BootstrapPhase {
            name: "health.sweep".into(),
            ok: true,
            detail: "dry-run: would verify process liveness".into(),
        };
    }
    let (ok, detail) = health_sweep(arch).await;
    BootstrapPhase {
        name: "health.sweep".into(),
        ok,
        detail,
    }
}

// ── Status ──────────────────────────────────────────────────────────

/// Health report for an already-bootstrapped gate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateStatus {
    /// Gate identity (from local `.gate` or env).
    pub gate_name: String,
    /// Architecture triple.
    pub arch: String,
    /// Per-subsystem probe results.
    pub probes: Vec<StatusProbe>,
    /// Overall gate health — all probes pass.
    pub healthy: bool,
}

/// A single status probe (e.g. depot integrity, mesh connectivity).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusProbe {
    /// Probe identifier (e.g. "depot.integrity").
    pub name: String,
    /// Pass/fail.
    pub ok: bool,
    /// Human-readable detail.
    pub detail: String,
}

/// Query the status of an already-bootstrapped gate (local).
///
/// Probes: depot integrity → mesh reachability → primal processes → depot freshness.
pub async fn status() -> Result<GateStatus> {
    let arch = crate::plasmid::detect_target_triple();
    let gate_name = resolve_local_gate_identity();
    let mut probes: Vec<StatusProbe> = Vec::new();

    let (depot_ok, depot_detail) = verify_local_depot(&arch);
    probes.push(StatusProbe {
        name: "depot.integrity".into(),
        ok: depot_ok,
        detail: depot_detail,
    });

    let (mesh_ok, mesh_detail) = probe_mesh_status().await;
    probes.push(StatusProbe {
        name: "mesh.reachability".into(),
        ok: mesh_ok,
        detail: mesh_detail,
    });

    let (procs_ok, procs_detail) = health_sweep(&arch).await;
    probes.push(StatusProbe {
        name: "primals.alive".into(),
        ok: procs_ok,
        detail: procs_detail,
    });

    let (fresh_ok, fresh_detail) = probe_depot_freshness(&arch);
    probes.push(StatusProbe {
        name: "depot.freshness".into(),
        ok: fresh_ok,
        detail: fresh_detail,
    });

    let healthy = probes.iter().all(|p| p.ok);

    Ok(GateStatus {
        gate_name,
        arch,
        probes,
        healthy,
    })
}

// ── Helpers ─────────────────────────────────────────────────────────

fn resolve_local_gate_identity() -> String {
    if let Ok(name) = std::env::var("MEMBRANE_GATE_NAME") {
        return name;
    }
    let candidates = [
        std::path::PathBuf::from("/opt/ecoPrimals/.gate"),
        dirs_home().join(".gate"),
    ];
    for path in &candidates {
        if let Ok(content) = std::fs::read_to_string(path) {
            let trimmed = content.trim().to_string();
            if !trimmed.is_empty() {
                return trimmed;
            }
        }
    }
    "unknown".into()
}

fn dirs_home() -> std::path::PathBuf {
    match std::env::var(cellmembrane_types::service::ENV_HOME) {
        Ok(h) => std::path::PathBuf::from(h),
        Err(_) => std::path::PathBuf::from("/tmp"),
    }
}

async fn probe_mesh_status() -> (bool, String) {
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
        return (false, "songbird socket not found".into());
    }

    let mesh_status = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "mesh.status",
        "params": {},
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
        let payload = mesh_status.to_string();
        if stdin.write_all(payload.as_bytes()).await.is_err() {
            return (false, "socat stdin write failed".into());
        }
        let _ = stdin.shutdown().await;
    }

    let result =
        tokio::time::timeout(std::time::Duration::from_secs(5), child.wait_with_output()).await;

    match result {
        Ok(Ok(out)) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(stdout.trim()) {
                let peers = json
                    .get("result")
                    .and_then(|r| r.get("peers"))
                    .and_then(serde_json::Value::as_array)
                    .map_or(0, Vec::len);
                let reachable = json
                    .get("result")
                    .and_then(|r| r.get("reachable"))
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                (
                    reachable > 0 || peers > 0,
                    format!("{peers} peers, {reachable} reachable"),
                )
            } else if stdout.contains("\"result\"") {
                (true, "mesh responding".into())
            } else {
                (false, format!("unexpected: {}", stdout.trim()))
            }
        }
        Ok(Ok(out)) => (false, format!("socat exit {}", out.status)),
        Ok(Err(e)) => (false, format!("socat error: {e}")),
        Err(_) => (false, "mesh.status timed out after 5s".into()),
    }
}

fn probe_depot_freshness(arch: &str) -> (bool, String) {
    let dest_root = resolve_plasmidbin_dir();
    let bin_dir = dest_root.join("primals").join(arch);

    if !bin_dir.is_dir() {
        return (false, format!("depot dir missing: {}", bin_dir.display()));
    }

    let primals = crate::plasmid::nucleus_primals();
    let mut present = 0u32;
    let mut missing = 0u32;

    for primal in &primals {
        if bin_dir.join(primal).is_file() {
            present += 1;
        } else {
            missing += 1;
        }
    }

    let total = present + missing;
    let ok = missing == 0;
    (ok, format!("{present}/{total} binaries present"))
}

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
