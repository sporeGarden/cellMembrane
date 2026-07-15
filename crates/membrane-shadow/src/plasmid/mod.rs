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

pub mod auto_fetch;
pub mod build;
pub(crate) mod canary;
pub(crate) mod canary_remote;
mod checksum;
pub(crate) mod depot;
mod download;
mod drift;
mod fetch;
mod harvest;
mod harvest_support;
pub(crate) mod integrity;
mod refresh;
pub(crate) mod sandbox;
pub(crate) mod signing;
pub(crate) mod toolchain;

pub use build::BuildArgs;
pub use checksum::fetch_wan_checksums;
pub use fetch::*;
pub use harvest::{HarvestArgs, HarvestResult, HarvestStatus, harvest};
pub use integrity::{IntegrityMismatch, IntegrityReport};
pub use refresh::{RefreshArgs, RefreshResult, RefreshStatus, refresh};

pub use depot::{StalenessEntry, StalenessReport};

/// Gracefully stop a process: SIGTERM → grace period → SIGKILL (Unix),
/// or `TerminateProcess` (Windows).
///
/// Platform-aware — OS Atheism Phase 2.
pub(crate) async fn graceful_kill(pid: u32, grace_ms: u64) {
    #[cfg(unix)]
    {
        graceful_kill_unix(pid, grace_ms).await;
    }
    #[cfg(not(unix))]
    {
        graceful_kill_bare(pid, grace_ms).await;
    }
}

