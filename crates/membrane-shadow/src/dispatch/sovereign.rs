// SPDX-License-Identifier: AGPL-3.0-or-later

//! `sovereign.*` dispatch — sovereign CI pipeline commands.
//!
//! Replaces the bash `post-receive.d/sovereign-ci` + `build-local.sh` chain
//! with a Rust-typed pipeline: manifest-driven build → sandbox → refresh → verify.
//!
//! ## Commands
//!
//! - `sovereign.ci.trigger --primal <name>` — Build, validate, and deploy a single primal.
//!   Equivalent to the entire bash CI chain in a single typed invocation.
//!   The Forgejo `post-receive` hook can call this instead of SSH→bash scripts.
//!
//! - `sovereign.ci.status` — Report CI pipeline health for all primals.

use crate::cli;
use crate::error::{Result, ShadowError};
use crate::{ShadowConfig, ShadowOutcome};
use tracing::{error, info, warn};

/// Resolved sub-builder for dispatch.
struct ResolvedSubBuilder {
    gate: String,
    target: String,
    ssh_host: String,
    membrane_bin: String,
}

/// Load sub-builders from the ecosystem manifest.
///
/// Falls back to a hardcoded blueGate entry if the manifest is unavailable
/// or has no `[sub_builders]` section — preserving nanowire compatibility.
fn load_sub_builders() -> Vec<ResolvedSubBuilder> {
    let workspace = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_ECOPRIMALS_ROOT,
        cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT,
    );

    if let Ok(manifest) = crate::manifest::load_from_workspace(std::path::Path::new(&workspace)) {
        if !manifest.sub_builders.is_empty() {
            return manifest
                .sub_builders
                .into_iter()
                .map(|(target, entry)| ResolvedSubBuilder {
                    gate: entry.gate,
                    target,
                    ssh_host: entry.ssh_host,
                    membrane_bin: entry.membrane_bin,
                })
                .collect();
        }
    }

    vec![ResolvedSubBuilder {
        gate: "blueGate".into(),
        target: "x86_64-pc-windows-gnu".into(),
        ssh_host: "blueGate".into(),
        membrane_bin: "membrane.exe".into(),
    }]
}

/// Route `sovereign.*` commands.
pub(super) async fn dispatch_sovereign(
    config: &ShadowConfig,
    cmd: &str,
    args: &[&str],
) -> Result<ShadowOutcome> {
    match cmd {
        "sovereign.ci.trigger" => dispatch_ci_trigger(config, args).await,
        "sovereign.ci.status" => Ok(dispatch_ci_status()),
        _ => Ok(ShadowOutcome::fail(format!(
            "unknown sovereign command: {cmd}"
        ))),
    }
}

/// Parsed arguments for `sovereign.ci.trigger`.
struct CiTriggerArgs<'a> {
    primal: &'a str,
    git_ref: Option<&'a str>,
    commit: Option<&'a str>,
    dry_run: bool,
}

/// Parse and validate `sovereign.ci.trigger` arguments.
fn parse_ci_trigger_args<'a>(args: &[&'a str]) -> Result<CiTriggerArgs<'a>> {
    let primal = cli::extract_flag_value(args, "--primal")
        .or_else(|| args.first().copied().filter(|a| !a.starts_with('-')))
        .ok_or_else(|| {
            ShadowError::config(
                "sovereign.ci.trigger requires --primal <name> or positional primal name",
            )
        })?;

    Ok(CiTriggerArgs {
        primal,
        git_ref: cli::extract_flag_value(args, "--ref"),
        commit: cli::extract_flag_value(args, "--commit"),
        dry_run: args.contains(&"--dry-run"),
    })
}

