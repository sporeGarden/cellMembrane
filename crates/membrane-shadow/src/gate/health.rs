// SPDX-License-Identifier: AGPL-3.0-or-later

//! Gate health probes — JSON-RPC UDS queries, process detection, depot status.
//!
//! Replaces shell-based socat/bash/pgrep probes with native async Rust.

use serde::{Deserialize, Serialize};
use std::path::Path;

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

    let sovereignty_probes = probe_sovereignty().await;
    probes.extend(sovereignty_probes);

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
    if let Some(result) = crate::bridge::try_bridge(
        "mesh_relay",
        "mesh.status",
        serde_json::json!({}),
    )
    .await
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
                .or_else(|| v.as_array().map(|a| a.len() as u64))
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
                .or_else(|| v.as_array().map(|a| a.len() as u64))
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

        if probe_primal_jsonrpc(primal).await || probe_primal_pgrep(primal) {
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
    if let Some(result) = crate::bridge::try_bridge(
        primal,
        "health",
        serde_json::json!({}),
    )
    .await
    {
        return result.get("status").is_some() || result.is_object();
    }

    let socket_paths = resolve_primal_socket_paths(primal);
    let request = r#"{"jsonrpc":"2.0","method":"health","params":{},"id":1}"#;

    for socket_path in &socket_paths {
        if !Path::new(socket_path).exists() {
            continue;
        }

        if let Ok(response) = uds_jsonrpc_call(socket_path, request).await {
            if response.contains("\"jsonrpc\"") || response.contains("\"result\"") || response.contains("\"error\"") {
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
        if !name.to_str().is_some_and(|s| s.chars().all(|c| c.is_ascii_digit())) {
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

// ── Sovereignty probes (S1-S4 live validation) ───────────────────────────

/// Resolve the sovereign domain for TLS and content probes.
/// Uses `MEMBRANE_DEPOT_HOSTNAME` env var with fallback to the types crate default.
fn resolve_sovereign_domain() -> String {
    std::env::var(cellmembrane_types::service::ENV_DEPOT_HOSTNAME)
        .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_DEPOT_HOSTNAME.into())
}

/// Probe all four sovereignty shadows (S1 TLS, S2 Relay, S3 Content, S4 Auth).
///
/// These are live WAN probes that validate the ecoPrimals sovereign infrastructure
/// is operational — replacing static documentation with runtime truth.
async fn probe_sovereignty() -> Vec<StatusProbe> {
    let (s1, s2, s3, s4) = tokio::join!(
        probe_s1_tls(),
        probe_s2_relay(),
        probe_s3_content(),
        probe_s4_auth(),
    );
    vec![s1, s2, s3, s4]
}

/// S1: Sovereign TLS — validate certificate and TTFB from sovereign domain.
async fn probe_s1_tls() -> StatusProbe {
    let domain = resolve_sovereign_domain();
    let url = format!("https://{domain}/");
    let start = std::time::Instant::now();

    let result = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .map_err(|e| format!("client: {e}"))?
            .head(&url)
            .send()
            .await
            .map_err(|e| format!("request: {e}"))
    })
    .await;

    match result {
        Ok(Ok(resp)) => {
            let ttfb_ms = start.elapsed().as_millis();
            let status = resp.status();
            if status.is_success() || status.as_u16() == 308 || status.as_u16() == 301 {
                StatusProbe {
                    name: "sovereignty.s1_tls".into(),
                    ok: true,
                    detail: format!("OPERATIONAL — {domain} {status} ({ttfb_ms}ms)"),
                }
            } else {
                StatusProbe {
                    name: "sovereignty.s1_tls".into(),
                    ok: false,
                    detail: format!("{domain} returned {status} ({ttfb_ms}ms)"),
                }
            }
        }
        Ok(Err(e)) => StatusProbe {
            name: "sovereignty.s1_tls".into(),
            ok: false,
            detail: format!("FAIL — {e}"),
        },
        Err(_) => StatusProbe {
            name: "sovereignty.s1_tls".into(),
            ok: false,
            detail: "TIMEOUT — TLS probe exceeded 5s".into(),
        },
    }
}

/// S2: Sovereign Relay — probe Songbird federation (:7700) TCP and TURN (:3478) TCP.
///
/// Federation port is always TCP. TURN may primarily use UDP but also listens on TCP.
/// Federation reachability is the primary signal; TURN TCP is best-effort.
async fn probe_s2_relay() -> StatusProbe {
    let vps_host = std::env::var(cellmembrane_types::service::ENV_VPS_MESH_PEER)
        .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_VPS_HOST.into());

    let fed_port = cellmembrane_types::service::DEFAULT_FEDERATION_PORT;
    let turn_port = cellmembrane_types::service::DEFAULT_TURN_PORT;
    let fed_addr = format!("{vps_host}:{fed_port}");
    let turn_addr = format!("{vps_host}:{turn_port}");

    let (fed_ok, turn_ok) = tokio::join!(tcp_reachable(&fed_addr), tcp_reachable(&turn_addr),);

    let detail = format!(
        "federation:{} TURN:{}",
        if fed_ok { "REACHABLE" } else { "UNREACHABLE" },
        if turn_ok {
            "TCP-OK"
        } else {
            "TCP-CLOSED(UDP-only)"
        },
    );

    StatusProbe {
        name: "sovereignty.s2_relay".into(),
        ok: fed_ok,
        detail,
    }
}

/// S3: Sovereign Content — probe WAN depot HTTPS availability and TTFB.
///
/// Probes the depot file server (Caddy) to confirm binaries are being served
/// over sovereign TLS. Uses the crypto spine binary as probe target (always present).
async fn probe_s3_content() -> StatusProbe {
    let arch = crate::plasmid::detect_target_triple();
    let probe_binary =
        cellmembrane_types::MembraneService::binary_for(cellmembrane_types::ServiceCapability::CryptoSigner);
    let domain = resolve_sovereign_domain();
    let url = format!("https://{domain}/depot/{arch}/{probe_binary}");
    let start = std::time::Instant::now();

    let result = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .map_err(|e| format!("client: {e}"))?
            .head(&url)
            .send()
            .await
            .map_err(|e| format!("request: {e}"))
    })
    .await;

    match result {
        Ok(Ok(resp)) => {
            let ttfb_ms = start.elapsed().as_millis();
            if resp.status().is_success() {
                let size_kb = resp
                    .headers()
                    .get("content-length")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
                    .map(|b| b / 1024);
                let size_info = size_kb.map_or(String::new(), |kb| format!(" {kb}KB"));
                StatusProbe {
                    name: "sovereignty.s3_content".into(),
                    ok: true,
                    detail: format!("OPERATIONAL — depot serving{size_info} ({ttfb_ms}ms TTFB)"),
                }
            } else {
                StatusProbe {
                    name: "sovereignty.s3_content".into(),
                    ok: false,
                    detail: format!("depot returned {} ({ttfb_ms}ms)", resp.status()),
                }
            }
        }
        Ok(Err(e)) => StatusProbe {
            name: "sovereignty.s3_content".into(),
            ok: false,
            detail: format!("FAIL — {e}"),
        },
        Err(_) => StatusProbe {
            name: "sovereignty.s3_content".into(),
            ok: false,
            detail: "TIMEOUT — content probe exceeded 5s".into(),
        },
    }
}

