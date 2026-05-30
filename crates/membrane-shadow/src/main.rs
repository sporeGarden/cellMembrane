// SPDX-License-Identifier: AGPL-3.0-or-later

//! `membrane` CLI — sovereign shadow function dispatcher.
//!
//! Typed Rust replacement for `membrane.sh`. Routes `domain.operation`
//! commands to the appropriate shadow function, returning structured
//! JSON or human-readable output.

use membrane_shadow::{ShadowConfig, ShadowOutcome, forgejo, gate, service};
use std::process::ExitCode;

fn usage() {
    eprintln!(
        r#"membrane — sovereign shadow functions for golgiBody VPS

Usage: membrane <domain.operation> [args...]

Repo (nestGate content.repo.*):
  repo.create <org/name>           Create repo on Forgejo
  repo.list [org]                  List repos (default: ecoPrimals)
  repo.delete <org/name>           Delete repo from Forgejo

Mirror (nestGate content.mirror.*):
  mirror.sync <org/name>           Trigger mirror sync for one repo
  mirror.sync-all                  Trigger sync on all mirrors
  mirror.status <org/name>         Show mirror status for a repo

Service (biomeOS gate.service.*):
  service.list                     List running membrane services
  service.status <unit>            Show service status
  service.restart <unit>           Restart a service
  service.logs <unit> [lines]      Show recent logs (default: 30)

Gate (biomeOS gate.*):
  gate.info                        VPS system info + service summary
  gate.pull                        Run cascade-pull on golgiBody
  gate.check                       Parity check on golgiBody workspace

Token (bearDog auth.token.*):
  token.list                       List all Forgejo API tokens
  token.create <name> [scopes]     Generate new API token
  token.revoke <id>                Delete token by database ID

Forgejo:
  forgejo.version                  Show Forgejo version

Options:
  --json                           Output as JSON (default: human-readable)
  -h, --help                       Show this help"#
    );
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let json_mode = args.iter().any(|a| a == "--json");
    let args: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .map(String::as_str)
        .collect();

    if args.is_empty() || args[0] == "-h" || args[0] == "help" {
        usage();
        return ExitCode::SUCCESS;
    }

    let config = ShadowConfig::from_env().await;
    let cmd = args[0];
    let rest = &args[1..];

    let outcome = dispatch(&config, cmd, rest).await;

    match outcome {
        Ok(o) => {
            if json_mode {
                println!("{}", serde_json::to_string_pretty(&o).unwrap_or_default());
            } else {
                println!("{}", o.message);
                if let Some(data) = &o.data {
                    println!("{}", serde_json::to_string_pretty(data).unwrap_or_default());
                }
            }
            if o.ok { ExitCode::SUCCESS } else { ExitCode::FAILURE }
        }
        Err(e) => {
            if json_mode {
                let o = ShadowOutcome::fail(&e);
                println!("{}", serde_json::to_string_pretty(&o).unwrap_or_default());
            } else {
                eprintln!("ERROR: {e}");
            }
            ExitCode::FAILURE
        }
    }
}

async fn dispatch(
    config: &ShadowConfig,
    cmd: &str,
    args: &[&str],
) -> membrane_shadow::Result<ShadowOutcome> {
    match cmd {
        // ── Repo ────────────────────────────────────────────────
        "repo.create" => {
            let path = require_arg(args, 0, "org/name")?;
            let (org, name) = split_repo_path(path)?;
            let repo = forgejo::repo_create(config, org, name).await?;
            Ok(ShadowOutcome::ok_with(
                format!("CREATED {}", repo.full_name),
                serde_json::to_value(&repo)?,
            ))
        }
        "repo.list" => {
            let org = args.first().copied().unwrap_or("ecoPrimals");
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
            ).tap_message(|m| format!("{m}\n{}", lines.join("\n"))))
        }
        "repo.delete" => {
            let path = require_arg(args, 0, "org/name")?;
            forgejo::repo_delete(config, path).await?;
            Ok(ShadowOutcome::ok(format!("DELETED {path}")))
        }

        // ── Mirror ──────────────────────────────────────────────
        "mirror.sync" => {
            let path = require_arg(args, 0, "org/name")?;
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
        "mirror.sync-all" => {
            let mut triggered = 0u32;
            let mut failed = 0u32;
            for org in &["ecoPrimals", "syntheticChemistry", "sporeGarden"] {
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
        "mirror.status" => {
            let path = require_arg(args, 0, "org/name")?;
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

        // ── Service ─────────────────────────────────────────────
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
            let unit = require_arg(args, 0, "unit-name")?;
            let s = service::status(config, unit).await?;
            let state = if s.active { "active" } else { "inactive" };
            let mem = s.memory.as_deref().unwrap_or("-");
            let pid = s.pid.map_or("-".to_string(), |p| p.to_string());
            Ok(ShadowOutcome::ok_with(
                format!("{unit}: {state}/{} pid={pid} mem={mem}", s.sub_state),
                serde_json::to_value(&s)?,
            ))
        }
        "service.restart" => {
            let unit = require_arg(args, 0, "unit-name")?;
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
            let unit = require_arg(args, 0, "unit-name")?;
            let lines: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(30);
            let output = service::logs(config, unit, lines).await?;
            Ok(ShadowOutcome::ok(output))
        }

        // ── Gate ────────────────────────────────────────────────
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

        // ── Token ───────────────────────────────────────────────
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
            let name = require_arg(args, 0, "token-name")?;
            let scopes = args.get(1).copied().unwrap_or(
                "write:repository,read:repository,write:organization,read:organization",
            );
            let token = forgejo::token_create(config, name, scopes).await?;
            Ok(ShadowOutcome::ok_with(
                format!("TOKEN: {token}\nname={name} scopes={scopes}"),
                serde_json::json!({ "token": token, "name": name, "scopes": scopes }),
            ))
        }
        "token.revoke" => {
            let id_str = require_arg(args, 0, "token-id")?;
            let id: u64 = id_str
                .parse()
                .map_err(|_| membrane_shadow::ShadowError::Parse(
                    format!("invalid token id: {id_str}")
                ))?;
            forgejo::token_revoke(config, id).await?;
            Ok(ShadowOutcome::ok(format!("REVOKED token id={id}")))
        }

        // ── Forgejo ─────────────────────────────────────────────
        "forgejo.version" => {
            let v = forgejo::version(config).await?;
            Ok(ShadowOutcome::ok(v))
        }

        _ => {
            eprintln!("unknown command: {cmd}");
            usage();
            Ok(ShadowOutcome::fail(format!("unknown command: {cmd}")))
        }
    }
}

fn require_arg<'a>(args: &[&'a str], idx: usize, name: &str) -> membrane_shadow::Result<&'a str> {
    args.get(idx).copied().ok_or_else(|| {
        membrane_shadow::ShadowError::Parse(format!("{name} required"))
    })
}

fn split_repo_path(path: &str) -> membrane_shadow::Result<(&str, &str)> {
    path.split_once('/').ok_or_else(|| {
        membrane_shadow::ShadowError::Parse(format!(
            "expected org/name format, got: {path}"
        ))
    })
}

trait TapMessage {
    fn tap_message(self, f: impl FnOnce(&str) -> String) -> Self;
}

impl TapMessage for ShadowOutcome {
    fn tap_message(mut self, f: impl FnOnce(&str) -> String) -> Self {
        self.message = f(&self.message);
        self
    }
}
