// SPDX-License-Identifier: AGPL-3.0-or-later

//! Gate bootstrap — full enrollment orchestration.
//!
//! Phases: detect arch → fetch depot → verify checksums (git + WAN) →
//! configure mesh → start NUCLEUS → health sweep → emit deployment.toml.

use crate::config::ShadowConfig;
use crate::error::Result;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

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
/// Phases: detect arch → fetch depot → verify checksums → configure mesh →
/// start NUCLEUS → health sweep → emit deployment.toml.
/// With `dry_run = true`, reports what would happen without executing side effects.
pub async fn bootstrap(
    config: &ShadowConfig,
    gate_name: &str,
    dry_run: bool,
    mobility: cellmembrane_types::GateMobility,
) -> Result<BootstrapResult> {
    let arch = crate::plasmid::detect_target_triple();
    let mut phases: Vec<BootstrapPhase> = Vec::new();

    let transport = resolve_gate_transport(gate_name);

    phases.push(BootstrapPhase {
        name: "arch.detect".into(),
        ok: true,
        detail: format!("{arch} ({mobility}) transport={transport}"),
    });

    phases.push(fetch_phase(config, &transport, dry_run).await);

    let verify_result = super::verify::verify_local_depot(&arch);
    phases.push(BootstrapPhase {
        name: "checksum.git".into(),
        ok: verify_result.0,
        detail: if dry_run {
            format!("dry-run: would verify — current: {}", verify_result.1)
        } else {
            verify_result.1
        },
    });

    phases.push(super::verify::verify_wan_checksums(&arch, dry_run).await);

    phases.push(sandbox_phase(&arch, dry_run).await);

    phases.push(mesh_phase(gate_name, &arch, dry_run).await);
    phases.push(nucleus_phase(&arch, dry_run));
    phases.push(health_phase(&arch, dry_run).await);

    if mobility.needs_reconnect_hook() {
        phases.push(mobility_phase(gate_name, dry_run));
    }

    let all_pass = phases.iter().all(|p| p.ok);

    phases.push(emit_deployment_toml(
        gate_name, &arch, mobility, dry_run, all_pass,
    ));

    Ok(BootstrapResult {
        gate_name: gate_name.to_string(),
        arch,
        phases,
        all_pass,
    })
}

// ── Phase implementations ──────────────────────────────────────────────

fn resolve_gate_transport(gate_name: &str) -> String {
    let Ok(workspace_root) = crate::temporal::resolve_workspace_root() else {
        return "wan".into();
    };
    let Ok(manifest) = crate::manifest::load_from_workspace(&workspace_root) else {
        return "wan".into();
    };
    manifest
        .gates
        .get(gate_name)
        .and_then(|p| p.transport.clone())
        .unwrap_or_else(|| "wan".into())
}

/// Map a profile transport string to the appropriate `FetchSource`.
///
/// `local` uses SSH/rsync (VPS layer). All remote transports currently
/// resolve to WAN HTTPS. As LAN rsync and ADB push mature, they will
/// diverge from the WAN fallback.
fn transport_to_fetch_source(transport: &str) -> crate::plasmid::FetchSource {
    match transport {
        "local" => crate::plasmid::FetchSource::Vps,
        _ => crate::plasmid::FetchSource::Wan,
    }
}

