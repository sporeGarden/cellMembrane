// SPDX-License-Identifier: AGPL-3.0-or-later

//! Command dispatch — routes `domain.operation` strings to typed handlers.
//!
//! Each domain group returns `Result<ShadowOutcome>` — the caller (main.rs)
//! handles JSON vs human output formatting.
//!
//! ## Graduated Composition
//!
//! For commands that map to primal capability domains, dispatch attempts
//! the Neural Bridge first (try-primal-first). If biomeOS routes the
//! capability to a running primal, that result is used. If the bridge
//! is unavailable or the method isn't routed, the shadow implementation
//! handles the request. This enables smooth graduation: as primals come
//! online, membrane-shadow automatically delegates without code changes.

mod builder;
mod content_dispatch;
mod data;
mod deploy_dispatch;
mod dispatch_harvest;
mod dispatch_validate;
mod dispatch_webhook;
mod gate;
mod gate_configure;
mod gate_keys;
mod gate_network;
mod impulse;
mod infra;
mod plasmid_dispatch;
mod provision_dispatch;
mod relay_dispatch;
mod sign_dispatch;
mod sovereign;
mod temporal;

use crate::error::ShadowError;
use crate::{ShadowConfig, ShadowOutcome, bridge, forgejo};

/// Map a CLI command to its primal capability domain + method for bridge routing.
///
/// Returns `None` for commands that are shadow-only (no primal equivalent)
/// or local-only (no SSH/IPC needed).
fn bridge_mapping(cmd: &str) -> Option<(&str, &str)> {
    match cmd {
        "gate.info" => Some(("gate", "gate.info")),
        "gate.pull" => Some(("gate", "gate.pull")),
        "gate.check" => Some(("gate", "gate.check")),
        "service.list" => Some(("gate", "gate.service.list")),
        "service.status" => Some(("gate", "gate.service.status")),
        "service.restart" => Some(("gate", "gate.service.restart")),
        "service.logs" => Some(("gate", "gate.service.logs")),
        "service.template" => Some(("gate", "gate.service.template")),
        "repo.list" => Some(("content", "content.repo.list")),
        "repo.create" => Some(("content", "content.repo.create")),
        "mirror.sync-all" => Some(("content", "content.mirror.sync_all")),
        "token.list" => Some(("auth", "auth.token.list")),
        "token.create" => Some(("auth", "auth.token.create")),
        "token.revoke" => Some(("auth", "auth.token.revoke")),
        "deploy.composition" => Some(("composition", "deploy")),
        "deploy.graph" => Some(("graph", "execute")),
        "deploy.resurrect" => Some(("lifecycle", "resurrect")),
        "lifecycle.status" => Some(("lifecycle", "status")),
        _ => None,
    }
}

