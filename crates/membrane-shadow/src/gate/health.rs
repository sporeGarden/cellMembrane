// SPDX-License-Identifier: AGPL-3.0-or-later

//! Gate health probes — JSON-RPC UDS queries, process detection, depot status.
//!
//! Replaces shell-based socat/bash/pgrep probes with native async Rust.

use serde::{Deserialize, Serialize};
use std::path::Path;

const STALE_THRESHOLD_DAYS: u64 = 7;
const SECS_PER_DAY: u64 = 86_400;
const SECS_PER_HOUR: u64 = 3_600;
const CERT_WARNING_THRESHOLD_DAYS: i64 = 14;
const MAX_CERT_PROBE_DOMAINS: usize = 5;

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

    probes.push(probe_rootpulse_ledger());

    let vcs_probe = probe_vcs_parity().await;
    probes.push(vcs_probe);

    if let Some(cert_probe) = probe_tls_cert_expiry().await {
        probes.push(cert_probe);
    }

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
        Err(e) => (false, e.to_string()),
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

    json.get("result")
        .map_or_else(|| (false, "no result field".into()), parse_mesh_json)
}

/// Health sweep: probe each primal via JSON-RPC, fall back to process detection.
///
/// Scoped to the local gate's composition profile when available, otherwise
/// checks all nucleus primals.
pub async fn health_sweep(arch: &str) -> (bool, String) {
    let dest_root = super::resolve_plasmidbin_dir();
    let bin_dir = dest_root.join("primals").join(arch);

    let gate = super::resolve_local_gate_identity();
    let composition_primals = crate::plasmid::resolve_gate_primals(&gate);
    let primals: Vec<&str> = composition_primals.iter().map(String::as_str).collect();
    let mut alive = 0u32;
    let mut dead = 0u32;

    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    for primal in &primals {
        let bin_path = bin_dir.join(primal);
        if !bin_path.exists() {
            tracing::debug!(primal = %primal, "health: binary not in depot — marking dead");
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
            tracing::debug!(primal = %primal, "health: primal not responding — marking dead");
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
    let request = crate::jsonrpc::HEALTH_REQUEST;

    for socket_path in &socket_paths {
        if !Path::new(socket_path).exists() {
            continue;
        }

        if let Ok(response) = uds_jsonrpc_call(socket_path, request).await {
            if response.contains("\"result\"") {
                return true;
            }
            if response.contains("\"error\"") {
                tracing::debug!(
                    primal = %primal,
                    "health: JSON-RPC responded with error — primal running but unhealthy"
                );
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

    let gate = super::resolve_local_gate_identity();
    let composition_primals = crate::plasmid::resolve_gate_primals(&gate);
    let primals: Vec<&str> = composition_primals.iter().map(String::as_str).collect();
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
            |_| {
                vec![
                    cellmembrane_types::service::INFRA_PLASMID_BIN.into(),
                    cellmembrane_types::service::INFRA_WATERING_HOLE.into(),
                ]
            },
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
    crate::git_ops::git_output_opt(repo_dir, &["rev-parse", refspec]).await
}

// ── Native UDS JSON-RPC client (delegates to crate::jsonrpc) ──────────

pub(crate) async fn uds_jsonrpc_call(socket_path: &str, request: &str) -> crate::Result<String> {
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

pub(crate) fn resolve_biomeos_socket_dir() -> String {
    std::env::var(cellmembrane_types::service::ENV_BIOMEOS_SOCKET_DIR).unwrap_or_else(|_| {
        let uid = resolve_uid();
        let ns = cellmembrane_types::service::NEURAL_API_NAMESPACE;
        format!("/run/user/{uid}/{ns}")
    })
}

pub(crate) fn resolve_uid() -> String {
    std::env::var("UID")
        .or_else(|_| std::env::var("EUID"))
        .unwrap_or_else(|_| {
            std::fs::read_to_string("/proc/self/loginuid")
                .unwrap_or_else(|_| "1000".into())
                .trim()
                .to_string()
        })
}

/// Probe rootpulse ledger state — checks if a session has been committed on this gate.
fn probe_rootpulse_ledger() -> StatusProbe {
    crate::temporal::post_sync::load_rootpulse_session().map_or_else(
        || StatusProbe {
            name: "rootpulse.ledger".into(),
            ok: false,
            detail:
                "no rootpulse session recorded — run rootpulse.commit or cascade with freshness"
                    .into(),
        },
        |s| StatusProbe {
            name: "rootpulse.ledger".into(),
            ok: true,
            detail: format!("last session: {s}"),
        },
    )
}

pub(crate) fn resolve_primal_socket_paths(primal: &str) -> Vec<String> {
    let socket_base = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_SOCKET_BASE,
        cellmembrane_types::service::DEFAULT_SOCKET_BASE,
    );
    let xdg_runtime = std::env::var(cellmembrane_types::service::ENV_XDG_RUNTIME_DIR)
        .unwrap_or_else(|_| format!("/run/user/{}", resolve_uid()));
    let ns = cellmembrane_types::service::NEURAL_API_NAMESPACE;
    let mut paths = vec![
        format!("{socket_base}/{primal}.sock"),
        format!("{xdg_runtime}/{ns}/{primal}.sock"),
    ];
    if let Some(svc) = cellmembrane_types::MembraneService::all()
        .iter()
        .find(|s| s.binary == primal)
    {
        if let Some(api) = svc.api_socket {
            paths.insert(0, format!("{socket_base}/{api}.sock"));
            paths.insert(0, format!("{socket_base}/{api}-default.sock"));
            paths.push(format!("{xdg_runtime}/{ns}/{api}-default.sock"));
        }
        for alias in svc.socket_aliases {
            paths.push(format!("{socket_base}/{alias}.sock"));
        }
    }
    paths
}

/// Probe TLS cert expiry for publicly-served domains.
///
/// Only runs on gates that serve TLS (have a `caddy_tls` or `tls_terminator`
/// role in the manifest). Returns `None` if TLS is not locally relevant.
///
/// Uses `openssl s_client` to probe each domain's cert expiry. Any cert
/// with <14 days remaining triggers a probe failure (EXP-03 monitoring).
async fn probe_tls_cert_expiry() -> Option<StatusProbe> {
    let workspace = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_ECOPRIMALS_ROOT,
        cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT,
    );
    let manifest = crate::manifest::load_from_workspace(std::path::Path::new(&workspace)).ok()?;
    let gate = super::resolve_local_gate_identity();

    let profile = manifest.gates.get(&gate)?;
    let is_tls_gate = profile.roles.iter().any(cellmembrane_types::GateRole::is_tls);
    if !is_tls_gate {
        return None;
    }

    let domains: Vec<String> = profile
        .domains
        .clone()
        .unwrap_or_default()
        .into_iter()
        .take(MAX_CERT_PROBE_DOMAINS)
        .collect();

    if domains.is_empty() {
        return Some(StatusProbe {
            name: "tls.cert_expiry".into(),
            ok: true,
            detail: "no domains configured".into(),
        });
    }

    let mut results: Vec<String> = Vec::new();
    let mut any_expiring = false;

    for domain in &domains {
        let d = domain.clone();
        let days = tokio::task::spawn_blocking(move || check_cert_days(&d))
            .await
            .unwrap_or(-1);
        if days < 0 {
            results.push(format!("{domain}: EXPIRED/unreachable"));
            any_expiring = true;
        } else if days < CERT_WARNING_THRESHOLD_DAYS {
            results.push(format!("{domain}: {days}d remaining (WARNING)"));
            any_expiring = true;
        } else {
            results.push(format!("{domain}: {days}d remaining"));
        }
    }

    Some(StatusProbe {
        name: "tls.cert_expiry".into(),
        ok: !any_expiring,
        detail: results.join(", "),
    })
}

/// Check TLS cert days remaining for a domain via local openssl probe.
fn check_cert_days(domain: &str) -> i64 {
    let cmd = format!(
        "echo | openssl s_client -connect {domain}:443 -servername {domain} 2>/dev/null \
         | openssl x509 -noout -enddate 2>/dev/null"
    );
    let Ok(result) = std::process::Command::new("sh")
        .args(["-c", &cmd])
        .output()
    else {
        return -1;
    };
    if !result.status.success() {
        return -1;
    }

    let stdout = String::from_utf8_lossy(&result.stdout);
    let not_after = stdout
        .lines()
        .find(|l| l.starts_with("notAfter="))
        .map_or("", |l| l.trim_start_matches("notAfter=").trim());

    crate::caddy::parse_days_remaining(not_after)
}

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
        let (ok, detail) = parse_mesh_json(&result);
        assert!(ok);
        assert!(detail.contains("3 peers"));
        assert!(detail.contains("2 reachable"));
    }

    #[test]
    fn parse_mesh_json_hub_listening() {
        let result = serde_json::json!({
            "relay_enabled": true,
            "reachable_peers": 0
        });
        let (ok, detail) = parse_mesh_json(&result);
        assert!(ok, "hub should be OK even with zero peers");
        assert!(detail.contains("hub listening"));
    }

    #[test]
    fn parse_mesh_json_zero_everything() {
        let result = serde_json::json!({});
        let (ok, detail) = parse_mesh_json(&result);
        assert!(!ok);
        assert!(detail.contains("0 peers"));
    }

    #[test]
    fn parse_mesh_json_peer_array() {
        let result = serde_json::json!({
            "peers": ["gate1", "gate2"],
            "reachable": 1
        });
        let (ok, detail) = parse_mesh_json(&result);
        assert!(ok);
        assert!(detail.contains("2 peers"));
    }

    #[test]
    fn parse_mesh_response_valid_jsonrpc() {
        let resp = r#"{"jsonrpc":"2.0","result":{"peers":4,"reachable":3},"id":1}"#;
        let (ok, detail) = parse_mesh_response(resp);
        assert!(ok);
        assert!(detail.contains("4 peers"));
        assert!(detail.contains("3 reachable"));
    }

    #[test]
    fn parse_mesh_response_error() {
        let resp =
            r#"{"jsonrpc":"2.0","error":{"code":-32601,"message":"method not found"},"id":1}"#;
        let (ok, detail) = parse_mesh_response(resp);
        assert!(!ok);
        assert!(detail.contains("method not found"));
    }

    #[test]
    fn parse_mesh_response_malformed_with_result_keyword() {
        let resp = r#"not json but has "result" in it"#;
        let (ok, detail) = parse_mesh_response(resp);
        assert!(ok);
        assert_eq!(detail, "mesh responding");
    }

    #[test]
    fn parse_mesh_response_malformed_no_result() {
        let resp = "garbage data";
        let (ok, detail) = parse_mesh_response(resp);
        assert!(!ok);
        assert!(detail.contains("unexpected"));
    }

    #[test]
    fn resolve_primal_socket_paths_includes_socket_base() {
        let paths = resolve_primal_socket_paths("beardog");
        assert!(paths.iter().any(|p| p.contains("beardog.sock")));
        assert!(paths.len() >= 2);
    }

    #[test]
    fn check_cert_days_unreachable_returns_negative() {
        let days = check_cert_days("unreachable.invalid.test");
        assert!(days <= 0, "unreachable domain should return <=0 days");
    }

    #[test]
    fn resolve_uid_returns_non_empty() {
        let uid = resolve_uid();
        assert!(!uid.is_empty());
        assert!(uid.parse::<u32>().is_ok(), "UID should be numeric");
    }
}
