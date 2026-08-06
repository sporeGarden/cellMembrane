// SPDX-License-Identifier: AGPL-3.0-or-later

//! Harvest scheduler dispatch — `harvest.*` commands for CI-EVO-01
//! webhook-to-batch build pipeline.

use crate::ShadowOutcome;
use crate::cli;
use crate::error::Result;

pub(super) async fn dispatch_harvest(cmd: &str, args: &[&str]) -> Result<ShadowOutcome> {
    use crate::plasmid::scheduler;

    match cmd {
        "harvest.ingest" => {
            let primal = cli::require_arg(args, 0, "primal_name")?;
            let commit = cli::extract_flag_value(args, "--commit").unwrap_or("HEAD");
            let pusher = cli::extract_flag_value(args, "--pusher").unwrap_or("operator");
            let queue = scheduler::ingest(primal, commit, pusher)?;
            Ok(ShadowOutcome::ok(format!(
                "harvest.ingest: {} queued ({})",
                primal,
                scheduler::format_queue(&queue)
            )))
        }
        "harvest.request" => {
            let primal = cli::require_arg(args, 0, "primal_name")?;
            let queue = scheduler::request_build(primal)?;
            Ok(ShadowOutcome::ok(format!(
                "harvest.request: {} → BUILD_REQUESTED\n{}",
                primal,
                scheduler::format_queue(&queue)
            )))
        }
        "harvest.queue" => {
            let queue = scheduler::load_queue();
            Ok(ShadowOutcome::ok(scheduler::format_queue(&queue)))
        }
        "harvest.schedule" => dispatch_schedule(args).await,
        "harvest.clear" => {
            let queue = scheduler::HarvestQueue::default();
            scheduler::save_queue(&queue)?;
            Ok(ShadowOutcome::ok("harvest queue cleared"))
        }
        _ => Ok(ShadowOutcome::fail(format!(
            "unknown harvest command: {cmd}"
        ))),
    }
}

async fn dispatch_schedule(args: &[&str]) -> Result<ShadowOutcome> {
    use crate::plasmid::scheduler;

    let dry_run = args.contains(&"--dry-run");
    let mut queue = scheduler::load_queue();
    let decision = scheduler::evaluate(&mut queue);

    if decision.build_now.is_empty() {
        return Ok(ShadowOutcome::ok_with(
            format!("harvest.schedule: nothing to build — {}", decision.reason),
            serde_json::json!({
                "waiting": decision.waiting,
                "auto_promoted": decision.auto_promoted,
            }),
        ));
    }

    if dry_run {
        return Ok(ShadowOutcome::ok_with(
            format!(
                "harvest.schedule (dry-run): would build [{}] — {}",
                decision.build_now.join(", "),
                decision.reason
            ),
            serde_json::json!({
                "build_now": decision.build_now,
                "waiting": decision.waiting,
                "auto_promoted": decision.auto_promoted,
            }),
        ));
    }

    scheduler::mark_building(&mut queue, &decision.build_now);
    scheduler::save_queue(&queue)?;

    let build_list = decision.build_now.join(", ");
    let harvest_args = crate::plasmid::HarvestArgs {
        primal: None,
        force: false,
        dry_run: false,
        depot_dir: None,
        target: None,
        local: true,
        push: false,
        with_restart: false,
    };

    let result = crate::plasmid::harvest(&harvest_args).await;

    let mut queue = scheduler::load_queue();
    match &result {
        Ok(outcome) if outcome.ok => {
            scheduler::mark_complete(&mut queue, &decision.build_now);
            tracing::info!(built = %build_list, "harvest.schedule: batch build complete");
        }
        _ => {
            scheduler::mark_failed(&mut queue, &decision.build_now);
            tracing::warn!(primals = %build_list, "harvest.schedule: batch build failed, re-queued");
        }
    }
    scheduler::save_queue(&queue)?;

    result
}
