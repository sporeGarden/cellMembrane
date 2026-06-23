// SPDX-License-Identifier: AGPL-3.0-or-later
//! Post-cascade sync pipeline — harvest, sandbox, refresh, auto-rebuild, auto-fetch.
//!
//! Extracted from `cascade.rs` for separation of concerns. This module owns the
//! binary lifecycle phases that run after repository sync completes.

use crate::error::Result;
use std::fmt::Write;

use super::cascade::{CascadeMode, CascadeOpts, PostSyncPhase};

/// Post-sync phases: harvest (if requested), rebuild (harvest+refresh), freshness, depot report.
pub(super) async fn run_post_sync_phases(
    opts: &CascadeOpts<'_>,
    root: &std::path::Path,
    m: &crate::manifest::EcosystemManifest,
    repos: &[(&str, &crate::manifest::RepoEntry)],
    lines: &mut Vec<String>,
) -> String {
    let mut harvest_info = String::new();
    let do_harvest = opts.post_sync != PostSyncPhase::None && opts.mode == CascadeMode::Sync;

    if do_harvest {
        match run_post_cascade_harvest(lines).await {
            Ok((built, built_primals, current, failures)) => {
                harvest_info = format!(" harvest={built}built/{current}current/{failures}failed");

                let wants_refresh = matches!(
                    opts.post_sync,
                    PostSyncPhase::Rebuild | PostSyncPhase::SandboxRebuild
                );

                if wants_refresh && built > 0 {
                    let refresh_targets = if opts.post_sync == PostSyncPhase::SandboxRebuild {
                        let passed = run_post_cascade_sandbox(&built_primals, lines).await;
                        let _ = write!(
                            harvest_info,
                            " sandbox={}/{}passed",
                            passed.len(),
                            built_primals.len()
                        );
                        if passed.is_empty() {
                            lines.push("  [sandbox] ALL BLOCKED — no binaries promoted".into());
                            Vec::new()
                        } else {
                            passed
                        }
                    } else {
                        built_primals
                    };

                    if !refresh_targets.is_empty() {
                        match run_post_cascade_refresh(Some(&refresh_targets), lines).await {
                            Ok(pushed) => {
                                let _ = write!(harvest_info, " refresh={pushed}pushed");
                            }
                            Err(e) => lines.push(format!("  [refresh] FAIL: {e}")),
                        }
                    }
                }
            }
            Err(e) => lines.push(format!("  [harvest] FAIL: {e}")),
        }
    }

    if opts.publish_freshness && opts.mode == CascadeMode::Sync {
        let is_designated_publisher = is_freshness_publisher();
        if is_designated_publisher {
            match crate::freshness::publish_freshness_toml(root, m, repos).await {
                Ok(()) => {
                    lines.push("  [freshness] PUBLISHED freshness.toml".to_string());
                    match crate::freshness::auto_commit_freshness(root, m, repos).await {
                        Ok(()) => {}
                        Err(e) => lines.push(format!("  [freshness] auto-push: {e}")),
                    }
                }
                Err(e) => lines.push(format!("  [freshness] FAIL: {e}")),
            }
        } else {
            lines.push("  [freshness] SKIP — not designated publisher (set FRESHNESS_PUBLISHER=1 to enable)".to_string());
        }
    }

    if opts.mode == CascadeMode::Sync {
        let heads = collect_cascade_heads(root, repos).await;
        if !heads.is_empty() {
            run_rootpulse_sovereignty(m.meta.wave, opts.gate, &heads, lines).await;
        }
    }

    if opts.mode == CascadeMode::Sync {
        run_depot_staleness_and_fetch(do_harvest, opts.restart_updated, lines).await;
    }

    harvest_info
}

