// SPDX-License-Identifier: AGPL-3.0-or-later

//! Data domain dispatch — manifest, identity, context, plasmid, relay, content.
//!
//! These domains handle local data operations (manifest reading, context braids,
//! binary fetching, K-Derm relay chain, content verification).

use crate::cli;
use crate::error::ShadowError;
use crate::{ShadowConfig, ShadowOutcome, context, identity, manifest, plasmid, relay, temporal};

// ── Manifest domain ──────────────────────────────────────────────────

pub(super) fn dispatch_manifest(cmd: &str, args: &[&str]) -> crate::Result<ShadowOutcome> {
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
            let header = args.first().map_or_else(
                || format!("{} repos total", repos.len()),
                |g| format!("{} repos for gate {g}", repos.len()),
            );
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

pub(super) fn dispatch_identity() -> crate::Result<ShadowOutcome> {
    let root = temporal::resolve_workspace_root()?;
    match identity::resolve(&root) {
        Ok(id) => Ok(ShadowOutcome::ok_with(
            format!("{} (via {:?})", id.name, id.source),
            serde_json::to_value(&id)?,
        )),
        Err(e) => Ok(ShadowOutcome::fail(e)),
    }
}

// ── Context domain (sweetGrass-external braids) ──────────────────────

pub(super) async fn dispatch_context(cmd: &str, args: &[&str]) -> crate::Result<ShadowOutcome> {
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

pub(super) async fn dispatch_plasmid(
    config: &ShadowConfig,
    cmd: &str,
    args: &[&str],
) -> crate::Result<ShadowOutcome> {
    match cmd {
        "plasmid.fetch" => {
            let source_str = cli::extract_flag_value(args, "--source").unwrap_or("github");
            let source: plasmid::FetchSource = source_str.parse()?;
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
        "plasmid.refresh" => {
            let refresh_args = plasmid::RefreshArgs {
                primal: cli::extract_flag_value(args, "--primal").map(Into::into),
                dry_run: args.contains(&"--dry-run"),
                source_dir: cli::extract_flag_value(args, "--source-dir").map(Into::into),
            };
            plasmid::refresh(config, &refresh_args).await
        }
        "plasmid.harvest" => {
            let harvest_args = plasmid::HarvestArgs {
                primal: cli::extract_flag_value(args, "--primal").map(Into::into),
                force: args.contains(&"--force"),
                dry_run: args.contains(&"--dry-run"),
                depot_dir: cli::extract_flag_value(args, "--depot").map(Into::into),
                target: cli::extract_flag_value(args, "--target").map(Into::into),
            };
            plasmid::harvest(&harvest_args).await
        }
        "plasmid.build" => {
            let primal = cli::extract_flag_value(args, "--primal")
                .or_else(|| args.iter().find(|a| !a.starts_with('-')).copied());
            let Some(primal) = primal else {
                return Err(ShadowError::Parse(
                    "plasmid.build requires --primal <name> or positional primal name".into(),
                ));
            };
            let build_args = plasmid::BuildArgs {
                primal: primal.to_string(),
                target: cli::extract_flag_value(args, "--target").map(Into::into),
                depot_dir: cli::extract_flag_value(args, "--depot").map(Into::into),
                dry_run: args.contains(&"--dry-run"),
            };
            plasmid::build::build(&build_args).await
        }
        "plasmid.pipeline" => {
            let primal = cli::extract_flag_value(args, "--primal");
            let dry_run = args.contains(&"--dry-run");
            plasmid::pipeline(config, primal, dry_run).await
        }
        "plasmid.ndk.check" => Ok(plasmid::ndk_check()),
        "plasmid.trigger" => plasmid::trigger(config).await,
        "plasmid.depot_sync" => plasmid::depot_sync(config).await,
        "plasmid.status" => plasmid::status().await,
        "plasmid.staleness" => match plasmid::detect_depot_staleness() {
            Ok(report) => {
                let stale_names: Vec<&str> = report
                    .entries
                    .iter()
                    .filter(|e| e.stale)
                    .map(|e| e.name.as_str())
                    .collect();
                Ok(ShadowOutcome::ok_with(
                    report.to_string(),
                    serde_json::json!({
                        "total": report.total,
                        "current": report.current_count,
                        "stale": report.stale_count,
                        "stale_primals": stale_names,
                    }),
                ))
            }
            Err(e) => Err(e),
        },
        _ => Ok(ShadowOutcome::fail(format!(
            "unknown plasmid command: {cmd}"
        ))),
    }
}

// ── Relay domain (K-Derm relay chain) ────────────────────────────────

pub(super) async fn dispatch_relay(cmd: &str, args: &[&str]) -> crate::Result<ShadowOutcome> {
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
        "relay.status" => relay_status().await,
        _ => Ok(ShadowOutcome::fail(format!("unknown relay command: {cmd}"))),
    }
}

async fn relay_status() -> crate::Result<ShadowOutcome> {
    let relay_cfg = relay::RelayConfig::from_env();
    let root = temporal::resolve_workspace_root()?;
    let m = manifest::load_from_workspace(&root)?;

    let ext_host = &relay_cfg.golgi_ext_host;
    let ssh_ok_ext = crate::ssh::check_connectivity(ext_host).await;

    let repo_count = m.repos.len();
    let topology = m.topology.as_ref().map_or("unknown", |t| t.model.as_str());

    let msg = format!(
        "=== Relay Chain Status ===\n\
         Topology:      {topology}\n\
         Ext host:      {ext_host} (SSH: {})\n\
         Forgejo remote: {}\n\
         Workspace:     {}\n\
         Repos:         {repo_count}\n\
         Relay mode:    Rust-native (membrane relay.run)",
        if ssh_ok_ext { "OK" } else { "FAIL" },
        relay_cfg.forgejo_remote,
        relay_cfg.ecoprimals_root.display(),
    );

    Ok(if ssh_ok_ext {
        ShadowOutcome::ok_with(
            msg,
            serde_json::json!({
                "ext_host": ext_host,
                "ext_ssh": ssh_ok_ext,
                "forgejo_remote": relay_cfg.forgejo_remote,
                "workspace": relay_cfg.ecoprimals_root.to_string_lossy(),
                "repo_count": repo_count,
                "topology": topology,
            }),
        )
    } else {
        ShadowOutcome {
            ok: false,
            message: msg,
            data: Some(serde_json::json!({
                "ext_host": ext_host,
                "ext_ssh": ssh_ok_ext,
            })),
        }
    })
}

// ── Content domain (S3 sporePrint content integrity) ─────────────────

pub(super) async fn dispatch_content(
    config: &ShadowConfig,
    cmd: &str,
    _args: &[&str],
) -> crate::Result<ShadowOutcome> {
    match cmd {
        "content.verify" => {
            let (caddy_out, caddy_code) =
                crate::ssh::exec_raw(config, "systemctl is-active caddy-tls").await?;
            let caddy_active = caddy_code == 0;

            let (nestgate_out, nestgate_code) =
                crate::ssh::exec_raw(config, "systemctl is-active nestgate-membrane").await?;
            let nestgate_active = nestgate_code == 0;

            let content_path =
                std::env::var(cellmembrane_types::service::ENV_NESTGATE_CONTENT_PATH)
                    .unwrap_or_else(|_| {
                        format!(
                            "{}/nestgate/content",
                            std::env::var(cellmembrane_types::service::ENV_INSTALL_BASE)
                                .unwrap_or_else(|_| {
                                    cellmembrane_types::service::DEFAULT_INSTALL_BASE.into()
                                })
                        )
                    });
            let (content_count_out, _) = crate::ssh::exec_raw(
                config,
                &format!("find {content_path} -type f 2>/dev/null | wc -l"),
            )
            .await?;
            let content_files: u32 = content_count_out.trim().parse().unwrap_or(0);

            let nestgate_port = std::env::var(cellmembrane_types::service::ENV_NESTGATE_PORT)
                .ok()
                .and_then(|v| v.parse::<u16>().ok())
                .unwrap_or(9500);
            let bind = std::env::var(cellmembrane_types::service::ENV_NUCLEUS_BIND)
                .unwrap_or_else(|_| "127.0.0.1".into());
            let (curl_out, curl_code) = crate::ssh::exec_raw(
                config,
                &format!("curl -s -o /dev/null -w '%{{http_code}}' http://{bind}:{nestgate_port}/health 2>/dev/null"),
            )
            .await?;
            let nestgate_http = curl_out.trim().to_string();
            let nestgate_responding = curl_code == 0 && nestgate_http == "200";

            let status = if caddy_active && nestgate_active && nestgate_responding {
                "READY"
            } else {
                "NOT READY"
            };

            let msg = format!(
                "=== S3 Content Verification ===\n\
                 Status:         {status}\n\
                 Caddy TLS:      {} ({})\n\
                 NestGate:       {} ({})\n\
                 NestGate HTTP:  {} ({bind}:{nestgate_port}/health)\n\
                 Content files:  {content_files}",
                if caddy_active { "active" } else { "inactive" },
                caddy_out.trim(),
                if nestgate_active {
                    "active"
                } else {
                    "inactive"
                },
                nestgate_out.trim(),
                if nestgate_responding {
                    "200 OK"
                } else {
                    &nestgate_http
                },
            );

            let ok = caddy_active && nestgate_active && nestgate_responding;
            Ok(if ok {
                ShadowOutcome::ok_with(
                    msg,
                    serde_json::json!({
                        "status": status,
                        "caddy": caddy_active,
                        "nestgate": nestgate_active,
                        "nestgate_http": nestgate_http,
                        "content_files": content_files,
                    }),
                )
            } else {
                ShadowOutcome {
                    ok: false,
                    message: msg,
                    data: Some(serde_json::json!({
                        "status": status,
                        "caddy": caddy_active,
                        "nestgate": nestgate_active,
                        "nestgate_http": nestgate_http,
                        "content_files": content_files,
                    })),
                }
            })
        }
        _ => Ok(ShadowOutcome::fail(format!(
            "unknown content command: {cmd}"
        ))),
    }
}