/// Dispatch a CLI command to the appropriate shadow function.
///
/// Attempts Neural Bridge (primal delegation) first for supported commands,
/// falling through to shadow implementation if unavailable.
///
/// Returns `Ok(ShadowOutcome)` for both success and domain-level failures.
/// Returns `Err` only for infrastructure failures (SSH, parse, etc.).
pub async fn run(config: &ShadowConfig, cmd: &str, args: &[&str]) -> crate::Result<ShadowOutcome> {
    if let Some((domain, method)) = bridge_mapping(cmd) {
        let params = serde_json::json!({ "args": args });
        match bridge::try_bridge(domain, method, params).await {
            Ok(Some(result)) => return Ok(ShadowOutcome::ok(result.to_string())),
            Err(e) => return Err(e),
            Ok(None) => {}
        }
    }

    match cmd {
        c if c.starts_with("repo.") => infra::dispatch_repo(config, cmd, args).await,
        c if c.starts_with("mirror.") => infra::dispatch_mirror(config, cmd, args).await,
        c if c.starts_with("service.") => infra::dispatch_service(config, cmd, args).await,
        c if c.starts_with("gate.")
            || c == "health.audit"
            || c.starts_with("firewall.")
            || c.starts_with("wireguard.") =>
        {
            gate::dispatch(config, cmd, args).await
        }
        c if c.starts_with("token.") => infra::dispatch_token(config, cmd, args).await,
        c if c.starts_with("temporal.") => temporal::dispatch_temporal(config, cmd, args).await,
        c if c.starts_with("manifest.") => data::dispatch_manifest(cmd, args).await,
        c if c.starts_with("topology.") => data::dispatch_topology(cmd, args).await,
        "identity.resolve" => data::dispatch_identity().await,
        c if c.starts_with("impulse.") => impulse::dispatch_impulse(cmd, args).await,
        c if c.starts_with("potential.") => {
            let cmd = cmd.to_owned();
            let args: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
            tokio::task::spawn_blocking(move || {
                let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                impulse::dispatch_potential(&cmd, &refs)
            })
            .await
            .unwrap_or_else(|ref e| Err(spawn_blocking_err(e)))
        }
        c if c.starts_with("context.") => data::dispatch_context(cmd, args).await,
        "depot.integrity" => {
            let args: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
            tokio::task::spawn_blocking(move || {
                let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                plasmid_dispatch::dispatch_depot_integrity(&refs)
            })
            .await
            .unwrap_or_else(|ref e| Err(spawn_blocking_err(e)))
        }
        c if c.starts_with("sign.") => {
            let cmd = cmd.to_owned();
            let args: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
            tokio::task::spawn_blocking(move || {
                let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                sign_dispatch::dispatch_sign(&cmd, &refs)
            })
            .await
            .unwrap_or_else(|ref e| Err(spawn_blocking_err(e)))
        }
        c if c.starts_with("plasmid.") => {
            plasmid_dispatch::dispatch_plasmid(config, cmd, args).await
        }
        c if c.starts_with("deploy.") => deploy_dispatch::dispatch_deploy(cmd, args).await,
        c if c.starts_with("lifecycle.") => deploy_dispatch::dispatch_lifecycle(cmd, args).await,
        c if c.starts_with("relay.") => relay_dispatch::dispatch_relay(cmd, args).await,
        "mesh.register" => Ok(crate::plasmid::register_capabilities_with_mesh().await),
        c if c.starts_with("freshness.") => dispatch_freshness(cmd, args).await,
        c if c.starts_with("content.") => {
            content_dispatch::dispatch_content(config, cmd, args).await
        }
        "forgejo.version" => {
            let v = forgejo::version(config).await?;
            Ok(ShadowOutcome::ok(v))
        }
        "builder.serve" => builder::serve(args).await,
        c if c.starts_with("sovereign.") => sovereign::dispatch_sovereign(config, cmd, args).await,
        c if c.starts_with("rootpulse.") => dispatch_validate::dispatch_rootpulse(cmd, args).await,
        c if c.starts_with("caddy.") => crate::caddy::dispatch(config, cmd, args).await,
        c if c.starts_with("dns.") => crate::dns::dispatch(config, cmd, args).await,
        c if c.starts_with("tower.") => crate::tower::dispatch(config, cmd, args).await,
        c if c.starts_with("gateway.") => crate::gateway::dispatch(config, cmd, args).await,
        c if c.starts_with("harvest.") => dispatch_harvest::dispatch_harvest(cmd, args).await,
        c if c.starts_with("webhook.") => {
            dispatch_webhook::dispatch_webhook(config, cmd, args).await
        }
        "pepti.validate" => {
            tracing::warn!("pepti.* namespace deprecated (Wave 120) — use gate.validate");
            dispatch_validate::gate_validate(
                config,
                args,
                Some(cellmembrane_types::MembraneComposition::Relay),
            )
            .await
        }
        #[cfg(feature = "cloudflare")]
        c if c.starts_with("cloudflare.") => crate::cloudflare::dispatch(cmd, args).await,
        _ => Ok(ShadowOutcome::fail(format!("unknown command: {cmd}"))),
    }
}

