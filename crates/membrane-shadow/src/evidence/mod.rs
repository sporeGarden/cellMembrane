// SPDX-License-Identifier: AGPL-3.0-or-later

//! Evidence depot — sovereign hosting of primary source documents.
//!
//! Extends the plasmidBin depot pattern (sporeGate → golgiBody SCP push)
//! to handle heterogeneous evidence files (PDF, PNG, TXT, HTML) organized
//! in subdirectories. Files are BLAKE3-hashed for integrity and pushed
//! to golgiBody where Caddy serves them as static files.
//!
//! ## Data Braid Pattern
//!
//! sporeGate is the authority — files live here as the source of truth.
//! golgiBody is the thin WAN mirror — receives SCP pushes and serves
//! via Caddy `file_server`. This is the same pattern used for binary
//! depot (`depot.primals.eco`) generalized to evidence documents.
//!
//! ## Progression
//!
//! - **Composition**: This module — `membrane evidence.*` commands
//! - **Capability**: Evidence as a `petalTongue` routed capability (future)
//! - **Braid**: Full provenance via nestGate CAS + sweetGrass (westGate scale)

pub(crate) mod manifest;
pub(crate) mod provenance;

use crate::error::{Result, ShadowError, ShadowOutcome};
use crate::seo::PublishSite;
use tracing::{error, info, warn};

/// Push evidence files from local depot to remote golgiBody via SCP.
///
/// Walks the local evidence directory recursively, BLAKE3-hashes each file,
/// and SCPs only changed/new files using atomic rename (.filename.new → mv).
/// Mirrors the directory structure on the remote side.
pub async fn push_evidence(
    config: &crate::ShadowConfig,
    local_dir: &std::path::Path,
    remote_dir: &str,
) -> Result<ShadowOutcome> {
    if !local_dir.exists() {
        return Ok(ShadowOutcome {
            ok: false,
            message: format!(
                "evidence push: local dir does not exist — {}",
                local_dir.display()
            ),
            data: None,
        });
    }

    // Pre-flight: check remote disk
    if let Ok((disk_out, 0)) =
        crate::ssh::exec_raw(config, "df --output=pcent / | tail -1").await
    {
        if let Ok(pct) = disk_out.trim().trim_end_matches('%').trim().parse::<u8>() {
            if pct >= 90 {
                return Ok(ShadowOutcome {
                    ok: false,
                    message: format!(
                        "evidence push BLOCKED: remote disk at {pct}% — free space before pushing"
                    ),
                    data: None,
                });
            }
            if pct >= 80 {
                warn!(
                    disk_pct = pct,
                    "remote disk at {pct}% — evidence push proceeding but disk is low"
                );
            }
        }
    }

    // Ensure remote base dir exists
    let ensure_cmd = format!("mkdir -p {remote_dir}");
    if let Err(e) = crate::ssh::exec_raw(config, &ensure_cmd).await {
        return Ok(ShadowOutcome {
            ok: false,
            message: format!("evidence push: cannot create remote dir — {e}"),
            data: None,
        });
    }

    // Walk local evidence dir recursively, collect all files
    let files = collect_evidence_files(local_dir)?;

    if files.is_empty() {
        return Ok(ShadowOutcome::ok(
            "evidence push: no files to push (directory empty)".to_string(),
        ));
    }

    let mut synced = 0usize;
    let mut current = 0usize;
    let mut failed = 0usize;

    for file_entry in &files {
        let rel_path = file_entry
            .strip_prefix(local_dir)
            .unwrap_or(file_entry.as_path());
        let rel_str = rel_path.to_string_lossy();

        match push_single_evidence(config, file_entry, local_dir, remote_dir).await {
            PushResult::Synced => {
                info!(file = %rel_str, "evidence: pushed");
                synced += 1;
            }
            PushResult::Current => {
                current += 1;
            }
            PushResult::Failed => {
                error!(file = %rel_str, "evidence: push failed");
                failed += 1;
            }
        }
    }

    let total = synced + current + failed;
    let ok = failed == 0;

    Ok(ShadowOutcome {
        ok,
        message: format!(
            "evidence push: {synced} pushed, {current} current, {failed} failed (of {total})"
        ),
        data: Some(serde_json::json!({
            "mode": "evidence_push",
            "synced": synced,
            "current": current,
            "failed": failed,
            "total": total,
            "remote_dir": remote_dir,
        })),
    })
}

