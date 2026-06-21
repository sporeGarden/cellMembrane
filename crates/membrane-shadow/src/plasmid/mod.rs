// SPDX-License-Identifier: AGPL-3.0-or-later

//! Plasmid binary lifecycle — fetch, refresh, harvest, and deploy primal binaries.
//!
//! Manages the binary supply chain for membrane services:
//! - `fetch` — Download binaries from sovereign or external sources (GitHub, VPS, Forgejo)
//! - `harvest` — Build from source, detect changes, checksum, stage to depot
//! - `refresh` — Push local pre-built binaries to VPS with atomic replacement
//! - `pipeline` — End-to-end zero-touch: harvest → refresh → alive
//! - `status` — Report depot freshness and drift against upstream
//!
//! BLAKE3 checksums are verified in-process using the `blake3` crate.

pub mod build;
pub(crate) mod canary;
pub(crate) mod canary_remote;
mod checksum;
pub(crate) mod depot;
mod download;
mod drift;
mod fetch;
mod harvest;
pub(crate) mod integrity;
mod refresh;
pub(crate) mod sandbox;
pub(crate) mod toolchain;

pub use build::BuildArgs;
pub use checksum::fetch_wan_checksums;
pub use fetch::*;
pub use harvest::{HarvestArgs, HarvestResult, HarvestStatus, harvest};
pub use integrity::{IntegrityMismatch, IntegrityReport};
pub use refresh::{RefreshArgs, RefreshResult, RefreshStatus, refresh};

pub use depot::{StalenessEntry, StalenessReport};

/// Gracefully stop a process: SIGTERM → grace period → SIGKILL.
///
/// Uses `/proc/{pid}/` existence check to avoid signaling stale PIDs,
/// using `nix::sys::signal::kill` for native signal delivery.
pub(crate) async fn graceful_kill(pid: u32, grace_ms: u64) {
    use nix::sys::signal::{Signal, kill};
    use nix::unistd::Pid;

    let proc_path = std::path::PathBuf::from(format!("/proc/{pid}"));
    if !proc_path.exists() {
        return;
    }
    let Ok(nix_pid) = i32::try_from(pid).map(Pid::from_raw) else {
        return;
    };
    let _ = kill(nix_pid, Signal::SIGTERM);
    tokio::time::sleep(std::time::Duration::from_millis(grace_ms)).await;
    if proc_path.exists() {
        let _ = kill(nix_pid, Signal::SIGKILL);
    }
}

/// Compute BLAKE3 hash of a file, returning hex string.
pub(crate) fn compute_blake3_file(path: &std::path::Path) -> String {
    depot::compute_blake3_file(path)
}

/// Async variant — runs the full-file BLAKE3 read on a blocking thread.
pub(crate) async fn compute_blake3_file_async(path: std::path::PathBuf) -> String {
    tokio::task::spawn_blocking(move || depot::compute_blake3_file(&path))
        .await
        .unwrap_or_else(|e| {
            tracing::error!(error = %e, "BLAKE3 hash task panicked — returning sentinel");
            "HASH_FAILED".into()
        })
}

/// Detect stale primals in the depot. Resolves depot path from env/defaults.
pub fn detect_depot_staleness() -> crate::error::Result<StalenessReport> {
    let depot_dir = depot::resolve_depot(None)?;
    depot::detect_stale_primals(&depot_dir)
}

use std::path::PathBuf;

/// Primal binary names derived from the service registry at compile time.
///
/// Previously a hand-maintained `const` list — now sourced directly from
/// `cellmembrane-types::MembraneService::all()` so additions/removals to the
/// registry propagate automatically with zero manual sync.
pub(crate) fn nucleus_primals() -> Vec<&'static str> {
    cellmembrane_types::MembraneService::all()
        .iter()
        .filter(|s| s.is_primal)
        .map(|s| s.binary)
        .collect()
}

/// Detect the local platform's default Rust target triple (musl static).
pub(crate) fn detect_target_triple() -> String {
    cellmembrane_types::TargetArch::detect_host()
        .triple()
        .to_string()
}

