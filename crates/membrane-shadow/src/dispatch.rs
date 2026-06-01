// SPDX-License-Identifier: AGPL-3.0-or-later

//! Command dispatch — routes `domain.operation` strings to typed handlers.
//!
//! Each domain group returns `Result<ShadowOutcome>` — the caller (main.rs)
//! handles JSON vs human output formatting.

use crate::cli::{self, TapMessage};
use crate::{
    ShadowConfig, ShadowOutcome, context, forgejo, gate, identity, impulse, manifest, plasmid,
    relay, service, temporal,
};

/// Dispatch a CLI command to the appropriate shadow function.
///
/// Returns `Ok(ShadowOutcome)` for both success and domain-level failures.
/// Returns `Err` only for infrastructure failures (SSH, parse, etc.).
pub async fn run(config: &ShadowConfig, cmd: &str, args: &[&str]) -> crate::Result<ShadowOutcome> {
    match cmd {
        c if c.starts_with("repo.") => dispatch_repo(config, cmd, args).await,
        c if c.starts_with("mirror.") => dispatch_mirror(config, cmd, args).await,
        c if c.starts_with("service.") => dispatch_service(config, cmd, args).await,
        c if c.starts_with("gate.") => dispatch_gate(config, cmd, args).await,
        c if c.starts_with("token.") => dispatch_token(config, cmd, args).await,
        c if c.starts_with("temporal.") => dispatch_temporal(config, cmd, args).await,
        c if c.starts_with("manifest.") => dispatch_manifest(cmd, args).await,
        "identity.resolve" => dispatch_identity().await,
        c if c.starts_with("impulse.") => dispatch_impulse(cmd, args).await,
        c if c.starts_with("potential.") => dispatch_potential(cmd, args).await,
        c if c.starts_with("context.") => dispatch_context(cmd, args).await,
        // Deprecated signal.* → impulse.*/potential.*
        c if c.starts_with("signal.") => dispatch_signal_deprecated(cmd, args).await,
        c if c.starts_with("plasmid.") => dispatch_plasmid(config, cmd, args).await,
        c if c.starts_with("relay.") => dispatch_relay(cmd, args).await,
        "forgejo.version" => {
            let v = forgejo::version(config).await?;
            Ok(ShadowOutcome::ok(v))
        }
        _ => Ok(ShadowOutcome::fail(format!("unknown command: {cmd}"))),
    }
}

// ── Repo domain ──────────────────────────────────────────────────────

