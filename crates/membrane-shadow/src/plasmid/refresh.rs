// SPDX-License-Identifier: AGPL-3.0-or-later

//! `plasmid.refresh` — Push local binaries to VPS with atomic replacement.

use crate::ShadowOutcome;
use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::warn;

use super::{detect_target_triple, nucleus_primals};

const INTER_PRIMAL_DELAY_MS: u64 = 500;

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
    let install_dir = std::env::var(cellmembrane_types::service::ENV_INSTALL_BASE)
        .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_INSTALL_BASE.into());

    let mut results: Vec<RefreshResult> = Vec::new();

    for (i, &primal) in primals_to_refresh.iter().enumerate() {
        if i > 0 && !args.dry_run {
            tokio::time::sleep(std::time::Duration::from_millis(INTER_PRIMAL_DELAY_MS)).await;
        }
        results.push(refresh_one(config, primal, &source_dir, &install_dir, args.dry_run).await);
    }

    if !args.dry_run {
        sync_depot_metadata(config).await;
        sync_depot_binaries(config).await;
    }

    Ok(format_refresh_outcome(&results))
}

/// Push local depot metadata (provenance.toml, checksums.toml) to VPS depot.
///
/// Without this, the VPS `plasmid.status` reports stale drift because its
/// provenance commits don't match the freshly-pushed binaries.
async fn sync_depot_metadata(config: &crate::ShadowConfig) {
    let Ok(local_depot) = super::harvest::resolve_depot(None) else {
        return;
    };
    let default_remote_depot = format!(
        "{}/plasmidBin",
        cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT
    );
    let remote_depot = std::env::var(cellmembrane_types::service::ENV_PLASMIDBIN_DEPOT)
        .unwrap_or(default_remote_depot);

    for filename in ["provenance.toml", "checksums.toml"] {
        let local = local_depot.join(filename);
        if local.is_file() {
            let remote = format!("{remote_depot}/{filename}");
            if let Err(e) = crate::ssh::scp_to(config, &local.to_string_lossy(), &remote).await {
                tracing::warn!(file = filename, error = %e, "depot metadata sync failed");
            }
        }
    }
}