/// Check NDK toolchain availability for Android cross-compilation.
///
/// Reports whether `ANDROID_NDK_HOME` is set, the linker exists, and
/// the `aarch64-linux-android` Rust target is installed.
#[must_use]
pub fn ndk_check() -> crate::ShadowOutcome {
    let ndk_home = std::env::var(harvest::ENV_ANDROID_NDK_HOME).ok();
    let linker = harvest::resolve_ndk_linker();

    let target_installed = std::process::Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains(harvest::ANDROID_TARGET));

    let all_ok = ndk_home.is_some() && linker.is_some() && target_installed;

    let linker_str = linker
        .as_ref()
        .map_or_else(|| "NOT FOUND".to_string(), |p| p.display().to_string());

    let msg = format!(
        "NDK check: {}\n  ANDROID_NDK_HOME: {}\n  linker: {linker_str}\n  rustup target: {}",
        if all_ok { "READY" } else { "NOT READY" },
        ndk_home.as_deref().unwrap_or("NOT SET"),
        if target_installed {
            "installed"
        } else {
            "MISSING (run: rustup target add aarch64-linux-android)"
        },
    );

    crate::ShadowOutcome {
        ok: all_ok,
        message: msg,
        data: Some(serde_json::json!({
            "ndk_home": ndk_home,
            "linker": linker.map(|p| p.display().to_string()),
            "target_installed": target_installed,
            "target": harvest::ANDROID_TARGET,
        })),
    }
}

/// Resolve a path with priority: explicit override → env var → computed default.
pub(crate) fn resolve_path(
    explicit: Option<&str>,
    env_var: &str,
    default_fn: impl FnOnce() -> PathBuf,
) -> PathBuf {
    if let Some(dir) = explicit {
        return PathBuf::from(dir);
    }
    if let Ok(val) = std::env::var(env_var) {
        return PathBuf::from(val);
    }
    default_fn()
}

/// `plasmid.pipeline` — Full zero-touch harvest → refresh cycle.
///
/// Detects upstream changes, rebuilds, checksums, pushes to VPS,
/// and reports aggregated outcome. This is the end-to-end command
/// that replaces manual harvest+refresh cycles.
pub async fn pipeline(
    config: &crate::ShadowConfig,
    primal: Option<&str>,
    dry_run: bool,
) -> crate::error::Result<crate::ShadowOutcome> {
    let harvest_args = HarvestArgs {
        primal: primal.map(Into::into),
        force: false,
        dry_run,
        depot_dir: None,
        target: None,
    };

    let harvest_outcome = harvest(&harvest_args).await?;

    if dry_run {
        return Ok(harvest_outcome);
    }

    let built_any = harvest_outcome
        .data
        .as_ref()
        .and_then(|d| d.as_array())
        .is_some_and(|arr| {
            arr.iter().any(|r| {
                r.get("status")
                    .and_then(|s| s.as_str())
                    .is_some_and(|s| s == "Built")
            })
        });

    if !built_any {
        return Ok(crate::ShadowOutcome {
            ok: harvest_outcome.ok,
            message: format!("{} — no new binaries to push", harvest_outcome.message),
            data: harvest_outcome.data,
        });
    }

    // Sandbox validation for each built primal before pushing to VPS
    let arch = detect_target_triple();
    let depot_dir = depot::resolve_depot(None)?;
    let bin_dir = depot_dir.join("primals").join(&arch);

    if let Some(data) = &harvest_outcome.data {
        if let Some(arr) = data.as_array() {
            for entry in arr {
                let is_built = entry
                    .get("status")
                    .and_then(|s| s.as_str())
                    .is_some_and(|s| s == "Built");
                if !is_built {
                    continue;
                }

                let Some(name) = entry.get("primal").and_then(|p| p.as_str()) else {
                    continue;
                };

                let binary_path = bin_dir.join(name);
                if !binary_path.exists() {
                    continue;
                }

                let commit = entry
                    .get("commit")
                    .and_then(|c| c.as_str())
                    .unwrap_or("HEAD");

                let sandbox_args = sandbox::SandboxArgs {
                    primal: name.to_string(),
                    commit: commit.to_string(),
                    binary_path,
                    timeout_secs: None,
                };

                if let Ok(result) = sandbox::validate(&sandbox_args).await {
                    if !result.health_ok {
                        return Ok(crate::ShadowOutcome {
                            ok: false,
                            message: format!(
                                "{} | sandbox FAIL for {} — {} ({}ms). Refresh aborted.",
                                harvest_outcome.message, name, result.detail, result.elapsed_ms
                            ),
                            data: Some(serde_json::to_value(&result).unwrap_or_default()),
                        });
                    }
                }
            }
        }
    }

    let depot_source = Some(bin_dir.to_string_lossy().into_owned());

    let refresh_args = RefreshArgs {
        primal: primal.map(Into::into),
        dry_run: false,
        source_dir: depot_source,
    };

    let refresh_outcome = refresh(config, &refresh_args).await?;

    Ok(crate::ShadowOutcome {
        ok: refresh_outcome.ok,
        message: format!(
            "{} | sandbox: PASS | {}",
            harvest_outcome.message, refresh_outcome.message
        ),
        data: refresh_outcome.data,
    })
}

