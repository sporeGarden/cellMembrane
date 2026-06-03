// SPDX-License-Identifier: AGPL-3.0-or-later
//! Full cascade sync — reads manifest, syncs all gate repos, reports parity.

use crate::error::Result;
use std::path::Path;

use super::types::SyncClassification;
use super::{check, resolve_workspace_root, sync_with_policy};

/// Cascade execution mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CascadeMode {
    /// Sync repos (pull leader, push followers).
    Sync,
    /// Only check temporal position — no mutations.
    CheckOnly,
    /// Dry run — show what would be done.
    DryRun,
}

/// Options for a cascade operation.
#[derive(Debug, Clone)]
pub struct CascadeOpts<'a> {
    /// Gate name to cascade (e.g. "golgiBody").
    pub gate: &'a str,
    /// Source preference (e.g. "forgejo").
    pub source: &'a str,
    /// Cascade execution mode.
    pub mode: CascadeMode,
    /// If true, clone repos not yet present locally.
    pub clone_missing: bool,
    /// If true, write freshness.toml after cascade.
    pub publish_freshness: bool,
}

/// Execute cascade with typed options.
pub async fn cascade_with_opts(opts: &CascadeOpts<'_>) -> Result<crate::ShadowOutcome> {
    let root = resolve_workspace_root()?;
    let m = crate::manifest::load_from_workspace(&root)?;

    let repos: Vec<(&str, &crate::manifest::RepoEntry)> = m.gate_repos(opts.gate);
    let total = repos.len() as u32;

    if opts.mode == CascadeMode::DryRun {
        let lines: Vec<String> = repos
            .iter()
            .map(|(name, e)| format!("  {:<25} {}", name, e.local_path))
            .collect();
        return Ok(crate::ShadowOutcome::ok(format!(
            "DRY RUN: {total} repos for {} (source={})\n{}",
            opts.gate,
            opts.source,
            lines.join("\n"),
        )));
    }

    let push_target = m.sync.push_target.clone();
    let mut synced = 0u32;
    let mut failed = 0u32;
    let mut cloned = 0u32;
    let mut lines = Vec::with_capacity(repos.len());

    for (name, entry) in &repos {
        let result = process_repo(
            &root,
            name,
            entry,
            opts.mode,
            opts.clone_missing,
            &push_target,
            &m,
        )
        .await;
        match result {
            RepoResult::Synced(msg) => {
                synced += 1;
                lines.push(msg);
            }
            RepoResult::Failed(msg) => {
                failed += 1;
                lines.push(msg);
            }
            RepoResult::Cloned(msg) => {
                cloned += 1;
                lines.push(msg);
            }
            RepoResult::Skipped(msg) => lines.push(msg),
        }
    }

    if opts.publish_freshness && opts.mode == CascadeMode::Sync {
        match crate::freshness::publish_freshness_toml(&root, &m, &repos).await {
            Ok(()) => lines.push("  [freshness] PUBLISHED freshness.toml".to_string()),
            Err(e) => lines.push(format!("  [freshness] FAIL: {e}")),
        }
    }

    let action = if opts.mode == CascadeMode::CheckOnly {
        "checked"
    } else {
        "synced"
    };
    let clone_info = if cloned > 0 {
        format!(" cloned={cloned}")
    } else {
        String::new()
    };
    let header = format!(
        "=== WaterFall Cascade ({action}) ===\n\
         Manifest: v{} wave {} ({} repos)\n\
         Gate:    {}\n\
         Source:  {}\n\
         Repos:   {total}\n\
         \n\
         {action}={synced} failed={failed}{clone_info}",
        m.meta.version, m.meta.wave, m.meta.total_repos, opts.gate, opts.source,
    );

    Ok(crate::ShadowOutcome::ok_with(
        format!("{header}\n{}", lines.join("\n")),
        serde_json::json!({
            "gate": opts.gate,
            "source": opts.source,
            "total": total,
            "synced": synced,
            "failed": failed,
            "cloned": cloned,
        }),
    ))
}

enum RepoResult {
    Synced(String),
    Failed(String),
    Cloned(String),
    Skipped(String),
}

async fn process_repo(
    root: &Path,
    name: &str,
    entry: &crate::manifest::RepoEntry,
    mode: CascadeMode,
    clone_missing: bool,
    push_target: &str,
    manifest: &crate::manifest::EcosystemManifest,
) -> RepoResult {
    let repo_path = &entry.local_path;
    let full_path = root.join(repo_path);

    if !full_path.join(".git").exists() {
        if clone_missing {
            return clone_repo(name, manifest, entry, &full_path).await;
        }
        return RepoResult::Skipped(format!("  {name:<35} SKIP (not cloned)"));
    }

    if mode == CascadeMode::CheckOnly {
        check_repo(root, name, repo_path).await
    } else {
        sync_repo(root, name, repo_path, push_target, manifest).await
    }
}

async fn clone_repo(
    name: &str,
    manifest: &crate::manifest::EcosystemManifest,
    entry: &crate::manifest::RepoEntry,
    full_path: &Path,
) -> RepoResult {
    let forgejo_url = manifest.forgejo_clone_url(entry);
    let clone_result = tokio::process::Command::new("git")
        .args(["clone", &forgejo_url, &full_path.to_string_lossy()])
        .output()
        .await;
    match clone_result {
        Ok(out) if out.status.success() => RepoResult::Cloned(format!("  {name:<35} CLONED")),
        _ => RepoResult::Failed(format!("  {name:<35} CLONE FAILED")),
    }
}

async fn check_repo(root: &Path, name: &str, repo_path: &str) -> RepoResult {
    match check(root, repo_path).await {
        Ok(matrix) => {
            let status = match matrix.classification {
                SyncClassification::Parity => "OK parity",
                SyncClassification::Converge => "OK converge",
                _ => return RepoResult::Failed(format!("  {name:<35} DIVERGE")),
            };
            RepoResult::Synced(format!("  {name:<35} {status}"))
        }
        Err(e) => RepoResult::Failed(format!("  {name:<35} FAIL {e}")),
    }
}

async fn sync_repo(
    root: &Path,
    name: &str,
    repo_path: &str,
    push_target: &str,
    manifest: &crate::manifest::EcosystemManifest,
) -> RepoResult {
    match sync_with_policy(root, repo_path, push_target, Some(manifest)).await {
        Ok(r) if r.ok => RepoResult::Synced(format!("  {name:<35} OK {}", r.summary)),
        Ok(r) => RepoResult::Failed(format!("  {name:<35} FAIL {}", r.summary)),
        Err(e) => RepoResult::Failed(format!("  {name:<35} FAIL {e}")),
    }
}
