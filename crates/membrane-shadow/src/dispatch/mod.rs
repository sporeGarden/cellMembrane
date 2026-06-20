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

mod content_dispatch;
mod data;
mod gate;
mod impulse;
mod infra;
mod plasmid_dispatch;
mod provision_dispatch;
mod relay_dispatch;
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
            .unwrap_or_else(|e| Err(ShadowError::Parse(format!("spawn_blocking: {e}"))))
        }
        c if c.starts_with("context.") => data::dispatch_context(cmd, args).await,
        "depot.integrity" => {
            let args: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
            tokio::task::spawn_blocking(move || {
                let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                plasmid_dispatch::dispatch_depot_integrity(&refs)
            })
            .await
            .unwrap_or_else(|e| Err(ShadowError::Parse(format!("spawn_blocking: {e}"))))
        }
        c if c.starts_with("plasmid.") => {
            plasmid_dispatch::dispatch_plasmid(config, cmd, args).await
        }
        c if c.starts_with("relay.") => relay_dispatch::dispatch_relay(cmd, args).await,
        c if c.starts_with("content.") => {
            content_dispatch::dispatch_content(config, cmd, args).await
        }
        "forgejo.version" => {
            let v = forgejo::version(config).await?;
            Ok(ShadowOutcome::ok(v))
        }
        c if c.starts_with("rootpulse.") => dispatch_rootpulse(cmd, args).await,
        c if c.starts_with("caddy.") => crate::caddy::dispatch(config, cmd, args).await,
        c if c.starts_with("webhook.") => dispatch_webhook(config, cmd, args).await,
        c if c.starts_with("pepti.") => dispatch_pepti(config, cmd, args).await,
        #[cfg(feature = "cloudflare")]
        c if c.starts_with("cloudflare.") => crate::cloudflare::dispatch(cmd, args).await,
        _ => Ok(ShadowOutcome::fail(format!("unknown command: {cmd}"))),
    }
}