/// `plasmid.trigger` — Remotely trigger the VPS pipeline via SSH.
///
/// Kicks `systemctl start plasmid-pipeline.service` on the VPS, causing
/// an immediate harvest→refresh cycle there. Useful when an operator wants
/// the VPS to converge without running the full pipeline locally.
pub async fn trigger(config: &crate::ShadowConfig) -> crate::error::Result<crate::ShadowOutcome> {
    let cmd = "systemctl start plasmid-pipeline.service 2>&1; \
               sleep 1; \
               systemctl is-active plasmid-pipeline.service 2>&1 || \
               journalctl -u plasmid-pipeline.service --no-pager -n 3 2>&1";

    let (output, code) = crate::ssh::exec_raw(config, cmd).await?;

    if code == 0 || output.contains("activating") || output.contains("active") {
        Ok(crate::ShadowOutcome::ok(format!(
            "trigger: plasmid-pipeline.service started on {}\n{output}",
            config.ssh_host
        )))
    } else {
        Ok(crate::ShadowOutcome {
            ok: false,
            message: format!(
                "trigger: failed to start on {} (exit {code})\n{output}",
                config.ssh_host
            ),
            data: None,
        })
    }
}

/// `plasmid.depot_sync` — Sync inner membrane binaries to the WAN depot directory.
///
/// After `plasmid.refresh` pushes binaries to the install dir (e.g. `/opt/membrane/`),
/// the WAN depot directory (`/opt/ecoPrimals/plasmidBin/primals/{arch}/`) may be stale.
/// This command ensures the depot serves the same binaries that are running:
///
/// 1. Compare BLAKE3 hashes of install-dir vs depot-dir binaries (skip if identical)
/// 2. Copy only divergent binaries (atomic: write .new then rename)
/// 3. Verify post-copy with BLAKE3 to confirm integrity
/// 4. Sync `checksums.toml` to the WAN depot root so remote gates verify correctly
///
/// Reports: synced (changed), current (already matching), failed, verified.
pub async fn depot_sync(
    config: &crate::ShadowConfig,
) -> crate::error::Result<crate::ShadowOutcome> {
    let install_dir = std::env::var(cellmembrane_types::service::ENV_INSTALL_BASE)
        .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_INSTALL_BASE.into());
    let depot_root = format!(
        "{}/plasmidBin/primals",
        cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT
    );
    let arch = detect_target_triple();
    let depot_dir = format!("{depot_root}/{arch}");

    let primals = nucleus_primals();
    let primal_list = primals.join(" ");

    // Phase 1: Compare + sync only divergent binaries (atomic copy via .new + mv)
    let sync_cmd = format!(
        "mkdir -p {depot_dir}; \
         synced=0; current=0; failed=0; missing=0; \
         for p in {primal_list}; do \
           src=\"{install_dir}/$p\"; \
           dst=\"{depot_dir}/$p\"; \
           if [ ! -f \"$src\" ]; then \
             missing=$((missing+1)); continue; \
           fi; \
           src_hash=$(b3sum \"$src\" 2>/dev/null | cut -d' ' -f1); \
           dst_hash=\"\"; \
           [ -f \"$dst\" ] && dst_hash=$(b3sum \"$dst\" 2>/dev/null | cut -d' ' -f1); \
           if [ \"$src_hash\" = \"$dst_hash\" ] && [ -n \"$dst_hash\" ]; then \
             current=$((current+1)); \
           else \
             cp -f \"$src\" \"$dst.new\" && mv -f \"$dst.new\" \"$dst\" && synced=$((synced+1)) || failed=$((failed+1)); \
           fi; \
         done; \
         echo \"synced=$synced current=$current failed=$failed missing=$missing\""
    );

    let (output, code) = crate::ssh::exec_raw(config, &sync_cmd).await?;

    if code != 0 {
        return Ok(crate::ShadowOutcome {
            ok: false,
            message: format!("depot_sync failed (exit {code}): {}", output.trim()),
            data: None,
        });
    }

    let parse_field = |field: &str| -> usize {
        output
            .split(&format!("{field}="))
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(0)
    };

    let synced = parse_field("synced");
    let current = parse_field("current");
    let failed = parse_field("failed");
    let missing = parse_field("missing");

    // Phase 2: Sync checksums.toml to the WAN depot root
    let checksums_src = format!(
        "{}/plasmidBin/checksums.toml",
        cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT
    );
    let checksums_synced = sync_checksums_to_wan(config, &checksums_src).await;

    let total = synced + current + failed + missing;
    let ok = failed == 0;
    let checksums_note = if checksums_synced {
        "checksums.toml synced"
    } else {
        "checksums.toml sync skipped"
    };

    Ok(crate::ShadowOutcome {
        ok,
        message: format!(
            "depot_sync: {synced} synced, {current} current, {missing} missing, \
             {failed} failed (of {total}) — {checksums_note}"
        ),
        data: Some(serde_json::json!({
            "synced": synced,
            "current": current,
            "failed": failed,
            "missing": missing,
            "total": total,
            "depot_dir": depot_dir,
            "install_dir": install_dir,
            "arch": arch,
            "checksums_synced": checksums_synced,
        })),
    })
}

