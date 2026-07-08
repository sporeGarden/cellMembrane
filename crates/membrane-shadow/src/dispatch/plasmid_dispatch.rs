// SPDX-License-Identifier: AGPL-3.0-or-later

//! Plasmid domain dispatch — fetch, harvest, build, pipeline, sandbox, canary.

use crate::cli;
use crate::error::ShadowError;
use crate::{ShadowConfig, ShadowOutcome, plasmid};

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
            let all = args.contains(&"--all");
            let harvest_args = plasmid::HarvestArgs {
                primal: if all {
                    None
                } else {
                    cli::extract_flag_value(args, "--primal").map(Into::into)
                },
                force: args.contains(&"--force") || all,
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
                return Err(ShadowError::Config(
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
        "plasmid.auto_fetch" => {
            let payload_str = args.first().copied().unwrap_or("{}");
            let payload: serde_json::Value =
                serde_json::from_str(payload_str).unwrap_or(serde_json::Value::Object(Default::default()));
            let notif = plasmid::auto_fetch::DepotUpdatedNotification::from_json(&payload)
                .unwrap_or(plasmid::auto_fetch::DepotUpdatedNotification {
                    primals_updated: Vec::new(),
                    manifest_hash: None,
                    builder: "manual".into(),
                });
            plasmid::auto_fetch::handle_depot_updated(&notif).await
        }
        c if c.starts_with("plasmid.sandbox") || c.starts_with("plasmid.canary") => {
            dispatch_plasmid_lifecycle(cmd, args).await
        }
        _ => Ok(ShadowOutcome::fail(format!(
            "unknown plasmid command: {cmd}"
        ))),
    }
}

async fn dispatch_plasmid_lifecycle(cmd: &str, args: &[&str]) -> crate::Result<ShadowOutcome> {
    match cmd {
        "plasmid.sandbox" => dispatch_sandbox_validate(args).await,
        "plasmid.sandbox.list" => {
            let instances = plasmid::sandbox::list_active();
            let msg = if instances.is_empty() {
                "no active sandboxes".to_string()
            } else {
                format!("{} active sandbox(es)", instances.len())
            };
            let data: Vec<serde_json::Value> = instances
                .iter()
                .map(|i| {
                    serde_json::json!({
                        "primal": i.primal,
                        "commit": i.commit,
                        "socket": i.socket_path.display().to_string(),
                    })
                })
                .collect();
            Ok(ShadowOutcome::ok_with(msg, serde_json::json!(data)))
        }
        "plasmid.canary.list" => {
            let slots = plasmid::canary::list().await;
            let msg = if slots.is_empty() {
                "canary pool empty".to_string()
            } else {
                format!("{} canary slot(s)", slots.len())
            };
            Ok(ShadowOutcome::ok_with(
                msg,
                serde_json::to_value(&slots).unwrap_or_default(),
            ))
        }
        "plasmid.canary.health" => {
            let results = plasmid::canary::canary_health_watch().await;
            let alive = results.iter().filter(|r| r.alive).count();
            let msg = format!("{alive}/{} canaries healthy", results.len());
            Ok(ShadowOutcome::ok_with(
                msg,
                serde_json::to_value(&results).unwrap_or_default(),
            ))
        }
        "plasmid.canary.promote" => {
            let primal = cli::extract_flag_value(args, "--primal").ok_or_else(|| {
                ShadowError::Config("plasmid.canary.promote requires --primal".into())
            })?;
            let install_dir = cellmembrane_types::service::env_or(
                cellmembrane_types::service::ENV_INSTALL_BASE,
                cellmembrane_types::service::DEFAULT_INSTALL_BASE,
            );
            let production_path = std::path::PathBuf::from(&install_dir).join(primal);

            match plasmid::canary::promote_canary(primal, &production_path).await {
                Ok(slot) => Ok(ShadowOutcome::ok(format!(
                    "canary promoted: {} (commit {}) → production",
                    slot.primal, slot.commit
                ))),
                Err(e) => Ok(ShadowOutcome::fail(format!("canary promote failed: {e}"))),
            }
        }
        "plasmid.canary.failover" => {
            let targets = plasmid::canary::failover_targets().await;
            let msg = format!("{} healthy canary failover targets", targets.len());
            Ok(ShadowOutcome::ok_with(
                msg,
                serde_json::to_value(&targets).unwrap_or_default(),
            ))
        }
        "plasmid.canary.audit" => {
            let auto_refresh = args.contains(&"--refresh");
            let reports = plasmid::canary::staleness_audit(auto_refresh).await;
            let stale_count = reports.iter().filter(|r| r.stale).count();
            let msg = if reports.is_empty() {
                "canary pool empty — nothing to audit".to_string()
            } else if stale_count == 0 {
                format!("{} canary(s) — all fresh", reports.len())
            } else if auto_refresh {
                format!("{stale_count}/{} stale canary(s) removed", reports.len())
            } else {
                format!(
                    "{stale_count}/{} stale (use --refresh to remove)",
                    reports.len()
                )
            };
            Ok(ShadowOutcome::ok_with(
                msg,
                serde_json::to_value(&reports).unwrap_or_default(),
            ))
        }
        "plasmid.canary.teardown" => {
            plasmid::canary::teardown_all().await;
            Ok(ShadowOutcome::ok("all canary instances terminated"))
        }
        _ => Ok(ShadowOutcome::fail(format!(
            "unknown lifecycle command: {cmd}"
        ))),
    }
}

async fn dispatch_sandbox_validate(args: &[&str]) -> crate::Result<ShadowOutcome> {
    let primal = cli::extract_flag_value(args, "--primal")
        .ok_or_else(|| ShadowError::Config("plasmid.sandbox requires --primal".into()))?;
    let commit = cli::extract_flag_value(args, "--commit")
        .unwrap_or("HEAD")
        .to_string();
    let timeout = cli::extract_flag_value(args, "--timeout").and_then(|s| s.parse::<u64>().ok());

    let arch = plasmid::detect_target_triple();
    let depot_dir = std::env::var(cellmembrane_types::service::ENV_PLASMIDBIN_DEPOT).map_or_else(
        |_| {
            crate::resolve_xdg_data_home()
                .join("ecoPrimals")
                .join(cellmembrane_types::service::PLASMID_BIN_DIR)
                .join("primals")
                .join(arch)
        },
        std::path::PathBuf::from,
    );
    let binary_path = depot_dir.join(primal);

    if !binary_path.exists() {
        return Ok(ShadowOutcome::fail(format!(
            "binary not found in depot: {}",
            binary_path.display()
        )));
    }

    let sandbox_args = plasmid::sandbox::SandboxArgs {
        primal: primal.to_string(),
        commit,
        binary_path,
        timeout_secs: timeout,
    };

    let promote = args.contains(&"--promote");
    if promote {
        let install_dir = cellmembrane_types::service::env_or(
            cellmembrane_types::service::ENV_INSTALL_BASE,
            cellmembrane_types::service::DEFAULT_INSTALL_BASE,
        );
        let production_path = std::path::PathBuf::from(install_dir).join(primal);

        match plasmid::sandbox::validate_and_promote(&sandbox_args, &production_path).await {
            Ok((result, old_binary)) => {
                let ok = result.health_ok;
                let msg = format!(
                    "sandbox {}: {} — {} ({}ms){}",
                    result.primal,
                    if ok { "PASS+PROMOTED" } else { "FAIL" },
                    result.detail,
                    result.elapsed_ms,
                    old_binary.map_or(String::new(), |p| format!(" (old → {})", p.display())),
                );
                Ok(ShadowOutcome {
                    ok,
                    message: msg,
                    data: Some(serde_json::to_value(&result).unwrap_or_default()),
                })
            }
            Err(e) => Ok(ShadowOutcome::fail(format!("sandbox+promote error: {e}"))),
        }
    } else {
        match plasmid::sandbox::validate(&sandbox_args).await {
            Ok(result) => {
                let ok = result.health_ok;
                let msg = format!(
                    "sandbox {}: {} — {} ({}ms)",
                    result.primal,
                    if ok { "PASS" } else { "FAIL" },
                    result.detail,
                    result.elapsed_ms,
                );
                Ok(ShadowOutcome {
                    ok,
                    message: msg,
                    data: Some(serde_json::to_value(&result).unwrap_or_default()),
                })
            }
            Err(e) => Ok(ShadowOutcome::fail(format!("sandbox error: {e}"))),
        }
    }
}

pub(super) fn dispatch_depot_integrity(args: &[&str]) -> crate::Result<ShadowOutcome> {
    let verify_only = args.contains(&"--verify");
    let depot_dir = crate::plasmid::depot::resolve_depot(cli::extract_flag_value(args, "--depot"))?;
    let report = if verify_only {
        crate::plasmid::integrity::verify_checksums(&depot_dir)?
    } else {
        crate::plasmid::integrity::generate_checksums(&depot_dir)?
    };
    Ok(ShadowOutcome::ok_with(
        format!(
            "{} binaries across {} arch(es)",
            report.total_binaries,
            report.architectures.len()
        ),
        serde_json::to_value(&report)?,
    ))
}
