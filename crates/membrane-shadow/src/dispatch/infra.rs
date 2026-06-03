// SPDX-License-Identifier: AGPL-3.0-or-later

//! Infrastructure domain dispatch — repo, mirror, service, gate, token.
//!
//! These domains interact with the VPS (golgiBody) via SSH and the Forgejo API.

use crate::cli::{self, TapMessage};
use crate::{ShadowConfig, ShadowOutcome, forgejo, gate, service};

// ── Repo domain ──────────────────────────────────────────────────────

pub(super) async fn dispatch_repo(
    config: &ShadowConfig,
    cmd: &str,
    args: &[&str],
) -> crate::Result<ShadowOutcome> {
    match cmd {
        "repo.create" => {
            let path = cli::require_arg(args, 0, "org/name")?;
            let (org, name) = cli::split_repo_path(path)?;
            let repo = forgejo::repo_create(config, org, name).await?;
            Ok(ShadowOutcome::ok_with(
                format!("CREATED {}", repo.full_name),
                serde_json::to_value(&repo)?,
            ))
        }
        "repo.list" => {
            let org = cli::require_arg(args, 0, "org")?;
            let repos = forgejo::repo_list(config, org).await?;
            let lines: Vec<String> = repos
                .iter()
                .map(|r| {
                    let kind = if r.mirror { "mirror" } else { "repo" };
                    format!("  {:30} {kind}", r.name)
                })
                .collect();
            Ok(ShadowOutcome::ok_with(
                format!("{} repos in {org}", repos.len()),
                serde_json::to_value(&repos)?,
            )
            .tap_message(|m| format!("{m}\n{}", lines.join("\n"))))
        }
        "repo.delete" => {
            let path = cli::require_arg(args, 0, "org/name")?;
            forgejo::repo_delete(config, path).await?;
            Ok(ShadowOutcome::ok(format!("DELETED {path}")))
        }
        _ => Ok(ShadowOutcome::fail(format!("unknown repo command: {cmd}"))),
    }
}

// ── Mirror domain ────────────────────────────────────────────────────

pub(super) async fn dispatch_mirror(
    config: &ShadowConfig,
    cmd: &str,
    args: &[&str],
) -> crate::Result<ShadowOutcome> {
    match cmd {
        "mirror.sync" => {
            let path = cli::require_arg(args, 0, "org/name")?;
            let result = forgejo::mirror_sync(config, path).await?;
            if result.triggered {
                Ok(ShadowOutcome::ok(format!("TRIGGERED {path}")))
            } else {
                Ok(ShadowOutcome::fail(format!(
                    "FAILED {path} (HTTP {})",
                    result.http_code
                )))
            }
        }
        "mirror.sync-all" => mirror_sync_all(config, args).await,
        "mirror.status" => {
            let path = cli::require_arg(args, 0, "org/name")?;
            let info = forgejo::mirror_status(config, path).await?;
            let msg = if info.mirror {
                format!(
                    "{path}: mirror interval={} last={}",
                    info.mirror_interval,
                    &info.mirror_updated[..19.min(info.mirror_updated.len())]
                )
            } else {
                format!("{path}: plain repo (not a mirror)")
            };
            Ok(ShadowOutcome::ok_with(msg, serde_json::to_value(&info)?))
        }
        "mirror.push-create" => {
            let full_name = cli::require_arg(args, 0, "org/name")?;
            let remote_url = cli::require_arg(args, 1, "remote_url")?;
            let mirror = forgejo::push_mirror_create(config, full_name, remote_url).await?;
            Ok(ShadowOutcome::ok_with(
                format!(
                    "PUSH MIRROR CREATED {} → {}",
                    full_name, mirror.remote_address
                ),
                serde_json::to_value(&mirror)?,
            ))
        }
        "mirror.push-list" => {
            let full_name = cli::require_arg(args, 0, "org/name")?;
            let mirrors = forgejo::push_mirror_list(config, full_name).await?;
            let lines: Vec<String> = mirrors
                .iter()
                .map(|m| {
                    let sync = if m.sync_on_commit {
                        "on-commit"
                    } else {
                        &m.interval
                    };
                    format!("  {} → {} ({sync})", m.remote_name, m.remote_address)
                })
                .collect();
            Ok(ShadowOutcome::ok_with(
                format!(
                    "{} push mirror(s) for {full_name}\n{}",
                    mirrors.len(),
                    lines.join("\n")
                ),
                serde_json::to_value(&mirrors)?,
            ))
        }
        "mirror.push-sync" => {
            let full_name = cli::require_arg(args, 0, "org/name")?;
            let result = forgejo::push_mirror_sync(config, full_name).await?;
            if result.triggered {
                Ok(ShadowOutcome::ok(format!(
                    "PUSH SYNC TRIGGERED {full_name}"
                )))
            } else {
                Ok(ShadowOutcome::fail(format!(
                    "PUSH SYNC FAILED {full_name} (HTTP {})",
                    result.http_code
                )))
            }
        }
        _ => Ok(ShadowOutcome::fail(format!(
            "unknown mirror command: {cmd}"
        ))),
    }
}

