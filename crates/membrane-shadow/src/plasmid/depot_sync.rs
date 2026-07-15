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

    let primals = super::nucleus_primals();
    let primal_list = primals.join(" ");

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

    Ok(format_outcome(&SyncResult {
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
enum PushBinaryResult {
    Synced,
    Current,
    Failed,
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

    let local_hash = match super::compute_blake3_file_async(local_path.clone()).await {
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
            let mv_cmd =
                format!("chmod 755 {remote_tmp} && mv -f {remote_tmp} {remote_path}");
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
            match push_single_binary(config, bin_entry, &remote_arch_dir, &arch_str).await {
                PushBinaryResult::Synced => synced += 1,
                PushBinaryResult::Current => current += 1,
                PushBinaryResult::Failed => failed += 1,
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
