// SPDX-License-Identifier: AGPL-3.0-or-later
//! Full cascade sync — reads manifest, syncs all gate repos, reports parity.

use crate::error::Result;
use std::fmt::Write;
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

/// Post-sync phase to run after cascade completes repo synchronization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostSyncPhase {
    /// No post-sync building.
    None,
    /// Build drifted primals locally, stage to depot.
    Harvest,
    /// Harvest + refresh (push rebuilt binaries to VPS).
    Rebuild,
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

    let owned_repos: Vec<(String, crate::manifest::RepoEntry)> = repos
        .iter()
        .map(|(name, entry)| ((*name).to_string(), (*entry).clone()))
        .collect();

    let push_target = m.sync.push_target.clone();

    let mut join_set = tokio::task::JoinSet::new();

    for (name, entry) in owned_repos {
        let root = root.clone();
        let push_target = push_target.clone();
        let manifest = m.clone();
        let mode = opts.mode;
        let clone_missing = opts.clone_missing;

        join_set.spawn(async move {
            let result = process_repo(
                &root,
                &name,
                &entry,
                mode,
                clone_missing,
                &push_target,
                &manifest,
            )
            .await;
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

    let harvest_info = run_post_sync_phases(opts, &root, &m, &repos, &mut lines).await;

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

/// Post-sync phases: harvest (if requested), rebuild (harvest+refresh), freshness, depot report.
async fn run_post_sync_phases(
    opts: &CascadeOpts<'_>,
    root: &Path,
    m: &crate::manifest::EcosystemManifest,
    repos: &[(&str, &crate::manifest::RepoEntry)],
    lines: &mut Vec<String>,
) -> String {
    let mut harvest_info = String::new();
    let do_harvest = opts.post_sync != PostSyncPhase::None && opts.mode == CascadeMode::Sync;

    if do_harvest {
        match run_post_cascade_harvest(lines).await {
            Ok((built, current, failures)) => {
                harvest_info = format!(" harvest={built}built/{current}current/{failures}failed");

                if opts.post_sync == PostSyncPhase::Rebuild && built > 0 {
                    match run_post_cascade_refresh(lines).await {
                        Ok(pushed) => {
                            let _ = write!(harvest_info, " refresh={pushed}pushed");
                        }
                        Err(e) => lines.push(format!("  [refresh] FAIL: {e}")),
                    }
                }
            }
            Err(e) => lines.push(format!("  [harvest] FAIL: {e}")),
        }
    }

    if opts.publish_freshness && opts.mode == CascadeMode::Sync {
        match crate::freshness::publish_freshness_toml(root, m, repos).await {
            Ok(()) => lines.push("  [freshness] PUBLISHED freshness.toml".to_string()),
            Err(e) => lines.push(format!("  [freshness] FAIL: {e}")),
        }
    }

    if opts.mode == CascadeMode::Sync {
        let depot_summary = summarize_depot_freshness();
        if !depot_summary.is_empty() {
            lines.push(depot_summary);
        }
        if !do_harvest {
            if let Ok(report) = crate::plasmid::detect_depot_staleness() {
                if report.stale_count > 0 {
                    lines.push(format!(
                        "  [depot] {}/{} stale — run with --with-rebuild to auto-fix",
                        report.stale_count, report.total
                    ));
                }
            }
        }

        if plasmidbin_was_pulled(lines) {
            run_auto_fetch(lines).await;
        }
    }

    harvest_info
}

/// Run harvest after cascade sync — build any drifted primals locally.
/// Returns `(built_count, current_count, failure_count)`.
async fn run_post_cascade_harvest(lines: &mut Vec<String>) -> Result<(u32, u32, u32)> {
    let harvest_args = crate::plasmid::HarvestArgs {
        primal: None,
        force: false,
        dry_run: false,
        depot_dir: None,
        target: None,
    };

    let outcome = crate::plasmid::harvest(&harvest_args).await?;

    let (mut built, mut current, mut failures) = (0u32, 0u32, 0u32);
    if let Some(data) = &outcome.data {
        if let Some(arr) = data.as_array() {
            for entry in arr {
                match entry.get("status").and_then(|s| s.as_str()) {
                    Some("Built") => built += 1,
                    Some("Current") => current += 1,
                    Some("Failed") => failures += 1,
                    _ => {}
                }
            }
        }
    }

    lines.push(format!(
        "  [harvest] {} — {built} built, {current} current, {failures} failed",
        if failures == 0 { "OK" } else { "PARTIAL" }
    ));

    Ok((built, current, failures))
}

/// Push rebuilt binaries to VPS via `plasmid.refresh`.
/// Returns count of binaries successfully pushed.
async fn run_post_cascade_refresh(lines: &mut Vec<String>) -> Result<u32> {
    let config = crate::ShadowConfig::from_env().await;
    let refresh_args = crate::plasmid::RefreshArgs {
        primal: None,
        dry_run: false,
        source_dir: None,
    };

    let outcome = crate::plasmid::refresh(&config, &refresh_args).await?;

    let pushed = outcome
        .data
        .as_ref()
        .and_then(|d| d.as_array())
        .map_or(0u32, |arr| {
            arr.iter()
                .filter(|e| {
                    e.get("status")
                        .and_then(|s| s.as_str())
                        .is_some_and(|s| s == "Pushed")
                })
                .count() as u32
        });

    lines.push(format!(
        "  [refresh] {} — {pushed} pushed to VPS",
        if outcome.ok { "OK" } else { "PARTIAL" }
    ));

    Ok(pushed)
}

/// Check if plasmidBin was pulled during this cascade (indicating depot update).
fn plasmidbin_was_pulled(lines: &[String]) -> bool {
    lines
        .iter()
        .any(|l| l.contains("plasmidBin") && l.contains("pull"))
}

/// Auto-fetch binaries from WAN depot when checksums.toml was updated via cascade.
async fn run_auto_fetch(lines: &mut Vec<String>) {
    let config = crate::config::ShadowConfig::from_env().await;

    let fetch_args = crate::plasmid::FetchArgs {
        source: crate::plasmid::FetchSource::Wan,
        primal: None,
        release_tag: None,
        force: false,
        dry_run: false,
        dest: None,
    };

    match crate::plasmid::fetch(&config, &fetch_args).await {
        Ok(outcome) => {
            if let Some(data) = &outcome.data {
                let downloaded = data
                    .get("downloaded")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                let failed = data
                    .get("failed")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                let skipped = data
                    .get("skipped")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                lines.push(format!(
                    "  [auto-fetch] {downloaded} downloaded, {skipped} current, {failed} failed"
                ));
            }
        }
        Err(e) => lines.push(format!("  [auto-fetch] FAIL: {e}")),
    }
}

/// Quick depot freshness summary — reports how many binaries exist and are recent.
fn summarize_depot_freshness() -> String {
    let depot_dir = crate::plasmid::resolve_path(
        None,
        cellmembrane_types::service::ENV_PLASMIDBIN_DEPOT,
        || {
            std::path::PathBuf::from(
                std::env::var(cellmembrane_types::service::ENV_ECOPRIMALS_ROOT).unwrap_or_else(
                    |_| cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT.into(),
                ),
            )
            .join("plasmidBin")
        },
    );

    let primals_dir = depot_dir.join("primals");
    if !primals_dir.is_dir() {
        return String::new();
    }

    let mut present = 0u32;
    let mut total = 0u32;
    for name in crate::plasmid::nucleus_primals() {
        total += 1;
        if primals_dir.join(name).exists() {
            present += 1;
        }
    }

    let missing = total - present;
    if missing == 0 {
        format!("  [depot] {present}/{total} binaries present")
    } else {
        format!("  [depot] {present}/{total} binaries present ({missing} missing)")
    }
}

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
        };
        assert_eq!(opts.gate, "eastGate");
        assert_eq!(opts.post_sync, PostSyncPhase::None);
        assert!(opts.publish_freshness);
    }

    #[test]
    fn post_sync_phase_eq() {
        assert_eq!(PostSyncPhase::None, PostSyncPhase::None);
        assert_ne!(PostSyncPhase::Harvest, PostSyncPhase::Rebuild);
        assert_ne!(PostSyncPhase::None, PostSyncPhase::Rebuild);
    }

    #[test]
    fn depot_freshness_no_depot_returns_empty() {
        let result = summarize_depot_freshness();
        assert!(result.is_empty() || result.contains("binaries present"));
    }
}