async fn mirror_sync_all(config: &ShadowConfig, args: &[&str]) -> crate::Result<ShadowOutcome> {
    let orgs: Vec<&str> = if args.is_empty() {
        vec!["ecoPrimals"]
    } else {
        args.to_vec()
    };
    let mut triggered = 0u32;
    let mut failed = 0u32;
    for org in &orgs {
        let repos = forgejo::repo_list(config, org).await?;
        for repo in &repos {
            if repo.mirror {
                let r = forgejo::mirror_sync(config, &repo.full_name).await?;
                if r.triggered {
                    triggered += 1;
                } else {
                    failed += 1;
                }
            }
        }
    }
    Ok(ShadowOutcome::ok(format!(
        "triggered={triggered} failed={failed}"
    )))
}

// ── Service domain ───────────────────────────────────────────────────

pub(super) async fn dispatch_service(
    config: &ShadowConfig,
    cmd: &str,
    args: &[&str],
) -> crate::Result<ShadowOutcome> {
    match cmd {
        "service.list" => {
            let services = service::list(config).await?;
            let lines: Vec<String> = services
                .iter()
                .map(|s| format!("  {:40} {}", s.unit, s.sub_state))
                .collect();
            Ok(ShadowOutcome::ok_with(
                format!("{} services running\n{}", services.len(), lines.join("\n")),
                serde_json::to_value(&services)?,
            ))
        }
        "service.status" => {
            let unit = cli::require_arg(args, 0, "unit-name")?;
            let s = service::status(config, unit).await?;
            let state = if s.active { "active" } else { "inactive" };
            let mem = s.memory.as_deref().unwrap_or("-");
            let pid = s.pid.map_or_else(|| "-".to_string(), |p| p.to_string());
            Ok(ShadowOutcome::ok_with(
                format!("{unit}: {state}/{} pid={pid} mem={mem}", s.sub_state),
                serde_json::to_value(&s)?,
            ))
        }
        "service.restart" => {
            let unit = cli::require_arg(args, 0, "unit-name")?;
            let s = service::restart(config, unit).await?;
            if s.active {
                Ok(ShadowOutcome::ok(format!("RESTARTED {unit}")))
            } else {
                Ok(ShadowOutcome::fail(format!(
                    "RESTART FAILED {unit} (state={})",
                    s.sub_state
                )))
            }
        }
        "service.logs" => {
            let unit = cli::require_arg(args, 0, "unit-name")?;
            let lines: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(30);
            let output = service::logs(config, unit, lines).await?;
            Ok(ShadowOutcome::ok(output))
        }
        _ => Ok(ShadowOutcome::fail(format!(
            "unknown service command: {cmd}"
        ))),
    }
}

// ── Gate domain ──────────────────────────────────────────────────────