fn parse_webhook_provider(args: &[&str]) -> crate::webhook::WebhookProvider {
    cli::extract_flag_value(args, "--provider").map_or(
        crate::webhook::WebhookProvider::Forgejo,
        |p| match p {
            "github" => crate::webhook::WebhookProvider::GitHub,
            _ => crate::webhook::WebhookProvider::Forgejo,
        },
    )
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
            let provider = parse_webhook_provider(args);
            crate::webhook::handle_push(&event, config, provider).await
        }
        "webhook.verify" => {
            let secret = std::env::var(cellmembrane_types::service::ENV_WEBHOOK_SECRET)
                .map_err(|_| ShadowError::Config("WEBHOOK_SECRET env var required".into()))?;
            let body = cli::require_arg(args, 0, "body")?;
            let sig = cli::extract_flag_value(args, "--signature")
                .ok_or_else(|| ShadowError::Config("--signature flag required".into()))?;
            let provider = parse_webhook_provider(args);
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

/// Dispatch composition trust barrier validation commands.
///
/// `gate.validate` validates any gate against its manifest composition tier.
/// `pepti.validate` is a backward-compatible alias (resolves to composition=Relay).
async fn dispatch_pepti(
    config: &ShadowConfig,
    cmd: &str,
    args: &[&str],
) -> crate::Result<ShadowOutcome> {
    match cmd {
        "pepti.validate" => {
            gate_validate(
                config,
                args,
                Some(cellmembrane_types::MembraneComposition::Relay),
            )
            .await
        }
        _ => Ok(ShadowOutcome::fail(format!("unknown pepti command: {cmd}"))),
    }
}

/// Generic composition trust barrier validation.
///
/// Validates that a remote gate conforms to its declared composition tier:
/// - Services above the tier must not be running
/// - Relay services must be present
/// - Identity (tower.env) must exist
/// - Data storage must match tier expectations
/// - Firewall must be appropriately restrictive
///
/// If `composition_override` is `None`, the composition is resolved from the
/// manifest gate profile or from `--composition <tier>` flag.
async fn gate_validate(
    config: &ShadowConfig,
    args: &[&str],
    composition_override: Option<cellmembrane_types::MembraneComposition>,
) -> crate::Result<ShadowOutcome> {
    let composition = if let Some(c) = composition_override {
        c
    } else if let Some(comp_str) = cli::extract_flag_value(args, "--composition") {
        cellmembrane_types::MembraneComposition::parse_name(comp_str).ok_or_else(|| {
            crate::error::ShadowError::Config(format!(
                "unknown composition tier: {comp_str} \
                 (valid: relay, rustdesk, tower, nest, nucleus, peptidoglycan)"
            ))
        })?
    } else {
        resolve_gate_composition(args).unwrap_or(cellmembrane_types::MembraneComposition::Relay)
    };

    let target_host = resolve_validate_host(config, args);
    let target_config = ShadowConfig {
        ssh_host: target_host.clone(),
        ..config.clone()
    };

    let checks = run_composition_checks(config, &target_config, &target_host, composition).await?;
    let all_pass = checks.iter().all(|(_, ok, _)| *ok);
    let msg = format_validate_report(&checks, composition);

    Ok(if all_pass {
        ShadowOutcome::ok_with(msg, checks_to_json(&checks, composition))
    } else {
        ShadowOutcome {
            ok: false,
            message: msg,
            data: Some(checks_to_json(&checks, composition)),
        }
    })
}

/// Execute composition trust barrier checks via SSH.
async fn run_composition_checks(
    config: &ShadowConfig,
    target_config: &ShadowConfig,
    target_host: &str,
    composition: cellmembrane_types::MembraneComposition,
) -> crate::Result<Vec<(&'static str, bool, String)>> {
    let mut checks: Vec<(&str, bool, String)> = Vec::new();

    let ssh_ok = crate::ssh::check_connectivity(target_host).await;
    checks.push(("ssh.reachable", ssh_ok, target_host.to_string()));
    if !ssh_ok {
        return Ok(checks);
    }

    let turn_port = cellmembrane_types::MembraneService::with_capability(
        cellmembrane_types::ServiceCapability::TurnServer,
    )
    .and_then(|s| s.port)
    .unwrap_or(cellmembrane_types::service::DEFAULT_TURN_PORT);
    let (turn_out, turn_code) = crate::ssh::exec_raw(
        target_config,
        &format!("ss -tlnp | grep -q ':{turn_port}' && echo OK || echo FAIL"),
    )
    .await?;
    let turn_ok = turn_code == 0 && turn_out.contains("OK");
    checks.push(("mesh.turn", turn_ok, format!("port {turn_port}")));

    let tower_env_path = format!("{}/tower.env", config.vps_root.trim_end_matches('/'));
    let (_, tower_code) = crate::ssh::exec_raw(
        target_config,
        &format!("test -f {tower_env_path} && echo OK || echo MISSING"),
    )
    .await?;
    let tower_ok = tower_code == 0;
    checks.push(("tower.env", tower_ok, tower_env_path));

    let find_cmd = format!(
        "find {} -name '*.db' -o -name '*.sqlite' 2>/dev/null | wc -l",
        cellmembrane_types::service::DEFAULT_INSTALL_BASE
    );
    let (data_out, _) = crate::ssh::exec_raw(target_config, &find_cmd).await?;
    let data_files: u32 = data_out.trim().parse().unwrap_or(99);
    let stores_nothing = matches!(
        composition,
        cellmembrane_types::MembraneComposition::Relay
            | cellmembrane_types::MembraneComposition::Peptidoglycan
    );
    let data_ok = if stores_nothing {
        data_files == 0
    } else {
        true
    };
    let data_label = if stores_nothing {
        "stores.nothing"
    } else {
        "data.present"
    };
    checks.push((data_label, data_ok, format!("{data_files} data files")));

    let (ufw_out, _) = crate::ssh::exec_raw(
        target_config,
        "ufw status | grep -cE 'ALLOW' 2>/dev/null || echo 0",
    )
    .await?;
    let ufw_rules: u32 = ufw_out.trim().parse().unwrap_or(0);
    checks.push((
        "firewall.minimal",
        ufw_rules <= 5,
        format!("{ufw_rules} ALLOW rules"),
    ));

    let higher_services: Vec<&str> = cellmembrane_types::MembraneService::all()
        .iter()
        .filter(|s| s.is_primal && s.min_composition > composition)
        .map(|s| s.systemd_unit)
        .collect();
    if higher_services.is_empty() {
        checks.push((
            "no.excess.services",
            true,
            String::from("top-tier composition — no excess possible"),
        ));
    } else {
        let check_cmd = format!(
            "systemctl is-active {} 2>/dev/null | grep -c active || echo 0",
            higher_services.join(" ")
        );
        let (services_out, _) = crate::ssh::exec_raw(target_config, &check_cmd).await?;
        let excess: u32 = services_out.trim().parse().unwrap_or(0);
        checks.push((
            "no.excess.services",
            excess == 0,
            format!("{excess} services above {composition} tier"),
        ));
    }

    Ok(checks)
}

/// Resolve the SSH host for validation from args, env var, or pepti fallback.
fn resolve_validate_host(config: &ShadowConfig, args: &[&str]) -> String {
    if let Some(host) = cli::extract_flag_value(args, "--host") {
        return host.to_string();
    }
    args.iter().find(|a| !a.starts_with("--")).map_or_else(
        || {
            std::env::var(cellmembrane_types::service::ENV_PEPTI_SSH_HOST)
                .unwrap_or_else(|_| config.ssh_host.clone())
        },
        |&h| h.to_string(),
    )
}

/// Resolve composition from manifest for the target gate.
fn resolve_gate_composition(args: &[&str]) -> Option<cellmembrane_types::MembraneComposition> {
    let gate_name = cli::extract_flag_value(args, "--gate")?;
    let root = crate::temporal::resolve_workspace_root().ok()?;
    let manifest = crate::manifest::load_from_workspace(&root).ok()?;
    let profile = manifest.gates.get(gate_name)?;
    profile
        .composition
        .as_ref()
        .and_then(|c| cellmembrane_types::MembraneComposition::parse_name(c))
}

fn format_validate_report(
    checks: &[(&str, bool, String)],
    composition: cellmembrane_types::MembraneComposition,
) -> String {
    use std::fmt::Write;
    let mut out = format!("=== Composition Trust Barrier Validation ({composition}) ===\n");
    for (name, ok, detail) in checks {
        let status = if *ok { "PASS" } else { "FAIL" };
        let _ = writeln!(out, "  [{status}] {name}: {detail}");
    }
    let passed = checks.iter().filter(|(_, ok, _)| *ok).count();
    let _ = write!(out, "\n  Result: {passed}/{} checks passed", checks.len());
    out
}

fn checks_to_json(
    checks: &[(&str, bool, String)],
    composition: cellmembrane_types::MembraneComposition,
) -> serde_json::Value {
    serde_json::json!({
        "composition": composition.to_string(),
        "checks": checks
            .iter()
            .map(|(name, ok, detail)| {
                serde_json::json!({
                    "check": name,
                    "pass": ok,
                    "detail": detail,
                })
            })
            .collect::<Vec<serde_json::Value>>(),
    })
}

/// Resolve gate name from `--gate` flag, `GATE_NAME` env var, or identity file.
async fn resolve_gate_name(args: &[&str], root: &std::path::Path) -> String {
    if let Some(g) = cli::extract_flag_value(args, "--gate") {
        return g.to_string();
    }
    if let Ok(g) = std::env::var(cellmembrane_types::service::ENV_GATE_NAME) {
        if !g.is_empty() {
            return g;
        }
    }
    let root = root.to_path_buf();
    tokio::task::spawn_blocking(move || {
        crate::identity::resolve(&root).map_or_else(|_| "unknown".into(), |id| id.name)
    })
    .await
    .unwrap_or_else(|_| "unknown".into())
}

/// Dispatch rootpulse sovereignty ledger commands.
async fn dispatch_rootpulse(cmd: &str, args: &[&str]) -> Result<ShadowOutcome> {
    match cmd {
        "rootpulse.commit" => dispatch_rootpulse_commit(args).await,
        "rootpulse.verify" => dispatch_rootpulse_verify(args).await,
        "rootpulse.status" => {
            let session = crate::temporal::post_sync::load_rootpulse_session_pub();
            Ok(session.map_or_else(
                || {
                    ShadowOutcome::ok_with(
                        "no rootpulse session recorded on this gate".to_string(),
                        serde_json::json!({ "last_session": null }),
                    )
                },
                |s| {
                    ShadowOutcome::ok_with(
                        format!("last rootpulse session: {s}"),
                        serde_json::json!({ "last_session": s }),
                    )
                },
            ))
        }
        _ => Ok(ShadowOutcome::fail(format!(
            "unknown rootpulse command: {cmd}"
        ))),
    }
}

async fn dispatch_rootpulse_commit(args: &[&str]) -> Result<ShadowOutcome> {
    let root = crate::temporal::resolve_workspace_root()?;
    let m = crate::manifest::load_from_workspace_async(&root).await?;
    let gate = resolve_gate_name(args, &root).await;
    let wave = cli::extract_flag_value(args, "--wave")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(m.meta.wave);

    let repos = m.gate_repos(&gate);
    let heads = crate::temporal::post_sync::collect_cascade_heads_pub(&root, &repos).await;

    if heads.is_empty() {
        return Ok(ShadowOutcome::fail(
            "no cloned repos found — nothing to commit",
        ));
    }

    match crate::sovereignty_ledger::rootpulse_commit(wave, &gate, &heads).await {
        Ok(session) => {
            crate::temporal::post_sync::persist_rootpulse_session_pub(wave, &gate, &session);
            Ok(ShadowOutcome::ok_with(
                format!("rootpulse committed: {session}"),
                serde_json::json!({
                    "session": session,
                    "wave": wave,
                    "gate": gate,
                    "repos": heads.len(),
                }),
            ))
        }
        Err(e) => Ok(ShadowOutcome::fail(format!("rootpulse commit failed: {e}"))),
    }
}

async fn dispatch_rootpulse_verify(args: &[&str]) -> Result<ShadowOutcome> {
    let root = crate::temporal::resolve_workspace_root()?;
    let m = crate::manifest::load_from_workspace_async(&root).await?;
    let gate = resolve_gate_name(args, &root).await;

    let repos = m.gate_repos(&gate);
    let heads = crate::temporal::post_sync::collect_cascade_heads_pub(&root, &repos).await;

    let checks = crate::sovereignty_ledger::sovereignty_verify(m.meta.wave, &heads).await;

    if checks.is_empty() {
        return Ok(ShadowOutcome::ok_with(
            "rootpulse ledger unavailable — graceful skip",
            serde_json::json!({ "status": "unavailable" }),
        ));
    }

    let verified = checks.iter().filter(|c| c.verified).count();
    let total = checks.len();
    let all_ok = verified == total;
    let detail_lines: Vec<String> = checks
        .iter()
        .map(|c| {
            let icon = if c.verified { "OK" } else { "MISMATCH" };
            format!("  [{icon}] {}: {}", c.repo, c.detail)
        })
        .collect();
    let msg = format!(
        "sovereignty: {verified}/{total} verified\n{}",
        detail_lines.join("\n")
    );
    Ok(ShadowOutcome {
        ok: all_ok,
        message: msg,
        data: Some(serde_json::json!({
            "verified": verified,
            "total": total,
            "checks": checks.iter().map(|c| serde_json::json!({
                "repo": c.repo,
                "verified": c.verified,
                "detail": c.detail,
            })).collect::<Vec<_>>(),
        })),
    })
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
        let provider = parse_webhook_provider(&["--provider", "github"]);
        assert_eq!(provider, crate::webhook::WebhookProvider::GitHub);

        let provider = parse_webhook_provider(&["--provider", "forgejo"]);
        assert_eq!(provider, crate::webhook::WebhookProvider::Forgejo);
    }

    #[test]
    fn webhook_provider_default_is_forgejo() {
        let provider = parse_webhook_provider(&[]);
        assert_eq!(provider, crate::webhook::WebhookProvider::Forgejo);
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
        let result = dispatch_rootpulse("rootpulse.status", &[]).await.unwrap();
        assert!(result.ok);
    }

    #[tokio::test]
    async fn rootpulse_unknown_subcommand() {
        let result = dispatch_rootpulse("rootpulse.invalid", &[]).await.unwrap();
        assert!(!result.ok);
        assert!(result.message.contains("unknown rootpulse command"));
    }
}
