// SPDX-License-Identifier: AGPL-3.0-or-later

//! Webhook harvest pipeline — selective build, sandbox, and refresh for push events.

use super::{PushEvent, WebhookAction};
use tracing::{error, info, warn};

pub(super) async fn run_harvest_pipeline(
    action: &WebhookAction,
    event: &PushEvent,
    config: &crate::ShadowConfig,
) -> crate::error::Result<crate::ShadowOutcome> {
    let harvest_args = crate::plasmid::HarvestArgs {
        primal: Some(action.repo_name.to_lowercase()),
        force: false,
        dry_run: false,
        depot_dir: None,
        target: None,
    };

    let harvest_outcome = crate::plasmid::harvest(&harvest_args).await?;

    if !harvest_outcome.ok {
        return Ok(crate::ShadowOutcome {
            ok: false,
            message: format!(
                "webhook: {} harvest failed — {}",
                action.repo_name, harvest_outcome.message
            ),
            data: harvest_outcome.data,
        });
    }

    let primal_lower = action.repo_name.to_lowercase();
    if let Some(fail) = run_sandbox(&primal_lower, event, &action.repo_name).await? {
        return Ok(fail);
    }

    let refresh_args = crate::plasmid::RefreshArgs {
        primal: Some(primal_lower),
        dry_run: false,
        source_dir: None,
    };

    let refresh_outcome = crate::plasmid::refresh(config, &refresh_args).await?;

    Ok(crate::ShadowOutcome {
        ok: refresh_outcome.ok,
        message: format!(
            "webhook: {} -> harvest: {} | sandbox: PASS | refresh: {}",
            action.repo_name, harvest_outcome.message, refresh_outcome.message
        ),
        data: refresh_outcome.data,
    })
}

pub(super) async fn run_sandbox(
    primal_lower: &str,
    event: &PushEvent,
    repo_name: &str,
) -> crate::error::Result<Option<crate::ShadowOutcome>> {
    let arch = crate::plasmid::detect_target_triple();
    let depot_binary = crate::plasmid::resolve_path(None, cellmembrane_types::service::ENV_PLASMIDBIN_DEPOT, || {
        crate::resolve_xdg_data_home()
            .join("ecoPrimals")
            .join("plasmidBin")
    })
    .join("primals")
    .join(&arch)
    .join(primal_lower);

    if !depot_binary.exists() {
        return Ok(None);
    }

    let commit_short = if event.after.len() >= 8 {
        &event.after[..8]
    } else {
        &event.after
    };

    let sandbox_args = crate::plasmid::sandbox::SandboxArgs {
        primal: primal_lower.to_string(),
        commit: commit_short.to_string(),
        binary_path: depot_binary,
        timeout_secs: None,
    };

    match crate::plasmid::sandbox::validate(&sandbox_args).await {
        Ok(result) if !result.health_ok => {
            error!(
                primal = %primal_lower,
                detail = %result.detail,
                "sandbox validation failed"
            );
            Ok(Some(crate::ShadowOutcome {
                ok: false,
                message: format!(
                    "webhook: {repo_name} sandbox validation FAILED — {} ({}ms). Production unchanged.",
                    result.detail, result.elapsed_ms
                ),
                data: Some(serde_json::to_value(&result).unwrap_or_default()),
            }))
        }
        Ok(result) => {
            info!(
                primal = %primal_lower,
                detail = %result.detail,
                elapsed_ms = result.elapsed_ms,
                "sandbox validation passed"
            );
            Ok(None)
        }
        Err(e) => {
            warn!(primal = %primal_lower, error = %e, "sandbox infra error — proceeding");
            Ok(None)
        }
    }
}
