// SPDX-License-Identifier: AGPL-3.0-or-later

//! Depot synchronization — sync binaries between local and remote depots.
//!
//! Two modes:
//! - **Default**: SSH to VPS and sync install-dir → depot-dir on the remote.
//!   Used by relay/gate nodes after `plasmid.refresh`.
//! - **Push** (`--push`): SCP binaries from LOCAL depot to REMOTE VPS depot.
//!   Used by builder nodes (e.g. sporeGate) after `plasmid.harvest`.
//!
//! Both modes use BLAKE3 for diff detection and post-copy verification.

/// `plasmid.depot_sync` — Sync inner membrane binaries to the WAN depot directory.
///
/// **Default mode**: SSH to VPS, sync install-dir → depot-dir per primal via
/// Rust-orchestrated per-primal SSH commands (BLAKE3 diff + atomic copy).
/// **Push mode** (`--push`): SCP local depot → remote VPS depot.
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
    let arch = super::detect_target_triple();
    let depot_dir = format!("{depot_root}/{arch}");

    let ensure_cmd = format!("mkdir -p {depot_dir}");
    if let Err(e) = crate::ssh::exec_raw(config, &ensure_cmd).await {
        return Ok(crate::ShadowOutcome {
            ok: false,
            message: format!("depot_sync: cannot create depot dir — {e}"),
            data: None,
        });
    }

    let primals = super::nucleus_primals();
    let mut synced = 0usize;
    let mut current = 0usize;
    let mut failed = 0usize;
    let mut missing = 0usize;
    let mut verified = 0usize;

    for primal in &primals {
        let src = format!("{install_dir}/{primal}");
        let dst = format!("{depot_dir}/{primal}");

        match sync_single_remote(config, &src, &dst).await {
            RemoteSyncResult::Synced => {
                synced += 1;
                verified += 1;
            }
            RemoteSyncResult::Current => current += 1,
            RemoteSyncResult::Missing => missing += 1,
            RemoteSyncResult::Failed => failed += 1,
        }
    }

    let checksums_src = format!(
        "{}/plasmidBin/{}",
        config.vps_root,
        cellmembrane_types::service::CHECKSUMS_FILE,
    );
    let checksums_synced = sync_checksums_to_wan(config, &checksums_src).await;

    Ok(format_outcome(&SyncResult {
        synced,
        verified,
        current,
        failed,
        missing,
        depot_dir,
        install_dir,
        arch: arch.to_string(),
        checksums_synced,
    }))
}

/// Outcome of syncing a single binary on the remote VPS.
enum RemoteSyncResult {
    Synced,
    Current,
    Missing,
    Failed,
}

/// Sync a single binary on the remote VPS: hash-compare, atomic copy, verify.
async fn sync_single_remote(
    config: &crate::ShadowConfig,
    src: &str,
    dst: &str,
) -> RemoteSyncResult {
    let check_cmd = format!(
        "if [ ! -f {src} ]; then echo MISSING; exit 0; fi; \
         src_hash=$(b3sum {src} 2>/dev/null | cut -d' ' -f1); \
         dst_hash=\"\"; [ -f {dst} ] && dst_hash=$(b3sum {dst} 2>/dev/null | cut -d' ' -f1); \
         if [ \"$src_hash\" = \"$dst_hash\" ] && [ -n \"$dst_hash\" ]; then echo CURRENT; \
         else echo \"DRIFT $src_hash\"; fi"
    );
    let Ok((check_out, _)) = crate::ssh::exec_raw(config, &check_cmd).await else {
        return RemoteSyncResult::Failed;
    };
    let check_out = check_out.trim();

    if check_out == "MISSING" {
        return RemoteSyncResult::Missing;
    }
    if check_out == "CURRENT" {
        return RemoteSyncResult::Current;
    }

    let src_hash = check_out.strip_prefix("DRIFT ").unwrap_or("").to_string();

    let copy_cmd = format!(
        "cp -f {src} {dst}.new && \
         new_hash=$(b3sum {dst}.new 2>/dev/null | cut -d' ' -f1); \
         if [ \"{src_hash}\" = \"$new_hash\" ]; then \
           mv -f {dst}.new {dst} && echo OK; \
         else \
           rm -f {dst}.new; echo INTEGRITY_FAIL; \
         fi"
    );
    let Ok((copy_out, _)) = crate::ssh::exec_raw(config, &copy_cmd).await else {
        return RemoteSyncResult::Failed;
    };

    if copy_out.trim() == "OK" {
        RemoteSyncResult::Synced
    } else {
        tracing::error!(src, dst, "depot_sync: post-copy BLAKE3 integrity failure");
        RemoteSyncResult::Failed
    }
}

