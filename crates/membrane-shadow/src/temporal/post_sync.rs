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
    check_content_health, run_commit_drift_pipeline, run_content_rebuild_if_needed,
    run_rootpulse_sovereignty,
};
use super::post_sync_harvest::{run_depot_staleness_and_fetch, run_post_cascade_refresh};

/// Post-sync phases: harvest (if requested), rebuild (harvest+refresh), freshness, depot report.
///
/// Returns `(harvest_info_string, all_ok)` — `all_ok` is false if harvest
/// or refresh reported failures (DIV-7 fix).
pub(super) async fn run_post_sync_phases(
    opts: &CascadeOpts<'_>,
    root: &std::path::Path,
    m: &crate::manifest::EcosystemManifest,
    repos: &[(&str, &crate::manifest::RepoEntry)],
    lines: &mut Vec<String>,
) -> (String, bool) {
    let mut harvest_info = String::new();
    let mut all_ok = true;
    let do_harvest = opts.post_sync != PostSyncPhase::None && opts.mode == CascadeMode::Sync;

    if do_harvest && should_delegate_build(opts.gate, m) {
        delegate_harvest_to_primary(m, lines).await;
    } else if do_harvest {
        match super::post_sync_harvest::run_post_cascade_harvest(lines).await {
            Ok((built, built_primals, current, failures)) => {
                harvest_info = format!(" harvest={built}built/{current}current/{failures}failed");
                if failures > 0 {
                    all_ok = false;
                }

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

                                if opts.depot_push && pushed > 0 {
                                    run_depot_push(lines).await;
                                }
                            }
                            Err(e) => {
                                lines.push(format!("  [refresh] FAIL: {e}"));
                                all_ok = false;
                            }
                        }
                    }
                }
            }
            Err(e) => {
                lines.push(format!("  [harvest] FAIL: {e}"));
                all_ok = false;
            }
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

    (harvest_info, all_ok)
}

/// Check whether this gate should delegate builds to the primary build authority.
///
/// Returns `true` if the manifest names a primary builder AND this gate is not it.
fn should_delegate_build(gate: &str, m: &crate::manifest::EcosystemManifest) -> bool {
    let authorities = m.build_authorities();
    match authorities.first() {
        Some(primary) if primary != gate => {
            tracing::info!(
                gate,
                primary = primary.as_str(),
                "this gate is not the primary builder — will delegate"
            );
            true
        }
        _ => false,
    }
}

/// Delegate a harvest request to the primary build authority via songBird mesh relay.
async fn delegate_harvest_to_primary(
    m: &crate::manifest::EcosystemManifest,
    lines: &mut Vec<String>,
) {
    let Some(primary) = m.build_authorities().into_iter().next() else {
        lines.push("  [delegate] no primary builder configured — skipping".into());
        return;
    };

    let endpoint = cellmembrane_types::TransportEndpoint::MeshRelay {
        peer_id: primary.clone(),
        capability: "build".into(),
    };

    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "plasmid.harvest",
        "params": {
            "local": true,
            "push": true,
        },
        "id": 1,
    })
    .to_string();

    lines.push(format!(
        "  [delegate] dispatching harvest to primary builder {primary} via mesh"
    ));

    match crate::jsonrpc::call_endpoint(&endpoint, &request).await {
        Ok(response) => {
            let parsed: serde_json::Value = serde_json::from_str(&response).unwrap_or_default();
            let ok = parsed
                .get("result")
                .and_then(|r| r.get("ok"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let msg = parsed
                .get("result")
                .and_then(|r| r.get("message"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("done");

            if ok {
                lines.push(format!("  [delegate] {primary} harvest OK — {msg}"));
            } else {
                lines.push(format!("  [delegate] {primary} harvest FAILED — {msg}"));
            }
        }
        Err(e) => {
            lines.push(format!(
                "  [delegate] mesh dispatch to {primary} failed — {e} — falling back to local"
            ));
            if let Ok((_built, _primals, _current, _failures)) =
                super::post_sync_harvest::run_post_cascade_harvest(lines).await
            {
                lines.push("  [delegate] local fallback harvest completed".into());
            }
        }
    }
}

/// Push local depot to golgi via SCP after successful harvest+refresh.
async fn run_depot_push(lines: &mut Vec<String>) {
    let Ok(depot_dir) = crate::plasmid::depot::resolve_depot(None) else {
        lines.push("  [depot-push] SKIP — depot not resolved".into());
        return;
    };
    match crate::plasmid::depot_sync_push_standalone(&depot_dir).await {
        Ok(outcome) if outcome.ok => {
            lines.push(format!("  [depot-push] OK — {}", outcome.message));
        }
        Ok(outcome) => {
            lines.push(format!("  [depot-push] PARTIAL — {}", outcome.message));
        }
        Err(e) => {
            lines.push(format!("  [depot-push] FAIL — {e}"));
        }
    }
}

#[cfg(test)]
#[path = "post_sync_tests.rs"]
mod tests;
