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
mod gate;
mod impulse;
mod infra;
mod temporal;

use crate::cli;
use crate::error::{Result, ShadowError};
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
        c if c.starts_with("gate.") || c == "health.audit" || c.starts_with("firewall.") => {
            gate::dispatch(config, cmd, args).await
        }
        c if c.starts_with("token.") => infra::dispatch_token(config, cmd, args).await,
        c if c.starts_with("temporal.") => temporal::dispatch_temporal(config, cmd, args).await,
        c if c.starts_with("manifest.") => data::dispatch_manifest(cmd, args).await,
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
            .unwrap_or_else(|e| Err(ShadowError::Config(format!("spawn_blocking: {e}"))))
        }
        c if c.starts_with("context.") => data::dispatch_context(cmd, args).await,
        "depot.integrity" => {
            let args: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
            tokio::task::spawn_blocking(move || {
                let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                data::dispatch_depot_integrity(&refs)
            })
            .await
            .unwrap_or_else(|e| Err(ShadowError::Config(format!("spawn_blocking: {e}"))))
        }
        c if c.starts_with("plasmid.") => data::dispatch_plasmid(config, cmd, args).await,
        c if c.starts_with("relay.") => data::dispatch_relay(cmd, args).await,
        c if c.starts_with("content.") => data::dispatch_content(config, cmd, args).await,
        "forgejo.version" => {
            let v = forgejo::version(config).await?;
            Ok(ShadowOutcome::ok(v))
        }
        c if c.starts_with("caddy.") => crate::caddy::dispatch(config, cmd, args).await,
        c if c.starts_with("webhook.") => dispatch_webhook(config, cmd, args).await,
        c if c.starts_with("pepti.") => dispatch_pepti(config, cmd, args).await,
        #[cfg(feature = "cloudflare")]
        c if c.starts_with("cloudflare.") => crate::cloudflare::dispatch(cmd, args).await,
        _ => Ok(ShadowOutcome::fail(format!("unknown command: {cmd}"))),
    }
}

/// Dispatch webhook commands.
async fn dispatch_webhook(
    config: &ShadowConfig,
    cmd: &str,
    args: &[&str],
) -> Result<ShadowOutcome> {
    match cmd {
        "webhook.test" => {
            let body = cli::require_arg(args, 0, "json_body")?;
            let event: crate::webhook::PushEvent = serde_json::from_str(body)
                .map_err(|e| ShadowError::Parse(format!("invalid push event JSON: {e}")))?;
            crate::webhook::handle_push(&event, config).await
        }
        "webhook.verify" => {
            let secret = std::env::var(cellmembrane_types::service::ENV_WEBHOOK_SECRET)
                .map_err(|_| ShadowError::Parse("WEBHOOK_SECRET env var required".into()))?;
            let body = cli::require_arg(args, 0, "body")?;
            let sig = cli::extract_flag_value(args, "--signature")
                .ok_or_else(|| ShadowError::Parse("--signature flag required".into()))?;
            crate::webhook::verify_signature(secret.as_bytes(), body.as_bytes(), sig)?;
            Ok(ShadowOutcome::ok("signature valid"))
        }
        _ => Ok(ShadowOutcome::fail(format!(
            "unknown webhook command: {cmd}"
        ))),
    }
}

/// Dispatch peptidoglycan trust barrier validation commands.
///
/// Validates that a peptidoglycan instance satisfies the fieldMouse contract:
/// stores nothing, relays opaquely, is disposable.
async fn dispatch_pepti(
    config: &ShadowConfig,
    cmd: &str,
    args: &[&str],
) -> crate::Result<ShadowOutcome> {
    match cmd {
        "pepti.validate" => pepti_validate(config, args).await,
        _ => Ok(ShadowOutcome::fail(format!("unknown pepti command: {cmd}"))),
    }
}

