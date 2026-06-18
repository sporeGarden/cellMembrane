// SPDX-License-Identifier: AGPL-3.0-or-later

//! Data domain dispatch — manifest, identity, context.
//!
//! Plasmid, relay, and content domains are in their own modules
//! (`plasmid_dispatch`, `relay_dispatch`, `content_dispatch`).

use crate::cli;
use crate::error::ShadowError;
use crate::{ShadowOutcome, context, identity, manifest, temporal};

// ── Manifest domain ──────────────────────────────────────────────────

pub(super) async fn dispatch_manifest(cmd: &str, args: &[&str]) -> crate::Result<ShadowOutcome> {
    let root = temporal::resolve_workspace_root()?;
    match cmd {
        "manifest.info" => {
            let m = manifest::load_from_workspace_async(&root).await?;
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
            let m = manifest::load_from_workspace_async(&root).await?;
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
            let m = manifest::load_from_workspace_async(&root).await?;
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

pub(super) async fn dispatch_identity() -> crate::Result<ShadowOutcome> {
    let root = temporal::resolve_workspace_root()?;
    match identity::resolve_async(&root).await {
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
            let filter_gate = cli::extract_flag_value(args, "--gate").map(str::to_owned);
            let filter_project = cli::extract_flag_value(args, "--project").map(str::to_owned);
            let braids = {
                let root = root.clone();
                let fg = filter_gate.clone();
                let fp = filter_project.clone();
                tokio::task::spawn_blocking(move || {
                    context::sense(&root, fg.as_deref(), fp.as_deref(), all)
                })
                .await
                .map_err(|e| ShadowError::Config(format!("spawn_blocking: {e}")))?
            }?;
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