/// Compare local evidence manifest against remote state.
pub async fn evidence_status(
    config: &crate::ShadowConfig,
    local_dir: &std::path::Path,
    remote_dir: &str,
) -> Result<ShadowOutcome> {
    let files = collect_evidence_files(local_dir)?;
    let mut local_only = Vec::new();
    let mut remote_drift = Vec::new();
    let mut in_sync = 0usize;

    for file_entry in &files {
        let rel_path = file_entry
            .strip_prefix(local_dir)
            .unwrap_or(file_entry.as_path());
        let rel_str = rel_path.to_string_lossy().replace('\\', "/");
        let remote_path = format!("{remote_dir}/{rel_str}");

        let local_hash = match crate::plasmid::compute_blake3_file_async(file_entry).await {
            Ok(h) => h,
            Err(_) => {
                local_only.push(rel_str.to_string());
                continue;
            }
        };

        let hash_cmd = format!("b3sum '{remote_path}' 2>/dev/null | cut -d' ' -f1");
        let remote_hash = crate::ssh::exec_raw(config, &hash_cmd)
            .await
            .map(|(h, _)| h.trim().to_string())
            .unwrap_or_default();

        if remote_hash.is_empty() {
            local_only.push(rel_str.to_string());
        } else if local_hash != remote_hash {
            remote_drift.push(rel_str.to_string());
        } else {
            in_sync += 1;
        }
    }

    let total = files.len();
    let ok = local_only.is_empty() && remote_drift.is_empty();

    Ok(ShadowOutcome {
        ok,
        message: format!(
            "evidence status: {in_sync} synced, {} local-only, {} drifted (of {total})",
            local_only.len(),
            remote_drift.len()
        ),
        data: Some(serde_json::json!({
            "in_sync": in_sync,
            "local_only": local_only,
            "remote_drift": remote_drift,
            "total": total,
        })),
    })
}

// ── Push helpers ────────────────────────────────────────────────────

enum PushResult {
    Synced,
    Current,
    Failed,
}

/// Push a single evidence file to the remote, using BLAKE3 diff + atomic rename.
async fn push_single_evidence(
    config: &crate::ShadowConfig,
    local_path: &std::path::Path,
    local_base: &std::path::Path,
    remote_base: &str,
) -> PushResult {
    let rel_path = local_path
        .strip_prefix(local_base)
        .unwrap_or(local_path);
    let rel_str = rel_path.to_string_lossy().replace('\\', "/");

    // BLAKE3 hash local file
    let local_hash = match crate::plasmid::compute_blake3_file_async(local_path).await {
        Ok(h) => h,
        Err(e) => {
            warn!(file = %rel_str, error = %e, "evidence: cannot hash local file");
            return PushResult::Failed;
        }
    };

    let remote_path = format!("{remote_base}/{rel_str}");

    // Check remote hash
    let hash_cmd = format!("b3sum '{remote_path}' 2>/dev/null | cut -d' ' -f1");
    let remote_hash = crate::ssh::exec_raw(config, &hash_cmd)
        .await
        .map(|(h, _)| h.trim().to_string())
        .unwrap_or_default();

    if local_hash == remote_hash {
        return PushResult::Current;
    }

    // Ensure remote parent directory exists
    if let Some(parent) = rel_path.parent() {
        if !parent.as_os_str().is_empty() {
            let parent_str = parent.to_string_lossy().replace('\\', "/");
            let mkdir_cmd = format!("mkdir -p '{remote_base}/{parent_str}'");
            if let Err(e) = crate::ssh::exec_raw(config, &mkdir_cmd).await {
                warn!(dir = %parent_str, error = %e, "evidence: cannot create remote parent dir");
                return PushResult::Failed;
            }
        }
    }

    // SCP with atomic rename
    let file_name = local_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let remote_parent = if let Some(p) = rel_path.parent() {
        let ps = p.to_string_lossy().replace('\\', "/");
        if ps.is_empty() {
            remote_base.to_string()
        } else {
            format!("{remote_base}/{ps}")
        }
    } else {
        remote_base.to_string()
    };
    let remote_tmp = format!("{remote_parent}/.{file_name}.new");

    match crate::ssh::scp_to(config, &local_path.to_string_lossy(), &remote_tmp).await {
        Ok(()) => {
            let mv_cmd = format!("chmod 644 '{remote_tmp}' && mv -f '{remote_tmp}' '{remote_path}'");
            if let Err(e) = crate::ssh::exec_raw(config, &mv_cmd).await {
                warn!(file = %rel_str, error = %e, "evidence: atomic rename failed");
                PushResult::Failed
            } else {
                PushResult::Synced
            }
        }
        Err(e) => {
            warn!(file = %rel_str, error = %e, "evidence: SCP failed");
            PushResult::Failed
        }
    }
}