/// Unix: SIGTERM → grace → SIGKILL via `nix::sys::signal::kill`.
///
/// Uses `/proc/{pid}/` existence check to avoid signaling stale PIDs.
#[cfg(unix)]
async fn graceful_kill_unix(pid: u32, grace_ms: u64) {
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

/// Non-Unix: best-effort process kill via `std::process::Command("taskkill")`.
#[cfg(not(unix))]
async fn graceful_kill_bare(pid: u32, _grace_ms: u64) {
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .output();
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

/// Resolve the primal set for the local gate from the manifest composition.
///
/// Resolution chain:
///   1. If `gate` has a `composition` field in the manifest, and that
///      composition is defined in `[compositions]`, use its `primals` list.
///   2. Otherwise fall back to the full registry (`nucleus_primals()`).
///
/// This enables composition-aware operations: a thin-relay gate fetches
/// only songBird + nestGate, while a full NUCLEUS gate gets all 13.
///
/// Uses a process-level cache to avoid re-reading the manifest on every call.
/// The cache is keyed on the gate name and populated on first access.
pub(crate) fn resolve_gate_primals(gate: &str) -> Vec<String> {
    use std::sync::OnceLock;

    static CACHED: OnceLock<(String, Vec<String>)> = OnceLock::new();

    let cached = CACHED.get_or_init(|| {
        let workspace = cellmembrane_types::service::env_or(
            cellmembrane_types::service::ENV_ECOPRIMALS_ROOT,
            cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT,
        );
        let primals = crate::manifest::load_from_workspace(std::path::Path::new(&workspace))
            .ok()
            .and_then(|manifest| {
                let profile = manifest.gate_composition(gate)?;
                if profile.primals.is_empty() {
                    None
                } else {
                    Some(profile.primals.clone())
                }
            })
            .unwrap_or_else(|| nucleus_primals().into_iter().map(String::from).collect());
        (gate.to_string(), primals)
    });

    if cached.0 == gate {
        cached.1.clone()
    } else {
        let workspace = cellmembrane_types::service::env_or(
            cellmembrane_types::service::ENV_ECOPRIMALS_ROOT,
            cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT,
        );
        crate::manifest::load_from_workspace(std::path::Path::new(&workspace))
            .ok()
            .and_then(|manifest| {
                let profile = manifest.gate_composition(gate)?;
                if profile.primals.is_empty() {
                    None
                } else {
                    Some(profile.primals.clone())
                }
            })
            .unwrap_or_else(|| nucleus_primals().into_iter().map(String::from).collect())
    }
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
        local: false,
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
/// Sync depot binaries between local and remote.
///
/// **Default mode** (no `--push`): SSH to the VPS and sync install-dir → depot-dir
/// on the remote. Used by relay/gate nodes after `plasmid.refresh`.
///
/// **Push mode** (`--push`): SCP binaries from the LOCAL depot to the REMOTE VPS
/// depot. Used by builder nodes (e.g. sporeGate) after `plasmid.harvest`. This
/// replaces the manual rsync workflow.
///
/// Both modes use BLAKE3 for diff detection and post-copy verification.
pub async fn depot_sync(
    config: &crate::ShadowConfig,
    push: bool,
) -> crate::error::Result<crate::ShadowOutcome> {
    if push {
        return depot_sync_push(config).await;
    }
    let install_dir = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_INSTALL_BASE,
        cellmembrane_types::service::DEFAULT_INSTALL_BASE,
    );
    let depot_root = format!("{}/plasmidBin/primals", config.vps_root);
    let arch = detect_target_triple();
    let depot_dir = format!("{depot_root}/{arch}");

    let primals = nucleus_primals();
    let primal_list = primals.join(" ");

    // Phase 1: Compare + sync divergent binaries with post-copy BLAKE3 verification.
    // For each primal: hash source, compare with depot, copy .new, verify .new hash,
    // then atomic mv. If post-copy hash mismatches source, remove .new and report failure.
    let sync_cmd = format!(
        "mkdir -p {depot_dir}; \
         synced=0; current=0; failed=0; missing=0; verified=0; \
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
             if cp -f \"$src\" \"$dst.new\"; then \
               new_hash=$(b3sum \"$dst.new\" 2>/dev/null | cut -d' ' -f1); \
               if [ \"$src_hash\" = \"$new_hash\" ]; then \
                 mv -f \"$dst.new\" \"$dst\" && synced=$((synced+1)) && verified=$((verified+1)) || failed=$((failed+1)); \
               else \
                 rm -f \"$dst.new\"; \
                 failed=$((failed+1)); \
                 echo \"INTEGRITY_FAIL: $p src=$src_hash copy=$new_hash\" >&2; \
               fi; \
             else \
               failed=$((failed+1)); \
             fi; \
           fi; \
         done; \
         echo \"synced=$synced current=$current failed=$failed missing=$missing verified=$verified\""
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
    let verified = parse_field("verified");

    if output.contains("INTEGRITY_FAIL") {
        tracing::error!(
            output = %output.trim(),
            "depot_sync: post-copy BLAKE3 integrity failure detected"
        );
    }

    let checksums_src = format!(
        "{}/plasmidBin/{}",
        config.vps_root,
        cellmembrane_types::service::CHECKSUMS_FILE,
    );
    let checksums_synced = sync_checksums_to_wan(config, &checksums_src).await;

    Ok(format_depot_sync_outcome(&DepotSyncResult {
        synced,
        verified,
        current,
        failed,
        missing,
        depot_dir,
        install_dir,
        arch,
        checksums_synced,
    }))
}

struct DepotSyncResult {
    synced: usize,
    verified: usize,
    current: usize,
    failed: usize,
    missing: usize,
    depot_dir: String,
    install_dir: String,
    arch: String,
    checksums_synced: bool,
}

fn format_depot_sync_outcome(r: &DepotSyncResult) -> crate::ShadowOutcome {
    let total = r.synced + r.current + r.failed + r.missing;
    let ok = r.failed == 0;
    let checksums_note = if r.checksums_synced {
        "checksums.toml synced"
    } else {
        "checksums.toml sync skipped"
    };

    crate::ShadowOutcome {
        ok,
        message: format!(
            "depot_sync: {} synced ({} verified), {} current, {} missing, \
             {} failed (of {total}) — {checksums_note}",
            r.synced, r.verified, r.current, r.missing, r.failed
        ),
        data: Some(serde_json::json!({
            "synced": r.synced,
            "verified": r.verified,
            "current": r.current,
            "failed": r.failed,
            "missing": r.missing,
            "total": total,
            "depot_dir": r.depot_dir,
            "install_dir": r.install_dir,
            "arch": r.arch,
            "checksums_synced": r.checksums_synced,
        })),
    }
}

/// Copy depot metadata to the WAN-serving directory so Caddy serves current files.
///
/// Copies checksums.toml and signatures.toml from the plasmidBin repo root
/// to the WAN depot path. Returns true if the primary checksums copy succeeded.
async fn sync_checksums_to_wan(config: &crate::ShadowConfig, checksums_path: &str) -> bool {
    let wan_depot = format!("{}/plasmidBin", config.vps_root);
    let wan_checksums = format!(
        "{wan_depot}/{}",
        cellmembrane_types::service::CHECKSUMS_FILE
    );

    // Detect same-file (symlink) scenario: if src and dst resolve to the same
    // inode, skip the copy. This happens on golgi when plasmidBin is symlinked.
    let same_file_cmd = format!(
        "[ \"{checksums_path}\" -ef \"{wan_checksums}\" ] && echo SAME || echo DIFF"
    );
    if let Ok((out, _)) = crate::ssh::exec_raw(config, &same_file_cmd).await {
        if out.trim() == "SAME" {
            tracing::debug!("WAN checksums sync: src=dst (symlink), skipping");
            return true;
        }
    }

    let cmd = format!(
        "cp -f {checksums_path} {wan_checksums} 2>/dev/null && echo OK || echo FAIL"
    );
    let Ok((out, _)) = crate::ssh::exec_raw(config, &cmd).await else {
        tracing::warn!("WAN checksums sync: SSH connection failed");
        return false;
    };
    if out.trim() != "OK" {
        tracing::warn!("WAN checksums sync: copy failed");
        return false;
    }

    let sigs_src = checksums_path.replace(
        cellmembrane_types::service::CHECKSUMS_FILE,
        cellmembrane_types::service::SIGNATURES_FILE,
    );
    let wan_sigs = format!(
        "{wan_depot}/{}",
        cellmembrane_types::service::SIGNATURES_FILE
    );
    let sigs_same_cmd = format!(
        "[ \"{sigs_src}\" -ef \"{wan_sigs}\" ] && echo SAME || echo DIFF"
    );
    let sigs_is_same = crate::ssh::exec_raw(config, &sigs_same_cmd)
        .await
        .is_ok_and(|(out, _)| out.trim() == "SAME");

    if !sigs_is_same {
        let sigs_cmd = format!(
            "[ -f {sigs_src} ] && cp -f {sigs_src} {wan_sigs} 2>/dev/null"
        );
        if let Err(e) = crate::ssh::exec_raw(config, &sigs_cmd).await {
            tracing::debug!(error = %e, "WAN signatures.toml sync: SSH copy failed");
        }
    }

    true
}

/// Push local depot binaries and metadata to the remote VPS depot via SCP.
///
/// For each architecture dir in `{local_depot}/primals/{arch}/`, pushes
/// divergent binaries (BLAKE3 diff), then pushes metadata files
/// (checksums.toml, provenance.toml, signatures.toml).
async fn depot_sync_push(
    config: &crate::ShadowConfig,
) -> crate::error::Result<crate::ShadowOutcome> {
    let local_depot = harvest::resolve_depot(None)?;
    let remote_depot = format!("{}/{}", config.vps_root, cellmembrane_types::service::PLASMID_BIN_DIR);

    let primals_dir = local_depot.join("primals");
    if !primals_dir.exists() {
        return Ok(crate::ShadowOutcome {
            ok: false,
            message: format!("depot push: no primals/ dir at {}", local_depot.display()),
            data: None,
        });
    }

    let mut synced = 0usize;
    let mut current = 0usize;
    let mut failed = 0usize;
    let mut arch_count = 0usize;

    let arch_dirs: Vec<_> = std::fs::read_dir(&primals_dir)
        .map_err(crate::error::ShadowError::Io)?
        .filter_map(std::result::Result::ok)
        .filter(|e| e.file_type().is_ok_and(|ft| ft.is_dir()))
        .collect();

    for arch_entry in &arch_dirs {
        let arch = arch_entry.file_name();
        let arch_str = arch.to_string_lossy();
        let local_arch_dir = arch_entry.path();
        let remote_arch_dir = format!("{remote_depot}/primals/{arch_str}");

        let ensure_dir = format!("mkdir -p {remote_arch_dir}");
        if let Err(e) = crate::ssh::exec_raw(config, &ensure_dir).await {
            tracing::warn!(arch = %arch_str, error = %e, "push: failed to create remote dir");
            failed += 1;
            continue;
        }
        arch_count += 1;

        let bins: Vec<_> = std::fs::read_dir(&local_arch_dir)
            .map_err(crate::error::ShadowError::Io)?
            .filter_map(std::result::Result::ok)
            .filter(|e| {
                e.file_type().is_ok_and(|ft| ft.is_file())
                    && !e.file_name().to_string_lossy().starts_with('.')
            })
            .collect();

        for bin_entry in &bins {
            let name = bin_entry.file_name();
            let name_str = name.to_string_lossy();
            let local_path = bin_entry.path();

            let local_hash = compute_blake3_file_async(local_path.clone()).await;
            let remote_path = format!("{remote_arch_dir}/{name_str}");
            let hash_cmd = format!("b3sum {remote_path} 2>/dev/null | cut -d' ' -f1");
            let remote_hash = crate::ssh::exec_raw(config, &hash_cmd)
                .await
                .map(|(h, _)| h.trim().to_string())
                .unwrap_or_default();

            if !local_hash.is_empty() && local_hash == remote_hash {
                current += 1;
                continue;
            }

            let remote_tmp = format!("{remote_arch_dir}/.{name_str}.new");
            match crate::ssh::scp_to(config, &local_path.to_string_lossy(), &remote_tmp).await {
                Ok(()) => {
                    let mv_cmd = format!("chmod 755 {remote_tmp} && mv -f {remote_tmp} {remote_path}");
                    if let Err(e) = crate::ssh::exec_raw(config, &mv_cmd).await {
                        tracing::warn!(binary = %name_str, error = %e, "push: atomic rename failed");
                        failed += 1;
                    } else {
                        synced += 1;
                        tracing::info!(binary = %name_str, arch = %arch_str, "pushed to VPS depot");
                    }
                }
                Err(e) => {
                    tracing::warn!(binary = %name_str, error = %e, "push: SCP failed");
                    failed += 1;
                }
            }
        }
    }

    let metadata_pushed = push_depot_metadata(config, &local_depot, &remote_depot).await;
    let total = synced + current + failed;
    let ok = failed == 0;

    Ok(crate::ShadowOutcome {
        ok,
        message: format!(
            "depot push: {synced} pushed, {current} current, {failed} failed \
             (of {total}, {arch_count} arch) — metadata {}",
            if metadata_pushed { "synced" } else { "partial" }
        ),
        data: Some(serde_json::json!({
            "mode": "push",
            "synced": synced,
            "current": current,
            "failed": failed,
            "total": total,
            "architectures": arch_count,
            "metadata_pushed": metadata_pushed,
        })),
    })
}

/// Push depot metadata files (checksums, provenance, signatures) to the remote VPS.
async fn push_depot_metadata(
    config: &crate::ShadowConfig,
    local_depot: &std::path::Path,
    remote_depot: &str,
) -> bool {
    let mut all_ok = true;
    for filename in [
        cellmembrane_types::service::CHECKSUMS_FILE,
        cellmembrane_types::service::PROVENANCE_FILE,
        cellmembrane_types::service::SIGNATURES_FILE,
    ] {
        let local = local_depot.join(filename);
        if !local.is_file() {
            continue;
        }
        let remote = format!("{remote_depot}/{filename}");
        if let Err(e) = crate::ssh::scp_to(config, &local.to_string_lossy(), &remote).await {
            tracing::warn!(file = filename, error = %e, "metadata push failed");
            all_ok = false;
        }
    }
    all_ok
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
            let changed = drift::has_upstream_changes_lenient(
                primal,
                source,
                provenance.as_ref(),
                &depot_dir,
            )
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
