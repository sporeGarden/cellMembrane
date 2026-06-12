// SPDX-License-Identifier: AGPL-3.0-or-later

//! Temporal domain dispatch — temporal.check, temporal.sync, temporal.cascade.

use crate::cli::{self, TapMessage};
use crate::{ShadowConfig, ShadowOutcome, identity, manifest, temporal};

pub(super) async fn dispatch_temporal(
    config: &ShadowConfig,
    cmd: &str,
    args: &[&str],
) -> crate::Result<ShadowOutcome> {
    match cmd {
        "temporal.check" => {
            let root = temporal::resolve_workspace_root()?;
            if args.is_empty() {
                return Err(crate::ShadowError::Parse(
                    "temporal.check requires at least one repo path".into(),
                ));
            }
            let mut matrices = Vec::with_capacity(args.len());
            for path in args {
                matrices.push(temporal::check(&root, path).await?);
            }
            let lines: Vec<String> = matrices.iter().map(ToString::to_string).collect();
            let parity = matrices
                .iter()
                .filter(|m| m.classification == temporal::SyncClassification::Parity)
                .count();
            Ok(ShadowOutcome::ok_with(
                format!("{}/{} parity\n{}", parity, matrices.len(), lines.join("\n")),
                serde_json::to_value(&matrices)?,
            ))
        }
        "temporal.sync" => {
            let root = temporal::resolve_workspace_root()?;
            if args.is_empty() {
                return Err(crate::ShadowError::Parse(
                    "temporal.sync requires at least one repo path".into(),
                ));
            }
            let push_target = manifest::load_from_workspace(&root)
                .map_or_else(|_| "all".into(), |m| m.sync.push_target);
            let mut results = Vec::with_capacity(args.len());
            let mut synced = 0u32;
            let mut failed = 0u32;
            for path in args {
                let r = temporal::sync_with_target(&root, path, &push_target).await?;
                if r.ok {
                    synced += 1;
                } else {
                    failed += 1;
                }
                results.push(r);
            }
            let lines: Vec<String> = results
                .iter()
                .map(|r| {
                    let status = if r.ok { "OK" } else { "FAIL" };
                    format!("  {:<35} {status} {}", r.repo_path, r.summary)
                })
                .collect();
            Ok(ShadowOutcome::ok_with(
                format!("synced={synced} failed={failed}\n{}", lines.join("\n")),
                serde_json::to_value(&results)?,
            ))
        }
        "temporal.cascade" => dispatch_cascade(config, args).await,
        _ => Ok(ShadowOutcome::fail(format!(
            "unknown temporal command: {cmd}"
        ))),
    }
}

/// `temporal.cascade` — thin dispatch to `temporal::cascade`.
async fn dispatch_cascade(_config: &ShadowConfig, args: &[&str]) -> crate::Result<ShadowOutcome> {
    let root = temporal::resolve_workspace_root()?;

    let gate_name = cli::extract_flag_value(args, "--gate")
        .or_else(|| {
            std::env::var(cellmembrane_types::service::ENV_GATE_NAME)
                .ok()
                .as_deref()
                .map(|_| "")
        })
        .unwrap_or("auto");

    let gate_name = if gate_name == "auto" || gate_name.is_empty() {
        identity::resolve(&root)
            .map_err(|_| {
                crate::ShadowError::Parse(
                    "cannot resolve gate identity — set GATE_NAME or configure identity".into(),
                )
            })?
            .name
    } else {
        gate_name.to_string()
    };

    let source = cli::extract_flag_value(args, "--source").unwrap_or("temporal");
    let check_only = args.contains(&"--check");
    let clone_missing = args.contains(&"--clone-missing");
    let dry_run = args.contains(&"--dry-run");
    let no_freshness = args.contains(&"--no-freshness");
    let check_installed = args.contains(&"--check-installed");
    let post_sync = if args.contains(&"--with-rebuild") {
        if args.contains(&"--skip-sandbox") {
            temporal::PostSyncPhase::Rebuild
        } else {
            temporal::PostSyncPhase::SandboxRebuild
        }
    } else if args.contains(&"--with-harvest") {
        temporal::PostSyncPhase::Harvest
    } else {
        temporal::PostSyncPhase::None
    };

    let mode = if dry_run {
        temporal::CascadeMode::DryRun
    } else if check_only {
        temporal::CascadeMode::CheckOnly
    } else {
        temporal::CascadeMode::Sync
    };

    let publish_freshness = !no_freshness && mode == temporal::CascadeMode::Sync;

    let restart_updated = args.contains(&"--with-restart");

    let mut outcome = temporal::cascade_with_opts(&temporal::CascadeOpts {
        gate: &gate_name,
        source,
        mode,
        clone_missing,
        publish_freshness,
        post_sync,
        restart_updated,
    })
    .await?;

    if check_installed {
        let freshness_report = temporal::check_installed_freshness()?;
        outcome = outcome.tap_message(|m| format!("{m}\n\n{freshness_report}"));
    }

    Ok(outcome)
}
