// SPDX-License-Identifier: AGPL-3.0-or-later

//! Cascade-triggered NUCLEUS restart — converge running primals to depot binaries.
//!
//! Compares the running binary (via BLAKE3 hash) against the depot binary.
//! If they differ, sandbox-validates then restarts the service unit.
//! This enables a "pull and converge" workflow without manual intervention.

/// Restart local NUCLEUS processes whose binaries were updated in the depot.
pub(super) async fn run_cascade_restart(lines: &mut Vec<String>) {
    let arch = crate::plasmid::detect_target_triple();
    let depot_dir = crate::plasmid::resolve_path(
        None,
        cellmembrane_types::service::ENV_PLASMIDBIN_DEPOT,
        || {
            std::path::PathBuf::from(cellmembrane_types::service::env_or(
                cellmembrane_types::service::ENV_ECOPRIMALS_ROOT,
                cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT,
            ))
            .join(cellmembrane_types::service::PLASMID_BIN_DIR)
        },
    );
    let bin_dir = depot_dir.join("primals").join(arch);

    let install_base = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_INSTALL_BASE,
        cellmembrane_types::service::DEFAULT_INSTALL_BASE,
    );
    let install_dir = std::path::Path::new(&install_base);

    let gate = crate::gate::resolve_local_gate_identity();
    let primals = crate::plasmid::resolve_gate_primals(&gate);
    let mut restarted = 0u32;
    let mut skipped = 0u32;
    let mut failed = 0u32;

    for primal in &primals {
        let depot_bin = bin_dir.join(primal);
        let installed_bin = install_dir.join(primal);

        if !depot_bin.exists() || !installed_bin.exists() {
            continue;
        }

        let depot_hash = match crate::plasmid::compute_blake3_file_async(&depot_bin).await {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(primal, error = %e, "cascade-restart: cannot hash depot binary");
                failed += 1;
                continue;
            }
        };
        let installed_hash = match crate::plasmid::compute_blake3_file_async(&installed_bin).await {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(primal, error = %e, "cascade-restart: cannot hash installed binary");
                failed += 1;
                continue;
            }
        };

        if depot_hash == installed_hash {
            skipped += 1;
            continue;
        }

        let sandbox_args = crate::plasmid::sandbox::SandboxArgs {
            primal: primal.clone(),
            commit: depot_hash[..8].to_string(),
            binary_path: depot_bin.clone(),
            timeout_secs: None,
        };

        let sandbox_ok = match crate::plasmid::sandbox::validate_with_deps(&sandbox_args).await {
            Ok(result) => result.health_ok,
            Err(e) => {
                lines.push(format!(
                    "  [cascade-restart] {primal} sandbox infra error (proceeding): {e}"
                ));
                true
            }
        };

        if !sandbox_ok {
            lines.push(format!(
                "  [cascade-restart] {primal} sandbox FAIL — skipping"
            ));
            failed += 1;
            continue;
        }

        if installed_bin.exists() {
            if let Err(e) = crate::plasmid::canary::retire_to_canary(
                primal,
                &installed_bin,
                &installed_hash[..8],
            )
            .await
            {
                tracing::warn!(error = %e, primal, "canary retirement failed");
            }
        }

        if tokio::fs::copy(&depot_bin, &installed_bin).await.is_err() {
            failed += 1;
            continue;
        }

        let unit = format!("{primal}-membrane.service");
        if crate::gate::nucleus::systemctl_async(&["restart", &unit]).await {
            restarted += 1;
        } else {
            failed += 1;
        }
    }

    let tag = if failed == 0 { "OK" } else { "PARTIAL" };
    lines.push(format!(
        "  [cascade-restart] {tag} — {restarted} restarted, {skipped} current, {failed} failed"
    ));
}