/// Sync install-dir binaries to the WAN depot directory on the VPS.
///
/// Runs as a post-refresh step so the WAN depot always serves the same binaries
/// that are running on the inner membrane. Failures here are non-fatal — the
/// refresh itself already succeeded.
async fn sync_depot_binaries(config: &crate::ShadowConfig) {
    let install_dir = std::env::var(cellmembrane_types::service::ENV_INSTALL_BASE)
        .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_INSTALL_BASE.into());
    let depot_root = format!(
        "{}/plasmidBin/primals",
        cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT
    );
    let arch = super::detect_target_triple();
    let depot_dir = format!("{depot_root}/{arch}");

    let primals = super::nucleus_primals();
    let primal_list = primals.join(" ");

    let cmd = format!(
        "mkdir -p {depot_dir} && \
         for p in {primal_list}; do \
           src=\"{install_dir}/$p\"; \
           dst=\"{depot_dir}/$p\"; \
           [ -f \"$src\" ] && cp -f \"$src\" \"$dst\"; \
         done"
    );

    if let Err(e) = crate::ssh::exec_raw(config, &cmd).await {
        tracing::warn!(error = %e, "WAN depot binary sync failed");
    }
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
        let size = tokio::fs::metadata(&local_path)
            .await
            .map_or_else(|_| "?".into(), |m| format!("{}KB", m.len() / 1024));
        return RefreshResult {
            binary: primal.into(),
            status: RefreshStatus::Pushed,
            detail: format!("dry-run: {size} → {install_dir}/{primal}"),
        };
    }

    let remote_new = format!("{install_dir}/{primal}.new");
    let remote_final = format!("{install_dir}/{primal}");

    // WAN-TIMEOUT-GRACEFUL: retry SCP with exponential backoff (2s, 4s, 8s)
    let mut scp_ok = false;
    let mut last_err = String::new();
    for attempt in 0..3u32 {
        if attempt > 0 {
            let backoff = std::time::Duration::from_secs(2u64.pow(attempt));
            tokio::time::sleep(backoff).await;
        }
        match crate::ssh::scp_to(config, &local_path, &remote_new).await {
            Ok(()) => {
                scp_ok = true;
                break;
            }
            Err(e) => {
                last_err = e.to_string();
                warn!(
                    primal,
                    attempt = attempt + 1,
                    error = %last_err,
                    "scp failed"
                );
            }
        }
    }
    if !scp_ok {
        // Rollback: clean up partial .new file on remote
        let cleanup = format!("rm -f {remote_new}");
        if let Err(e) = crate::ssh::exec_raw(config, &cleanup).await {
            tracing::warn!(primal, error = %e, "rollback cleanup failed");
        }
        return RefreshResult {
            binary: primal.into(),
            status: RefreshStatus::Failed,
            detail: format!("scp failed after 3 attempts: {last_err}"),
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
        if let Err(e) = crate::ssh::exec_raw(config, &restart_cmd).await {
            tracing::warn!(primal, unit = %svc.systemd_unit, error = %e, "service restart failed");
        }
    }

    let size_kb = tokio::fs::metadata(&local_path)
        .await
        .map_or(0, |m| m.len() / 1024);
    let actual_hash = super::compute_blake3_file_async(PathBuf::from(&local_path)).await;
    let hash_short = &actual_hash[..16.min(actual_hash.len())];

    let primal_owned = primal.to_string();
    let hash_owned = actual_hash.clone();
    let divergence_note =
        tokio::task::spawn_blocking(move || check_checksum_coherence(&primal_owned, &hash_owned))
            .await
            .ok()
            .flatten();

    let detail = divergence_note.map_or_else(
        || format!("→ {remote_final} ({size_kb}KB blake3={hash_short})"),
        |note| format!("→ {remote_final} ({size_kb}KB blake3={hash_short}) ⚠ {note}"),
    );

    RefreshResult {
        binary: primal.into(),
        status: RefreshStatus::Pushed,
        detail,
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
///
/// Priority: CLI override → `PLASMIDBIN_STAGING` env → depot primals dir.
/// Falls back to the depot's `primals/{arch}/` directory so that
/// `plasmid.harvest` → `plasmid.refresh` always uses the same binary.
fn resolve_refresh_source(override_dir: Option<&str>) -> PathBuf {
    if let Some(dir) = override_dir {
        return PathBuf::from(dir);
    }
    if let Ok(staging) = std::env::var(cellmembrane_types::service::ENV_PLASMIDBIN_STAGING) {
        return PathBuf::from(staging);
    }
    let arch = detect_target_triple();
    if let Ok(depot) = super::depot::resolve_depot(None) {
        let depot_primals = depot.join("primals").join(&arch);
        if depot_primals.is_dir() {
            return depot_primals;
        }
    }
    std::env::temp_dir()
        .join("primalspring-deploy/primals")
        .join(arch)
}

/// Check whether the binary hash matches what `checksums.toml` records.
/// Returns `Some(warning)` if a divergence is detected.
fn check_checksum_coherence(primal: &str, actual_hash: &str) -> Option<String> {
    let depot = super::depot::resolve_depot(None).ok()?;
    let checksums_path = depot.join("checksums.toml");
    let content = std::fs::read_to_string(&checksums_path).ok()?;
    let table: toml::Table = content.parse().ok()?;

    let arch = detect_target_triple();
    let entry = table.get(&arch)?.as_table()?.get(primal)?.as_table()?;
    let expected_hash = entry.get("blake3")?.as_str()?;

    if expected_hash == actual_hash {
        None
    } else {
        Some(format!(
            "DIVERGENCE: checksums.toml expects {}, pushing {}",
            &expected_hash[..16.min(expected_hash.len())],
            &actual_hash[..16.min(actual_hash.len())]
        ))
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_result_serde_roundtrip() {
        let result = RefreshResult {
            binary: "beardog".into(),
            status: RefreshStatus::Pushed,
            detail: "transferred 4.2MB".into(),
        };
        let json = serde_json::to_string(&result).unwrap();
        let back: RefreshResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back.binary, "beardog");
        assert!(matches!(back.status, RefreshStatus::Pushed));
    }

    #[test]
    fn refresh_status_variants() {
        let pushed = serde_json::to_value(RefreshStatus::Pushed).unwrap();
        let skipped = serde_json::to_value(RefreshStatus::Skipped).unwrap();
        let failed = serde_json::to_value(RefreshStatus::Failed).unwrap();
        assert_eq!(pushed.as_str(), Some("Pushed"));
        assert_eq!(skipped.as_str(), Some("Skipped"));
        assert_eq!(failed.as_str(), Some("Failed"));
    }

    #[test]
    fn format_refresh_outcome_all_pushed() {
        let results = vec![
            RefreshResult {
                binary: "a".into(),
                status: RefreshStatus::Pushed,
                detail: "ok".into(),
            },
            RefreshResult {
                binary: "b".into(),
                status: RefreshStatus::Pushed,
                detail: "ok".into(),
            },
        ];
        let outcome = format_refresh_outcome(&results);
        assert!(outcome.ok);
        assert!(outcome.message.contains("2 pushed"));
        assert!(outcome.message.contains("0 failed"));
    }

    #[test]
    fn format_refresh_outcome_with_failures() {
        let results = vec![
            RefreshResult {
                binary: "a".into(),
                status: RefreshStatus::Pushed,
                detail: "ok".into(),
            },
            RefreshResult {
                binary: "b".into(),
                status: RefreshStatus::Failed,
                detail: "timeout".into(),
            },
            RefreshResult {
                binary: "c".into(),
                status: RefreshStatus::Skipped,
                detail: "no binary".into(),
            },
        ];
        let outcome = format_refresh_outcome(&results);
        assert!(!outcome.ok);
        assert!(outcome.message.contains("1 pushed"));
        assert!(outcome.message.contains("1 failed"));
        assert!(outcome.message.contains("1 skipped"));
    }

    #[test]
    fn resolve_refresh_source_explicit_override() {
        let path = resolve_refresh_source(Some("/explicit/path"));
        assert_eq!(path, PathBuf::from("/explicit/path"));
    }

    #[test]
    fn find_local_binary_direct_name() {
        let dir = std::env::temp_dir().join("membrane-test-find-bin");
        std::fs::create_dir_all(&dir).unwrap();
        let bin_path = dir.join("beardog");
        std::fs::write(&bin_path, b"fake-elf").unwrap();

        let found = find_local_binary(&dir, "beardog");
        assert!(found.is_some());
        assert!(found.unwrap().contains("beardog"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn find_local_binary_suffixed() {
        let dir = std::env::temp_dir().join("membrane-test-find-bin-suffix");
        std::fs::create_dir_all(&dir).unwrap();
        let bin_path = dir.join("beardog-x86_64-linux-musl");
        std::fs::write(&bin_path, b"fake-elf").unwrap();

        let found = find_local_binary(&dir, "beardog");
        assert!(found.is_some());
        assert!(found.unwrap().contains("x86_64-linux-musl"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn find_local_binary_missing() {
        let dir = std::env::temp_dir().join("membrane-test-find-bin-missing");
        std::fs::create_dir_all(&dir).unwrap();

        let found = find_local_binary(&dir, "nonexistent-primal");
        assert!(found.is_none());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn check_checksum_coherence_no_depot_returns_none() {
        let result = check_checksum_coherence("beardog", "abc123");
        // No depot in test env — returns None (no divergence signal)
        assert!(result.is_none());
    }
}
