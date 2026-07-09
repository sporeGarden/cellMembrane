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
    phases.push(blocking_phase("identity.git", identity_phase).await);

    phases.push(timed_phase("depot.fetch", fetch_phase(config, transport, dry_run)).await);

    let verify_arch = arch.clone();
    let verify_result =
        tokio::task::spawn_blocking(move || super::verify::verify_local_depot(&verify_arch))
            .await
            .unwrap_or_else(|_| (false, "spawn_blocking failed".into()));
    phases.push(BootstrapPhase {
        name: "checksum.git".into(),
        ok: verify_result.0,
        detail: if dry_run {
            format!("dry-run: would verify — current: {}", verify_result.1)
        } else {
            verify_result.1
        },
    });

    phases.push(
        timed_phase(
            "checksum.wan",
            super::verify::verify_wan_checksums(&arch, dry_run),
        )
        .await,
    );

    phases.push(timed_phase("sandbox.validate", sandbox_phase(&arch, dry_run)).await);

    let install_arch = arch.clone();
    phases.push(
        blocking_phase("install.link", move || {
            install_phase(&install_arch, dry_run)
        })
        .await,
    );

    let nucleus_arch = arch.clone();
    phases.push(
        blocking_phase("nucleus.start", move || {
            super::nucleus::nucleus_phase(&nucleus_arch, dry_run)
        })
        .await,
    );

    if !dry_run {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    phases.push(
        timed_phase(
            "mesh.configure",
            super::mesh::mesh_phase(gate_name, &arch, dry_run),
        )
        .await,
    );
    phases.push(timed_phase("health.sweep", health_phase(&arch, dry_run)).await);

    if mobility.needs_reconnect_hook() {
        let mob_gate = gate_name.to_string();
        phases.push(
            blocking_phase("mobility.hook", move || mobility_phase(&mob_gate, dry_run)).await,
        );
    }

    let all_pass = phases.iter().all(|p| p.ok);

    let emit_gate = gate_name.to_string();
    let emit_arch = arch.clone();
    phases.push(
        blocking_phase("deployment.emit", move || {
            emit_deployment_toml(&emit_gate, &emit_arch, mobility, dry_run, all_pass)
        })
        .await,
    );

    Ok(BootstrapResult {
        gate_name: gate_name.to_string(),
        arch,
        phases,
        all_pass,
    })
}

// ── Phase implementations ──────────────────────────────────────────────

/// Check if a git global config key is set and non-empty.
fn git_global_config_is_set(key: &str) -> bool {
    std::process::Command::new("git")
        .args(["config", "--global", key])
        .output()
        .ok()
        .is_some_and(|o| o.status.success() && !o.stdout.is_empty())
}

fn identity_phase() -> BootstrapPhase {
    let name_set = git_global_config_is_set("user.name");
    let email_set = git_global_config_is_set("user.email");

    let ssh_ok = ssh_identity_ok();

    if name_set && email_set && ssh_ok {
        return BootstrapPhase {
            name: "identity.git".into(),
            ok: true,
            detail: "git user.name, user.email, and SSH key configured".into(),
        };
    }

    let mut missing = Vec::new();
    if !name_set {
        missing.push("user.name");
    }
    if !email_set {
        missing.push("user.email");
    }

    let mut detail = if missing.is_empty() {
        String::new()
    } else {
        format!(
            "git {} not set — run: git config --global user.name \"ecoPrimal\" \
             && git config --global user.email \"ecoPrimal@pm.me\"",
            missing.join(" and ")
        )
    };

    if !ssh_ok {
        if !detail.is_empty() {
            detail.push_str("; ");
        }
        detail.push_str("SSH key (~/.ssh/id_ed25519) not found");
    }

    BootstrapPhase {
        name: "identity.git".into(),
        ok: false,
        detail,
    }
}

fn ssh_identity_ok() -> bool {
    let home = cellmembrane_types::service::env_or(cellmembrane_types::service::ENV_HOME, "/root");
    std::path::Path::new(&home).join(".ssh/id_ed25519").exists()
}

