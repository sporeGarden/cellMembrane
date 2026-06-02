// SPDX-License-Identifier: AGPL-3.0-or-later

//! Data domain dispatch — manifest, identity, context, plasmid, relay.
//!
//! These domains handle local data operations (manifest reading, context braids,
//! binary fetching, K-Derm relay chain).

use crate::cli;
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
        _ => Ok(ShadowOutcome::fail(format!("unknown relay command: {cmd}"))),
    }
}