async fn pepti_validate(config: &ShadowConfig, args: &[&str]) -> crate::Result<ShadowOutcome> {
    let pepti_host = args.first().map_or_else(
        || {
            std::env::var(cellmembrane_types::service::ENV_PEPTI_SSH_HOST)
                .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_PEPTI_SSH_ALIAS.into())
        },
        |&h| h.to_string(),
    );

    let pepti_config = ShadowConfig {
        ssh_host: pepti_host.clone(),
        ..config.clone()
    };

    let mut checks: Vec<(&str, bool, String)> = Vec::new();

    // Check 1: SSH reachable
    let ssh_ok = crate::ssh::check_connectivity(&pepti_host).await;
    checks.push(("ssh.reachable", ssh_ok, pepti_host));

    if !ssh_ok {
        let msg = format_pepti_report(&checks);
        return Ok(ShadowOutcome {
            ok: false,
            message: msg,
            data: Some(checks_to_json(&checks)),
        });
    }

    // Check 2: TURN relay running (capability-discovered port)
    let turn_port = cellmembrane_types::MembraneService::with_capability(
        cellmembrane_types::ServiceCapability::TurnServer,
    )
    .and_then(|s| s.port)
    .unwrap_or(cellmembrane_types::service::DEFAULT_TURN_PORT);
    let (turn_out, turn_code) = crate::ssh::exec_raw(
        &pepti_config,
        &format!("ss -tlnp | grep -q ':{turn_port}' && echo OK || echo FAIL"),
    )
    .await?;
    let turn_ok = turn_code == 0 && turn_out.contains("OK");
    checks.push(("mesh.turn", turn_ok, format!("port {turn_port}")));

    // Check 3: tower.env exists (identity)
    let tower_env_path = format!("{}/tower.env", config.vps_root.trim_end_matches('/'));
    let (tower_out, tower_code) = crate::ssh::exec_raw(
        &pepti_config,
        &format!("test -f {tower_env_path} && echo OK || echo MISSING"),
    )
    .await?;
    let tower_ok = tower_code == 0 && tower_out.contains("OK");
    checks.push(("tower.env", tower_ok, tower_env_path));

    // Check 4: No primary data stored (nothing in content dirs)
    let find_cmd = format!(
        "find {} -name '*.db' -o -name '*.sqlite' 2>/dev/null | wc -l",
        cellmembrane_types::service::DEFAULT_INSTALL_BASE
    );
    let (data_out, _) = crate::ssh::exec_raw(&pepti_config, &find_cmd).await?;
    let data_files: u32 = data_out.trim().parse().unwrap_or(99);
    let no_data = data_files == 0;
    checks.push((
        "stores.nothing",
        no_data,
        format!("{data_files} data files"),
    ));

    // Check 5: Firewall — only relay ports open
    let (ufw_out, _) = crate::ssh::exec_raw(
        &pepti_config,
        "ufw status | grep -cE 'ALLOW' 2>/dev/null || echo 0",
    )
    .await?;
    let ufw_rules: u32 = ufw_out.trim().parse().unwrap_or(0);
    let minimal_firewall = ufw_rules <= 5;
    checks.push((
        "firewall.minimal",
        minimal_firewall,
        format!("{ufw_rules} ALLOW rules"),
    ));

    // Check 6: No services above Relay composition running (peptidoglycan is relay-tier)
    let higher_services: Vec<&str> = cellmembrane_types::MembraneService::all()
        .iter()
        .filter(|s| {
            s.is_primal && s.min_composition > cellmembrane_types::MembraneComposition::Relay
        })
        .map(|s| s.systemd_unit)
        .collect();
    let check_cmd = format!(
        "systemctl is-active {} 2>/dev/null | grep -c active || echo 0",
        higher_services.join(" ")
    );
    let (services_out, _) = crate::ssh::exec_raw(&pepti_config, &check_cmd).await?;
    let inner_services: u32 = services_out.trim().parse().unwrap_or(0);
    let no_inner_services = inner_services == 0;
    checks.push((
        "no.inner.services",
        no_inner_services,
        format!("{inner_services} inner services active"),
    ));

    let all_pass = checks.iter().all(|(_, ok, _)| *ok);
    let msg = format_pepti_report(&checks);

    Ok(if all_pass {
        ShadowOutcome::ok_with(msg, checks_to_json(&checks))
    } else {
        ShadowOutcome {
            ok: false,
            message: msg,
            data: Some(checks_to_json(&checks)),
        }
    })
}

fn format_pepti_report(checks: &[(&str, bool, String)]) -> String {
    use std::fmt::Write;
    let mut out = String::from("=== Peptidoglycan Trust Barrier Validation ===\n");
    for (name, ok, detail) in checks {
        let status = if *ok { "PASS" } else { "FAIL" };
        let _ = writeln!(out, "  [{status}] {name}: {detail}");
    }
    let passed = checks.iter().filter(|(_, ok, _)| *ok).count();
    let _ = write!(out, "\n  Result: {passed}/{} checks passed", checks.len());
    out
}

fn checks_to_json(checks: &[(&str, bool, String)]) -> serde_json::Value {
    serde_json::json!(
        checks
            .iter()
            .map(|(name, ok, detail)| {
                serde_json::json!({
                    "check": name,
                    "pass": ok,
                    "detail": detail,
                })
            })
            .collect::<Vec<_>>()
    )
}