/// Depot staleness reporting, auto-rebuild, and auto-fetch pipeline.
async fn run_depot_staleness_and_fetch(
    did_harvest: bool,
    restart_updated: bool,
    lines: &mut Vec<String>,
) {
    let depot_summary = tokio::task::spawn_blocking(summarize_depot_freshness)
        .await
        .unwrap_or_default();
    if !depot_summary.is_empty() {
        lines.push(depot_summary);
    }
    if !did_harvest {
        let staleness = tokio::task::spawn_blocking(crate::plasmid::detect_depot_staleness)
            .await
            .ok()
            .and_then(std::result::Result::ok);
        if let Some(report) = staleness.filter(|r| r.stale_count > 0) {
            let auto_rebuild = std::env::var(cellmembrane_types::service::ENV_AUTO_REBUILD)
                .is_ok_and(|v| matches!(v.as_str(), "1" | "true" | "yes"));

            if auto_rebuild {
                lines.push(format!(
                    "  [depot] {}/{} stale — MEMBRANE_AUTO_REBUILD: triggering rebuild",
                    report.stale_count, report.total
                ));
                run_auto_rebuild(lines).await;
            } else {
                lines.push(format!(
                    "  [depot] {}/{} stale — run with --with-rebuild to auto-fix",
                    report.stale_count, report.total
                ));
            }
        }
    }

    if plasmidbin_was_pulled(lines) {
        run_auto_fetch(lines).await;
        if restart_updated {
            run_cascade_restart(lines).await;
        }
    }
}