async fn fetch_phase(config: &ShadowConfig, transport: &str, dry_run: bool) -> BootstrapPhase {
    let source = transport_to_fetch_source(transport);
    if dry_run {
        return BootstrapPhase {
            name: "depot.fetch".into(),
            ok: true,
            detail: format!(
                "dry-run: would fetch all primals via {source} (transport={transport})"
            ),
        };
    }
    let fetch_args = crate::plasmid::FetchArgs {
        source,
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
                format!("{downloaded} downloaded, {failed} failed (via {source})"),
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

async fn sandbox_phase(arch: &str, dry_run: bool) -> BootstrapPhase {
    if dry_run {
        return BootstrapPhase {
            name: "sandbox.validate".into(),
            ok: true,
            detail: "dry-run: would sandbox-validate Tower primals before install".into(),
        };
    }

    let Ok(depot_dir) = crate::plasmid::depot::resolve_depot(None) else {
        return BootstrapPhase {
            name: "sandbox.validate".into(),
            ok: true,
            detail: "skipped: depot not resolved (sandbox validation optional)".into(),
        };
    };

    let bin_dir = depot_dir.join("primals").join(arch);
    if !bin_dir.exists() {
        return BootstrapPhase {
            name: "sandbox.validate".into(),
            ok: true,
            detail: format!("skipped: no binaries at {}", bin_dir.display()),
        };
    }

    let tower_services = cellmembrane_types::MembraneService::for_composition(
        cellmembrane_types::MembraneComposition::Tower,
    );
    let tower_primals: Vec<&str> = tower_services.iter().map(|s| s.binary).collect();
    let mut passed = 0u32;
    let mut failed = 0u32;
    let mut details = Vec::new();

    for primal in &tower_primals {
        let binary_path = bin_dir.join(primal);
        if !binary_path.exists() {
            continue;
        }
        let args = crate::plasmid::sandbox::SandboxArgs {
            primal: (*primal).to_string(),
            commit: "bootstrap".into(),
            binary_path,
            timeout_secs: Some(20),
        };
        match crate::plasmid::sandbox::validate(&args).await {
            Ok(result) if result.health_ok => {
                passed += 1;
                details.push(format!("{primal}:PASS"));
            }
            Ok(result) => {
                failed += 1;
                details.push(format!("{primal}:FAIL({})", result.detail));
            }
            Err(e) => {
                details.push(format!("{primal}:SKIP({e})"));
            }
        }
    }

    let ok = failed == 0;
    let detail = format!("{passed} passed, {failed} failed [{}]", details.join(", "));
    BootstrapPhase {
        name: "sandbox.validate".into(),
        ok,
        detail,
    }
}

async fn mesh_phase(gate_name: &str, arch: &str, dry_run: bool) -> BootstrapPhase {
    if dry_run {
        let vps_peer = std::env::var(cellmembrane_types::service::ENV_VPS_MESH_PEER)
            .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_VPS_MESH_PEER.into());
        return BootstrapPhase {
            name: "mesh.configure".into(),
            ok: true,
            detail: format!("dry-run: would mesh.init to {vps_peer} as {gate_name}"),
        };
    }
    let (ok, detail) = configure_mesh(gate_name, arch).await;
    BootstrapPhase {
        name: "mesh.configure".into(),
        ok,
        detail,
    }
}

fn nucleus_phase(arch: &str, dry_run: bool) -> BootstrapPhase {
    if dry_run {
        return BootstrapPhase {
            name: "nucleus.start".into(),
            ok: true,
            detail: "dry-run: would start NUCLEUS primals".into(),
        };
    }
    let (ok, detail) = start_nucleus_primals(arch);
    BootstrapPhase {
        name: "nucleus.start".into(),
        ok,
        detail,
    }
}

async fn health_phase(arch: &str, dry_run: bool) -> BootstrapPhase {
    if dry_run {
        return BootstrapPhase {
            name: "health.sweep".into(),
            ok: true,
            detail: "dry-run: would probe all NUCLEUS primals".into(),
        };
    }
    let (ok, detail) = super::health::health_sweep(arch).await;
    BootstrapPhase {
        name: "health.sweep".into(),
        ok,
        detail,
    }
}

fn mobility_phase(gate_name: &str, dry_run: bool) -> BootstrapPhase {
    if dry_run {
        return BootstrapPhase {
            name: "mobility.hook".into(),
            ok: true,
            detail: "dry-run: would write NM dispatcher reconnect hook".into(),
        };
    }

    let hook_dir_str = std::env::var("NM_DISPATCHER_DIR")
        .unwrap_or_else(|_| "/etc/NetworkManager/dispatcher.d".into());
    let hook_dir = std::path::Path::new(&hook_dir_str);
    let hook_path = hook_dir.join("99-membrane-reconnect");
    let hook_content = format!(
        "#!/bin/sh\n\
         # Auto-generated by gate.bootstrap for mobile gate: {gate_name}\n\
         [ \"$2\" = \"up\" ] && membrane gate.status --quiet 2>/dev/null &\n"
    );

    let ok = crate::atomic_write(&hook_path, hook_content.as_bytes()).is_ok()
        && std::process::Command::new("chmod")
            .args(["755", &hook_path.display().to_string()])
            .status()
            .is_ok_and(|s| s.success());

    BootstrapPhase {
        name: "mobility.hook".into(),
        ok,
        detail: if ok {
            format!("wrote {}", hook_path.display())
        } else {
            format!("failed to write {} (needs root?)", hook_path.display())
        },
    }
}

fn emit_deployment_toml(
    gate_name: &str,
    arch: &str,
    mobility: cellmembrane_types::GateMobility,
    dry_run: bool,
    all_pass: bool,
) -> BootstrapPhase {
    let install_base = std::env::var(cellmembrane_types::service::ENV_INSTALL_BASE)
        .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_INSTALL_BASE.into());
    let deployment_path = std::path::Path::new(&install_base).join("deployment.toml");

    if dry_run {
        return BootstrapPhase {
            name: "deployment.emit".into(),
            ok: true,
            detail: format!("dry-run: would write {}", deployment_path.display()),
        };
    }

    let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
    let hostname = match std::process::Command::new("hostname").output() {
        Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        Err(_) => "unknown".into(),
    };

    let content = format!(
        "# deployment.toml — gate.bootstrap provenance record\n\
         # guideStone P2: Reference-Traceable\n\
         \n\
         [deployment]\n\
         gate = \"{gate_name}\"\n\
         arch = \"{arch}\"\n\
         mobility = \"{mobility}\"\n\
         hostname = \"{hostname}\"\n\
         timestamp = \"{timestamp}\"\n\
         all_pass = {all_pass}\n\
         membrane_version = \"{}\"\n",
        env!("CARGO_PKG_VERSION"),
    );

    let ok = crate::atomic_write(&deployment_path, content.as_bytes()).is_ok();

    BootstrapPhase {
        name: "deployment.emit".into(),
        ok,
        detail: if ok {
            format!("wrote {}", deployment_path.display())
        } else {
            format!("failed to write {}", deployment_path.display())
        },
    }
}

