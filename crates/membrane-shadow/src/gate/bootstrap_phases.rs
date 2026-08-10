// SPDX-License-Identifier: AGPL-3.0-or-later

//! Bootstrap phase implementations — each phase returns a `BootstrapPhase` result.
//!
//! Extracted from `bootstrap.rs` for independent readability. The orchestrator
//! in `bootstrap.rs` calls these via `timed_phase` / `blocking_phase` wrappers.

use super::bootstrap::BootstrapPhase;

/// Check if a git global config key is set and non-empty.
fn git_global_config_is_set(key: &str) -> bool {
    std::process::Command::new("git")
        .args(["config", "--global", key])
        .output()
        .ok()
        .is_some_and(|o| o.status.success() && !o.stdout.is_empty())
}

pub(super) fn identity_phase() -> BootstrapPhase {
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

pub(super) fn permissions_phase(dry_run: bool) -> BootstrapPhase {
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

    let socket_base = cellmembrane_types::service::resolve_socket_base();
    for dir in [
        membrane_dir.as_str(),
        depot_str.as_str(),
        socket_base.as_str(),
    ] {
        if std::fs::create_dir_all(dir).is_ok() {
            if cellmembrane_types::PlatformAccess::Executable
                .apply(std::path::Path::new(dir))
                .is_ok()
            {
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
        if let Err(e) = cellmembrane_types::PlatformAccess::Executable.apply(dest) {
            tracing::warn!(error = %e, path = %dest.display(), "set executable failed");
        }
        true
    } else {
        false
    }
}

pub(super) fn install_phase(arch: &str, dry_run: bool) -> BootstrapPhase {
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

pub(super) async fn sign_verify_phase(dry_run: bool) -> BootstrapPhase {
    let depot = super::local::resolve_plasmidbin_dir();
    let probe = tokio::task::spawn_blocking(move || {
        let valid = crate::plasmid::signing::verify_depot_with_policy(
            &depot,
            cellmembrane_types::DepotTrustPolicy::RequireSigned,
        );
        if valid {
            super::ProbeResult::pass("Ed25519 signature verified")
        } else {
            super::ProbeResult::fail(
                "Ed25519 signature verification FAILED — depot unsigned or tampered",
            )
        }
    })
    .await
    .unwrap_or_else(|_| super::ProbeResult::fail("spawn_blocking failed"));

    BootstrapPhase {
        name: "sign.verify".into(),
        ok: if dry_run { true } else { probe.ok },
        detail: if dry_run {
            format!("dry-run: would verify — current: {}", probe.detail)
        } else {
            probe.detail
        },
    }
}

pub(super) async fn fetch_phase(
    config: &crate::config::ShadowConfig,
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
        trust_policy: cellmembrane_types::DepotTrustPolicy::RequireSigned,
    };
    let probe = match crate::plasmid::fetch(config, &fetch_args).await {
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
            super::ProbeResult {
                ok: failed == 0,
                detail: format!("{downloaded} downloaded, {failed} failed (via {source})"),
            }
        }
        Err(e) => super::ProbeResult::fail(format!("fetch error: {e}")),
    };
    BootstrapPhase {
        name: "depot.fetch".into(),
        ok: probe.ok,
        detail: probe.detail,
    }
}

pub(super) async fn sandbox_phase(arch: &str, dry_run: bool) -> BootstrapPhase {
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
        match crate::plasmid::sandbox::validate_with_deps(&args).await {
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

pub(super) async fn health_phase(arch: &str, dry_run: bool) -> BootstrapPhase {
    if dry_run {
        return BootstrapPhase {
            name: "health.sweep".into(),
            ok: true,
            detail: "dry-run: would probe all NUCLEUS primals".into(),
        };
    }
    let probe = super::health::health_sweep(arch).await;
    BootstrapPhase {
        name: "health.sweep".into(),
        ok: probe.ok,
        detail: probe.detail,
    }
}

pub(super) fn mobility_phase(gate_name: &str, dry_run: bool) -> BootstrapPhase {
    if dry_run {
        return BootstrapPhase {
            name: "mobility.hook".into(),
            ok: true,
            detail: "dry-run: would write NM dispatcher reconnect hook + gate-name".into(),
        };
    }

    let mut details = Vec::new();

    let gate_name_written = write_gate_name_file(gate_name);
    if gate_name_written {
        details.push("gate-name written".to_string());
    } else {
        details.push("gate-name write failed (non-fatal)".to_string());
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

    let hook_ok = crate::atomic_write(&hook_path, hook_content.as_bytes()).is_ok()
        && cellmembrane_types::PlatformAccess::Executable
            .apply(&hook_path)
            .is_ok();

    if hook_ok {
        details.push(format!("hook: {}", hook_path.display()));
    } else {
        details.push(format!(
            "hook write failed: {} (needs root?)",
            hook_path.display()
        ));
    }

    BootstrapPhase {
        name: "mobility.hook".into(),
        ok: hook_ok,
        detail: details.join("; "),
    }
}

/// Write the gate-name file so NM dispatcher hooks and external tooling
/// can resolve gate identity without the Rust binary.
///
/// Tries system-scope `/etc/membrane/gate-name` first (root gates),
/// falls back to user-scope `~/.config/membrane/gate-name`.
fn write_gate_name_file(gate_name: &str) -> bool {
    let config_dir = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_CONFIG_DIR,
        cellmembrane_types::service::DEFAULT_CONFIG_DIR,
    );
    let system_path = std::path::Path::new(&config_dir).join("gate-name");

    if std::fs::create_dir_all(&config_dir).is_ok()
        && crate::atomic_write(&system_path, format!("{gate_name}\n").as_bytes()).is_ok()
    {
        return true;
    }

    let home = cellmembrane_types::service::env_or(cellmembrane_types::service::ENV_HOME, "/tmp");
    let user_dir = std::path::Path::new(&home).join(".config/membrane");
    if std::fs::create_dir_all(&user_dir).is_ok() {
        let user_path = user_dir.join("gate-name");
        return crate::atomic_write(&user_path, format!("{gate_name}\n").as_bytes()).is_ok();
    }

    false
}

pub(super) fn emit_deployment_toml(
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

    let timestamp = crate::utc_now_iso8601();
    let hostname = std::fs::read_to_string(super::PROC_HOSTNAME)
        .map(|s| s.trim().to_string())
        .or_else(|_| std::fs::read_to_string(super::ETC_HOSTNAME).map(|s| s.trim().to_string()))
        .unwrap_or_else(|_| cellmembrane_types::service::UNKNOWN_LABEL.into());

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
    fn git_global_config_nonexistent_key_returns_false() {
        assert!(!git_global_config_is_set("nonexistent.key.xyz.test"));
    }
}
