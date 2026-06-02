// SPDX-License-Identifier: AGPL-3.0-or-later

//! Command dispatch — routes `domain.operation` strings to typed handlers.
//!
//! Each domain group returns `Result<ShadowOutcome>` — the caller (main.rs)
//! handles JSON vs human output formatting.

mod data;
mod impulse;
mod infra;
mod temporal;

use crate::{ShadowConfig, ShadowOutcome, forgejo};

/// Dispatch a CLI command to the appropriate shadow function.
///
/// Returns `Ok(ShadowOutcome)` for both success and domain-level failures.
/// Returns `Err` only for infrastructure failures (SSH, parse, etc.).
pub async fn run(config: &ShadowConfig, cmd: &str, args: &[&str]) -> crate::Result<ShadowOutcome> {
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