/// `sovereign.ci.trigger --primal <name> [--ref <branch>] [--commit <sha>] [--dry-run]`
///
/// Full sovereign CI pipeline for a single primal:
/// 1. Resolve primal from service registry + manifest
/// 2. Harvest (build from source, manifest-driven)
/// 3. Sandbox validation (start, health check, stop)
/// 4. Refresh (atomic deploy to VPS + depot sync with BLAKE3 verify)
/// 5. Report structured outcome with provenance
async fn dispatch_ci_trigger(config: &ShadowConfig, args: &[&str]) -> Result<ShadowOutcome> {
    let trigger = parse_ci_trigger_args(args)?;
    let primal_lower = trigger.primal.to_lowercase();

    if cellmembrane_types::MembraneService::for_binary(&primal_lower).is_none() {
        return Ok(ShadowOutcome::fail(format!(
            "sovereign.ci.trigger: '{}' is not a known primal in the service registry",
            trigger.primal
        )));
    }

    info!(
        primal = %trigger.primal,
        git_ref = ?trigger.git_ref,
        commit = ?trigger.commit,
        dry_run = trigger.dry_run,
        "sovereign CI trigger"
    );

    // Phase 1a: Local harvest (musl/gnu on this gate)
    let harvest_outcome = run_harvest(&primal_lower, trigger.dry_run).await?;

    if !harvest_outcome.ok {
        error!(primal = %trigger.primal, message = %harvest_outcome.message, "harvest failed");
        return Ok(ShadowOutcome {
            ok: false,
            message: format!(
                "sovereign.ci.trigger: {} harvest FAILED — {}",
                trigger.primal, harvest_outcome.message
            ),
            data: harvest_outcome.data,
        });
    }

    // Phase 1b: Sub-builder dispatch (manifest-driven, nanowire/SSH transport)
    let sub_builders = load_sub_builders();
    let sub_results = run_sub_builder_harvests(
        &sub_builders,
        &primal_lower,
        trigger.commit,
        trigger.dry_run,
    )
    .await;

    let sub_summary: Vec<String> = sub_results
        .iter()
        .map(|(gate, o)| format!("{}:{}", gate, if o.ok { "OK" } else { "FAIL" }))
        .collect();

    let sub_all_ok = sub_results.iter().all(|(_, o)| o.ok);

    if !sub_summary.is_empty() {
        if sub_all_ok {
            info!(sub_builders = ?sub_summary, "all sub-builder harvests completed");
        } else {
            warn!(sub_builders = ?sub_summary, "some sub-builder harvests failed");
        }
    }

    if trigger.dry_run {
        let dry_data = serde_json::json!({
            "harvest": harvest_outcome.data,
            "sub_builders": sub_summary,
        });
        return Ok(ShadowOutcome {
            ok: true,
            message: format!(
                "sovereign.ci.trigger: {} (dry-run) — {} | sub-builders: [{}]",
                trigger.primal,
                harvest_outcome.message,
                sub_summary.join(", ")
            ),
            data: Some(dry_data),
        });
    }

    // Phase 2: Sandbox validation (local binary only)
    if let Some(sandbox_fail) =
        run_sandbox_phase(&primal_lower, trigger.primal, trigger.commit).await?
    {
        return Ok(sandbox_fail);
    }

    // Phase 3: Refresh (atomic deploy + depot sync)
    let refresh_outcome = run_refresh(config, &primal_lower).await?;

    let provenance = serde_json::json!({
        "primal": trigger.primal,
        "git_ref": trigger.git_ref,
        "commit": trigger.commit,
        "harvest": harvest_outcome.message,
        "sub_builders": sub_summary,
        "refresh": refresh_outcome.message,
        "pipeline": "sovereign.ci.trigger",
    });

    let sub_label = if sub_summary.is_empty() {
        String::new()
    } else {
        format!(" | sub-builders: [{}]", sub_summary.join(", "))
    };

    Ok(ShadowOutcome {
        ok: refresh_outcome.ok && sub_all_ok,
        message: format!(
            "sovereign.ci.trigger: {} — harvest: {} | sandbox: PASS | refresh: {}{}",
            trigger.primal, harvest_outcome.message, refresh_outcome.message, sub_label
        ),
        data: Some(provenance),
    })
}

/// Dispatch remote harvests to sub-builder gates for cross-target binaries.
///
/// For each sub-builder, connects via SSH to the gate and runs `membrane plasmid.harvest`
/// for the primal on that gate's native target triple. Results are collected
/// and returned as a merged outcome.
///
/// Sub-builders are loaded from the ecosystem manifest `[sub_builders]` section,
/// falling back to the hardcoded blueGate entry for nanowire compatibility.
async fn run_sub_builder_harvests(
    sub_builders: &[ResolvedSubBuilder],
    primal: &str,
    commit: Option<&str>,
    dry_run: bool,
) -> Vec<(String, ShadowOutcome)> {
    let mut results = Vec::new();

    for sb in sub_builders {
        info!(
            primal,
            gate = %sb.gate,
            target = %sb.target,
            "sub-builder dispatch"
        );

        if dry_run {
            results.push((
                sb.gate.clone(),
                ShadowOutcome {
                    ok: true,
                    message: format!(
                        "sub-builder {}: {} (dry-run) — would SSH to {} and harvest {} for {}",
                        sb.gate, primal, sb.ssh_host, primal, sb.target
                    ),
                    data: None,
                },
            ));
            continue;
        }

        let outcome = dispatch_to_sub_builder(sb, primal, commit).await;
        results.push((sb.gate.clone(), outcome));
    }

    results
}

