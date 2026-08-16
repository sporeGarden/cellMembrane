// SPDX-License-Identifier: AGPL-3.0-or-later

//! Gate health probes — JSON-RPC UDS queries, process detection, depot status.
//!
//! Replaces shell-based socat/bash/pgrep probes with native async Rust.

mod auxiliary;

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

    let arch_clone = arch;
    let depot = tokio::task::spawn_blocking(move || super::verify::verify_local_depot(arch_clone))
        .await
        .unwrap_or_else(|_| super::ProbeResult::fail("depot verify task panicked"));
    probes.push(StatusProbe {
        name: "depot.integrity".into(),
        ok: depot.ok,
        detail: depot.detail,
    });

    let mesh = probe_mesh_status().await;
    probes.push(StatusProbe {
        name: "mesh.reachability".into(),
        ok: mesh.ok,
        detail: mesh.detail,
    });

    let procs = health_sweep(arch).await;
    probes.push(StatusProbe {
        name: "primals.alive".into(),
        ok: procs.ok,
        detail: procs.detail,
    });

    let arch_for_freshness = arch;
    let fresh =
        tokio::task::spawn_blocking(move || auxiliary::probe_depot_freshness(arch_for_freshness))
            .await
            .unwrap_or_else(|_| super::ProbeResult::fail("freshness probe panicked"));
    probes.push(StatusProbe {
        name: "depot.freshness".into(),
        ok: fresh.ok,
        detail: fresh.detail,
    });

    let sovereignty_probes = super::sovereignty::probe_sovereignty().await;
    probes.extend(sovereignty_probes);

    probes.push(auxiliary::probe_rootpulse_ledger());

    let vcs_probe = auxiliary::probe_vcs_parity().await;
    probes.push(vcs_probe);

    if let Some(cert_probe) = auxiliary::probe_tls_cert_expiry().await {
        probes.push(cert_probe);
    }

    let crash_loop_report =
        match tokio::task::spawn_blocking(|| super::crash_loop::scan_only(None)).await {
            Ok(r) => Some(r),
            Err(e) => {
                tracing::debug!(error = %e, "crash-loop scan spawn failed");
                None
            }
        };
    if let Some(ref report) = crash_loop_report {
        let crash_ok = !report.has_loops();
        let detail = if crash_ok {
            format!("{} services scanned, no crash-loops", report.scanned)
        } else {
            let units: Vec<&str> = report.loops.iter().map(|e| e.unit.as_str()).collect();
            format!("{} crash-loop(s): {}", report.loops.len(), units.join(", "))
        };
        probes.push(StatusProbe {
            name: "service.crash-loop".into(),
            ok: crash_ok,
            detail,
        });
    }

    let healthy = probes.iter().all(|p| p.ok);

    Ok(GateStatus {
        gate_name,
        arch: arch.to_string(),
        probes,
        healthy,
    })
}

// ── Mesh probes ──────────────────────────────────────────────────

/// Probe mesh status via neuralAPI-routed `capability.call` with fallback to direct UDS.
async fn probe_mesh_status() -> super::ProbeResult {
    if let Ok(Some(result)) =
        crate::bridge::try_bridge("mesh_relay", "mesh.status", serde_json::json!({})).await
    {
        return parse_mesh_json(&result);
    }

    let socket_path = resolve_mesh_relay_socket();

    if !Path::new(&socket_path).exists() {
        return super::ProbeResult::fail("mesh relay socket not found");
    }

    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "mesh.status",
        "params": {},
        "id": 1
    });

    match uds_jsonrpc_call(&socket_path, &request.to_string()).await {
        Ok(response) => parse_mesh_response(&response),
        Err(e) => super::ProbeResult::fail(e.to_string()),
    }
}

fn parse_mesh_json(result: &serde_json::Value) -> super::ProbeResult {
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

    let ok = reachable > 0 || peers > 0 || federation;
    super::ProbeResult { ok, detail }
}

