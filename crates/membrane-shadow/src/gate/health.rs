// SPDX-License-Identifier: AGPL-3.0-or-later

//! Gate health probes — JSON-RPC UDS queries, process detection, depot status.
//!
//! Replaces shell-based socat/bash/pgrep probes with native async Rust.

use serde::{Deserialize, Serialize};
use std::path::Path;

const STALE_THRESHOLD_DAYS: u64 = 7;
const SECS_PER_DAY: u64 = 86_400;
const SECS_PER_HOUR: u64 = 3_600;

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

/// Query the status of an already-bootstrapped gate (local).
///
/// Probes: depot integrity → mesh reachability → primal processes → depot freshness → sovereignty.
pub async fn status() -> crate::error::Result<GateStatus> {
    let arch = crate::plasmid::detect_target_triple();
    let gate_name = super::resolve_local_gate_identity();
    let mut probes: Vec<StatusProbe> = Vec::new();

    let arch_clone = arch.clone();
    let (depot_ok, depot_detail) =
        tokio::task::spawn_blocking(move || super::verify::verify_local_depot(&arch_clone))
            .await
            .unwrap_or_else(|_| (false, "depot verify task panicked".into()));
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

    let arch_for_freshness = arch.clone();
    let (fresh_ok, fresh_detail) =
        tokio::task::spawn_blocking(move || probe_depot_freshness(&arch_for_freshness))
            .await
            .unwrap_or_else(|_| (false, "freshness probe panicked".into()));
    probes.push(StatusProbe {
        name: "depot.freshness".into(),
        ok: fresh_ok,
        detail: fresh_detail,
    });

    let sovereignty_probes = super::sovereignty::probe_sovereignty().await;
    probes.extend(sovereignty_probes);

    let vcs_probe = probe_vcs_parity().await;
    probes.push(vcs_probe);

    let healthy = probes.iter().all(|p| p.ok);

    Ok(GateStatus {
        gate_name,
        arch,
        probes,
        healthy,
    })
}

/// Probe mesh status via neuralAPI-routed `capability.call` with fallback to direct UDS.
async fn probe_mesh_status() -> (bool, String) {
    if let Some(result) =
        crate::bridge::try_bridge("mesh_relay", "mesh.status", serde_json::json!({})).await
    {
        return parse_mesh_json(&result);
    }

    let socket_path = resolve_mesh_relay_socket();

    if !Path::new(&socket_path).exists() {
        return (false, "mesh relay socket not found".into());
    }

    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "mesh.status",
        "params": {},
        "id": 1
    });

    match uds_jsonrpc_call(&socket_path, &request.to_string()).await {
        Ok(response) => parse_mesh_response(&response),
        Err(e) => (false, e),
    }
}

fn parse_mesh_json(result: &serde_json::Value) -> (bool, String) {
    let peers = result
        .get("reachable_peers")
        .or_else(|| result.get("peers"))
        .and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_array().map(|a| u64::try_from(a.len()).unwrap_or(0)))
        })
        .unwrap_or(0);
    let reachable = result
        .get("reachable")
        .or_else(|| result.get("reachable_peers"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let federation = result
        .get("relay_enabled")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);

    let detail = if federation && peers == 0 {
        format!("hub listening, {reachable} reachable (no inbound peers yet)")
    } else {
        format!("{peers} peers, {reachable} reachable")
    };

    (reachable > 0 || peers > 0 || federation, detail)
}

fn parse_mesh_response(response: &str) -> (bool, String) {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(response.trim()) else {
        if response.contains("\"result\"") {
            return (true, "mesh responding".into());
        }
        return (false, format!("unexpected: {}", response.trim()));
    };

    if let Some(err) = json.get("error") {
        let msg = err
            .get("message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown error");
        return (false, format!("mesh error: {msg}"));
    }

    let result = json.get("result");
    let peers = result
        .and_then(|r| r.get("reachable_peers").or_else(|| r.get("peers")))
        .and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_array().map(|a| u64::try_from(a.len()).unwrap_or(0)))
        })
        .unwrap_or(0);
    let reachable = result
        .and_then(|r| r.get("reachable").or_else(|| r.get("reachable_peers")))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let federation = result
        .and_then(|r| r.get("relay_enabled"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);

    let detail = if federation && peers == 0 {
        format!("hub listening, {reachable} reachable (no inbound peers yet)")
    } else {
        format!("{peers} peers, {reachable} reachable")
    };

    (reachable > 0 || peers > 0 || federation, detail)
}