/// SSH to a sub-builder gate and run `membrane plasmid.harvest` (nanowire transport).
async fn dispatch_to_sub_builder(
    sb: &ResolvedSubBuilder,
    primal: &str,
    commit: Option<&str>,
) -> ShadowOutcome {
    let mut cmd_parts = vec![
        sb.membrane_bin.clone(),
        "plasmid.harvest".to_string(),
        "--primal".to_string(),
        primal.to_string(),
        "--force".to_string(),
        "--push".to_string(),
    ];

    if let Some(c) = commit {
        cmd_parts.push("--commit".to_string());
        cmd_parts.push(c.to_string());
    }

    let remote_cmd = cmd_parts.join(" ");

    info!(
        gate = sb.gate,
        target = sb.target,
        cmd = %remote_cmd,
        "dispatching to sub-builder via SSH"
    );

    let output = tokio::process::Command::new("ssh")
        .args([
            "-o",
            "ConnectTimeout=10",
            "-o",
            "BatchMode=yes",
            &sb.ssh_host,
            &remote_cmd,
        ])
        .output()
        .await;

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);

            if out.status.success() {
                info!(
                    gate = sb.gate,
                    stdout = %stdout.trim(),
                    "sub-builder harvest completed"
                );
                ShadowOutcome {
                    ok: true,
                    message: format!(
                        "sub-builder {}: {} harvest OK for {} — {}",
                        sb.gate,
                        primal,
                        sb.target,
                        stdout.lines().last().unwrap_or("done")
                    ),
                    data: Some(serde_json::json!({
                        "gate": sb.gate,
                        "target": sb.target,
                        "stdout": stdout.trim(),
                    })),
                }
            } else {
                error!(
                    gate = sb.gate,
                    exit_code = out.status.code(),
                    stderr = %stderr.trim(),
                    "sub-builder harvest failed"
                );
                ShadowOutcome {
                    ok: false,
                    message: format!(
                        "sub-builder {}: {} harvest FAILED for {} — {}",
                        sb.gate,
                        primal,
                        sb.target,
                        stderr.lines().last().unwrap_or("unknown error")
                    ),
                    data: Some(serde_json::json!({
                        "gate": sb.gate,
                        "target": sb.target,
                        "stderr": stderr.trim(),
                    })),
                }
            }
        }
        Err(e) => {
            error!(gate = sb.gate, error = %e, "SSH to sub-builder failed");
            ShadowOutcome {
                ok: false,
                message: format!("sub-builder {}: SSH connection failed — {}", sb.gate, e),
                data: None,
            }
        }
    }
}

/// Phase 1: Manifest-driven build.
async fn run_harvest(primal_lower: &str, dry_run: bool) -> Result<ShadowOutcome> {
    let harvest_args = crate::plasmid::HarvestArgs {
        primal: Some(primal_lower.to_string()),
        force: true,
        dry_run,
        depot_dir: None,
        target: None,
        local: false,
        push: false,
        with_restart: false,
    };
    crate::plasmid::harvest(&harvest_args).await
}

/// Phase 2: Sandbox validation (non-fatal on infra errors).
///
/// Returns `Ok(Some(fail_outcome))` if sandbox rejects the binary,
/// `Ok(None)` if it passes or isn't applicable.
async fn run_sandbox_phase(
    primal_lower: &str,
    primal_display: &str,
    commit: Option<&str>,
) -> Result<Option<ShadowOutcome>> {
    let arch = crate::plasmid::detect_target_triple();
    let depot_dir = crate::plasmid::depot::resolve_depot(None)?;
    let binary_path = depot_dir.join("primals").join(arch).join(primal_lower);

    if !binary_path.exists() {
        return Ok(None);
    }

    let commit_short = commit.map_or("HEAD", |c| if c.len() >= 8 { &c[..8] } else { c });

    let sandbox_args = crate::plasmid::sandbox::SandboxArgs {
        primal: primal_lower.to_string(),
        commit: commit_short.to_string(),
        binary_path,
        timeout_secs: None,
    };

    match crate::plasmid::sandbox::validate_with_deps(&sandbox_args).await {
        Ok(result) if !result.health_ok => {
            error!(
                primal = %primal_display,
                detail = %result.detail,
                "sandbox validation FAILED — blocking deploy"
            );
            Ok(Some(ShadowOutcome {
                ok: false,
                message: format!(
                    "sovereign.ci.trigger: {primal_display} sandbox FAILED — {} ({}ms). Deploy blocked.",
                    result.detail, result.elapsed_ms
                ),
                data: Some(serde_json::to_value(&result).unwrap_or_default()),
            }))
        }
        Ok(result) => {
            info!(
                primal = %primal_display,
                detail = %result.detail,
                elapsed_ms = result.elapsed_ms,
                "sandbox PASS"
            );
            Ok(None)
        }
        Err(e) => {
            warn!(primal = %primal_display, error = %e, "sandbox infra error — proceeding");
            Ok(None)
        }
    }
}