fn parse_mesh_response(response: &str) -> super::ProbeResult {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(response.trim()) else {
        if response.contains("\"result\"") {
            return super::ProbeResult::pass("mesh responding");
        }
        return super::ProbeResult::fail(format!("unexpected: {}", response.trim()));
    };

    if let Some(err) = json.get("error") {
        let msg = err
            .get("message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown error");
        return super::ProbeResult::fail(format!("mesh error: {msg}"));
    }

    json.get("result").map_or_else(
        || super::ProbeResult::fail("no result field"),
        parse_mesh_json,
    )
}

// ── Primal health sweep ──────────────────────────────────────────

/// Health sweep: probe each primal using its registry-declared `HealthCheckMethod`.
///
/// Dispatches per-service: JSON-RPC liveness for primals, TCP connect for
/// `RustDesk`, HTTPS probe for Caddy, DNS probe for Knot, socket existence for
/// UDS-only services. Falls back to process detection when all probes fail.
///
/// Scoped to the local gate's composition profile when available, otherwise
/// checks all nucleus primals.
pub(crate) async fn health_sweep(arch: &str) -> super::ProbeResult {
    use cellmembrane_types::service::capability::HealthCheckMethod;

    let dest_root = super::resolve_plasmidbin_dir();
    let bin_dir = dest_root.join("primals").join(arch);

    let gate = super::resolve_local_gate_identity();
    let composition_primals = crate::plasmid::resolve_gate_primals(&gate);
    let primals: Vec<&str> = composition_primals.iter().map(String::as_str).collect();
    let mut alive = 0u32;
    let mut dead = 0u32;
    let mut details: Vec<String> = Vec::new();

    tokio::time::sleep(std::time::Duration::from_secs(
        cellmembrane_types::service::MESH_SOCKET_WAIT_INTERVAL_SECS,
    ))
    .await;

    for primal in &primals {
        let bin_path = bin_dir.join(primal);
        if !bin_path.exists() {
            tracing::debug!(primal = %primal, "health: binary not in depot — marking dead");
            dead += 1;
            continue;
        }

        let svc = cellmembrane_types::MembraneService::for_binary(primal);
        let method = svc.map_or(
            HealthCheckMethod::Liveness,
            cellmembrane_types::MembraneService::uds_health_check,
        );

        let probed = match method {
            HealthCheckMethod::Liveness => probe_primal_jsonrpc(primal).await,
            HealthCheckMethod::TcpConnect => {
                svc.and_then(|s| s.port).is_some_and(probe_tcp_connect)
            }
            HealthCheckMethod::HttpsProbe => probe_https(primal),
            HealthCheckMethod::DnsProbe => probe_dns(),
            HealthCheckMethod::SocketExists => {
                let paths = resolve_primal_socket_paths(primal);
                if paths.iter().any(|p| Path::new(p).exists()) {
                    true
                } else {
                    try_tcp_health_probe(primal).await
                }
            }
            HealthCheckMethod::SystemdActive => {
                let unit = svc.map_or_else(
                    || format!("{primal}.service"),
                    |s| s.systemd_unit.to_string(),
                );
                probe_systemd_active(&unit).await
            }
        };

        if probed {
            alive += 1;
        } else {
            let primal_name = (*primal).to_string();
            let pgrep_found = tokio::task::spawn_blocking(move || probe_primal_pgrep(&primal_name))
                .await
                .unwrap_or(false);
            if pgrep_found {
                alive += 1;
            } else {
                tracing::debug!(primal = %primal, method = %method, "health: primal not responding");
                details.push(format!("{primal}({method})"));
                dead += 1;
            }
        }
    }

    let total = alive + dead;
    let ok = dead == 0;
    let mut detail = format!("{alive}/{total} primals alive");
    if !details.is_empty() {
        use std::fmt::Write;
        let _ = write!(detail, " — dead: {}", details.join(", "));
    }
    super::ProbeResult { ok, detail }
}

