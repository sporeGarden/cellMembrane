// SPDX-License-Identifier: AGPL-3.0-or-later

//! `membrane` CLI — sovereign shadow function dispatcher.
//!
//! Typed Rust replacement for `membrane.sh`. Routes `domain.operation`
//! commands to the appropriate shadow function, returning structured
//! JSON or human-readable output.

use membrane_shadow::{ShadowConfig, ShadowOutcome, context, forgejo, gate, identity, impulse, manifest, service, temporal};
use std::process::ExitCode;

fn usage() {
    eprintln!(
        r"membrane — sovereign shadow functions for golgiBody VPS

Usage: membrane <domain.operation> [args...]

Repo (nestGate content.repo.*):
  repo.create <org/name>           Create repo on Forgejo
  repo.list <org>                  List repos in org
  repo.delete <org/name>           Delete repo from Forgejo

Mirror (nestGate content.mirror.*):
  mirror.sync <org/name>           Trigger mirror sync for one repo
  mirror.sync-all [org...]         Trigger sync on all mirrors (default: ecoPrimals)
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

Temporal (waterFall temporal.*):
  temporal.check [repo_path...]    Temporal position matrix (local, all remotes)
  temporal.sync  [repo_path...]    Pull leader, push followers (ff-only)

Manifest (ecosystem manifest):
  manifest.info                    Show manifest metadata + sync config
  manifest.repos [gate]            List repos (all, or filtered by gate profile)
  manifest.orgs                    List all orgs from manifest

Identity:
  identity.resolve                 Show current gate identity

Token (bearDog auth.token.*):
  token.list                       List all Forgejo API tokens
  token.create <name> [scopes]     Generate new API token
  token.revoke <id>                Delete token by database ID

Impulse — rP action potentials (rootPulse ACTION):
  impulse.post --to <gate> --type <type> --subject <text>  Fire an impulse
  impulse.ack <id> [--note <text>]                         Acknowledge (receptor bind)
  impulse.archive                                          Discharge spent impulses

Potential — qS membrane potential (quorumSignal SENSE):
  potential.sense [--all] [--count]    Measure pending potential for this gate
  potential.check                      Gradient health across the mesh

Context — sweetGrass-external braids (developer state weaving):
  context.weave --project <path> --summary <text>  Weave a context braid
  context.sense [--gate <gate>] [--project <path>] [--all]  Sense context
  context.clear [--project <path>] [--expired]     Clear/decay braids

Forgejo:
  forgejo.version                  Show Forgejo version

Options:
  --json                           Output as JSON (default: human-readable)
  -h, --help                       Show this help"
    );
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let json_mode = args.iter().any(|a| a == "--json");
    let args: Vec<&str> = args
        .iter()
        .filter(|a| a.as_str() != "--json")
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
            let org = require_arg(args, 0, "org")?;
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
            let pid = s.pid.map_or_else(|| "-".to_string(), |p| p.to_string());
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

        // ── Temporal ──────────────────────────────────────────────
        "temporal.check" => {
            let root = temporal::resolve_workspace_root()?;
            if args.is_empty() {
                return Err(membrane_shadow::ShadowError::Parse(
                    "temporal.check requires at least one repo path".into(),
                ));
            }
            let matrices: Vec<temporal::TemporalMatrix> = {
                let mut v = Vec::with_capacity(args.len());
                for path in args {
                    v.push(temporal::check(&root, path).await?);
                }
                v
            };
            let lines: Vec<String> = matrices.iter().map(ToString::to_string).collect();
            let parity = matrices
                .iter()
                .filter(|m| m.classification == temporal::SyncClassification::Parity)
                .count();
            Ok(ShadowOutcome::ok_with(
                format!(
                    "{}/{} parity\n{}",
                    parity,
                    matrices.len(),
                    lines.join("\n")
                ),
                serde_json::to_value(&matrices)?,
            ))
        }
        "temporal.sync" => {
            let root = temporal::resolve_workspace_root()?;
            if args.is_empty() {
                return Err(membrane_shadow::ShadowError::Parse(
                    "temporal.sync requires at least one repo path".into(),
                ));
            }
            let mut results = Vec::with_capacity(args.len());
            let mut synced = 0u32;
            let mut failed = 0u32;
            for path in args {
                let r = temporal::sync(&root, path).await?;
                if r.ok {
                    synced += 1;
                } else {
                    failed += 1;
                }
                results.push(r);
            }
            let lines: Vec<String> = results
                .iter()
                .map(|r| {
                    let status = if r.ok { "OK" } else { "FAIL" };
                    format!("  {:<35} {status} {}", r.repo_path, r.summary)
                })
                .collect();
            Ok(ShadowOutcome::ok_with(
                format!(
                    "synced={synced} failed={failed}\n{}",
                    lines.join("\n")
                ),
                serde_json::to_value(&results)?,
            ))
        }

        // ── Manifest ──────────────────────────────────────────────
        "manifest.info" => {
            let root = temporal::resolve_workspace_root()?;
            let m = manifest::load_from_workspace(&root)?;
            let msg = format!(
                "manifest v{} wave {} ({} repos)\n\
                 sync: source={} branch={} divergence={} push_followers={}",
                m.meta.version, m.meta.wave, m.meta.total_repos,
                m.sync.default_source, m.sync.default_branch,
                m.sync.divergence_policy, m.sync.push_to_followers,
            );
            Ok(ShadowOutcome::ok_with(msg, serde_json::to_value(&m.meta)?))
        }
        "manifest.repos" => {
            let root = temporal::resolve_workspace_root()?;
            let m = manifest::load_from_workspace(&root)?;
            let repos: Vec<(&str, &manifest::RepoEntry)> = if let Some(gate_name) = args.first() {
                m.gate_repos(gate_name)
            } else {
                m.repos.iter().map(|(n, e)| (n.as_str(), e)).collect()
            };
            let lines: Vec<String> = repos
                .iter()
                .map(|(name, e)| format!("  {:<25} {:<30} {:<18} {}", name, e.local_path, e.membrane, e.category))
                .collect();
            let header = if let Some(g) = args.first() {
                format!("{} repos for gate {g}", repos.len())
            } else {
                format!("{} repos total", repos.len())
            };
            Ok(ShadowOutcome::ok(format!("{header}\n{}", lines.join("\n"))))
        }
        "manifest.orgs" => {
            let root = temporal::resolve_workspace_root()?;
            let m = manifest::load_from_workspace(&root)?;
            let orgs = m.orgs();
            Ok(ShadowOutcome::ok(format!("{} orgs: {}", orgs.len(), orgs.join(", "))))
        }

        // ── Identity ─────────────────────────────────────────────
        "identity.resolve" => {
            let root = temporal::resolve_workspace_root()?;
            match identity::resolve(&root) {
                Ok(id) => Ok(ShadowOutcome::ok_with(
                    format!("{} (via {:?})", id.name, id.source),
                    serde_json::to_value(&id)?,
                )),
                Err(e) => Ok(ShadowOutcome::fail(e)),
            }
        }

        // ── Impulse (rootPulse ACTION) ─────────────────────────
        "impulse.post" => {
            let root = temporal::resolve_workspace_root()?;
            let post_args = parse_impulse_post_args(args)?;
            let imp = impulse::post(&root, &post_args).await?;
            Ok(ShadowOutcome::ok_with(
                format!(
                    "FIRED [{}] {} → {}: {}",
                    imp.impulse.impulse_type,
                    imp.from.gate,
                    imp.to.gates.join(","),
                    imp.content.subject,
                ),
                serde_json::to_value(&imp)?,
            ))
        }
        "impulse.ack" => {
            let root = temporal::resolve_workspace_root()?;
            let impulse_id = require_arg(args, 0, "impulse-id")?;
            let note = extract_flag_value(args, "--note").unwrap_or("");
            let imp = impulse::ack(&root, impulse_id, note).await?;
            Ok(ShadowOutcome::ok(format!(
                "ACKED: {} (note: {})",
                imp.impulse.id,
                if note.is_empty() { "-" } else { note },
            )))
        }
        "impulse.archive" => {
            let root = temporal::resolve_workspace_root()?;
            let archived = impulse::archive(&root).await?;
            if archived.is_empty() {
                Ok(ShadowOutcome::ok("No impulses to discharge.".to_string()))
            } else {
                Ok(ShadowOutcome::ok(format!(
                    "Discharged {} impulse(s): {}",
                    archived.len(),
                    archived.join(", "),
                )))
            }
        }

        // ── Potential (quorumSignal SENSE) ─────────────────────
        "potential.sense" => {
            let root = temporal::resolve_workspace_root()?;
            let all = args.iter().any(|a| *a == "--all");
            let count_only = args.iter().any(|a| *a == "--count");
            let (impulses, count) = impulse::sense(&root, all, count_only)?;
            if count_only {
                Ok(ShadowOutcome::ok(count.to_string()))
            } else if impulses.is_empty() {
                Ok(ShadowOutcome::ok("Membrane potential: resting (no pending impulses).".to_string()))
            } else {
                let lines: Vec<String> = impulses
                    .iter()
                    .map(|(_, s)| {
                        let ack_mark = if s.meta.ack_required && s.acks.is_empty() {
                            " [NEEDS ACK]"
                        } else if !s.acks.is_empty() {
                            " [ACKED]"
                        } else {
                            ""
                        };
                        format!(
                            "  [{}] {}/{}: {}{}",
                            s.impulse.impulse_type,
                            s.from.gate,
                            s.from.team,
                            s.content.subject,
                            ack_mark,
                        )
                    })
                    .collect();
                Ok(ShadowOutcome::ok_with(
                    format!("{count} active impulse(s)\n{}", lines.join("\n")),
                    serde_json::to_value(
                        &impulses.iter().map(|(_, s)| s).collect::<Vec<_>>()
                    )?,
                ))
            }
        }
        "potential.check" => {
            let root = temporal::resolve_workspace_root()?;
            let health = impulse::check(&root)?;
            let wave_lines: Vec<String> = health
                .by_wave
                .iter()
                .map(|(w, c)| format!("  wave {w}: {c} impulse(s)"))
                .collect();
            let msg = format!(
                "Membrane potential gradient:\n\
                 Total active:    {}\n\
                 Needs ack:       {}\n\
                 Expired:         {}\n\
                 Current wave:    {}\n\
                 {}",
                health.total,
                health.needs_ack,
                health.expired,
                health.current_wave,
                if wave_lines.is_empty() { String::new() } else { format!("Volume:\n{}", wave_lines.join("\n")) },
            );
            Ok(ShadowOutcome::ok_with(msg, serde_json::to_value(&health)?))
        }

        // ── Context (sweetGrass-external braids) ─────────────
        "context.weave" => {
            let root = temporal::resolve_workspace_root()?;
            let weave_args = parse_context_weave_args(args)?;
            let braid = context::weave(&root, &weave_args).await?;
            Ok(ShadowOutcome::ok_with(
                format!(
                    "WOVEN [{status}] {gate}/{slug}: {summary}",
                    status = braid.strands.focus.status,
                    gate = braid.braid.gate,
                    slug = context_slug(&braid.braid.project),
                    summary = braid.strands.focus.summary,
                ),
                serde_json::to_value(&braid)?,
            ))
        }
        "context.sense" => {
            let root = temporal::resolve_workspace_root()?;
            let all = args.iter().any(|a| *a == "--all");
            let filter_gate = extract_flag_value(args, "--gate");
            let filter_project = extract_flag_value(args, "--project");
            let braids = context::sense(&root, filter_gate, filter_project, all)?;
            if braids.is_empty() {
                Ok(ShadowOutcome::ok("No context braids woven (resting state).".to_string()))
            } else {
                let lines: Vec<String> = braids
                    .iter()
                    .map(|b| {
                        format!(
                            "  [{status}] {gate}/{project}: {summary}",
                            status = b.strands.focus.status,
                            gate = b.braid.gate,
                            project = context_slug(&b.braid.project),
                            summary = b.strands.focus.summary,
                        )
                    })
                    .collect();
                Ok(ShadowOutcome::ok_with(
                    format!("{} context braid(s)\n{}", braids.len(), lines.join("\n")),
                    serde_json::to_value(&braids)?,
                ))
            }
        }
        "context.clear" => {
            let root = temporal::resolve_workspace_root()?;
            let project = extract_flag_value(args, "--project");
            let expired = args.iter().any(|a| *a == "--expired");
            let cleared = context::clear(&root, project, expired).await?;
            if cleared.is_empty() {
                Ok(ShadowOutcome::ok("No braids to clear.".to_string()))
            } else {
                Ok(ShadowOutcome::ok(format!(
                    "Cleared {} braid(s): {}",
                    cleared.len(),
                    cleared.join(", "),
                )))
            }
        }

        // ── Deprecated signal.* aliases ──────────────────────
        "signal.post" => {
            eprintln!("DEPRECATED: signal.post is now impulse.post (see IMPULSE_POTENTIAL_STANDARD.md)");
            let root = temporal::resolve_workspace_root()?;
            let post_args = parse_impulse_post_args(args)?;
            let imp = impulse::post(&root, &post_args).await?;
            Ok(ShadowOutcome::ok_with(
                format!(
                    "FIRED [{}] {} → {}: {}",
                    imp.impulse.impulse_type, imp.from.gate,
                    imp.to.gates.join(","), imp.content.subject,
                ),
                serde_json::to_value(&imp)?,
            ))
        }
        "signal.list" => {
            eprintln!("DEPRECATED: signal.list is now potential.sense (see IMPULSE_POTENTIAL_STANDARD.md)");
            let root = temporal::resolve_workspace_root()?;
            let all = args.iter().any(|a| *a == "--all");
            let (impulses, count) = impulse::sense(&root, all, false)?;
            if impulses.is_empty() {
                Ok(ShadowOutcome::ok("No active impulses.".to_string()))
            } else {
                let lines: Vec<String> = impulses
                    .iter()
                    .map(|(_, s)| format!("  [{}] {}/{}: {}", s.impulse.impulse_type, s.from.gate, s.from.team, s.content.subject))
                    .collect();
                Ok(ShadowOutcome::ok_with(
                    format!("{count} active impulse(s)\n{}", lines.join("\n")),
                    serde_json::to_value(&impulses.iter().map(|(_, s)| s).collect::<Vec<_>>())?,
                ))
            }
        }
        "signal.ack" => {
            eprintln!("DEPRECATED: signal.ack is now impulse.ack (see IMPULSE_POTENTIAL_STANDARD.md)");
            let root = temporal::resolve_workspace_root()?;
            let impulse_id = require_arg(args, 0, "impulse-id")?;
            let note = extract_flag_value(args, "--note").unwrap_or("");
            let imp = impulse::ack(&root, impulse_id, note).await?;
            Ok(ShadowOutcome::ok(format!("ACKED: {} (note: {})", imp.impulse.id, if note.is_empty() { "-" } else { note })))
        }
        "signal.archive" => {
            eprintln!("DEPRECATED: signal.archive is now impulse.archive (see IMPULSE_POTENTIAL_STANDARD.md)");
            let root = temporal::resolve_workspace_root()?;
            let archived = impulse::archive(&root).await?;
            if archived.is_empty() {
                Ok(ShadowOutcome::ok("No impulses to discharge.".to_string()))
            } else {
                Ok(ShadowOutcome::ok(format!("Discharged {} impulse(s): {}", archived.len(), archived.join(", "))))
            }
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

fn parse_impulse_post_args<'a>(args: &[&'a str]) -> membrane_shadow::Result<impulse::PostArgs<'a>> {
    let to_str = extract_flag_value(args, "--to")
        .ok_or_else(|| membrane_shadow::ShadowError::Parse("--to <gate> required".into()))?;
    let subject = extract_flag_value(args, "--subject")
        .ok_or_else(|| membrane_shadow::ShadowError::Parse("--subject required".into()))?;

    let type_str = extract_flag_value(args, "--type").unwrap_or("status");
    let impulse_type = match type_str {
        "frago" => impulse::ImpulseType::Frago,
        "status" => impulse::ImpulseType::Status,
        "request" => impulse::ImpulseType::Request,
        "announce" => impulse::ImpulseType::Announce,
        _ => {
            return Err(membrane_shadow::ShadowError::Parse(format!(
                "unknown impulse type: {type_str} (expected: frago|status|request|announce)"
            )));
        }
    };

    let priority_str = extract_flag_value(args, "--priority").unwrap_or("routine");
    let priority = match priority_str {
        "routine" => impulse::Priority::Routine,
        "priority" => impulse::Priority::Priority,
        "flash" => impulse::Priority::Flash,
        _ => impulse::Priority::Routine,
    };

    let to_gates: Vec<&str> = to_str.split(',').collect();

    Ok(impulse::PostArgs {
        to_gates,
        impulse_type,
        priority,
        subject,
        body: extract_flag_value(args, "--body").unwrap_or(""),
        project: extract_flag_value(args, "--project").unwrap_or(""),
        team: extract_flag_value(args, "--team").unwrap_or(""),
    })
}