async fn dispatch_repo(
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

async fn dispatch_mirror(
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

// ── Service domain ───────────────────────────────────────────────────

async fn dispatch_service(
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

async fn dispatch_gate(
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
        _ => Ok(ShadowOutcome::fail(format!("unknown gate command: {cmd}"))),
    }
}

// ── Token domain ─────────────────────────────────────────────────────

async fn dispatch_token(
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

// ── Temporal domain ──────────────────────────────────────────────────

async fn dispatch_temporal(
    config: &ShadowConfig,
    cmd: &str,
    args: &[&str],
) -> crate::Result<ShadowOutcome> {
    match cmd {
        "temporal.check" => {
            let root = temporal::resolve_workspace_root()?;
            if args.is_empty() {
                return Err(crate::ShadowError::Parse(
                    "temporal.check requires at least one repo path".into(),
                ));
            }
            let mut matrices = Vec::with_capacity(args.len());
            for path in args {
                matrices.push(temporal::check(&root, path).await?);
            }
            let lines: Vec<String> = matrices.iter().map(ToString::to_string).collect();
            let parity = matrices
                .iter()
                .filter(|m| m.classification == temporal::SyncClassification::Parity)
                .count();
            Ok(ShadowOutcome::ok_with(
                format!("{}/{} parity\n{}", parity, matrices.len(), lines.join("\n")),
                serde_json::to_value(&matrices)?,
            ))
        }
        "temporal.sync" => {
            let root = temporal::resolve_workspace_root()?;
            if args.is_empty() {
                return Err(crate::ShadowError::Parse(
                    "temporal.sync requires at least one repo path".into(),
                ));
            }
            let push_target = manifest::load_from_workspace(&root)
                .map_or_else(|_| "all".into(), |m| m.sync.push_target);
            let mut results = Vec::with_capacity(args.len());
            let mut synced = 0u32;
            let mut failed = 0u32;
            for path in args {
                let r = temporal::sync_with_target(&root, path, &push_target).await?;
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
                format!("synced={synced} failed={failed}\n{}", lines.join("\n")),
                serde_json::to_value(&results)?,
            ))
        }
        "temporal.cascade" => dispatch_cascade(config, args).await,
        _ => Ok(ShadowOutcome::fail(format!(
            "unknown temporal command: {cmd}"
        ))),
    }
}

/// `temporal.cascade` — thin dispatch to `temporal::cascade`.
async fn dispatch_cascade(_config: &ShadowConfig, args: &[&str]) -> crate::Result<ShadowOutcome> {
    let root = temporal::resolve_workspace_root()?;

    let gate_name = cli::extract_flag_value(args, "--gate")
        .or_else(|| std::env::var("GATE_NAME").ok().as_deref().map(|_| ""))
        .unwrap_or("auto");

    let gate_name = if gate_name == "auto" || gate_name.is_empty() {
        identity::resolve(&root).map_or_else(|_| "eastGate".into(), |id| id.name)
    } else {
        gate_name.to_string()
    };

    let source = cli::extract_flag_value(args, "--source").unwrap_or("temporal");
    let check_only = args.contains(&"--check");
    let clone_missing = args.contains(&"--clone-missing");
    let dry_run = args.contains(&"--dry-run");

    temporal::cascade(&gate_name, source, check_only, clone_missing, dry_run).await
}

// ── Manifest domain ──────────────────────────────────────────────────

async fn dispatch_manifest(cmd: &str, args: &[&str]) -> crate::Result<ShadowOutcome> {
    let root = temporal::resolve_workspace_root()?;
    match cmd {
        "manifest.info" => {
            let m = manifest::load_from_workspace(&root)?;
            let topo = m.topology.as_ref().map_or_else(
                || "monoderm (no topology section)".to_string(),
                |t| {
                    let roles = t.roles.as_ref().map_or_else(
                        || "no roles assigned".to_string(),
                        |r| {
                            format!(
                                "receiver={} mediator={} publisher={}",
                                r.push_receiver, r.sync_mediator, r.external_publisher
                            )
                        },
                    );
                    format!(
                        "{}: {} → {} → {} ({})",
                        t.model, t.inner_membrane, t.peptidoglycan, t.outer_membrane, roles
                    )
                },
            );
            let msg = format!(
                "manifest v{} wave {} ({} repos)\n\
                 sync: source={} branch={} push_target={} divergence={}\n\
                 topology: {}",
                m.meta.version,
                m.meta.wave,
                m.meta.total_repos,
                m.sync.default_source,
                m.sync.default_branch,
                m.sync.push_target,
                m.sync.divergence_policy,
                topo,
            );
            Ok(ShadowOutcome::ok_with(msg, serde_json::to_value(&m)?))
        }
        "manifest.repos" => {
            let m = manifest::load_from_workspace(&root)?;
            let repos: Vec<(&str, &manifest::RepoEntry)> = if let Some(gate_name) = args.first() {
                m.gate_repos(gate_name)
            } else {
                m.repos.iter().map(|(n, e)| (n.as_str(), e)).collect()
            };
            let lines: Vec<String> = repos
                .iter()
                .map(|(name, e)| {
                    format!(
                        "  {:<25} {:<30} {:<18} {}",
                        name, e.local_path, e.membrane, e.category
                    )
                })
                .collect();
            let header = if let Some(g) = args.first() {
                format!("{} repos for gate {g}", repos.len())
            } else {
                format!("{} repos total", repos.len())
            };
            Ok(ShadowOutcome::ok(format!("{header}\n{}", lines.join("\n"))))
        }
        "manifest.orgs" => {
            let m = manifest::load_from_workspace(&root)?;
            let orgs = m.orgs();
            Ok(ShadowOutcome::ok(format!(
                "{} orgs: {}",
                orgs.len(),
                orgs.join(", ")
            )))
        }
        _ => Ok(ShadowOutcome::fail(format!(
            "unknown manifest command: {cmd}"
        ))),
    }
}

// ── Identity domain ──────────────────────────────────────────────────

async fn dispatch_identity() -> crate::Result<ShadowOutcome> {
    let root = temporal::resolve_workspace_root()?;
    match identity::resolve(&root) {
        Ok(id) => Ok(ShadowOutcome::ok_with(
            format!("{} (via {:?})", id.name, id.source),
            serde_json::to_value(&id)?,
        )),
        Err(e) => Ok(ShadowOutcome::fail(e)),
    }
}

// ── Impulse domain (rootPulse ACTION) ────────────────────────────────

async fn dispatch_impulse(cmd: &str, args: &[&str]) -> crate::Result<ShadowOutcome> {
    let root = temporal::resolve_workspace_root()?;
    match cmd {
        "impulse.post" => {
            let post_args = cli::parse_impulse_post_args(args)?;
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
            let impulse_id = cli::require_arg(args, 0, "impulse-id")?;
            let note = cli::extract_flag_value(args, "--note").unwrap_or("");
            let imp = impulse::ack(&root, impulse_id, note).await?;
            Ok(ShadowOutcome::ok(format!(
                "ACKED: {} (note: {})",
                imp.impulse.id,
                if note.is_empty() { "-" } else { note },
            )))
        }
        "impulse.archive" => {
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
        _ => Ok(ShadowOutcome::fail(format!(
            "unknown impulse command: {cmd}"
        ))),
    }
}

// ── Potential domain (quorumSignal SENSE) ────────────────────────────

async fn dispatch_potential(cmd: &str, args: &[&str]) -> crate::Result<ShadowOutcome> {
    let root = temporal::resolve_workspace_root()?;
    match cmd {
        "potential.sense" => {
            let all = args.contains(&"--all");
            let count_only = args.contains(&"--count");
            let (impulses, count) = impulse::sense(&root, all, count_only)?;
            if count_only {
                Ok(ShadowOutcome::ok(count.to_string()))
            } else if impulses.is_empty() {
                Ok(ShadowOutcome::ok(
                    "Membrane potential: resting (no pending impulses).".to_string(),
                ))
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
                    serde_json::to_value(&impulses.iter().map(|(_, s)| s).collect::<Vec<_>>())?,
                ))
            }
        }
        "potential.check" => {
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
                if wave_lines.is_empty() {
                    String::new()
                } else {
                    format!("Volume:\n{}", wave_lines.join("\n"))
                },
            );
            Ok(ShadowOutcome::ok_with(msg, serde_json::to_value(&health)?))
        }
        _ => Ok(ShadowOutcome::fail(format!(
            "unknown potential command: {cmd}"
        ))),
    }
}