/// Probe a primal via neuralAPI `capability.call` with fallback to direct UDS JSON-RPC.
///
/// Prefers routing through biomeOS neuralAPI when available — validates the full
/// orchestration stack. Falls back to direct UDS when neuralAPI is unavailable.
/// Any valid JSON-RPC response (including method-not-found errors) proves
/// the primal is alive.
///
/// Under G65, also attempts protocol negotiation on the primary socket to
/// discover which protocols the primal supports at runtime.
async fn probe_primal_jsonrpc(primal: &str) -> bool {
    match crate::bridge::try_bridge(primal, "health", serde_json::json!({})).await {
        Ok(Some(result)) => return result.get("status").is_some() || result.is_object(),
        Err(_) => return true,
        Ok(None) => {}
    }

    let socket_paths = resolve_primal_socket_paths(primal);
    let request = crate::jsonrpc::HEALTH_REQUEST;

    for socket_path in &socket_paths {
        if !Path::new(socket_path).exists() {
            continue;
        }

        if let Ok(neg) = super::sockets::negotiate_protocol(
            socket_path,
            &[
                cellmembrane_types::IpcProtocol::Tarpc,
                cellmembrane_types::IpcProtocol::JsonRpc,
            ],
        )
        .await
        {
            if neg.negotiated {
                tracing::info!(
                    primal = %primal,
                    selected = %neg.selected,
                    "G65 negotiation succeeded"
                );
                return true;
            }
        }

        if let Ok(response) = uds_jsonrpc_call(socket_path, request).await
            && let Ok(json) = serde_json::from_str::<serde_json::Value>(&response)
        {
            if json.get("result").is_some() {
                return true;
            }
            if json.get("error").is_some() {
                tracing::debug!(
                    primal = %primal,
                    "health: JSON-RPC responded with error — primal running but unhealthy"
                );
            }
        }
    }

    try_tcp_health_probe(primal).await
}

/// TCP JSON-RPC health fallback for platforms without UDS (Windows, NamedPipe-absent).
///
/// Checks the service registry for a known TCP port, then sends the standard
/// health request over `TransportEndpoint::Tcp { 127.0.0.1, port }`. This
/// mirrors the `builder.serve` pattern already used by sub-builders on port 9800.
async fn try_tcp_health_probe(primal: &str) -> bool {
    let svc = cellmembrane_types::MembraneService::for_binary(primal);
    let Some(port) = svc.and_then(|s| s.port) else {
        return false;
    };

    let endpoint = cellmembrane_types::TransportEndpoint::Tcp {
        host: cellmembrane_types::service::BIND_LOOPBACK.into(),
        port,
    };
    let request = crate::jsonrpc::HEALTH_REQUEST;

    match crate::jsonrpc::call_endpoint(&endpoint, request).await {
        Ok(response) => {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&response) {
                if json.get("result").is_some() || json.get("error").is_some() {
                    tracing::debug!(
                        primal = %primal,
                        port,
                        "health: TCP fallback succeeded"
                    );
                    return true;
                }
            }
            false
        }
        Err(_) => false,
    }
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
        if let Ok(comm) = std::fs::read_to_string(&comm_path)
            && comm.trim() == primal
        {
            return true;
        }
    }
    false
}

/// TCP connect probe — verifies a service is listening on the given port.
fn probe_tcp_connect(port: u16) -> bool {
    use std::net::{SocketAddr, TcpStream};
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(2)).is_ok()
}

/// HTTPS probe — verifies a reverse proxy is listening on its declared port.
fn probe_https(primal: &str) -> bool {
    cellmembrane_types::MembraneService::for_binary(primal)
        .and_then(|s| s.port)
        .is_some_and(probe_tcp_connect)
}

/// DNS probe — verifies a DNS server is listening on port 53.
fn probe_dns() -> bool {
    probe_tcp_connect(53)
}

async fn probe_systemd_active(unit: &str) -> bool {
    let unit_owned = unit.to_string();
    tokio::task::spawn_blocking(move || {
        std::process::Command::new("systemctl")
            .args(["is-active", "--quiet", &unit_owned])
            .status()
            .is_ok_and(|s| s.success())
    })
    .await
    .unwrap_or(false)
}

// ── Socket resolution (delegated to gate/sockets.rs) ──────────

