// SPDX-License-Identifier: AGPL-3.0-or-later
//! Post-cascade sync pipeline — orchestrator.
//!
//! Coordinates harvest, sandbox, refresh, auto-rebuild, auto-fetch,
//! freshness, sovereignty, and content rebuild phases after repository
//! sync completes.
//!
//! Phase implementations live in `post_sync_harvest.rs` (binary lifecycle)
//! and `post_sync_content.rs` (sovereignty/content/drift).

use std::fmt::Write;

use super::cascade::{CascadeMode, CascadeOpts, PostSyncPhase};

#[cfg(test)]
use super::post_sync_content::is_build_authority;
#[cfg(test)]
pub(super) use super::post_sync_content::summarize_depot_freshness;
pub(crate) use super::post_sync_content::{
    collect_cascade_heads, load_rootpulse_session, persist_rootpulse_session,
};
#[cfg(test)]
use super::post_sync_harvest::plasmidbin_was_pulled;
pub(super) use super::post_sync_harvest::run_post_cascade_sandbox;

use super::post_sync_content::{
    check_content_health, is_freshness_publisher, run_commit_drift_pipeline,
    run_content_rebuild_if_needed, run_rootpulse_sovereignty,
};
use super::post_sync_harvest::{run_depot_staleness_and_fetch, run_post_cascade_refresh};

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
        match super::post_sync_harvest::run_post_cascade_harvest(lines).await {
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
        match crate::freshness::publish_gate_heads(root, repos).await {
            Ok(()) => {
                lines.push("  [freshness] PUBLISHED heads/<gate>.toml".to_string());
                match crate::freshness::auto_commit_gate_heads(root, repos).await {
                    Ok(()) => {}
                    Err(e) => lines.push(format!("  [freshness] auto-push heads: {e}")),
                }
            }
            Err(e) => lines.push(format!("  [freshness] gate heads FAIL: {e}")),
        }

        let is_designated_publisher = is_freshness_publisher();
        if is_designated_publisher {
            match crate::freshness::unify_freshness(root).await {
                Ok(()) => {
                    lines.push("  [freshness] UNIFIED freshness.toml (compat)".to_string());
                }
                Err(e) => lines.push(format!("  [freshness] unify FAIL: {e}")),
            }
        }
    }

    if opts.mode == CascadeMode::Sync {
        let heads = collect_cascade_heads(root, repos).await;
        if !heads.is_empty() {
            run_rootpulse_sovereignty(m.meta.wave, opts.gate, &heads, lines).await;
        }

        run_commit_drift_pipeline(lines).await;
        run_depot_staleness_and_fetch(do_harvest, opts.restart_updated, lines).await;
        run_content_rebuild_if_needed(root, lines).await;
        check_content_health(root, lines).await;
    }

    harvest_info
}

#[cfg(test)]
#[path = "post_sync_tests.rs"]
mod tests;
