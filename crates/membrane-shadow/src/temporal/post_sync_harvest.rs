// SPDX-License-Identifier: AGPL-3.0-or-later

//! Post-cascade harvest, sandbox, refresh, and auto-rebuild pipeline.
//!
//! Extracted from `post_sync.rs` — these functions manage the binary
//! lifecycle phases that keep the depot current after a cascade sync.

use crate::error::Result;

pub(super) use super::nucleus_restart::run_cascade_restart;

/// Depot staleness reporting, auto-rebuild, and auto-fetch pipeline.
pub(super) async fn run_depot_staleness_and_fetch(
    did_harvest: bool,
    restart_updated: bool,
    lines: &mut Vec<String>,
) {
    let depot_summary =
        tokio::task::spawn_blocking(super::post_sync_content::summarize_depot_freshness)
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

    let crash_report = crate::gate::crash_loop::scan_and_break_async(None).await;
    if crash_report.has_loops() {
        lines.push(format!(
            "  [crash-loop] BREAKER TRIGGERED: {} service(s) disabled",
            crash_report.disabled_count(),
        ));
        for entry in &crash_report.loops {
            lines.push(format!(
                "  [crash-loop]   {}: {} restarts → {}",
                entry.unit, entry.restart_count, entry.action,
            ));
        }
    }
}

/// Run harvest after cascade sync — build any drifted primals locally,
/// then fan out to manifest-registered sub-builders for cross-arch targets.
/// Returns `(built_count, built_primal_names, current_count, failure_count)`.
pub(super) async fn run_post_cascade_harvest(
    lines: &mut Vec<String>,
) -> Result<(u32, Vec<String>, u32, u32)> {
    let harvest_args = crate::plasmid::HarvestArgs {
        primal: None,
        force: false,
        dry_run: false,
        depot_dir: None,
        target: None,
        local: false,
        push: false,
        with_restart: false,
    };

    let outcome = crate::plasmid::harvest(&harvest_args).await?;

    let (mut built, mut current, mut failures) = (0u32, 0u32, 0u32);
    let mut built_primals: Vec<String> = Vec::new();
    if let Some(data) = &outcome.data
        && let Some(arr) = data.as_array()
    {
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

    lines.push(format!(
        "  [harvest] {} — {built} built, {current} current, {failures} failed",
        if failures == 0 { "OK" } else { "PARTIAL" }
    ));

    if built > 0 {
        dispatch_to_sub_builders(&built_primals, lines).await;
    }

    Ok((built, built_primals, current, failures))
}

/// Fan out harvest to manifest-registered sub-builders via Tower Atomic mesh.
///
/// For each primal that was rebuilt locally, dispatch a `plasmid.harvest`
/// request to every sub-builder registered in `ecosystem_manifest.toml`.
/// Sub-builders rebuild for their own target triple and stage to their
/// local depot. The foreman collects results via mesh — zero SSH.
async fn dispatch_to_sub_builders(built_primals: &[String], lines: &mut Vec<String>) {
    let sub_builders = crate::dispatch::sovereign_sub_builders();
    if sub_builders.is_empty() {
        return;
    }

    let mut dispatched = 0u32;
    let mut succeeded = 0u32;
    let mut failed_gates: Vec<String> = Vec::new();

    for sb in &sub_builders {
        for primal in built_primals {
            dispatched += 1;
            let request = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "plasmid.harvest",
                "params": {
                    "primal": primal,
                    "force": true,
                    "local": true,
                    "push": false,
                },
                "id": dispatched,
            })
            .to_string();

            match crate::jsonrpc::call_endpoint(&sb.endpoint, &request).await {
                Ok(response) => {
                    let parsed: serde_json::Value =
                        serde_json::from_str(&response).unwrap_or_default();
                    let ok = parsed
                        .get("result")
                        .and_then(|r| r.get("ok"))
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false);
                    if ok {
                        succeeded += 1;
                    } else {
                        failed_gates.push(format!("{}:{}", sb.gate, primal));
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        gate = %sb.gate,
                        target = %sb.target,
                        primal,
                        error = %e,
                        "sub-builder mesh dispatch failed"
                    );
                    failed_gates.push(format!("{}:{}", sb.gate, primal));
                }
            }
        }
    }

    if dispatched > 0 {
        if failed_gates.is_empty() {
            lines.push(format!(
                "  [sub-builders] OK — {succeeded}/{dispatched} cross-arch harvests via mesh"
            ));
        } else {
            lines.push(format!(
                "  [sub-builders] PARTIAL — {succeeded}/{dispatched} OK, failed: {}",
                failed_gates.join(", ")
            ));
        }
    }
}

/// Sandbox-validate built primals before allowing refresh.
/// Returns the subset of `built_primals` that passed health validation.
pub(super) async fn run_post_cascade_sandbox(
    built_primals: &[String],
    lines: &mut Vec<String>,
) -> Vec<String> {
    let Ok(depot_dir) = crate::plasmid::depot::resolve_depot(None) else {
        lines.push("  [sandbox] BLOCKED — depot not resolved, no primals promoted".into());
        return Vec::new();
    };

    let arch = crate::plasmid::detect_target_triple();
    let bin_dir = depot_dir.join("primals").join(arch);

    if let Err(e) = cellmembrane_types::PlatformAccess::Executable.apply(&bin_dir) {
        tracing::debug!(error = %e, "sandbox bin_dir chmod (non-fatal)");
    }

    let mut passed: Vec<String> = Vec::new();
    let mut failed_names: Vec<String> = Vec::new();

    for primal in built_primals {
        let binary_path = bin_dir.join(primal);
        if !binary_path.exists() {
            lines.push(format!("  [sandbox] {primal}: SKIP (binary not in depot)"));
            continue;
        }

        if let Err(e) = cellmembrane_types::PlatformAccess::Executable.apply(&binary_path) {
            tracing::warn!(error = %e, path = %binary_path.display(), "pre-sandbox chmod failed");
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
pub(super) async fn run_post_cascade_refresh(
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
pub(super) fn plasmidbin_was_pulled(lines: &[String]) -> bool {
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
        trust_policy: cellmembrane_types::DepotTrustPolicy::VerifyIfPresent,
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
