// SPDX-License-Identifier: AGPL-3.0-or-later

//! Webhook dispatch — `webhook.*` commands for push event handling
//! and signature verification.

use crate::cli;
use crate::error::{Result, ShadowError};
use crate::{ShadowConfig, ShadowOutcome};

pub(super) fn parse_webhook_provider(args: &[&str]) -> Result<crate::webhook::WebhookProvider> {
    let Some(p) = cli::extract_flag_value(args, "--provider") else {
        return Ok(crate::webhook::WebhookProvider::Forgejo);
    };
    p.parse::<crate::webhook::WebhookProvider>()
        .map_err(ShadowError::Config)
}

pub(super) async fn dispatch_webhook(
    config: &ShadowConfig,
    cmd: &str,
    args: &[&str],
) -> Result<ShadowOutcome> {
    match cmd {
        "webhook.test" => {
            let body = cli::require_arg(args, 0, "json_body")?;
            let event: crate::webhook::PushEvent =
                serde_json::from_str(body).map_err(ShadowError::Json)?;
            let provider = parse_webhook_provider(args)?;
            crate::webhook::handle_push(&event, config, provider).await
        }
        "webhook.listen" => {
            let socket = cli::extract_flag_value(args, "--socket");
            crate::webhook::listener::listen(config, socket).await?;
            Ok(ShadowOutcome::ok("webhook listener stopped"))
        }
        "webhook.verify" => {
            let secret = cellmembrane_types::service::resolve_webhook_secret_env()
                .ok_or_else(|| ShadowError::config("MEMBRANE_WEBHOOK_SECRET env var required"))?;
            let body = cli::require_arg(args, 0, "body")?;
            let sig = cli::extract_flag_value(args, "--signature")
                .ok_or_else(|| ShadowError::config("--signature flag required"))?;
            let provider = parse_webhook_provider(args)?;
            crate::webhook::verify_provider_signature(
                provider,
                secret.as_bytes(),
                body.as_bytes(),
                sig,
            )?;
            Ok(ShadowOutcome::ok("signature valid"))
        }
        _ => Ok(ShadowOutcome::fail(format!(
            "unknown webhook command: {cmd}"
        ))),
    }
}