// ── Context domain (sweetGrass-external braids) ──────────────────────

async fn dispatch_context(cmd: &str, args: &[&str]) -> crate::Result<ShadowOutcome> {
    let root = temporal::resolve_workspace_root()?;
    match cmd {
        "context.weave" => {
            let weave_args = cli::parse_context_weave_args(args)?;
            let braid = context::weave(&root, &weave_args).await?;
            Ok(ShadowOutcome::ok_with(
                format!(
                    "WOVEN [{status}] {gate}/{slug}: {summary}",
                    status = braid.strands.focus.status,
                    gate = braid.braid.gate,
                    slug = cli::context_slug(&braid.braid.project),
                    summary = braid.strands.focus.summary,
                ),
                serde_json::to_value(&braid)?,
            ))
        }
        "context.sense" => {
            let all = args.contains(&"--all");
            let filter_gate = cli::extract_flag_value(args, "--gate");
            let filter_project = cli::extract_flag_value(args, "--project");
            let braids = context::sense(&root, filter_gate, filter_project, all)?;
            if braids.is_empty() {
                Ok(ShadowOutcome::ok(
                    "No context braids woven (resting state).".to_string(),
                ))
            } else {
                let lines: Vec<String> = braids
                    .iter()
                    .map(|b| {
                        format!(
                            "  [{status}] {gate}/{project}: {summary}",
                            status = b.strands.focus.status,
                            gate = b.braid.gate,
                            project = cli::context_slug(&b.braid.project),
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
            let project = cli::extract_flag_value(args, "--project");
            let expired = args.contains(&"--expired");
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
        _ => Ok(ShadowOutcome::fail(format!(
            "unknown context command: {cmd}"
        ))),
    }
}

// ── Plasmid domain ───────────────────────────────────────────────────

async fn dispatch_plasmid(
    config: &ShadowConfig,
    cmd: &str,
    args: &[&str],
) -> crate::Result<ShadowOutcome> {
    match cmd {
        "plasmid.fetch" => {
            let source_str = cli::extract_flag_value(args, "--source").unwrap_or("github");
            let source = plasmid::FetchSource::from_str(source_str)?;
            let fetch_args = plasmid::FetchArgs {
                source,
                primal: cli::extract_flag_value(args, "--primal").map(Into::into),
                release_tag: cli::extract_flag_value(args, "--release").map(Into::into),
                force: args.contains(&"--force"),
                dry_run: args.contains(&"--dry-run"),
                dest: cli::extract_flag_value(args, "--dest").map(Into::into),
            };
            plasmid::fetch(config, &fetch_args).await
        }
        _ => Ok(ShadowOutcome::fail(format!(
            "unknown plasmid command: {cmd}"
        ))),
    }
}

// ── Deprecated signal.* aliases ──────────────────────────────────────

async fn dispatch_signal_deprecated(cmd: &str, args: &[&str]) -> crate::Result<ShadowOutcome> {
    let new_cmd = match cmd {
        "signal.post" => "impulse.post",
        "signal.ack" => "impulse.ack",
        "signal.archive" => "impulse.archive",
        "signal.list" => "potential.sense",
        _ => {
            return Ok(ShadowOutcome::fail(format!(
                "unknown signal command: {cmd}"
            )));
        }
    };
    eprintln!("DEPRECATED: {cmd} → {new_cmd} (see IMPULSE_POTENTIAL_STANDARD.md)");

    match new_cmd {
        "impulse.post" => dispatch_impulse(new_cmd, args).await,
        "impulse.ack" => dispatch_impulse(new_cmd, args).await,
        "impulse.archive" => dispatch_impulse(new_cmd, args).await,
        "potential.sense" => dispatch_potential(new_cmd, args).await,
        _ => unreachable!(),
    }
}

// ── Relay domain (K-Derm relay chain) ────────────────────────────────

async fn dispatch_relay(cmd: &str, args: &[&str]) -> crate::Result<ShadowOutcome> {
    match cmd {
        "relay.run" => {
            let config = relay::RelayConfig::from_env();
            let result = relay::run(&config, args).await?;
            let summary = format!(
                "relay complete: pulled={} pushed={} impulses={} failures={}",
                result.pulled.len(),
                result.pushed.len(),
                result.impulses_sensed,
                result.pull_failures.len() + result.push_failures.len(),
            );
            Ok(ShadowOutcome::ok_with(
                summary,
                serde_json::to_value(&result)?,
            ))
        }
        "relay.mediate" => {
            let config = relay::RelayConfig::from_env();
            let paths: Vec<&str> = if args.is_empty() {
                vec!["infra/wateringHole"]
            } else {
                args.to_vec()
            };
            let (pulled, failures) = relay::mediate(&config, &paths).await;
            let summary = format!(
                "mediate: pulled={} failures={}",
                pulled.len(),
                failures.len()
            );
            Ok(ShadowOutcome::ok_with(
                summary,
                serde_json::json!({
                    "pulled": pulled,
                    "failures": failures,
                }),
            ))
        }
        "relay.ship" => {
            let config = relay::RelayConfig::from_env();
            let paths: Vec<&str> = if args.is_empty() {
                vec!["infra/wateringHole"]
            } else {
                args.to_vec()
            };
            let (pushed, skipped, failures) = relay::ship_extracellular(&config, &paths).await;
            let summary = format!(
                "ship: pushed={} skipped={} failures={}",
                pushed.len(),
                skipped.len(),
                failures.len()
            );
            Ok(ShadowOutcome::ok_with(
                summary,
                serde_json::json!({
                    "pushed": pushed,
                    "skipped": skipped,
                    "failures": failures,
                }),
            ))
        }
        _ => Ok(ShadowOutcome::fail(format!("unknown relay command: {cmd}"))),
    }
}