// ── Mesh configuration (native UDS) ───────────────────────────────────

async fn configure_mesh(gate_name: &str, arch: &str) -> (bool, String) {
    let relay_binary =
        cellmembrane_types::MembraneService::binary_for(cellmembrane_types::ServiceCapability::MeshRelay);

    let dest_root = super::resolve_plasmidbin_dir();
    let relay_bin = dest_root.join("primals").join(arch).join(relay_binary);

    if !relay_bin.exists() {
        return (false, format!("{relay_binary} binary not found"));
    }

    let socket_dir = super::health::resolve_biomeos_socket_dir();
    let socket_path = std::path::PathBuf::from(&socket_dir)
        .join(format!("{relay_binary}.sock"))
        .display()
        .to_string();

    if !std::path::Path::new(&socket_path).exists() {
        return (
            false,
            format!(
                "{relay_binary} socket not found at {socket_path} — start {relay_binary} first"
            ),
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

    let request = mesh_init.to_string();
    let connect_result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::net::UnixStream::connect(&socket_path),
    )
    .await;

    let stream = match connect_result {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => return (false, format!("connect error: {e}")),
        Err(_) => return (false, "connect timeout".into()),
    };

    let (mut reader, mut writer) = stream.into_split();

    if writer
        .write_all(&crate::ribocipher::CLEAR_JSONRPC_SIGNAL)
        .await
        .is_err()
    {
        return (
            false,
            format!("failed to write signal to {relay_binary} socket"),
        );
    }
    if writer.write_all(request.as_bytes()).await.is_err() {
        return (false, format!("failed to write to {relay_binary} socket"));
    }
    let _ = writer.shutdown().await;

    let mut buf = Vec::with_capacity(4096);
    let read_result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        reader.read_to_end(&mut buf),
    )
    .await;

    match read_result {
        Ok(Ok(_)) => {
            let stdout = String::from_utf8_lossy(&buf);
            if stdout.contains("\"result\"") || stdout.contains("\"ok\"") {
                (true, format!("mesh.init sent to {vps_peer} as {gate_name}"))
            } else {
                (
                    true,
                    format!("mesh.init sent (response: {})", stdout.trim()),
                )
            }
        }
        Ok(Err(e)) => (false, format!("read error: {e}")),
        Err(_) => (false, "mesh.init response timed out".into()),
    }
}

// ── NUCLEUS start ──────────────────────────────────────────────────────

fn start_nucleus_primals(arch: &str) -> (bool, String) {
    let dest_root = super::resolve_plasmidbin_dir();
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

        if let Some(svc) = cellmembrane_types::MembraneService::for_binary(primal) {
            if svc.is_mesh_infrastructure() {
                skipped += 1;
                continue;
            }
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