/// Health sweep: probe each primal via JSON-RPC, fall back to process detection.
pub async fn health_sweep(arch: &str) -> (bool, String) {
    let dest_root = super::resolve_plasmidbin_dir();
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

        let primal_name = (*primal).to_string();
        let pgrep_found = tokio::task::spawn_blocking(move || probe_primal_pgrep(&primal_name))
            .await
            .unwrap_or(false);
        if probe_primal_jsonrpc(primal).await || pgrep_found {
            alive += 1;
        } else {
            dead += 1;
        }
    }

    let total = alive + dead;
    let ok = dead == 0;
    (ok, format!("{alive}/{total} primals alive"))
}

/// Probe a primal via neuralAPI `capability.call` with fallback to direct UDS JSON-RPC.
///
/// Prefers routing through biomeOS neuralAPI when available — validates the full
/// orchestration stack. Falls back to direct UDS when neuralAPI is unavailable.
/// Any valid JSON-RPC response (including method-not-found errors) proves
/// the primal is alive.
async fn probe_primal_jsonrpc(primal: &str) -> bool {
    if let Some(result) = crate::bridge::try_bridge(primal, "health", serde_json::json!({})).await {
        return result.get("status").is_some() || result.is_object();
    }

    let socket_paths = resolve_primal_socket_paths(primal);
    let request = r#"{"jsonrpc":"2.0","method":"health","params":{},"id":1}"#;

    for socket_path in &socket_paths {
        if !Path::new(socket_path).exists() {
            continue;
        }

        if let Ok(response) = uds_jsonrpc_call(socket_path, request).await {
            if response.contains("\"jsonrpc\"")
                || response.contains("\"result\"")
                || response.contains("\"error\"")
            {
                return true;
            }
        }
    }

    false
}

/// Fallback: detect running process via /proc/*/comm (no external deps).
fn probe_primal_pgrep(primal: &str) -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !name
            .to_str()
            .is_some_and(|s| s.chars().all(|c| c.is_ascii_digit()))
        {
            continue;
        }
        let comm_path = entry.path().join("comm");
        if let Ok(comm) = std::fs::read_to_string(&comm_path) {
            if comm.trim() == primal {
                return true;
            }
        }
    }
    false
}

fn probe_depot_freshness(arch: &str) -> (bool, String) {
    let dest_root = super::resolve_plasmidbin_dir();
    let bin_dir = dest_root.join("primals").join(arch);

    if !bin_dir.is_dir() {
        return (false, format!("depot dir missing: {}", bin_dir.display()));
    }

    let primals = crate::plasmid::nucleus_primals();
    let mut present = 0u32;
    let mut missing = 0u32;
    let mut oldest_age_secs: u64 = 0;

    let now = std::time::SystemTime::now();
    for primal in &primals {
        let path = bin_dir.join(primal);
        if path.is_file() {
            present += 1;
            if let Ok(meta) = std::fs::metadata(&path) {
                if let Ok(modified) = meta.modified() {
                    if let Ok(age) = now.duration_since(modified) {
                        oldest_age_secs = oldest_age_secs.max(age.as_secs());
                    }
                }
            }
        } else {
            missing += 1;
        }
    }

    let total = present + missing;
    let age_days = oldest_age_secs / SECS_PER_DAY;
    let ok = missing == 0 && age_days < STALE_THRESHOLD_DAYS;

    let age_str = if oldest_age_secs > 0 {
        if age_days > 0 {
            format!(", oldest {age_days}d")
        } else {
            let hours = oldest_age_secs / SECS_PER_HOUR;
            format!(", oldest {hours}h")
        }
    } else {
        String::new()
    };

    (ok, format!("{present}/{total} binaries present{age_str}"))
}

