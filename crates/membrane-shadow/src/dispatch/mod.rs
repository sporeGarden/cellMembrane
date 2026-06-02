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

mod data;
mod impulse;
mod infra;
mod temporal;

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
        "repo.list" => Some(("content", "content.repo.list")),
        "repo.create" => Some(("content", "content.repo.create")),
        "mirror.sync-all" => Some(("content", "content.mirror.sync_all")),
        "token.list" => Some(("auth", "auth.token.list")),
        "token.create" => Some(("auth", "auth.token.create")),
        "token.revoke" => Some(("auth", "auth.token.revoke")),
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
        if let Some(result) = bridge::try_bridge(domain, method, params).await {
            return Ok(ShadowOutcome::ok(result.to_string()));
        }
    }

    match cmd {
        c if c.starts_with("repo.") => infra::dispatch_repo(config, cmd, args).await,
        c if c.starts_with("mirror.") => infra::dispatch_mirror(config, cmd, args).await,
        c if c.starts_with("service.") => infra::dispatch_service(config, cmd, args).await,
        c if c.starts_with("gate.") => infra::dispatch_gate(config, cmd, args).await,
        c if c.starts_with("token.") => infra::dispatch_token(config, cmd, args).await,
        c if c.starts_with("temporal.") => temporal::dispatch_temporal(config, cmd, args).await,
        c if c.starts_with("manifest.") => data::dispatch_manifest(cmd, args),
        "identity.resolve" => data::dispatch_identity(),
        c if c.starts_with("impulse.") => impulse::dispatch_impulse(cmd, args).await,
        c if c.starts_with("potential.") => impulse::dispatch_potential(cmd, args),
        c if c.starts_with("context.") => data::dispatch_context(cmd, args).await,
        c if c.starts_with("plasmid.") => data::dispatch_plasmid(config, cmd, args).await,
        c if c.starts_with("relay.") => data::dispatch_relay(cmd, args).await,
        "forgejo.version" => {
            let v = forgejo::version(config).await?;
            Ok(ShadowOutcome::ok(v))
        }
        _ => Ok(ShadowOutcome::fail(format!("unknown command: {cmd}"))),
    }
}
