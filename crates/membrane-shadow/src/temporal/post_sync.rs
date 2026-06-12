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
                    let refresh_targets =
                        if opts.post_sync == PostSyncPhase::SandboxRebuild {
                            let passed = run_post_cascade_sandbox(&built_primals, lines).await;
                            let _ = write!(
                                harvest_info,
                                " sandbox={}/{}passed",
                                passed.len(),
                                built_primals.len()
                            );
                            if passed.is_empty() {
                                lines.push(
                                    "  [sandbox] ALL BLOCKED — no binaries promoted".into(),
                                );
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
                    let auto_rebuild =
                        std::env::var(cellmembrane_types::service::ENV_AUTO_REBUILD)
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
        }

        if plasmidbin_was_pulled(lines) {
            run_auto_fetch(lines).await;

            if opts.restart_updated {
                run_cascade_restart(lines).await;
            }
        }
    }

    harvest_info
}

/// Run harvest after cascade sync — build any drifted primals locally.
/// Returns `(built_count, built_primal_names, current_count, failure_count)`.
async fn run_post_cascade_harvest(
    lines: &mut Vec<String>,
) -> Result<(u32, Vec<String>, u32, u32)> {
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
                arr.iter()
                    .filter(|e| {
                        e.get("status")
                            .and_then(|s| s.as_str())
                            .is_some_and(|s| s == "Pushed")
                    })
                    .count() as u32
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

/// Restart local NUCLEUS processes whose binaries were updated in the depot.
///
/// Compares the running binary (via `/proc/{pid}/exe` readlink) against the depot
/// binary hash. If they differ, restarts the service unit. This enables a "pull and
/// converge" workflow without manual intervention.
pub(super) async fn run_cascade_restart(lines: &mut Vec<String>) {
    let arch = crate::plasmid::detect_target_triple();
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
    let bin_dir = depot_dir.join("primals").join(&arch);

    let install_base = std::env::var(cellmembrane_types::service::ENV_INSTALL_BASE)
        .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_INSTALL_BASE.into());
    let install_dir = std::path::Path::new(&install_base);

    let primals = crate::plasmid::nucleus_primals();
    let mut restarted = 0u32;
    let mut skipped = 0u32;
    let mut failed = 0u32;

    for primal in &primals {
        let depot_bin = bin_dir.join(primal);
        let installed_bin = install_dir.join(primal);

        if !depot_bin.exists() || !installed_bin.exists() {
            continue;
        }

        let depot_hash = crate::plasmid::compute_blake3_file(&depot_bin);
        let installed_hash = crate::plasmid::compute_blake3_file(&installed_bin);

        if depot_hash == installed_hash {
            skipped += 1;
            continue;
        }

        let sandbox_args = crate::plasmid::sandbox::SandboxArgs {
            primal: (*primal).to_string(),
            commit: depot_hash[..8].to_string(),
            binary_path: depot_bin.clone(),
            timeout_secs: None,
        };

        let sandbox_ok = match crate::plasmid::sandbox::validate_with_deps(&sandbox_args).await {
            Ok(result) => result.health_ok,
            Err(e) => {
                lines.push(format!(
                    "  [cascade-restart] {primal} sandbox infra error (proceeding): {e}"
                ));
                true
            }
        };

        if !sandbox_ok {
            lines.push(format!(
                "  [cascade-restart] {primal} sandbox FAIL — skipping"
            ));
            failed += 1;
            continue;
        }

        // Retire current production binary to canary pool before overwriting
        if installed_bin.exists() {
            let _ = crate::plasmid::canary::retire_to_canary(
                primal,
                &installed_bin,
                &installed_hash[..8],
            )
            .await;
        }

        if std::fs::copy(&depot_bin, &installed_bin).is_err() {
            failed += 1;
            continue;
        }

        let unit = format!("membrane-nucleus@{primal}.service");
        let restart = tokio::process::Command::new("systemctl")
            .args(["--user", "restart", &unit])
            .output()
            .await;

        match restart {
            Ok(o) if o.status.success() => restarted += 1,
            _ => failed += 1,
        }
    }

    let tag = if failed == 0 { "OK" } else { "PARTIAL" };
    lines.push(format!(
        "  [cascade-restart] {tag} — {restarted} restarted, {skipped} current, {failed} failed"
    ));
}

/// Quick depot freshness summary — reports how many binaries exist and are recent.
pub(super) fn summarize_depot_freshness() -> String {
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