use super::sockets::resolve_mesh_relay_socket;
pub(crate) use super::sockets::{resolve_primal_socket_paths, uds_jsonrpc_call};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mesh_json_with_peers() {
        let result = serde_json::json!({
            "peers": 3,
            "reachable": 2,
            "relay_enabled": false
        });
        let probe = parse_mesh_json(&result);
        assert!(probe.ok);
        assert!(probe.detail.contains("3 peers"));
        assert!(probe.detail.contains("2 reachable"));
    }

    #[test]
    fn parse_mesh_json_hub_listening() {
        let result = serde_json::json!({
            "relay_enabled": true,
            "reachable_peers": 0
        });
        let probe = parse_mesh_json(&result);
        assert!(probe.ok, "hub should be OK even with zero peers");
        assert!(probe.detail.contains("hub listening"));
    }

    #[test]
    fn parse_mesh_json_zero_everything() {
        let result = serde_json::json!({});
        let probe = parse_mesh_json(&result);
        assert!(!probe.ok);
        assert!(probe.detail.contains("0 peers"));
    }

    #[test]
    fn parse_mesh_json_peer_array() {
        let result = serde_json::json!({
            "peers": ["gate1", "gate2"],
            "reachable": 1
        });
        let probe = parse_mesh_json(&result);
        assert!(probe.ok);
        assert!(probe.detail.contains("2 peers"));
    }

    #[test]
    fn parse_mesh_response_valid_jsonrpc() {
        let resp = r#"{"jsonrpc":"2.0","result":{"peers":4,"reachable":3},"id":1}"#;
        let probe = parse_mesh_response(resp);
        assert!(probe.ok);
        assert!(probe.detail.contains("4 peers"));
        assert!(probe.detail.contains("3 reachable"));
    }

    #[test]
    fn parse_mesh_response_error() {
        let resp =
            r#"{"jsonrpc":"2.0","error":{"code":-32601,"message":"method not found"},"id":1}"#;
        let probe = parse_mesh_response(resp);
        assert!(!probe.ok);
        assert!(probe.detail.contains("method not found"));
    }

    #[test]
    fn parse_mesh_response_malformed_with_result_keyword() {
        let resp = r#"not json but has "result" in it"#;
        let probe = parse_mesh_response(resp);
        assert!(probe.ok);
        assert_eq!(probe.detail, "mesh responding");
    }

    #[test]
    fn parse_mesh_response_malformed_no_result() {
        let resp = "garbage data";
        let probe = parse_mesh_response(resp);
        assert!(!probe.ok);
        assert!(probe.detail.contains("unexpected"));
    }

    #[test]
    fn check_cert_days_unreachable_returns_negative() {
        let days = auxiliary::check_cert_days("unreachable.invalid.test");
        assert!(days <= 0, "unreachable domain should return <=0 days");
    }

    #[test]
    fn probe_tcp_connect_fails_on_unlikely_port() {
        assert!(!probe_tcp_connect(1), "port 1 should not be listening");
    }

    #[test]
    fn probe_https_unknown_primal_returns_false() {
        assert!(!probe_https("nonexistent-primal-xyz"));
    }

    #[tokio::test]
    async fn tcp_fallback_no_port_returns_false() {
        assert!(
            !try_tcp_health_probe("beardog").await,
            "beardog has no TCP port — should return false"
        );
    }

    #[tokio::test]
    async fn tcp_fallback_unknown_primal_returns_false() {
        assert!(
            !try_tcp_health_probe("nonexistent-primal-xyz").await,
            "unknown primal has no registry entry — should return false"
        );
    }

    fn health_method_registry_wiring() {
        use cellmembrane_types::service::capability::HealthCheckMethod;
        let caddy = cellmembrane_types::MembraneService::for_binary("caddy");
        assert_eq!(
            caddy.map(|s| s.health_method),
            Some(HealthCheckMethod::HttpsProbe),
            "caddy should use HttpsProbe"
        );
        let knot = cellmembrane_types::MembraneService::for_binary("knot-dns");
        assert_eq!(
            knot.map(|s| s.health_method),
            Some(HealthCheckMethod::DnsProbe),
            "knot-dns should use DnsProbe"
        );
        let hbbs = cellmembrane_types::MembraneService::for_binary("hbbs");
        assert_eq!(
            hbbs.map(|s| s.health_method),
            Some(HealthCheckMethod::TcpConnect),
            "hbbs should use TcpConnect"
        );
        let beardog = cellmembrane_types::MembraneService::for_binary("beardog");
        assert_eq!(
            beardog.map(|s| s.health_method),
            Some(HealthCheckMethod::Liveness),
            "beardog should use Liveness"
        );
    }
}
