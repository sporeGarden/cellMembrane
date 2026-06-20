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

/// Run git cascade for an ecosystem repo push (non-primal).
///
/// Forgejo push: sync the repo locally (`temporal.sync`) so this gate stays current.
/// GitHub push: relay inward — pull from GitHub mirror, push to Forgejo.
pub(super) async fn run_cascade_pipeline(
    action: &WebhookAction,
    _config: &crate::ShadowConfig,
) -> crate::error::Result<crate::ShadowOutcome> {
    let root = crate::temporal::resolve_workspace_root()?;
    let manifest = crate::manifest::load_from_workspace_async(&root).await?;
    let push_target = manifest.sync.push_target;

    let repo_path = manifest
        .repos
        .iter()
        .find(|(_, entry)| {
            entry
                .local_path
                .to_lowercase()
                .contains(&action.repo_name.to_lowercase())
        })
        .map(|(_, entry)| entry.local_path.clone());

    let path = repo_path.unwrap_or_else(|| {
        format!(
            "{}/{}",
            cellmembrane_types::service::INFRA_WATERING_HOLE,
            action.repo_name
        )
    });

    match action.provider {
        super::WebhookProvider::Forgejo => {
            info!(
                repo = %action.repo_name,
                path = %path,
                "webhook cascade: syncing from Forgejo push"
            );
            let result = crate::temporal::sync_with_target(&root, &path, push_target).await?;
            Ok(crate::ShadowOutcome::ok(format!(
                "webhook: {} cascade sync — {}",
                action.repo_name,
                if result.ok { "parity" } else { &result.summary }
            )))
        }
        super::WebhookProvider::GitHub => {
            info!(
                repo = %action.repo_name,
                path = %path,
                "webhook cascade: relaying from GitHub push"
            );
            let relay_config = crate::relay::RelayConfig::from_env();
            let (pulled, failures) = crate::relay::mediate(&relay_config, &[path.as_str()]).await;
            let ok = failures.is_empty();
            Ok(crate::ShadowOutcome {
                ok,
                message: format!(
                    "webhook: {} GitHub relay — pulled={} failed={}",
                    action.repo_name,
                    pulled.len(),
                    failures.len()
                ),
                data: Some(serde_json::json!({
                    "pulled": pulled,
                    "failures": failures,
                })),
            })
        }
    }
}

pub(super) async fn run_sandbox(
    primal_lower: &str,
    event: &PushEvent,
    repo_name: &str,
) -> crate::error::Result<Option<crate::ShadowOutcome>> {
    let arch = crate::plasmid::detect_target_triple();
    let depot_binary = crate::plasmid::resolve_path(
        None,
        cellmembrane_types::service::ENV_PLASMIDBIN_DEPOT,
        || {
            crate::resolve_xdg_data_home()
                .join("ecoPrimals")
                .join(cellmembrane_types::service::PLASMID_BIN_DIR)
        },
    )
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
