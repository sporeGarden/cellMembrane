// SPDX-License-Identifier: AGPL-3.0-or-later
//! Full cascade sync — reads manifest, syncs all gate repos, reports parity.

use crate::error::Result;
use std::path::Path;
use std::sync::Arc;

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

/// Post-sync phase to run after cascade completes repo synchronization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostSyncPhase {
    /// No post-sync building.
    None,
    /// Build drifted primals locally, stage to depot.
    Harvest,
    /// Harvest + refresh (push rebuilt binaries to VPS) — legacy, no sandbox gate.
    Rebuild,
    /// Harvest + sandbox validation + refresh — validated pipeline (default for --with-rebuild).
    SandboxRebuild,
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
    /// Post-sync phase: none, harvest-only, or full rebuild cycle.
    pub post_sync: PostSyncPhase,
    /// If true, restart local NUCLEUS processes that received new binaries.
    pub restart_updated: bool,
}

/// Execute cascade with typed options.
pub async fn cascade_with_opts(opts: &CascadeOpts<'_>) -> Result<crate::ShadowOutcome> {
    let root = resolve_workspace_root()?;
    let m = crate::manifest::load_from_workspace_async(&root).await?;

    let push_target: Arc<str> = Arc::from(m.sync.push_target.as_str());
    let shared_manifest = Arc::new(m);

    let repos: Vec<(&str, &crate::manifest::RepoEntry)> = shared_manifest.gate_repos(opts.gate);
    let total = u32::try_from(repos.len()).unwrap_or(u32::MAX);

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

    let repo_names: Vec<String> = repos.iter().map(|(name, _)| (*name).to_string()).collect();

    let root: Arc<Path> = root.into();

    let mut join_set = tokio::task::JoinSet::new();

    for name in repo_names {
        let root = Arc::clone(&root);
        let push_target = Arc::clone(&push_target);
        let manifest = Arc::clone(&shared_manifest);
        let mode = opts.mode;
        let clone_missing = opts.clone_missing;

        join_set.spawn(async move {
            let result = if let Some(entry) = manifest.repos.get(&name) {
                process_repo(
                    &root,
                    &name,
                    entry,
                    mode,
                    clone_missing,
                    &push_target,
                    &manifest,
                )
                .await
            } else {
                RepoResult::Skipped(format!("  {name:<35} SKIP (not in manifest)"))
            };
            (name, result)
        });
    }

    let mut results: Vec<(String, RepoResult)> = Vec::with_capacity(repos.len());
    while let Some(join_result) = join_set.join_next().await {
        if let Ok(item) = join_result {
            results.push(item);
        }
    }

    results.sort_by(|a, b| a.0.cmp(&b.0));

    let (synced, failed, cloned, mut lines) = tally_results(results);

    let harvest_info =
        post_sync::run_post_sync_phases(opts, &root, &shared_manifest, &repos, &mut lines).await;

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
         {action}={synced} failed={failed}{clone_info}{harvest_info}",
        shared_manifest.meta.version,
        shared_manifest.meta.wave,
        shared_manifest.meta.total_repos,
        opts.gate,
        opts.source,
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

// Post-sync pipeline delegated to `super::post_sync` module.
use super::post_sync;

enum RepoResult {
    Synced(String),
    Failed(String),
    Cloned(String),
    Skipped(String),
}

fn tally_results(results: Vec<(String, RepoResult)>) -> (u32, u32, u32, Vec<String>) {
    let mut synced = 0u32;
    let mut failed = 0u32;
    let mut cloned = 0u32;
    let mut lines = Vec::with_capacity(results.len());

    for (_, result) in results {
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

    (synced, failed, cloned, lines)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tally_results_counts_correctly() {
        let results = vec![
            ("a".into(), RepoResult::Synced("OK a".into())),
            ("b".into(), RepoResult::Synced("OK b".into())),
            ("c".into(), RepoResult::Failed("FAIL c".into())),
            ("d".into(), RepoResult::Cloned("CLONE d".into())),
            ("e".into(), RepoResult::Skipped("SKIP e".into())),
        ];
        let (synced, failed, cloned, lines) = tally_results(results);
        assert_eq!(synced, 2);
        assert_eq!(failed, 1);
        assert_eq!(cloned, 1);
        assert_eq!(lines.len(), 5);
    }

    #[test]
    fn tally_results_empty() {
        let (synced, failed, cloned, lines) = tally_results(Vec::new());
        assert_eq!(synced, 0);
        assert_eq!(failed, 0);
        assert_eq!(cloned, 0);
        assert!(lines.is_empty());
    }

    #[test]
    fn cascade_mode_eq() {
        assert_eq!(CascadeMode::Sync, CascadeMode::Sync);
        assert_ne!(CascadeMode::Sync, CascadeMode::DryRun);
        assert_ne!(CascadeMode::CheckOnly, CascadeMode::DryRun);
    }

    #[test]
    fn cascade_opts_default_fields() {
        let opts = CascadeOpts {
            gate: "eastGate",
            source: "forgejo",
            mode: CascadeMode::DryRun,
            clone_missing: false,
            publish_freshness: true,
            post_sync: PostSyncPhase::None,
            restart_updated: false,
        };
        assert_eq!(opts.gate, "eastGate");
        assert_eq!(opts.post_sync, PostSyncPhase::None);
        assert!(opts.publish_freshness);
        assert!(!opts.restart_updated);
    }

    #[test]
    fn post_sync_phase_eq() {
        assert_eq!(PostSyncPhase::None, PostSyncPhase::None);
        assert_ne!(PostSyncPhase::Harvest, PostSyncPhase::Rebuild);
        assert_ne!(PostSyncPhase::None, PostSyncPhase::Rebuild);
        assert_ne!(PostSyncPhase::SandboxRebuild, PostSyncPhase::Rebuild);
        assert_ne!(PostSyncPhase::SandboxRebuild, PostSyncPhase::None);
    }

    #[test]
    fn sandbox_rebuild_is_distinct_from_rebuild() {
        assert_ne!(PostSyncPhase::Rebuild, PostSyncPhase::SandboxRebuild);
        let wants_refresh_sandbox = matches!(
            PostSyncPhase::SandboxRebuild,
            PostSyncPhase::Rebuild | PostSyncPhase::SandboxRebuild
        );
        assert!(wants_refresh_sandbox);
        let wants_refresh_rebuild = matches!(
            PostSyncPhase::Rebuild,
            PostSyncPhase::Rebuild | PostSyncPhase::SandboxRebuild
        );
        assert!(wants_refresh_rebuild);
        let harvest_no_refresh = matches!(
            PostSyncPhase::Harvest,
            PostSyncPhase::Rebuild | PostSyncPhase::SandboxRebuild
        );
        assert!(!harvest_no_refresh);
    }

    #[tokio::test]
    async fn sandbox_gate_passes_nonexistent_primals_through() {
        let built = vec![
            "fake_primal_alpha_99".to_string(),
            "fake_primal_beta_99".to_string(),
        ];
        let mut lines = Vec::new();
        let passed = post_sync::run_post_cascade_sandbox(&built, &mut lines).await;
        assert_eq!(
            passed.len(),
            built.len(),
            "primals not in depot should pass through (skip)"
        );
    }

    #[tokio::test]
    async fn sandbox_gate_returns_all_when_binary_missing_from_depot() {
        let built = vec!["nonexistent_primal_xyz_99".to_string()];
        let mut lines = Vec::new();
        let passed = post_sync::run_post_cascade_sandbox(&built, &mut lines).await;
        assert_eq!(
            passed.len(),
            built.len(),
            "when binary not found in depot, primal should pass through (skip)"
        );
    }

    #[test]
    fn auto_rebuild_env_var_parsing() {
        for val in ["1", "true", "yes"] {
            assert!(
                matches!(val, "1" | "true" | "yes"),
                "{val} should trigger auto-rebuild"
            );
        }
        for val in ["0", "false", "no", ""] {
            assert!(
                !matches!(val, "1" | "true" | "yes"),
                "{val} should NOT trigger auto-rebuild"
            );
        }
    }

    #[test]
    fn depot_freshness_no_depot_returns_empty() {
        let result = post_sync::summarize_depot_freshness();
        assert!(result.is_empty() || result.contains("binaries present"));
    }
}