// ── File collection ─────────────────────────────────────────────────

/// Recursively collect all files in a directory, excluding hidden files and manifests.
fn collect_evidence_files(dir: &std::path::Path) -> Result<Vec<std::path::PathBuf>> {
    let mut files = Vec::new();
    let mut dirs = vec![dir.to_path_buf()];

    while let Some(d) = dirs.pop() {
        let rd = std::fs::read_dir(&d).map_err(|e| {
            ShadowError::Io(std::io::Error::other(format!(
                "cannot read evidence dir {}: {e}",
                d.display()
            )))
        })?;
        for entry in rd.flatten() {
            let ft = entry.file_type().unwrap_or_else(|_| {
                std::fs::metadata(entry.path())
                    .map(|m| m.file_type())
                    .unwrap_or_else(|_| std::fs::metadata("/dev/null").unwrap().file_type())
            });
            let name = entry.file_name();
            let name_str = name.to_string_lossy();

            // Skip hidden files/dirs and the manifest itself
            if name_str.starts_with('.') || name_str == "evidence-manifest.toml" {
                continue;
            }

            if ft.is_dir() {
                dirs.push(entry.path());
            } else if ft.is_file() {
                files.push(entry.path());
            }
        }
    }

    files.sort();
    Ok(files)
}

/// Resolve evidence paths for a publish site.
///
/// Returns `(local_evidence_dir, remote_evidence_dir)` if the site has evidence configured.
pub fn resolve_evidence_paths(
    site: &PublishSite,
) -> Option<(std::path::PathBuf, String)> {
    let evidence_dir = site.evidence_dir?;
    let local = std::path::PathBuf::from(evidence_dir);
    // Remote evidence goes alongside the public_dir on golgiBody
    // e.g. /opt/ecoPrimals/detroit/public → /opt/ecoPrimals/detroit/evidence
    let remote = format!(
        "{}/evidence",
        site.public_dir
            .rsplit_once('/')
            .map(|(parent, _)| parent)
            .unwrap_or(site.public_dir)
    );
    Some((local, remote))
}

