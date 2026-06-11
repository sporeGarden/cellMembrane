// SPDX-License-Identifier: AGPL-3.0-or-later

//! Gate health probes — JSON-RPC UDS queries, process detection, depot status.
//!
//! Replaces shell-based socat/bash/pgrep probes with native async Rust.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

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
/// Probes: depot integrity → mesh reachability → primal processes → depot freshness.
pub async fn status() -> crate::error::Result<GateStatus> {
    let arch = crate::plasmid::detect_target_triple();
    let gate_name = super::resolve_local_gate_identity();
    let mut probes: Vec<StatusProbe> = Vec::new();

    let (depot_ok, depot_detail) = super::verify::verify_local_depot(&arch);
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

/// Probe mesh status via JSON-RPC on the mesh relay UDS socket.
async fn probe_mesh_status() -> (bool, String) {
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

fn parse_mesh_response(response: &str) -> (bool, String) {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(response.trim()) else {
        if response.contains("\"result\"") {
            return (true, "mesh responding".into());
        }
        return (false, format!("unexpected: {}", response.trim()));
    };

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

        if probe_primal_jsonrpc(primal).await || probe_primal_pgrep(primal).await {
            alive += 1;
        } else {
            dead += 1;
        }
    }

    let total = alive + dead;
    let ok = dead == 0;
    (ok, format!("{alive}/{total} primals alive"))
}

/// Probe a primal via native async UDS JSON-RPC `health` method.
///
/// Tries standard socket paths: `/run/membrane/{primal}.sock` first,
/// then `$XDG_RUNTIME_DIR/biomeos/{primal}.sock`.
async fn probe_primal_jsonrpc(primal: &str) -> bool {
    let socket_paths = resolve_primal_socket_paths(primal);
    let request = r#"{"jsonrpc":"2.0","method":"health","params":{},"id":1}"#;

    for socket_path in &socket_paths {
        if !Path::new(socket_path).exists() {
            continue;
        }

        if let Ok(response) = uds_jsonrpc_call(socket_path, request).await {
            if response.contains("\"result\"") || response.contains("\"status\"") {
                return true;
            }
        }
    }

    false
}

/// Fallback: detect running process via pgrep.
async fn probe_primal_pgrep(primal: &str) -> bool {
    tokio::process::Command::new("pgrep")
        .args(["-f", &format!("{primal}.*server")])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .is_ok_and(|s| s.success())
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

// ── Native UDS JSON-RPC client ─────────────────────────────────────────

/// Send a JSON-RPC request over a Unix Domain Socket, read the response.
///
/// This replaces all `bash -c 'echo ... | socat ...'` patterns with a
/// native async implementation using `tokio::net::UnixStream`.
async fn uds_jsonrpc_call(socket_path: &str, request: &str) -> std::result::Result<String, String> {
    let stream = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        tokio::net::UnixStream::connect(socket_path),
    )
    .await
    .map_err(|_| format!("connect timeout: {socket_path}"))?
    .map_err(|e| format!("connect error: {e}"))?;

    let (mut reader, mut writer) = stream.into_split();

    writer
        .write_all(request.as_bytes())
        .await
        .map_err(|e| format!("write error: {e}"))?;
    writer
        .shutdown()
        .await
        .map_err(|e| format!("shutdown error: {e}"))?;

    let mut buf = Vec::with_capacity(4096);
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        reader.read_to_end(&mut buf),
    )
    .await
    .map_err(|_| "read timeout".to_string())?
    .map_err(|e| format!("read error: {e}"))?;

    String::from_utf8(buf).map_err(|e| format!("utf8 error: {e}"))
}

/// Resolve the mesh relay UDS socket path via capability discovery.
fn resolve_mesh_relay_socket() -> String {
    let relay = cellmembrane_types::MembraneService::with_capability(
        cellmembrane_types::ServiceCapability::MeshRelay,
    );
    let binary_name = relay.map_or("songbird", |s| s.binary);
    let socket_dir = resolve_biomeos_socket_dir();
    format!("{socket_dir}/{binary_name}.sock")
}

fn resolve_biomeos_socket_dir() -> String {
    std::env::var("BIOMEOS_SOCKET_DIR").unwrap_or_else(|_| {
        let uid = resolve_uid();
        format!("/run/user/{uid}/biomeos")
    })
}

fn resolve_uid() -> String {
    std::env::var("UID")
        .or_else(|_| std::env::var("EUID"))
        .unwrap_or_else(|_| {
            std::fs::read_to_string("/proc/self/loginuid")
                .unwrap_or_else(|_| "1000".into())
                .trim()
                .to_string()
        })
}

fn resolve_primal_socket_paths(primal: &str) -> Vec<String> {
    let xdg_runtime =
        std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| format!("/run/user/{}", resolve_uid()));
    vec![
        format!("/run/membrane/{primal}.sock"),
        format!("{xdg_runtime}/biomeos/{primal}.sock"),
    ]
}

/// Resolve the plasmidBin directory (shared with verify and bootstrap).
pub(super) fn resolve_plasmidbin_dir() -> PathBuf {
    crate::plasmid::resolve_path(None, "ECOPRIMALS_PLASMID_BIN", || {
        let data_home = std::env::var(cellmembrane_types::service::ENV_XDG_DATA_HOME)
            .unwrap_or_else(|_| {
                let home = std::env::var(cellmembrane_types::service::ENV_HOME)
                    .unwrap_or_else(|_| "/tmp".into());
                format!("{home}/.local/share")
            });
        PathBuf::from(format!("{data_home}/ecoPrimals/plasmidBin"))
    })
}