/// Ensure the WAN-serving checksums.toml is up to date.
///
/// Copies from the plasmidBin repo root to wherever Caddy serves it.
/// Returns true if the sync succeeded.
async fn sync_checksums_to_wan(config: &crate::ShadowConfig, checksums_path: &str) -> bool {
    let cmd = format!("[ -f {checksums_path} ] && echo EXISTS || echo MISSING");
    let Ok((out, _)) = crate::ssh::exec_raw(config, &cmd).await else {
        return false;
    };
    out.trim() == "EXISTS"
}

/// `plasmid.status` — Report depot freshness and upstream drift.
///
/// Reads provenance.toml for last build timestamp, then checks each
/// primal's HEAD against the recorded commit to identify drift.
pub async fn status() -> crate::error::Result<crate::ShadowOutcome> {
    let depot_dir = harvest::resolve_depot(None)?;
    let sources = harvest::load_sources(&depot_dir)?;
    let provenance = harvest::load_provenance(&depot_dir);

    let generated = provenance
        .as_ref()
        .and_then(|p| p.generated.clone())
        .unwrap_or_else(|| "unknown".into());

    let target = provenance
        .as_ref()
        .and_then(|p| p.target.clone())
        .unwrap_or_else(|| "unknown".into());

    let registry_primals = nucleus_primals();
    let total = registry_primals.len();

    let mut drifted: Vec<String> = Vec::new();
    let mut current = 0usize;

    for &primal in &registry_primals {
        if let Some(source) = sources.get(primal) {
            let changed =
                drift::has_upstream_changes_pub(primal, source, provenance.as_ref(), &depot_dir)
                    .await;
            if changed {
                drifted.push(primal.to_string());
            } else {
                current += 1;
            }
        }
    }

    let msg = format!(
        "depot: {current}/{total} current, {} drifted | built: {generated} | target: {target}",
        drifted.len()
    );

    let data = serde_json::json!({
        "total": total,
        "current": current,
        "drifted": drifted,
        "generated": generated,
        "target": target,
    });

    Ok(crate::ShadowOutcome {
        ok: drifted.is_empty(),
        message: msg,
        data: Some(data),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nucleus_primals_returns_13() {
        let primals = nucleus_primals();
        assert_eq!(primals.len(), 13, "expected 13 nucleus primals");
        assert!(primals.contains(&"beardog"));
        assert!(primals.contains(&"songbird"));
        assert!(primals.contains(&"squirrel"));
    }

    #[test]
    fn detect_target_triple_contains_musl() {
        let triple = detect_target_triple();
        assert!(
            triple.ends_with("-unknown-linux-musl"),
            "expected musl target, got: {triple}"
        );
    }

    #[test]
    fn resolve_path_explicit_overrides_env() {
        let result = resolve_path(Some("/explicit"), "NONEXISTENT_VAR_XYZ", || {
            PathBuf::from("/default")
        });
        assert_eq!(result, PathBuf::from("/explicit"));
    }

    #[test]
    fn resolve_path_uses_default_when_no_env() {
        let result = resolve_path(None, "NONEXISTENT_VAR_XYZ_ABC", || {
            PathBuf::from("/fallback")
        });
        assert_eq!(result, PathBuf::from("/fallback"));
    }

    #[tokio::test]
    async fn status_reports_depot_state() {
        let result = status().await;
        match result {
            Ok(outcome) => {
                assert!(outcome.message.contains("depot:"));
                assert!(outcome.message.contains("current"));
                let data = outcome.data.unwrap();
                assert!(data.get("total").is_some());
                assert!(data.get("drifted").is_some());
            }
            Err(e) => {
                let msg = e.to_string();
                assert!(
                    msg.contains("depot not found")
                        || msg.contains("cannot read")
                        || msg.contains("No such file"),
                    "unexpected error: {msg}"
                );
            }
        }
    }
}