/// S4: Sovereign Auth — probe `BearDog` BTSP enforcement via local UDS health.
///
/// Tries neuralAPI capability routing first, then direct UDS. Any JSON-RPC
/// response (including `-32601 method_not_found` or BTSP errors) proves
/// the crypto spine is alive and enforcing.
async fn probe_s4_auth() -> StatusProbe {
    let binary_name =
        cellmembrane_types::MembraneService::binary_for(cellmembrane_types::ServiceCapability::CryptoSigner);

    // Try neuralAPI routing first
    if let Some(result) = crate::bridge::try_bridge(binary_name, "health", serde_json::json!({})).await {
        let status = result.get("status").and_then(|v| v.as_str()).unwrap_or("alive");
        let btsp = result.get("auth_mode").and_then(|v| v.as_str()).unwrap_or("");
        let detail = if btsp == "btsp" {
            "ENFORCED — BearDog BTSP active (via neuralAPI)".to_string()
        } else {
            format!("RESPONDING — {binary_name} {status} (via neuralAPI)")
        };
        return StatusProbe {
            name: "sovereignty.s4_auth".into(),
            ok: true,
            detail,
        };
    }

    // Direct UDS probe — any JSON-RPC response (even errors) proves alive
    let socket_paths = resolve_primal_socket_paths(binary_name);
    let request = r#"{"jsonrpc":"2.0","method":"health","params":{},"id":1}"#;

    for socket_path in &socket_paths {
        if !Path::new(socket_path).exists() {
            continue;
        }
        if let Ok(response) = uds_jsonrpc_call(socket_path, request).await {
            if response.contains("\"jsonrpc\"")
                || response.contains("\"result\"")
                || response.contains("\"error\"")
                || response.contains("BTSP")
            {
                let enforced = response.contains("BTSP handshake required")
                    || response.contains("\"auth_mode\":\"btsp\"");
                let detail = if enforced {
                    "ENFORCED — BearDog BTSP active (direct UDS)".to_string()
                } else {
                    format!(
                        "RESPONDING — {binary_name} alive ({})",
                        &response[..response.len().min(80)]
                    )
                };
                return StatusProbe {
                    name: "sovereignty.s4_auth".into(),
                    ok: true,
                    detail,
                };
            }
        }
    }

    StatusProbe {
        name: "sovereignty.s4_auth".into(),
        ok: false,
        detail: "UNREACHABLE — BearDog not responding on UDS".into(),
    }
}

/// TCP reachability check with 3s timeout.
async fn tcp_reachable(addr: &str) -> bool {
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        tokio::net::TcpStream::connect(addr),
    )
    .await
    .is_ok_and(|r| r.is_ok())
}

// ── Native UDS JSON-RPC client (delegates to crate::jsonrpc) ──────────

async fn uds_jsonrpc_call(socket_path: &str, request: &str) -> std::result::Result<String, String> {
    crate::jsonrpc::call(Path::new(socket_path), request).await
}

/// Resolve the mesh relay UDS socket path via capability discovery.
fn resolve_mesh_relay_socket() -> String {
    let binary_name =
        cellmembrane_types::MembraneService::binary_for(cellmembrane_types::ServiceCapability::MeshRelay);
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
        format!("/run/user/{uid}/biomeos")
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

fn resolve_primal_socket_paths(primal: &str) -> Vec<String> {
    let socket_base = std::env::var(cellmembrane_types::service::ENV_SOCKET_BASE)
        .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_SOCKET_BASE.into());
    let xdg_runtime = std::env::var(cellmembrane_types::service::ENV_XDG_RUNTIME_DIR)
        .unwrap_or_else(|_| format!("/run/user/{}", resolve_uid()));
    vec![
        format!("{socket_base}/{primal}.sock"),
        format!("{xdg_runtime}/biomeos/{primal}.sock"),
    ]
}