/// VCS parity probe: check that origin and forgejo are at the same commit for
/// locally-cloned repos. Reports drift count — any drift is a WARN that auto-
/// reconciliation should resolve within the next cascade cycle.
async fn probe_vcs_parity() -> StatusProbe {
    let Ok(workspace) = crate::temporal::resolve_workspace_root() else {
        return StatusProbe {
            name: "vcs.parity".into(),
            ok: true,
            detail: "workspace not found (VPS/minimal)".into(),
        };
    };

    let local_paths: Vec<String> = crate::manifest::load_from_workspace_async(&workspace)
        .await
        .map_or_else(
            |_| vec!["infra/plasmidBin".into(), "infra/wateringHole".into()],
            |m| m.repos.values().map(|r| r.local_path.clone()).collect(),
        );

    let mut drift_count = 0u32;
    let mut checked = 0u32;

    for repo_path in &local_paths {
        let repo_dir = workspace.join(repo_path);
        if !repo_dir.join(".git").exists() {
            continue;
        }
        let origin_head = git_rev_parse(&repo_dir, "origin/main").await;
        let forgejo_head = git_rev_parse(&repo_dir, "forgejo/main").await;
        if let (Some(o), Some(f)) = (origin_head, forgejo_head) {
            checked += 1;
            if o != f {
                drift_count += 1;
            }
        }
    }

    let ok = drift_count == 0;
    let detail = format!("{checked} repos checked, {drift_count} drifted");
    StatusProbe {
        name: "vcs.parity".into(),
        ok,
        detail,
    }
}

async fn git_rev_parse(repo_dir: &Path, refspec: &str) -> Option<String> {
    let output = tokio::process::Command::new("git")
        .args(["rev-parse", refspec])
        .current_dir(repo_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .await
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

// ── Native UDS JSON-RPC client (delegates to crate::jsonrpc) ──────────

pub(crate) async fn uds_jsonrpc_call(
    socket_path: &str,
    request: &str,
) -> std::result::Result<String, String> {
    let policy = crate::ribocipher::RiboCipherConfig::probe_policy();
    crate::jsonrpc::call_with_policy(Path::new(socket_path), request, &policy).await
}

/// Resolve the mesh relay UDS socket path via capability discovery.
fn resolve_mesh_relay_socket() -> String {
    let binary_name = cellmembrane_types::MembraneService::binary_for(
        cellmembrane_types::ServiceCapability::MeshRelay,
    );
    let paths = resolve_primal_socket_paths(binary_name);
    paths
        .into_iter()
        .find(|p| Path::new(p).exists())
        .unwrap_or_else(|| {
            let socket_dir = resolve_biomeos_socket_dir();
            format!("{socket_dir}/{binary_name}.sock")
        })
}

pub(super) fn resolve_biomeos_socket_dir() -> String {
    std::env::var("BIOMEOS_SOCKET_DIR").unwrap_or_else(|_| {
        let uid = resolve_uid();
        let ns = cellmembrane_types::service::NEURAL_API_NAMESPACE;
        format!("/run/user/{uid}/{ns}")
    })
}

pub(super) fn resolve_uid() -> String {
    std::env::var("UID")
        .or_else(|_| std::env::var("EUID"))
        .unwrap_or_else(|_| {
            std::fs::read_to_string("/proc/self/loginuid")
                .unwrap_or_else(|_| "1000".into())
                .trim()
                .to_string()
        })
}

pub(crate) fn resolve_primal_socket_paths(primal: &str) -> Vec<String> {
    let socket_base = std::env::var(cellmembrane_types::service::ENV_SOCKET_BASE)
        .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_SOCKET_BASE.into());
    let xdg_runtime = std::env::var(cellmembrane_types::service::ENV_XDG_RUNTIME_DIR)
        .unwrap_or_else(|_| format!("/run/user/{}", resolve_uid()));
    let mut paths = vec![
        format!("{socket_base}/{primal}.sock"),
        format!(
            "{xdg_runtime}/{}/{primal}.sock",
            cellmembrane_types::service::NEURAL_API_NAMESPACE
        ),
    ];
    // Check registry for alternative API socket (capability-driven, not hardcoded)
    if let Some(svc) = cellmembrane_types::MembraneService::all()
        .iter()
        .find(|s| s.binary == primal)
    {
        if let Some(api) = svc.api_socket {
            paths.insert(0, format!("{socket_base}/{api}.sock"));
        }
    }
    paths
}