fn permissions_phase(dry_run: bool) -> BootstrapPhase {
    let membrane_dir = super::resolve_install_base();
    let depot_dir = super::resolve_plasmidbin_dir();
    let depot_str = depot_dir.to_string_lossy().to_string();

    if dry_run {
        return BootstrapPhase {
            name: "permissions.set".into(),
            ok: true,
            detail: format!(
                "dry-run: would ensure {membrane_dir} + {depot_str} exist with correct perms"
            ),
        };
    }

    let mut ok = true;
    let mut details = Vec::new();

    for dir in [membrane_dir.as_str(), depot_str.as_str()] {
        if std::fs::create_dir_all(dir).is_ok() {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o755);
            if std::fs::set_permissions(dir, perms).is_ok() {
                details.push(format!("{dir}:OK"));
            } else {
                details.push(format!("{dir}:perms-failed"));
                ok = false;
            }
        } else {
            details.push(format!("{dir}:mkdir-failed"));
            ok = false;
        }
    }

    BootstrapPhase {
        name: "permissions.set".into(),
        ok,
        detail: details.join(", "),
    }
}

/// Hardlink or copy a binary to dest, setting 0755 permissions.
fn link_or_copy_binary(src: &std::path::Path, dest: &std::path::Path) -> bool {
    if !src.exists() {
        return false;
    }
    if let Err(e) = std::fs::remove_file(dest) {
        tracing::debug!(error = %e, "pre-link cleanup (may not exist)");
    }
    if std::fs::hard_link(src, dest).is_ok() || std::fs::copy(src, dest).is_ok() {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o755)) {
            tracing::warn!(error = %e, path = %dest.display(), "chmod 755 failed");
        }
        true
    } else {
        false
    }
}

fn install_phase(arch: &str, dry_run: bool) -> BootstrapPhase {
    let install_dir = super::resolve_install_base();

    if dry_run {
        return BootstrapPhase {
            name: "install.link".into(),
            ok: true,
            detail: format!("dry-run: would hardlink primals from depot → {install_dir}"),
        };
    }

    let depot_root = super::resolve_plasmidbin_dir();
    let bin_dir = depot_root.join("primals").join(arch);
    let target_dir = std::path::Path::new(install_dir.as_str());

    if !bin_dir.is_dir() {
        return BootstrapPhase {
            name: "install.link".into(),
            ok: false,
            detail: format!("no binaries at {}", bin_dir.display()),
        };
    }

    let mut installed = 0u32;
    let mut failed = 0u32;

    let gate = super::resolve_local_gate_identity();
    let composition_primals = crate::plasmid::resolve_gate_primals(&gate);
    for primal in &composition_primals {
        let src = bin_dir.join(primal);
        if !src.exists() {
            continue;
        }
        if link_or_copy_binary(&src, &target_dir.join(primal)) {
            installed += 1;
        } else {
            failed += 1;
        }
    }

    link_or_copy_binary(&bin_dir.join("membrane"), &target_dir.join("membrane"));

    let ok = failed == 0 && installed > 0;
    BootstrapPhase {
        name: "install.link".into(),
        ok,
        detail: format!("{installed} primals installed → {install_dir}, {failed} failed"),
    }
}

async fn fetch_phase(
    config: &ShadowConfig,
    transport: cellmembrane_types::GateTransport,
    dry_run: bool,
) -> BootstrapPhase {
    let source = super::mesh::transport_to_fetch_source(transport);
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

    let hook_dir_str = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_NM_DISPATCHER_DIR,
        cellmembrane_types::service::DEFAULT_NM_DISPATCHER_DIR,
    );
    let hook_dir = std::path::Path::new(&hook_dir_str);
    let hook_path = hook_dir.join("99-membrane-reconnect");
    let hook_content = format!(
        "#!/bin/sh\n\
         # Auto-generated by gate.bootstrap for mobile gate: {gate_name}\n\
         [ \"$2\" = \"up\" ] && membrane gate.status --quiet 2>/dev/null &\n"
    );

    let ok = crate::atomic_write(&hook_path, hook_content.as_bytes()).is_ok() && {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook_path, std::fs::Permissions::from_mode(0o755)).is_ok()
    };

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
    let install_base = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_INSTALL_BASE,
        cellmembrane_types::service::DEFAULT_INSTALL_BASE,
    );
    let deployment_path = std::path::Path::new(&install_base).join("deployment.toml");

    if dry_run {
        return BootstrapPhase {
            name: "deployment.emit".into(),
            ok: true,
            detail: format!("dry-run: would write {}", deployment_path.display()),
        };
    }

    let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
    let hostname = std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|s| s.trim().to_string())
        .or_else(|_| std::fs::read_to_string("/etc/hostname").map(|s| s.trim().to_string()))
        .unwrap_or_else(|_| "unknown".into());

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

    #[test]
    fn git_global_config_nonexistent_key_returns_false() {
        assert!(!git_global_config_is_set("nonexistent.key.xyz.test"));
    }
}