fn extract_flag_value<'a>(args: &[&'a str], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| *a == flag)
        .and_then(|i| args.get(i + 1).copied())
}

fn parse_context_weave_args<'a>(args: &[&'a str]) -> membrane_shadow::Result<context::WeaveArgs<'a>> {
    let project = extract_flag_value(args, "--project")
        .ok_or_else(|| membrane_shadow::ShadowError::Parse("--project <path> required".into()))?;
    let summary = extract_flag_value(args, "--summary")
        .ok_or_else(|| membrane_shadow::ShadowError::Parse("--summary required".into()))?;

    let status_str = extract_flag_value(args, "--status").unwrap_or("active");
    let status = match status_str {
        "active" => context::FocusStatus::Active,
        "paused" => context::FocusStatus::Paused,
        "blocked" => context::FocusStatus::Blocked,
        "complete" => context::FocusStatus::Complete,
        _ => {
            return Err(membrane_shadow::ShadowError::Parse(format!(
                "unknown status: {status_str} (expected: active|paused|blocked|complete)"
            )));
        }
    };

    let ttl_str = extract_flag_value(args, "--ttl").unwrap_or("48");
    let ttl_hours: u32 = ttl_str.parse().unwrap_or(48);

    Ok(context::WeaveArgs {
        project,
        summary,
        status,
        breadcrumbs: extract_flag_value(args, "--breadcrumbs").unwrap_or(""),
        next: extract_flag_value(args, "--next").unwrap_or(""),
        blockers: extract_flag_value(args, "--blockers").unwrap_or(""),
        notes: extract_flag_value(args, "--notes").unwrap_or(""),
        ttl_hours,
    })
}

fn context_slug(project: &str) -> String {
    project
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}