async fn dispatch_freshness(cmd: &str, _args: &[&str]) -> crate::Result<ShadowOutcome> {
    match cmd {
        "freshness.check" => {
            let report = tokio::task::spawn_blocking(crate::freshness::check_installed_freshness)
                .await
                .map_err(|e| {
                    ShadowError::Io(std::io::Error::other(format!(
                        "spawn_blocking panicked: {e}"
                    )))
                })??;
            Ok(ShadowOutcome::ok(report))
        }
        "freshness.publish" => {
            let root = crate::temporal::resolve_workspace_root()?;
            let manifest = crate::manifest::load_from_workspace_async(&root).await?;
            let repos: Vec<(&str, &crate::manifest::RepoEntry)> = manifest
                .repos
                .iter()
                .map(|(k, v)| (k.as_str(), v))
                .collect();
            crate::freshness::publish_gate_heads(&root, &repos).await?;
            Ok(ShadowOutcome::ok(
                "gate heads published to heads/<gate>.toml".to_string(),
            ))
        }
        _ => Ok(ShadowOutcome::fail(format!(
            "unknown freshness command: {cmd}"
        ))),
    }
}

fn spawn_blocking_err(e: &tokio::task::JoinError) -> ShadowError {
    ShadowError::Io(std::io::Error::other(format!(
        "spawn_blocking panicked: {e}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_mapping_known_commands() {
        assert!(bridge_mapping("gate.info").is_some());
        assert!(bridge_mapping("gate.pull").is_some());
        assert!(bridge_mapping("service.list").is_some());
        assert!(bridge_mapping("repo.list").is_some());
        assert!(bridge_mapping("token.create").is_some());
    }

    #[test]
    fn bridge_mapping_unknown_returns_none() {
        assert!(bridge_mapping("rootpulse.commit").is_none());
        assert!(bridge_mapping("temporal.cascade").is_none());
        assert!(bridge_mapping("depot.integrity").is_none());
        assert!(bridge_mapping("unknown.command").is_none());
    }

    #[test]
    fn bridge_mapping_returns_correct_domain() {
        let (domain, method) = bridge_mapping("gate.info").unwrap();
        assert_eq!(domain, "gate");
        assert_eq!(method, "gate.info");

        let (domain, method) = bridge_mapping("repo.list").unwrap();
        assert_eq!(domain, "content");
        assert_eq!(method, "content.repo.list");
    }

    #[test]
    fn webhook_provider_parse() {
        let provider = dispatch_webhook::parse_webhook_provider(&["--provider", "github"]).unwrap();
        assert_eq!(provider, crate::webhook::WebhookProvider::GitHub);

        let provider =
            dispatch_webhook::parse_webhook_provider(&["--provider", "forgejo"]).unwrap();
        assert_eq!(provider, crate::webhook::WebhookProvider::Forgejo);
    }

    #[test]
    fn webhook_provider_default_is_forgejo() {
        let provider = dispatch_webhook::parse_webhook_provider(&[]).unwrap();
        assert_eq!(provider, crate::webhook::WebhookProvider::Forgejo);
    }

    #[test]
    fn webhook_provider_rejects_unknown() {
        let err = dispatch_webhook::parse_webhook_provider(&["--provider", "gitlab"]).unwrap_err();
        assert!(err.to_string().contains("unknown webhook provider"));
    }

    #[tokio::test]
    async fn unknown_command_returns_fail() {
        let config = ShadowConfig::default();
        let result = run(&config, "nonexistent.command", &[]).await.unwrap();
        assert!(!result.ok);
        assert!(result.message.contains("unknown command"));
    }

    #[tokio::test]
    async fn rootpulse_status_returns_ok() {
        let result = dispatch_validate::dispatch_rootpulse("rootpulse.status", &[])
            .await
            .unwrap();
        assert!(result.ok);
    }

    #[tokio::test]
    async fn rootpulse_unknown_subcommand() {
        let result = dispatch_validate::dispatch_rootpulse("rootpulse.invalid", &[])
            .await
            .unwrap();
        assert!(!result.ok);
        assert!(result.message.contains("unknown rootpulse command"));
    }
}
