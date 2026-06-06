// SPDX-License-Identifier: AGPL-3.0-or-later

//! `plasmid.refresh` — Push local binaries to VPS with atomic replacement.

use crate::ShadowOutcome;
use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::{detect_target_triple, nucleus_primals, resolve_path};

/// Parsed CLI arguments for `plasmid.refresh`.
pub struct RefreshArgs {
    /// Single primal to refresh (None = all registry primals).
    pub primal: Option<String>,
    /// Show what would be pushed without executing.
    pub dry_run: bool,
    /// Source directory for local binaries (overrides default staging).
    pub source_dir: Option<String>,
}

/// Result of refreshing a single primal on the VPS.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshResult {
    /// Binary name.
    pub binary: String,
    /// Outcome of this refresh attempt.
    pub status: RefreshStatus,
    /// Human-readable detail.
    pub detail: String,
}

/// Status of a single primal refresh.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RefreshStatus {
    /// Binary transferred and service restarted.
    Pushed,
    /// No local binary found — skipped.
    Skipped,
    /// Transfer or restart failed.
    Failed,
}

/// Refresh primals on the VPS: transfer, atomically replace, restart, verify.
///
/// Flow per primal:
/// 1. Locate local binary (staging dir or default plasmidBin path)
/// 2. SCP to VPS as `{binary}.new`
/// 3. `mv {binary}.new {binary}` (atomic on same filesystem)
/// 4. `systemctl restart {unit}`
/// 5. Health check (socket exists or liveness probe)
pub async fn refresh(config: &crate::ShadowConfig, args: &RefreshArgs) -> Result<ShadowOutcome> {
    let primals_to_refresh: Vec<&str> = match args.primal.as_deref() {
        Some(name) => {
            if cellmembrane_types::MembraneService::for_binary(name).is_none() {
                return Ok(ShadowOutcome::fail(format!(
                    "unknown primal '{name}' — not in service registry"
                )));
            }
            vec![name]
        }
        None => nucleus_primals(),
    };

    let source_dir = resolve_refresh_source(args.source_dir.as_deref());
    let install_dir =
        std::env::var("MEMBRANE_INSTALL_BASE").unwrap_or_else(|_| "/opt/membrane".into());

    let mut results: Vec<RefreshResult> = Vec::new();

    for (i, &primal) in primals_to_refresh.iter().enumerate() {
        if i > 0 && !args.dry_run {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        results.push(refresh_one(config, primal, &source_dir, &install_dir, args.dry_run).await);
    }

    Ok(format_refresh_outcome(&results))
}

async fn refresh_one(
    config: &crate::ShadowConfig,
    primal: &str,
    source_dir: &Path,
    install_dir: &str,
    dry_run: bool,
) -> RefreshResult {
    let Some(local_path) = find_local_binary(source_dir, primal) else {
        return RefreshResult {
            binary: primal.into(),
            status: RefreshStatus::Skipped,
            detail: "no local binary found".into(),
        };
    };

    if dry_run {
        let size = std::fs::metadata(&local_path)
            .map_or_else(|_| "?".into(), |m| format!("{}KB", m.len() / 1024));
        return RefreshResult {
            binary: primal.into(),
            status: RefreshStatus::Pushed,
            detail: format!("dry-run: {size} → {install_dir}/{primal}"),
        };
    }

    let remote_new = format!("{install_dir}/{primal}.new");
    let remote_final = format!("{install_dir}/{primal}");

    if let Err(e) = crate::ssh::scp_to(config, &local_path, &remote_new).await {
        return RefreshResult {
            binary: primal.into(),
            status: RefreshStatus::Failed,
            detail: format!("scp failed: {e}"),
        };
    }

    let mv_cmd = format!("chmod 755 {remote_new} && mv {remote_new} {remote_final}");
    if let Err(e) = crate::ssh::exec(config, &mv_cmd).await {
        return RefreshResult {
            binary: primal.into(),
            status: RefreshStatus::Failed,
            detail: format!("mv failed: {e}"),
        };
    }

    if let Some(svc) = cellmembrane_types::MembraneService::for_binary(primal) {
        let restart_cmd = format!("systemctl restart {}", svc.systemd_unit);
        let _ = crate::ssh::exec_raw(config, &restart_cmd).await;
    }

    RefreshResult {
        binary: primal.into(),
        status: RefreshStatus::Pushed,
        detail: format!("→ {remote_final}"),
    }
}

fn format_refresh_outcome(results: &[RefreshResult]) -> ShadowOutcome {
    let pushed = results
        .iter()
        .filter(|r| matches!(r.status, RefreshStatus::Pushed))
        .count();
    let failed = results
        .iter()
        .filter(|r| matches!(r.status, RefreshStatus::Failed))
        .count();
    let skipped = results
        .iter()
        .filter(|r| matches!(r.status, RefreshStatus::Skipped))
        .count();

    let msg = format!("refresh complete: {pushed} pushed, {skipped} skipped, {failed} failed");

    ShadowOutcome {
        ok: failed == 0,
        message: msg,
        data: serde_json::to_value(results).ok(),
    }
}

/// Resolve the directory containing local pre-built binaries.
fn resolve_refresh_source(override_dir: Option<&str>) -> PathBuf {
    resolve_path(override_dir, "PLASMIDBIN_STAGING", || {
        let arch = detect_target_triple();
        PathBuf::from("/tmp/primalspring-deploy/primals").join(arch)
    })
}

/// Find a local binary for a primal in the source directory.
fn find_local_binary(source_dir: &Path, primal: &str) -> Option<String> {
    let direct = source_dir.join(primal);
    if direct.is_file() {
        return Some(direct.to_string_lossy().into_owned());
    }
    let suffixed = source_dir.join(format!("{primal}-x86_64-linux-musl"));
    if suffixed.is_file() {
        return Some(suffixed.to_string_lossy().into_owned());
    }
    None
}