pub(super) async fn dispatch_gate(
    config: &ShadowConfig,
    cmd: &str,
    _args: &[&str],
) -> crate::Result<ShadowOutcome> {
    match cmd {
        "gate.info" => {
            let info = gate::info(config).await?;
            let svc_lines: Vec<String> = info
                .services
                .iter()
                .map(|s| format!("  {:40} {}", s.unit, s.state))
                .collect();
            let msg = format!(
                "{hostname} ({gate})\n\
                 uptime:  {uptime}\n\
                 load:    {load}\n\
                 memory:  {memory}\n\
                 disk:    {disk}\n\
                 repos:   {repos}\n\
                 \n\
                 services ({n}):\n\
                 {svcs}",
                hostname = info.hostname,
                gate = info.gate_identity,
                uptime = info.uptime,
                load = info.load,
                memory = info.memory,
                disk = info.disk,
                repos = info.repo_count,
                n = info.services.len(),
                svcs = svc_lines.join("\n"),
            );
            Ok(ShadowOutcome::ok_with(msg, serde_json::to_value(&info)?))
        }
        "gate.pull" => {
            let result = gate::pull(config).await?;
            Ok(ShadowOutcome::ok_with(
                format!(
                    "pulled {}/{} repos on {}",
                    result.synced, result.total, result.gate
                ),
                serde_json::to_value(&result)?,
            ))
        }
        "gate.check" => {
            let result = gate::check(config).await?;
            let msg = format!(
                "{}: {}/{} in sync{}{}",
                result.gate,
                result.synced,
                result.total,
                if result.drifted > 0 {
                    format!(", {} drifted", result.drifted)
                } else {
                    String::new()
                },
                if result.missing > 0 {
                    format!(", {} missing", result.missing)
                } else {
                    String::new()
                },
            );
            Ok(ShadowOutcome::ok_with(msg, serde_json::to_value(&result)?))
        }
        "gate.health" => gate_health(config).await,
        _ => Ok(ShadowOutcome::fail(format!("unknown gate command: {cmd}"))),
    }
}

async fn gate_health(config: &ShadowConfig) -> crate::Result<ShadowOutcome> {
    let services = service::list(config).await?;
    let total = services.len();
    let healthy = services.iter().filter(|s| s.sub_state == "running").count();
    let degraded: Vec<&str> = services
        .iter()
        .filter(|s| s.sub_state != "running")
        .map(|s| s.unit.as_str())
        .collect();

    let disk = crate::ssh::exec(config, "df --output=pcent / | tail -1")
        .await
        .unwrap_or_default()
        .trim()
        .to_string();

    let status = if degraded.is_empty() {
        "HEALTHY"
    } else {
        "DEGRADED"
    };

    let msg = format!(
        "=== Gate Health ===\n\
         Status:   {status}\n\
         Services: {healthy}/{total} running\n\
         Disk:     {disk}\n\
         {}",
        if degraded.is_empty() {
            String::new()
        } else {
            format!("Degraded: {}", degraded.join(", "))
        }
    );

    let ok = degraded.is_empty();
    Ok(if ok {
        ShadowOutcome::ok_with(
            msg,
            serde_json::json!({
                "status": status,
                "services_total": total,
                "services_healthy": healthy,
                "disk": disk,
            }),
        )
    } else {
        ShadowOutcome {
            ok: false,
            message: msg,
            data: Some(serde_json::json!({
                "status": status,
                "services_total": total,
                "services_healthy": healthy,
                "degraded": degraded,
                "disk": disk,
            })),
        }
    })
}

// ── Token domain ─────────────────────────────────────────────────────

pub(super) async fn dispatch_token(
    config: &ShadowConfig,
    cmd: &str,
    args: &[&str],
) -> crate::Result<ShadowOutcome> {
    match cmd {
        "token.list" => {
            let tokens = forgejo::token_list(config).await?;
            let lines: Vec<String> = tokens
                .iter()
                .map(|t| format!("  id={:<4} name={:<30} created={}", t.id, t.name, t.created))
                .collect();
            Ok(ShadowOutcome::ok_with(
                format!("{} tokens\n{}", tokens.len(), lines.join("\n")),
                serde_json::to_value(&tokens)?,
            ))
        }
        "token.create" => {
            let name = cli::require_arg(args, 0, "token-name")?;
            let scopes = args
                .get(1)
                .copied()
                .unwrap_or("write:repository,read:repository,write:organization,read:organization");
            let token = forgejo::token_create(config, name, scopes).await?;
            Ok(ShadowOutcome::ok_with(
                format!("TOKEN: {token}\nname={name} scopes={scopes}"),
                serde_json::json!({ "token": token, "name": name, "scopes": scopes }),
            ))
        }
        "token.revoke" => {
            let id_str = cli::require_arg(args, 0, "token-id")?;
            let id: u64 = id_str
                .parse()
                .map_err(|_| crate::ShadowError::Parse(format!("invalid token id: {id_str}")))?;
            forgejo::token_revoke(config, id).await?;
            Ok(ShadowOutcome::ok(format!("REVOKED token id={id}")))
        }
        _ => Ok(ShadowOutcome::fail(format!("unknown token command: {cmd}"))),
    }
}
