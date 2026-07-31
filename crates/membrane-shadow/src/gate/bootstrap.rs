// SPDX-License-Identifier: AGPL-3.0-or-later

//! Gate bootstrap — full enrollment orchestration.
//!
//! Phases: detect arch → permissions → fetch depot → verify checksums (git + WAN) →
//! sandbox validate → install (hardlink to /opt/membrane) → start NUCLEUS (systemd) →
//! mesh.init (songbird → VPS peer) → health sweep → emit deployment.toml.

use crate::config::ShadowConfig;
use crate::error::Result;
use serde::{Deserialize, Serialize};

/// Maximum time for any single bootstrap phase before it's marked failed.
const PHASE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(
    cellmembrane_types::service::DEFAULT_BOOTSTRAP_PHASE_TIMEOUT_SECS,
);

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

pub(super) async fn timed_phase<F>(name: &str, fut: F) -> BootstrapPhase
where
    F: std::future::Future<Output = BootstrapPhase>,
{
    tokio::time::timeout(PHASE_TIMEOUT, fut)
        .await
        .unwrap_or_else(|_| BootstrapPhase {
            name: name.into(),
            ok: false,
            detail: format!("timeout after {}s", PHASE_TIMEOUT.as_secs()),
        })
}

/// Run a sync phase on the blocking threadpool to avoid stalling the executor.
async fn blocking_phase<F>(name: &'static str, f: F) -> BootstrapPhase
where
    F: FnOnce() -> BootstrapPhase + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .unwrap_or_else(|_| BootstrapPhase {
            name: name.into(),
            ok: false,
            detail: "task panicked".into(),
        })
}

/// Orchestrate full gate enrollment in one command.
///
/// Phases: detect arch → set permissions → fetch depot → verify checksums →
/// sandbox validate → install primals → generate secrets → write systemd units →
/// start NUCLEUS → mesh.init → health sweep → emit deployment.toml.
///
/// With `dry_run = true`, reports what would happen without executing side effects.
pub async fn bootstrap(
    config: &ShadowConfig,
    gate_name: &str,
    dry_run: bool,
    mobility: cellmembrane_types::GateMobility,
) -> Result<BootstrapResult> {
    let arch = crate::plasmid::detect_target_triple();
    let mut phases: Vec<BootstrapPhase> = Vec::new();

    let transport = super::mesh::resolve_gate_transport(gate_name);

    phases.push(BootstrapPhase {
        name: "arch.detect".into(),
        ok: true,
        detail: format!("{arch} ({mobility}) transport={transport}"),
    });

    phases.push(blocking_phase("permissions.set", move || permissions_phase(dry_run)).await);

    phases.push(
        blocking_phase("crash_loop.preflight", || {
            let report = super::crash_loop::scan_and_break(None);
            let detail = if report.loops.is_empty() {
                format!("scanned {} units — no crash loops", report.scanned)
            } else {
                format!(
                    "scanned {} — disabled {} crash-looping services",
                    report.scanned,
                    report.disabled_count()
                )
            };
            BootstrapPhase {
                name: "crash_loop.preflight".into(),
                ok: true,
                detail,
            }
        })
        .await,
    );

    phases.push(blocking_phase("identity.git", identity_phase).await);

    phases.push(timed_phase("depot.fetch", fetch_phase(config, transport, dry_run)).await);

    let verify_arch = arch;
    let verify_probe =
        tokio::task::spawn_blocking(move || super::verify::verify_local_depot(verify_arch))
            .await
            .unwrap_or_else(|_| super::ProbeResult::fail("spawn_blocking failed"));
    phases.push(BootstrapPhase {
        name: "checksum.git".into(),
        ok: verify_probe.ok,
        detail: if dry_run {
            format!("dry-run: would verify — current: {}", verify_probe.detail)
        } else {
            verify_probe.detail
        },
    });

    phases.push(
        timed_phase(
            "checksum.wan",
            super::verify::verify_wan_checksums(arch, dry_run),
        )
        .await,
    );

    phases.push(timed_phase("sign.verify", sign_verify_phase(dry_run)).await);

    phases.push(timed_phase("sandbox.validate", sandbox_phase(arch, dry_run)).await);

    let install_arch = arch;
    phases.push(blocking_phase("install.link", move || install_phase(install_arch, dry_run)).await);

    let nucleus_arch = arch;
    phases.push(
        blocking_phase("nucleus.start", move || {
            super::nucleus::nucleus_phase(nucleus_arch, dry_run)
        })
        .await,
    );

    if !dry_run {
        tokio::time::sleep(std::time::Duration::from_secs(
            cellmembrane_types::service::MESH_SOCKET_WAIT_INTERVAL_SECS,
        ))
        .await;
    }
    phases.push(
        timed_phase(
            "mesh.configure",
            super::mesh::mesh_phase(gate_name, arch, dry_run),
        )
        .await,
    );
    phases.push(timed_phase("health.sweep", health_phase(arch, dry_run)).await);

    if mobility.needs_reconnect_hook() {
        let mob_gate = gate_name.to_string();
        phases.push(
            blocking_phase("mobility.hook", move || mobility_phase(&mob_gate, dry_run)).await,
        );
    }

    let all_pass = phases.iter().all(|p| p.ok);

    let emit_gate = gate_name.to_string();
    let emit_arch = arch;
    phases.push(
        blocking_phase("deployment.emit", move || {
            emit_deployment_toml(&emit_gate, emit_arch, mobility, dry_run, all_pass)
        })
        .await,
    );

    Ok(BootstrapResult {
        gate_name: gate_name.to_string(),
        arch: arch.to_string(),
        phases,
        all_pass,
    })
}

