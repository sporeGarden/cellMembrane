// SPDX-License-Identifier: AGPL-3.0-or-later

//! Relay domain dispatch — K-Derm relay chain operations.

use crate::{ShadowOutcome, manifest, relay, temporal};

/// Resolve all repo `local_path` values from the ecosystem manifest.
///
/// Falls back to just wateringHole if the manifest can't be loaded.
fn resolve_all_repo_paths() -> Vec<&'static str> {
    let Ok(root) = temporal::resolve_workspace_root() else {
        return vec![cellmembrane_types::service::INFRA_WATERING_HOLE];
    };
    let Ok(m) = manifest::load_from_workspace(&root) else {
        return vec![cellmembrane_types::service::INFRA_WATERING_HOLE];
    };
    let paths: Vec<&'static str> = m
        .repos
        .values()
        .map(|e| &*Box::leak(e.local_path.clone().into_boxed_str()))
        .collect();
    if paths.is_empty() {
        vec![cellmembrane_types::service::INFRA_WATERING_HOLE]
    } else {
        paths
    }
}

pub(super) async fn dispatch_relay(cmd: &str, args: &[&str]) -> crate::Result<ShadowOutcome> {
    match cmd {
        "relay.run" => {
            let config = relay::RelayConfig::from_env();
            let result = relay::run(&config, args).await?;
            let summary = format!(
                "relay complete: absorbed={} pulled={} pushed={} impulses={} failures={}",
                result.absorbed.len(),
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
        "relay.absorb" => {
            let config = relay::RelayConfig::from_env();
            let paths: Vec<&str> = if args.is_empty() {
                resolve_all_repo_paths()
            } else {
                args.to_vec()
            };
            let absorbed = relay::absorb_extracellular(&config, &paths).await;
            let summary = format!(
                "absorb: {} repos synced from GitHub → Forgejo",
                absorbed.len()
            );
            Ok(ShadowOutcome::ok_with(
                summary,
                serde_json::json!({ "absorbed": absorbed }),
            ))
        }
        "relay.mediate" => {
            let config = relay::RelayConfig::from_env();
            let paths: Vec<&str> = if args.is_empty() {
                vec![cellmembrane_types::service::INFRA_WATERING_HOLE]
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
                vec![cellmembrane_types::service::INFRA_WATERING_HOLE]
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
        "relay.parity" => dispatch_parity(args).await,
        _ => Ok(ShadowOutcome::fail(format!("unknown relay command: {cmd}"))),
    }
}

async fn dispatch_parity(args: &[&str]) -> crate::Result<ShadowOutcome> {
    let config = relay::RelayConfig::from_env();
    let paths = if args.is_empty() {
        resolve_all_repo_paths()
    } else {
        args.to_vec()
    };
    let report = relay::check_parity(&config, &paths).await;
    let divergent_count = report.iter().filter(|r| !r.at_parity).count();
    let summary = if divergent_count == 0 {
        format!("parity: all {} repos at parity", report.len())
    } else {
        format!("parity: {divergent_count}/{} repos DIVERGED", report.len())
    };
    let ok = divergent_count == 0;
    let mut outcome = if ok {
        ShadowOutcome::ok_with(summary, serde_json::to_value(&report)?)
    } else {
        ShadowOutcome {
            ok: false,
            message: summary,
            data: serde_json::to_value(&report).ok(),
        }
    };
    let lines: Vec<String> = report
        .iter()
        .map(|r| {
            let mark = if r.at_parity { "✓" } else { "✗" };
            format!("  [{mark}] {}: {}", r.repo, r.detail)
        })
        .collect();
    if !lines.is_empty() {
        outcome.message = format!("{}\n{}", outcome.message, lines.join("\n"));
    }
    Ok(outcome)
}

async fn relay_status() -> crate::Result<ShadowOutcome> {
    let relay_cfg = relay::RelayConfig::from_env();
    let root = temporal::resolve_workspace_root()?;
    let m = manifest::load_from_workspace_async(&root).await?;

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