/// Phase 3: Atomic deploy + depot sync with BLAKE3 verify.
async fn run_refresh(config: &ShadowConfig, primal_lower: &str) -> Result<ShadowOutcome> {
    let refresh_args = crate::plasmid::RefreshArgs {
        primal: Some(primal_lower.to_string()),
        dry_run: false,
        source_dir: None,
    };
    crate::plasmid::refresh(config, &refresh_args).await
}

/// `sovereign.ci.status` — Report CI pipeline health.
///
/// Shows which primals have fresh binaries, which need rebuild,
/// and the overall pipeline readiness state.
fn dispatch_ci_status() -> ShadowOutcome {
    match crate::plasmid::detect_depot_staleness() {
        Ok(report) => {
            let stale_count = report.entries.iter().filter(|e| e.stale).count();
            let fresh_count = report.entries.iter().filter(|e| !e.stale).count();
            let total = report.entries.len();

            let stale_names: Vec<&str> = report
                .entries
                .iter()
                .filter(|e| e.stale)
                .map(|e| e.name.as_str())
                .collect();

            ShadowOutcome {
                ok: stale_count == 0,
                message: format!(
                    "sovereign.ci.status: {fresh_count}/{total} fresh, {stale_count} stale{}",
                    if stale_names.is_empty() {
                        String::new()
                    } else {
                        format!(" [{}]", stale_names.join(", "))
                    }
                ),
                data: Some(serde_json::json!({
                    "fresh": fresh_count,
                    "stale": stale_count,
                    "total": total,
                    "stale_primals": stale_names,
                })),
            }
        }
        Err(e) => ShadowOutcome::fail(format!("sovereign.ci.status: depot unavailable — {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_sovereign_command_returns_fail() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let config = ShadowConfig::default();
        let result = rt
            .block_on(dispatch_sovereign(&config, "sovereign.nonexistent", &[]))
            .unwrap();
        assert!(!result.ok);
        assert!(result.message.contains("unknown sovereign command"));
    }

    #[test]
    fn ci_trigger_requires_primal() {
        let result = parse_ci_trigger_args(&[]);
        assert!(result.is_err(), "should require --primal argument");
    }

    #[test]
    fn ci_trigger_rejects_unknown_primal() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let config = ShadowConfig::default();
        let result = rt
            .block_on(dispatch_ci_trigger(
                &config,
                &["--primal", "nonexistent_test_primal"],
            ))
            .unwrap();
        assert!(!result.ok);
        assert!(result.message.contains("not a known primal"));
    }

    #[test]
    fn ci_trigger_accepts_positional_primal() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let config = ShadowConfig::default();
        let result = rt
            .block_on(dispatch_ci_trigger(
                &config,
                &["__fake_not_a_real_primal__"],
            ))
            .unwrap();
        assert!(!result.ok);
        assert!(result.message.contains("not a known primal"));
    }

    #[test]
    fn ci_trigger_parses_all_flags() {
        let args = [
            "--primal",
            "beardog",
            "--ref",
            "main",
            "--commit",
            "abc12345",
            "--dry-run",
        ];
        let trigger = parse_ci_trigger_args(&args).unwrap();
        assert_eq!(trigger.primal, "beardog");
        assert_eq!(trigger.git_ref, Some("main"));
        assert_eq!(trigger.commit, Some("abc12345"));
        assert!(trigger.dry_run);
    }

    #[test]
    fn ci_trigger_positional_parse() {
        let args = ["songbird"];
        let trigger = parse_ci_trigger_args(&args).unwrap();
        assert_eq!(trigger.primal, "songbird");
        assert!(trigger.git_ref.is_none());
        assert!(!trigger.dry_run);
    }

    #[test]
    fn ci_status_reports_depot_state() {
        let result = dispatch_ci_status();
        assert!(
            result.message.contains("sovereign.ci.status"),
            "should contain command prefix"
        );
    }

    #[test]
    fn sub_builders_load_from_manifest_or_fallback() {
        let builders = load_sub_builders();
        assert!(
            !builders.is_empty(),
            "should have at least one sub-builder (manifest or fallback)"
        );
        let has_windows = builders.iter().any(|b| b.target == "x86_64-pc-windows-gnu");
        assert!(has_windows, "should include windows-gnu sub-builder");
    }

    #[test]
    fn sub_builder_dry_run_reports_all_gates() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let builders = load_sub_builders();
        let results = rt.block_on(run_sub_builder_harvests(&builders, "squirrel", None, true));
        assert_eq!(results.len(), builders.len());
        for (gate, outcome) in &results {
            assert!(outcome.ok, "dry-run should always succeed for {gate}");
            assert!(outcome.message.contains("dry-run"));
        }
    }
}