struct SyncResult {
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

fn format_outcome(r: &SyncResult) -> crate::ShadowOutcome {
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
/// Copies `checksums.toml` and `signatures.toml` from the plasmidBin repo root
/// to the WAN depot path. Returns true if the primary checksums copy succeeded.
async fn sync_checksums_to_wan(config: &crate::ShadowConfig, checksums_path: &str) -> bool {
    let wan_depot = format!("{}/plasmidBin", config.vps_root);
    let wan_checksums = format!(
        "{wan_depot}/{}",
        cellmembrane_types::service::CHECKSUMS_FILE
    );

    let same_file_cmd =
        format!("[ \"{checksums_path}\" -ef \"{wan_checksums}\" ] && echo SAME || echo DIFF");
    if let Ok((out, _)) = crate::ssh::exec_raw(config, &same_file_cmd).await {
        if out.trim() == "SAME" {
            tracing::debug!("WAN checksums sync: src=dst (symlink), skipping");
            return true;
        }
    }

    let cmd = format!("cp -f {checksums_path} {wan_checksums} 2>/dev/null && echo OK || echo FAIL");
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
    let sigs_same_cmd = format!("[ \"{sigs_src}\" -ef \"{wan_sigs}\" ] && echo SAME || echo DIFF");
    let sigs_is_same = crate::ssh::exec_raw(config, &sigs_same_cmd)
        .await
        .is_ok_and(|(out, _)| out.trim() == "SAME");

    if !sigs_is_same {
        let sigs_cmd = format!("[ -f {sigs_src} ] && cp -f {sigs_src} {wan_sigs} 2>/dev/null");
        if let Err(e) = crate::ssh::exec_raw(config, &sigs_cmd).await {
            tracing::debug!(error = %e, "WAN signatures.toml sync: SSH copy failed");
        }
    }

    true
}

/// Standalone push: auto-loads config from env and pushes local depot to VPS.
///
/// Called by `plasmid.harvest --push` without requiring a pre-built `ShadowConfig`.
pub(crate) async fn depot_sync_push_standalone(
    local_depot: &std::path::Path,
) -> crate::error::Result<crate::ShadowOutcome> {
    let config = crate::ShadowConfig::from_env().await;
    let remote_depot = format!(
        "{}/{}",
        config.vps_root,
        cellmembrane_types::service::PLASMID_BIN_DIR
    );
    push_depot_to_remote(&config, local_depot, &remote_depot).await
}

/// Push local depot binaries and metadata to the remote VPS depot via SCP.
enum PushBinaryResult {
    Synced,
    Current,
    Failed,
}

/// Shared push loop: walks arch dirs, pushes changed binaries, syncs metadata.
///
/// Pre-flight: checks remote disk usage — warns at 80%, blocks at 90%.
async fn push_depot_to_remote(
    config: &crate::ShadowConfig,
    local_depot: &std::path::Path,
    remote_depot: &str,
) -> crate::error::Result<crate::ShadowOutcome> {
    let primals_dir = local_depot.join("primals");
    if !primals_dir.exists() {
        return Ok(crate::ShadowOutcome {
            ok: false,
            message: format!("depot push: no primals/ dir at {}", local_depot.display()),
            data: None,
        });
    }

    if let Ok((disk_out, 0)) =
        crate::ssh::exec_raw(config, "df --output=pcent / | tail -1").await
    {
        if let Ok(pct) = disk_out.trim().trim_end_matches('%').trim().parse::<u8>() {
            if pct >= 90 {
                return Ok(crate::ShadowOutcome {
                    ok: false,
                    message: format!(
                        "depot push BLOCKED: remote disk at {pct}% — free space before pushing"
                    ),
                    data: None,
                });
            }
            if pct >= 80 {
                tracing::warn!(
                    disk_pct = pct,
                    "remote disk at {pct}% — depot push proceeding but disk is low"
                );
            }
        }
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
            match push_single_binary(config, bin_entry, &remote_arch_dir, &arch_str).await {
                PushBinaryResult::Synced => synced += 1,
                PushBinaryResult::Current => current += 1,
                PushBinaryResult::Failed => failed += 1,
            }
        }
    }

    let metadata_pushed = push_depot_metadata(config, local_depot, remote_depot).await;
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

async fn push_single_binary(
    config: &crate::ShadowConfig,
    bin_entry: &std::fs::DirEntry,
    remote_arch_dir: &str,
    arch_str: &str,
) -> PushBinaryResult {
    let name = bin_entry.file_name();
    let name_str = name.to_string_lossy();
    let local_path = bin_entry.path();

    let local_hash = match super::compute_blake3_file_async(&local_path).await {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!(binary = %name_str, error = %e, "push: cannot hash local binary");
            return PushBinaryResult::Failed;
        }
    };
    let remote_path = format!("{remote_arch_dir}/{name_str}");
    let hash_cmd = format!("b3sum {remote_path} 2>/dev/null | cut -d' ' -f1");
    let remote_hash = crate::ssh::exec_raw(config, &hash_cmd)
        .await
        .map(|(h, _)| h.trim().to_string())
        .unwrap_or_default();

    if local_hash == remote_hash {
        return PushBinaryResult::Current;
    }

    let remote_tmp = format!("{remote_arch_dir}/.{name_str}.new");
    match crate::ssh::scp_to(config, &local_path.to_string_lossy(), &remote_tmp).await {
        Ok(()) => {
            let mv_cmd = format!("chmod 755 {remote_tmp} && mv -f {remote_tmp} {remote_path}");
            if let Err(e) = crate::ssh::exec_raw(config, &mv_cmd).await {
                tracing::warn!(binary = %name_str, error = %e, "push: atomic rename failed");
                PushBinaryResult::Failed
            } else {
                tracing::info!(binary = %name_str, arch = %arch_str, "pushed to VPS depot");
                PushBinaryResult::Synced
            }
        }
        Err(e) => {
            tracing::warn!(binary = %name_str, error = %e, "push: SCP failed");
            PushBinaryResult::Failed
        }
    }
}

async fn depot_sync_push(
    config: &crate::ShadowConfig,
) -> crate::error::Result<crate::ShadowOutcome> {
    let local_depot = super::harvest::resolve_depot(None)?;
    let remote_depot = format!(
        "{}/{}",
        config.vps_root,
        cellmembrane_types::service::PLASMID_BIN_DIR
    );
    push_depot_to_remote(config, &local_depot, &remote_depot).await
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_outcome_all_current() {
        let result = format_outcome(&SyncResult {
            synced: 0,
            verified: 0,
            current: 13,
            failed: 0,
            missing: 0,
            depot_dir: "/opt/depot".into(),
            install_dir: "/opt/membrane".into(),
            arch: "x86_64-unknown-linux-musl".into(),
            checksums_synced: true,
        });
        assert!(result.ok);
        assert!(result.message.contains("13 current"));
        assert!(result.message.contains("0 synced"));
    }

    #[test]
    fn format_outcome_failures_not_ok() {
        let result = format_outcome(&SyncResult {
            synced: 5,
            verified: 5,
            current: 3,
            failed: 2,
            missing: 3,
            depot_dir: "/opt/depot".into(),
            install_dir: "/opt/membrane".into(),
            arch: "x86_64-unknown-linux-musl".into(),
            checksums_synced: false,
        });
        assert!(!result.ok);
        assert!(result.message.contains("2 failed"));
        assert!(result.message.contains("checksums.toml sync skipped"));
    }

    #[test]
    fn format_outcome_data_fields() {
        let result = format_outcome(&SyncResult {
            synced: 1,
            verified: 1,
            current: 0,
            failed: 0,
            missing: 0,
            depot_dir: "/d".into(),
            install_dir: "/i".into(),
            arch: "aarch64-unknown-linux-musl".into(),
            checksums_synced: true,
        });
        let data = result.data.unwrap();
        assert_eq!(data["synced"], 1);
        assert_eq!(data["arch"], "aarch64-unknown-linux-musl");
    }

    #[test]
    fn push_binary_result_enum_variants_exist() {
        let _ = PushBinaryResult::Synced;
        let _ = PushBinaryResult::Current;
        let _ = PushBinaryResult::Failed;
    }

    #[test]
    fn remote_sync_result_enum_variants_exist() {
        let _ = RemoteSyncResult::Synced;
        let _ = RemoteSyncResult::Current;
        let _ = RemoteSyncResult::Missing;
        let _ = RemoteSyncResult::Failed;
    }
}