/// Dispatch `evidence.*` CLI commands.
pub async fn dispatch(
    config: &crate::ShadowConfig,
    cmd: &str,
    args: &[&str],
) -> Result<ShadowOutcome> {
    match cmd {
        "evidence.push" => {
            let repo_name = crate::cli::require_arg(args, 0, "site_name")?;
            let site = crate::seo::find_publish_site(repo_name).ok_or_else(|| {
                ShadowError::config(format!(
                    "unknown publish site: {repo_name} (known: {})",
                    crate::seo::PUBLISH_SITES
                        .iter()
                        .map(|s| s.repo_name)
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            })?;
            let (local, remote) = resolve_evidence_paths(site).ok_or_else(|| {
                ShadowError::config(format!(
                    "site {repo_name} has no evidence_dir configured"
                ))
            })?;
            push_evidence(config, &local, &remote).await
        }
        "evidence.manifest" => {
            let repo_name = crate::cli::require_arg(args, 0, "site_name")?;
            let site = crate::seo::find_publish_site(repo_name).ok_or_else(|| {
                ShadowError::config(format!("unknown publish site: {repo_name}"))
            })?;
            let evidence_dir = site.evidence_dir.ok_or_else(|| {
                ShadowError::config(format!(
                    "site {repo_name} has no evidence_dir configured"
                ))
            })?;
            let local = std::path::Path::new(evidence_dir);
            manifest::generate_manifest(local).await
        }
        "evidence.status" => {
            let repo_name = crate::cli::require_arg(args, 0, "site_name")?;
            let site = crate::seo::find_publish_site(repo_name).ok_or_else(|| {
                ShadowError::config(format!("unknown publish site: {repo_name}"))
            })?;
            let (local, remote) = resolve_evidence_paths(site).ok_or_else(|| {
                ShadowError::config(format!(
                    "site {repo_name} has no evidence_dir configured"
                ))
            })?;
            evidence_status(config, &local, &remote).await
        }
        "evidence.braid" => {
            let repo_name = crate::cli::require_arg(args, 0, "site_name")?;
            let site = crate::seo::find_publish_site(repo_name).ok_or_else(|| {
                ShadowError::config(format!("unknown publish site: {repo_name}"))
            })?;
            let evidence_dir = site.evidence_dir.ok_or_else(|| {
                ShadowError::config(format!(
                    "site {repo_name} has no evidence_dir configured"
                ))
            })?;
            let collection = crate::cli::extract_flag_value(args, "--collection");
            let local = std::path::Path::new(evidence_dir);
            provenance::braid_site_evidence(local, collection.as_deref()).await
        }
        _ => Ok(ShadowOutcome::fail(format!(
            "unknown evidence command: {cmd}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_evidence_files_skips_hidden() {
        let dir = std::env::temp_dir().join("membrane-evidence-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("file.pdf"), b"pdf").unwrap();
        std::fs::write(dir.join("sub/nested.png"), b"png").unwrap();
        std::fs::write(dir.join(".hidden"), b"skip").unwrap();
        std::fs::write(dir.join("evidence-manifest.toml"), b"skip").unwrap();

        let files = collect_evidence_files(&dir).unwrap();
        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|f| f.ends_with("file.pdf")));
        assert!(files.iter().any(|f| f.ends_with("nested.png")));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn collect_evidence_empty_dir() {
        let dir = std::env::temp_dir().join("membrane-evidence-empty");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let files = collect_evidence_files(&dir).unwrap();
        assert!(files.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resolve_evidence_paths_none_without_config() {
        let site = crate::seo::PublishSite {
            repo_name: "test",
            host: "test.primals.eco",
            worktree: "/opt/test",
            public_dir: "/opt/test/public",
            build_subdir: None,
            evidence_dir: None,
            seo: crate::seo::SiteConfig {
                host: "test.primals.eco",
                sitemap: "https://test.primals.eco/sitemap.xml",
                indexnow_key: "test",
            },
        };
        assert!(resolve_evidence_paths(&site).is_none());
    }

    #[test]
    fn resolve_evidence_paths_with_config() {
        let site = crate::seo::PublishSite {
            repo_name: "detroit",
            host: "detroit.primals.eco",
            worktree: "/opt/ecoPrimals/detroit/worktree",
            public_dir: "/opt/ecoPrimals/detroit/public",
            build_subdir: Some("site"),
            evidence_dir: Some("/home/sporegate/Development/detroit/evidence"),
            seo: crate::seo::SiteConfig {
                host: "detroit.primals.eco",
                sitemap: "https://detroit.primals.eco/sitemap.xml",
                indexnow_key: "test",
            },
        };
        let (local, remote) = resolve_evidence_paths(&site).unwrap();
        assert_eq!(
            local,
            std::path::PathBuf::from("/home/sporegate/Development/detroit/evidence")
        );
        assert_eq!(remote, "/opt/ecoPrimals/detroit/evidence");
    }
}