// Phase implementations live in `bootstrap_phases.rs`.
use super::bootstrap_phases::{
    emit_deployment_toml, fetch_phase, health_phase, identity_phase, install_phase, mobility_phase,
    permissions_phase, sandbox_phase, sign_verify_phase,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_phase_serializes() {
        let phase = BootstrapPhase {
            name: "depot.fetch".into(),
            ok: true,
            detail: "13/13 fetched".into(),
        };
        let json = serde_json::to_string(&phase).unwrap();
        assert!(json.contains("depot.fetch"));
        assert!(json.contains("13/13"));
    }

    #[test]
    fn bootstrap_result_all_pass() {
        let result = BootstrapResult {
            gate_name: "testGate".into(),
            arch: "x86_64-unknown-linux-musl".into(),
            phases: vec![
                BootstrapPhase {
                    name: "fetch".into(),
                    ok: true,
                    detail: "done".into(),
                },
                BootstrapPhase {
                    name: "health".into(),
                    ok: true,
                    detail: "ok".into(),
                },
            ],
            all_pass: true,
        };
        assert!(result.all_pass);
        assert_eq!(result.phases.len(), 2);
    }

    #[test]
    fn bootstrap_result_partial_failure() {
        let result = BootstrapResult {
            gate_name: "testGate".into(),
            arch: "x86_64-unknown-linux-musl".into(),
            phases: vec![
                BootstrapPhase {
                    name: "fetch".into(),
                    ok: true,
                    detail: "done".into(),
                },
                BootstrapPhase {
                    name: "health".into(),
                    ok: false,
                    detail: "timeout after 120s".into(),
                },
            ],
            all_pass: false,
        };
        assert!(!result.all_pass);
        assert!(!result.phases[1].ok);
    }

    #[test]
    fn emit_deployment_toml_dry_run() {
        let phase = emit_deployment_toml(
            "testGate",
            "x86_64-unknown-linux-musl",
            cellmembrane_types::GateMobility::Fixed,
            true,
            true,
        );
        assert!(phase.ok);
        assert!(phase.detail.contains("dry-run"));
        assert!(phase.detail.contains("deployment.toml"));
    }

    #[test]
    fn phase_timeout_is_configured() {
        assert!(
            PHASE_TIMEOUT.as_secs() >= 60,
            "bootstrap phase timeout should be at least 60s"
        );
    }
}
