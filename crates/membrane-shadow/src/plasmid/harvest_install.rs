// SPDX-License-Identifier: AGPL-3.0-or-later

//! NUCLEUS install lifecycle — stop, atomic copy, restart.
//!
//! After harvest builds primal binaries into the depot staging directory,
//! this module bridges them to the NUCLEUS install path (CI-DIV-03 separation).

use std::path::Path;
use tracing::{info, warn};

use super::harvest::{HarvestResult, HarvestStatus};

const NUCLEUS_STOP_GRACE_SECS: u64 = 1;
const PKILL_SETTLE_MS: u64 = 500;

/// Stop NUCLEUS, atomically install built binaries, restart NUCLEUS.
///
/// The harvest depot and NUCLEUS install path are different directories
/// (CI-DIV-03). This bridges them: for each built primal, copies the
/// binary from the harvest depot to the NUCLEUS install path using
/// atomic rename (write .new, mv over original).
pub(super) async fn install_and_restart(
    results: &[HarvestResult],
    depot_dir: &Path,
) -> (String, bool) {
    let built: Vec<&str> = results
        .iter()
        .filter(|r| matches!(r.status, HarvestStatus::Built))
        .map(|r| r.binary.as_str())
        .collect();

    if built.is_empty() {
        return ("install: nothing to install".to_string(), true);
    }

    let target = super::detect_target_triple();
    let harvest_staging = depot_dir.join("primals").join(target);
    let install_dir = crate::resolve_xdg_data_home()
        .join("ecoPrimals")
        .join(cellmembrane_types::service::PLASMID_BIN_DIR)
        .join("primals")
        .join(target);

    if !install_dir.exists() {
        if let Err(e) = std::fs::create_dir_all(&install_dir) {
            return (
                format!("install: failed to create install dir — {e}"),
                false,
            );
        }
    }

    let stop_ok = std::process::Command::new("sudo")
        .args(["systemctl", "stop", "membrane-nucleus.target"])
        .status()
        .is_ok_and(|s| s.success());

    if !stop_ok {
        warn!("NUCLEUS stop returned non-zero — continuing with pkill fallback");
    }

    tokio::time::sleep(std::time::Duration::from_secs(NUCLEUS_STOP_GRACE_SECS)).await;
    for &primal in &built {
        if let Err(e) = std::process::Command::new("pkill")
            .args(["-f", primal])
            .status()
        {
            tracing::warn!(primal, error = %e, "pkill fallback failed — process may linger");
        }
    }
    tokio::time::sleep(std::time::Duration::from_millis(PKILL_SETTLE_MS)).await;

    let (installed, failed) = atomic_copy_binaries(&built, &harvest_staging, &install_dir);

    let start_ok = std::process::Command::new("sudo")
        .args(["systemctl", "start", "membrane-nucleus.target"])
        .status()
        .is_ok_and(|s| s.success());

    if failed.is_empty() && start_ok {
        info!(installed, "NUCLEUS install + restart complete");
        (
            format!(
                "install: {installed}/{} installed, NUCLEUS restarted",
                built.len()
            ),
            true,
        )
    } else {
        let fail_msg = if failed.is_empty() {
            String::new()
        } else {
            format!(" failures=[{}]", failed.join("; "))
        };
        let restart_msg = if start_ok {
            ""
        } else {
            " NUCLEUS restart FAILED"
        };
        warn!(
            installed,
            failures = failed.len(),
            "install completed with issues"
        );
        (
            format!(
                "install: {installed}/{} installed{fail_msg}{restart_msg}",
                built.len()
            ),
            false,
        )
    }
}

fn atomic_copy_binaries(
    built: &[&str],
    staging: &Path,
    install_dir: &Path,
) -> (usize, Vec<String>) {
    let mut installed = 0usize;
    let mut failed = Vec::new();
    for &primal in built {
        let src = staging.join(primal);
        let dst = install_dir.join(primal);
        let tmp = install_dir.join(format!("{primal}.new"));

        if !src.exists() {
            failed.push(format!("{primal}: not in harvest depot"));
            continue;
        }

        match std::fs::copy(&src, &tmp) {
            Ok(_) => match std::fs::rename(&tmp, &dst) {
                Ok(()) => {
                    installed += 1;
                }
                Err(e) => {
                    failed.push(format!("{primal}: rename failed — {e}"));
                    if let Err(e) = std::fs::remove_file(&tmp) {
                        tracing::debug!(%primal, path = %tmp.display(), %e, "tmp cleanup after failed rename");
                    }
                }
            },
            Err(e) => {
                failed.push(format!("{primal}: copy failed — {e}"));
            }
        }
    }
    (installed, failed)
}