/// Run harvest after cascade sync — build any drifted primals locally.
/// Returns `(built_count, built_primal_names, current_count, failure_count)`.
async fn run_post_cascade_harvest(lines: &mut Vec<String>) -> Result<(u32, Vec<String>, u32, u32)> {
    let harvest_args = crate::plasmid::HarvestArgs {
        primal: None,
        force: false,
        dry_run: false,
        depot_dir: None,
        target: None,
    };

    let outcome = crate::plasmid::harvest(&harvest_args).await?;

    let (mut built, mut current, mut failures) = (0u32, 0u32, 0u32);
    let mut built_primals: Vec<String> = Vec::new();
    if let Some(data) = &outcome.data {
        if let Some(arr) = data.as_array() {
            for entry in arr {
                match entry.get("status").and_then(|s| s.as_str()) {
                    Some("Built") => {
                        built += 1;
                        if let Some(name) = entry.get("binary").and_then(|b| b.as_str()) {
                            built_primals.push(name.to_string());
                        }
                    }
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

    Ok((built, built_primals, current, failures))
}

/// Sandbox-validate built primals before allowing refresh.
/// Returns the subset of `built_primals` that passed health validation.
pub(super) async fn run_post_cascade_sandbox(
    built_primals: &[String],
    lines: &mut Vec<String>,
) -> Vec<String> {
    let Ok(depot_dir) = crate::plasmid::depot::resolve_depot(None) else {
        lines.push("  [sandbox] SKIP — depot not resolved".into());
        return built_primals.to_vec();
    };

    let arch = crate::plasmid::detect_target_triple();
    let bin_dir = depot_dir.join("primals").join(&arch);

    let mut passed: Vec<String> = Vec::new();
    let mut failed_names: Vec<String> = Vec::new();

    for primal in built_primals {
        let binary_path = bin_dir.join(primal);
        if !binary_path.exists() {
            lines.push(format!("  [sandbox] {primal}: SKIP (binary not in depot)"));
            passed.push(primal.clone());
            continue;
        }

        let args = crate::plasmid::sandbox::SandboxArgs {
            primal: primal.clone(),
            commit: "cascade-rebuild".into(),
            binary_path,
            timeout_secs: Some(20),
        };

        match crate::plasmid::sandbox::validate_with_deps(&args).await {
            Ok(result) if result.health_ok => {
                passed.push(primal.clone());
            }
            Ok(result) => {
                failed_names.push(primal.clone());
                lines.push(format!(
                    "  [sandbox] {primal}: FAIL — {} ({}ms)",
                    result.detail, result.elapsed_ms
                ));
            }
            Err(e) => {
                failed_names.push(primal.clone());
                lines.push(format!("  [sandbox] {primal}: ERROR — {e}"));
            }
        }
    }

    let total = built_primals.len();
    let pass_count = passed.len();
    if failed_names.is_empty() {
        lines.push(format!("  [sandbox] OK — {pass_count}/{total} passed"));
    } else {
        lines.push(format!(
            "  [sandbox] PARTIAL — {pass_count}/{total} passed, blocked: {}",
            failed_names.join(", ")
        ));
    }

    passed
}

/// Push rebuilt binaries to VPS via `plasmid.refresh`.
/// When `filter` is `Some`, only those primals are refreshed.
/// Returns count of binaries successfully pushed.
async fn run_post_cascade_refresh(
    filter: Option<&[String]>,
    lines: &mut Vec<String>,
) -> Result<u32> {
    let config = crate::ShadowConfig::from_env().await;

    let mut total_pushed = 0u32;

    if let Some(primals) = filter {
        for primal in primals {
            let refresh_args = crate::plasmid::RefreshArgs {
                primal: Some(primal.clone()),
                dry_run: false,
                source_dir: None,
            };
            match crate::plasmid::refresh(&config, &refresh_args).await {
                Ok(outcome) => {
                    let pushed =
                        outcome
                            .data
                            .as_ref()
                            .and_then(|d| d.as_array())
                            .map_or(0u32, |arr| {
                                u32::try_from(
                                    arr.iter()
                                        .filter(|e| {
                                            e.get("status")
                                                .and_then(|s| s.as_str())
                                                .is_some_and(|s| s == "Pushed")
                                        })
                                        .count(),
                                )
                                .unwrap_or(u32::MAX)
                            });
                    total_pushed += pushed;
                }
                Err(e) => lines.push(format!("  [refresh] {primal}: FAIL — {e}")),
            }
        }
    } else {
        let refresh_args = crate::plasmid::RefreshArgs {
            primal: None,
            dry_run: false,
            source_dir: None,
        };
        let outcome = crate::plasmid::refresh(&config, &refresh_args).await?;
        total_pushed = outcome
            .data
            .as_ref()
            .and_then(|d| d.as_array())
            .map_or(0u32, |arr| {
                u32::try_from(
                    arr.iter()
                        .filter(|e| {
                            e.get("status")
                                .and_then(|s| s.as_str())
                                .is_some_and(|s| s == "Pushed")
                        })
                        .count(),
                )
                .unwrap_or(u32::MAX)
            });
    }

    lines.push(format!(
        "  [refresh] {} — {total_pushed} pushed to VPS",
        if total_pushed > 0 { "OK" } else { "PARTIAL" }
    ));

    Ok(total_pushed)
}

/// Auto-rebuild pipeline triggered by `MEMBRANE_AUTO_REBUILD` when staleness is detected.
/// Runs: harvest -> sandbox -> refresh (full validated pipeline).
async fn run_auto_rebuild(lines: &mut Vec<String>) {
    match run_post_cascade_harvest(lines).await {
        Ok((built, built_primals, _current, _failures)) => {
            if built == 0 {
                lines.push("  [auto-rebuild] nothing to rebuild".into());
                return;
            }
            let passed = run_post_cascade_sandbox(&built_primals, lines).await;
            if passed.is_empty() {
                lines.push("  [auto-rebuild] sandbox blocked all — no refresh".into());
                return;
            }
            match run_post_cascade_refresh(Some(&passed), lines).await {
                Ok(pushed) => {
                    lines.push(format!(
                        "  [auto-rebuild] DONE — {built} harvested, {} sandbox-passed, {pushed} pushed",
                        passed.len()
                    ));
                }
                Err(e) => lines.push(format!("  [auto-rebuild] refresh FAIL: {e}")),
            }
        }
        Err(e) => lines.push(format!("  [auto-rebuild] harvest FAIL: {e}")),
    }
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

pub(super) use super::nucleus_restart::run_cascade_restart;

/// Commit cascade state to rootPulse and verify sovereignty.
async fn run_rootpulse_sovereignty(
    wave_id: u32,
    gate: &str,
    heads: &std::collections::BTreeMap<String, String>,
    lines: &mut Vec<String>,
) {
    match crate::freshness::rootpulse_commit(wave_id, gate, heads).await {
        Ok(session) => {
            lines.push(format!("  [rootpulse] COMMITTED {session}"));
            persist_rootpulse_session(wave_id, gate, &session);
        }
        Err(e) => {
            lines.push(format!("  [rootpulse] SKIP: {e}"));
        }
    }

    let checks = crate::freshness::sovereignty_verify(wave_id, heads).await;
    if !checks.is_empty() {
        let verified = checks.iter().filter(|c| c.verified).count();
        let total = checks.len();
        if verified == total {
            lines.push(format!("  [sovereignty] VERIFIED {verified}/{total}"));
        } else {
            lines.push(format!("  [sovereignty] {verified}/{total} verified"));
            for check in &checks {
                if !check.verified {
                    lines.push(format!("    \u{26a0} {}: {}", check.repo, check.detail));
                }
            }
        }
    }
}

/// Collect HEAD SHAs for all cloned repos in the cascade set.
pub async fn collect_cascade_heads(
    root: &std::path::Path,
    repos: &[(&str, &crate::manifest::RepoEntry)],
) -> std::collections::BTreeMap<String, String> {
    let mut heads = std::collections::BTreeMap::new();
    for (name, entry) in repos {
        let repo_dir = root.join(&entry.local_path);
        if repo_dir.join(".git").exists() {
            if let Ok(sha) = crate::git_ops::git_output(&repo_dir, &["rev-parse", "HEAD"]).await {
                heads.insert((*name).to_string(), sha);
            }
        }
    }
    heads
}

/// Quick depot freshness summary — reports how many binaries exist and are recent.
pub(super) fn summarize_depot_freshness() -> String {
    let depot_dir = crate::plasmid::resolve_path(
        None,
        cellmembrane_types::service::ENV_PLASMIDBIN_DEPOT,
        || {
            std::path::PathBuf::from(cellmembrane_types::service::env_or(
                cellmembrane_types::service::ENV_ECOPRIMALS_ROOT,
                cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT,
            ))
            .join(cellmembrane_types::service::PLASMID_BIN_DIR)
        },
    );

    let arch = crate::plasmid::detect_target_triple();
    let primals_dir = depot_dir.join("primals").join(&arch);
    if !primals_dir.is_dir() {
        return String::new();
    }

    let mut present = 0u32;
    let mut total = 0u32;
    let mut stale = 0u32;
    let now = std::time::SystemTime::now();
    let stale_threshold = std::time::Duration::from_secs(
        cellmembrane_types::service::DEFAULT_STALENESS_THRESHOLD_SECS,
    );

    for name in crate::plasmid::nucleus_primals() {
        total += 1;
        let path = primals_dir.join(name);
        if path.exists() {
            present += 1;
            if let Ok(meta) = std::fs::metadata(&path) {
                if let Ok(modified) = meta.modified() {
                    if now.duration_since(modified).unwrap_or_default() > stale_threshold {
                        stale += 1;
                    }
                }
            }
        }
    }

    let missing = total - present;
    let suffix = match (missing, stale) {
        (0, 0) => String::new(),
        (0, s) => format!(" ({s} stale — run with --with-rebuild to auto-fix)"),
        (m, 0) => format!(" ({m} missing)"),
        (m, s) => format!(" ({m} missing, {s} stale)"),
    };
    format!("  [depot] {present}/{total} binaries present{suffix}")
}

/// Check if this gate is the designated freshness publisher.
///
/// Single-writer policy: exactly one gate per mesh publishes freshness.toml to
/// avoid multi-writer race conditions. Determined purely by capability:
/// `FRESHNESS_PUBLISHER=1` (set in the gate's service environment).
///
/// Any gate with build authority can be the publisher — the identity is
/// infrastructure configuration, not code knowledge.
fn is_freshness_publisher() -> bool {
    std::env::var(cellmembrane_types::service::ENV_FRESHNESS_PUBLISHER)
        .is_ok_and(|v| matches!(v.as_str(), "1" | "true" | "yes"))
}

/// Persist rootpulse session to gate-local state (not the shared manifest).
///
/// Writes to `{workspace}/infra/wateringHole/.rootpulse_state.toml`.
pub fn persist_rootpulse_session(wave_id: u32, gate: &str, session_id: &str) {
    let Ok(root) = crate::temporal::resolve_workspace_root() else {
        return;
    };
    let state_path = root
        .join(cellmembrane_types::service::INFRA_WATERING_HOLE)
        .join(".rootpulse_state.toml");
    let content = format!(
        "# Last rootpulse commit — auto-generated, do not edit\n\
         wave = {wave_id}\n\
         gate = \"{gate}\"\n\
         session = \"{session_id}\"\n\
         timestamp = \"{}\"\n",
        chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ")
    );
    let _ = std::fs::write(&state_path, content);
}

/// Load the last rootpulse session ID from gate-local state.
pub fn load_rootpulse_session() -> Option<String> {
    let root = crate::temporal::resolve_workspace_root().ok()?;
    let state_path = root
        .join(cellmembrane_types::service::INFRA_WATERING_HOLE)
        .join(".rootpulse_state.toml");
    let contents = std::fs::read_to_string(&state_path).ok()?;
    let table: toml::Table = contents.parse().ok()?;
    table
        .get("session")
        .and_then(|v| v.as_str())
        .map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plasmidbin_was_pulled_positive() {
        let lines = vec![
            "  [parity] primals/bearDog — pull".into(),
            "  [parity] infra/plasmidBin — pull".into(),
        ];
        assert!(plasmidbin_was_pulled(&lines));
    }

    #[test]
    fn plasmidbin_was_pulled_negative() {
        let lines = vec![
            "  [parity] primals/bearDog — pull".into(),
            "  [parity] infra/wateringHole — pull".into(),
        ];
        assert!(!plasmidbin_was_pulled(&lines));
    }

    #[test]
    fn is_freshness_publisher_defaults_false() {
        if std::env::var(cellmembrane_types::service::ENV_FRESHNESS_PUBLISHER).is_err() {
            assert!(!is_freshness_publisher());
        }
    }

    #[test]
    fn rootpulse_state_toml_roundtrip() {
        let toml_content = r#"wave = 116
gate = "sporeGate"
session = "rp-116-abc123"
timestamp = "2026-06-19T12:00:00Z"
"#;
        let table: toml::Table = toml_content.parse().unwrap();
        let session = table
            .get("session")
            .and_then(|v| v.as_str())
            .map(String::from);
        assert_eq!(session.as_deref(), Some("rp-116-abc123"));

        let wave = table.get("wave").and_then(toml::Value::as_integer).unwrap();
        assert_eq!(wave, 116);

        let gate = table.get("gate").and_then(toml::Value::as_str).unwrap();
        assert_eq!(gate, "sporeGate");
    }

    #[test]
    fn summarize_depot_suffix_formatting() {
        let missing = 2u32;
        let stale = 3u32;
        let suffix = match (missing, stale) {
            (0, 0) => String::new(),
            (0, s) => format!(" ({s} stale — run with --with-rebuild to auto-fix)"),
            (m, 0) => format!(" ({m} missing)"),
            (m, s) => format!(" ({m} missing, {s} stale)"),
        };
        assert_eq!(suffix, " (2 missing, 3 stale)");

        let suffix_none = match (0u32, 0u32) {
            (0, 0) => String::new(),
            (0, s) => format!(" ({s} stale — run with --with-rebuild to auto-fix)"),
            (m, 0) => format!(" ({m} missing)"),
            (m, s) => format!(" ({m} missing, {s} stale)"),
        };
        assert!(suffix_none.is_empty());
    }
}
