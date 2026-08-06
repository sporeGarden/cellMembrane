// SPDX-License-Identifier: AGPL-3.0-or-later

//! Composition trust barrier validation and rootpulse sovereignty ledger.
//!
//! `gate.validate` checks that a remote gate conforms to its declared
//! composition tier. `rootpulse.*` commands manage the sovereignty ledger
//! for cascade provenance.

use crate::cli;
use crate::error::Result;
use crate::{ShadowConfig, ShadowOutcome};

pub(super) async fn gate_validate(
    config: &ShadowConfig,
    args: &[&str],
    composition_override: Option<cellmembrane_types::MembraneComposition>,
) -> crate::Result<ShadowOutcome> {
    let composition = if let Some(c) = composition_override {
        c
    } else if let Some(comp_str) = cli::extract_flag_value(args, "--composition") {
        cellmembrane_types::MembraneComposition::parse_name(comp_str).ok_or_else(|| {
            crate::error::ShadowError::config(format!(
                "unknown composition tier: {comp_str} \
                 (valid: relay, rustdesk, tower, nest, nucleus, peptidoglycan)"
            ))
        })?
    } else {
        resolve_gate_composition(args).unwrap_or(cellmembrane_types::MembraneComposition::Relay)
    };

    let target_host = resolve_validate_host(config, args);
    let target_config = ShadowConfig {
        ssh_host: target_host.clone(),
        ..config.clone()
    };

    let checks = run_composition_checks(config, &target_config, &target_host, composition).await?;
    let all_pass = checks.iter().all(|(_, ok, _)| *ok);
    let msg = format_validate_report(&checks, composition);

    Ok(if all_pass {
        ShadowOutcome::ok_with(msg, checks_to_json(&checks, composition))
    } else {
        ShadowOutcome {
            ok: false,
            message: msg,
            data: Some(checks_to_json(&checks, composition)),
        }
    })
}

async fn run_composition_checks(
    config: &ShadowConfig,
    target_config: &ShadowConfig,
    target_host: &str,
    composition: cellmembrane_types::MembraneComposition,
) -> crate::Result<Vec<(&'static str, bool, String)>> {
    let mut checks: Vec<(&str, bool, String)> = Vec::new();

    let ssh_ok = crate::ssh::check_connectivity(target_host).await;
    checks.push(("ssh.reachable", ssh_ok, target_host.to_string()));
    if !ssh_ok {
        return Ok(checks);
    }

    let turn_port = cellmembrane_types::MembraneService::with_capability(
        cellmembrane_types::ServiceCapability::TurnServer,
    )
    .and_then(|s| s.port)
    .unwrap_or(cellmembrane_types::service::DEFAULT_TURN_PORT);
    let (turn_out, turn_code) = crate::ssh::exec_raw(
        target_config,
        &format!("ss -tlnp | grep -q ':{turn_port}' && echo OK || echo FAIL"),
    )
    .await?;
    let turn_ok = turn_code == 0 && turn_out.contains("OK");
    checks.push(("mesh.turn", turn_ok, format!("port {turn_port}")));

    let tower_env_path = format!("{}/tower.env", config.vps_root.trim_end_matches('/'));
    let (_, tower_code) = crate::ssh::exec_raw(
        target_config,
        &format!("test -f {tower_env_path} && echo OK || echo MISSING"),
    )
    .await?;
    let tower_ok = tower_code == 0;
    checks.push(("tower.env", tower_ok, tower_env_path));

    let install_base = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_INSTALL_BASE,
        cellmembrane_types::service::DEFAULT_INSTALL_BASE,
    );
    let find_cmd =
        format!("find {install_base} -name '*.db' -o -name '*.sqlite' 2>/dev/null | wc -l");
    let (data_out, _) = crate::ssh::exec_raw(target_config, &find_cmd).await?;
    let data_files: u32 = data_out.trim().parse().unwrap_or(99);
    let stores_nothing = matches!(
        composition,
        cellmembrane_types::MembraneComposition::Relay
            | cellmembrane_types::MembraneComposition::Peptidoglycan
    );
    let data_ok = if stores_nothing {
        data_files == 0
    } else {
        true
    };
    let data_label = if stores_nothing {
        "stores.nothing"
    } else {
        "data.present"
    };
    checks.push((data_label, data_ok, format!("{data_files} data files")));

    let (ufw_out, _) = crate::ssh::exec_raw(
        target_config,
        "ufw status | grep -cE 'ALLOW' 2>/dev/null || echo 0",
    )
    .await?;
    let ufw_rules: u32 = ufw_out.trim().parse().unwrap_or(0);
    checks.push((
        "firewall.minimal",
        ufw_rules <= 5,
        format!("{ufw_rules} ALLOW rules"),
    ));

    let higher_services: Vec<&str> = cellmembrane_types::MembraneService::all()
        .iter()
        .filter(|s| s.is_primal && s.min_composition > composition)
        .map(|s| s.systemd_unit)
        .collect();
    if higher_services.is_empty() {
        checks.push((
            "no.excess.services",
            true,
            "top-tier composition — no excess possible".into(),
        ));
    } else {
        let check_cmd = format!(
            "systemctl is-active {} 2>/dev/null | grep -c active || echo 0",
            higher_services.join(" ")
        );
        let (services_out, _) = crate::ssh::exec_raw(target_config, &check_cmd).await?;
        let excess: u32 = services_out.trim().parse().unwrap_or(0);
        checks.push((
            "no.excess.services",
            excess == 0,
            format!("{excess} services above {composition} tier"),
        ));
    }

    Ok(checks)
}

fn resolve_validate_host(config: &ShadowConfig, args: &[&str]) -> String {
    if let Some(host) = cli::extract_flag_value(args, "--host") {
        return host.to_string();
    }
    args.iter().find(|a| !a.starts_with("--")).map_or_else(
        || {
            std::env::var(cellmembrane_types::service::ENV_VALIDATE_SSH_HOST)
                .or_else(|_| std::env::var(cellmembrane_types::service::ENV_PEPTI_SSH_HOST))
                .unwrap_or_else(|_| config.ssh_host.clone())
        },
        |&h| h.to_string(),
    )
}

fn resolve_gate_composition(args: &[&str]) -> Option<cellmembrane_types::MembraneComposition> {
    let gate_name = cli::extract_flag_value(args, "--gate")?;
    let root = crate::temporal::resolve_workspace_root().ok()?;
    let manifest = crate::manifest::load_from_workspace(&root).ok()?;
    let profile = manifest.gates.get(gate_name)?;
    profile
        .composition
        .as_ref()
        .and_then(|c| cellmembrane_types::MembraneComposition::parse_name(c))
}

fn format_validate_report(
    checks: &[(&str, bool, String)],
    composition: cellmembrane_types::MembraneComposition,
) -> String {
    use std::fmt::Write;
    let mut out = format!("=== Composition Trust Barrier Validation ({composition}) ===\n");
    for (name, ok, detail) in checks {
        let status = if *ok { "PASS" } else { "FAIL" };
        let _ = writeln!(out, "  [{status}] {name}: {detail}");
    }
    let passed = checks.iter().filter(|(_, ok, _)| *ok).count();
    let _ = write!(out, "\n  Result: {passed}/{} checks passed", checks.len());
    out
}

fn checks_to_json(
    checks: &[(&str, bool, String)],
    composition: cellmembrane_types::MembraneComposition,
) -> serde_json::Value {
    serde_json::json!({
        "composition": composition.to_string(),
        "checks": checks
            .iter()
            .map(|(name, ok, detail)| {
                serde_json::json!({
                    "check": name,
                    "pass": ok,
                    "detail": detail,
                })
            })
            .collect::<Vec<serde_json::Value>>(),
    })
}

// ── Rootpulse sovereignty ledger ─────────────────────────────────────

async fn resolve_gate_name(args: &[&str], root: &std::path::Path) -> String {
    let explicit = cli::extract_flag_value(args, "--gate");
    crate::gate::resolve_gate_name_async(explicit, Some(root)).await
}

pub(super) async fn dispatch_rootpulse(cmd: &str, args: &[&str]) -> Result<ShadowOutcome> {
    match cmd {
        "rootpulse.commit" => dispatch_rootpulse_commit(args).await,
        "rootpulse.verify" => dispatch_rootpulse_verify(args).await,
        "rootpulse.status" => {
            let session = crate::temporal::post_sync::load_rootpulse_session();
            Ok(session.map_or_else(
                || {
                    ShadowOutcome::ok_with(
                        "no rootpulse session recorded on this gate",
                        serde_json::json!({ "last_session": null }),
                    )
                },
                |s| {
                    ShadowOutcome::ok_with(
                        format!("last rootpulse session: {s}"),
                        serde_json::json!({ "last_session": s }),
                    )
                },
            ))
        }
        _ => Ok(ShadowOutcome::fail(format!(
            "unknown rootpulse command: {cmd}"
        ))),
    }
}

async fn dispatch_rootpulse_commit(args: &[&str]) -> Result<ShadowOutcome> {
    let root = crate::temporal::resolve_workspace_root()?;
    let m = crate::manifest::load_from_workspace_async(&root).await?;
    let gate = resolve_gate_name(args, &root).await;
    let wave = cli::extract_flag_value(args, "--wave")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(m.meta.wave);

    let repos = m.gate_repos(&gate);
    let heads = crate::temporal::post_sync::collect_cascade_heads(&root, &repos).await;

    if heads.is_empty() {
        return Ok(ShadowOutcome::fail(
            "no cloned repos found — nothing to commit",
        ));
    }

    match crate::sovereignty_ledger::rootpulse_commit(wave, &gate, &heads).await {
        Ok(session) => {
            crate::temporal::post_sync::persist_rootpulse_session(wave, &gate, &session);
            Ok(ShadowOutcome::ok_with(
                format!("rootpulse committed: {session}"),
                serde_json::json!({
                    "session": session,
                    "wave": wave,
                    "gate": gate,
                    "repos": heads.len(),
                }),
            ))
        }
        Err(e) => Ok(ShadowOutcome::fail(format!("rootpulse commit failed: {e}"))),
    }
}

async fn dispatch_rootpulse_verify(args: &[&str]) -> Result<ShadowOutcome> {
    let root = crate::temporal::resolve_workspace_root()?;
    let m = crate::manifest::load_from_workspace_async(&root).await?;
    let gate = resolve_gate_name(args, &root).await;

    let repos = m.gate_repos(&gate);
    let heads = crate::temporal::post_sync::collect_cascade_heads(&root, &repos).await;

    let checks = crate::sovereignty_ledger::sovereignty_verify(m.meta.wave, &heads).await;

    if checks.is_empty() {
        return Ok(ShadowOutcome::ok_with(
            "rootpulse ledger unavailable — graceful skip",
            serde_json::json!({ "status": "unavailable" }),
        ));
    }

    let verified = checks.iter().filter(|c| c.verified).count();
    let total = checks.len();
    let all_ok = verified == total;
    let detail_lines: Vec<String> = checks
        .iter()
        .map(|c| {
            let icon = if c.verified { "OK" } else { "MISMATCH" };
            format!("  [{icon}] {}: {}", c.repo, c.detail)
        })
        .collect();
    let msg = format!(
        "sovereignty: {verified}/{total} verified\n{}",
        detail_lines.join("\n")
    );
    Ok(ShadowOutcome {
        ok: all_ok,
        message: msg,
        data: Some(serde_json::json!({
            "verified": verified,
            "total": total,
            "checks": checks.iter().map(|c| serde_json::json!({
                "repo": c.repo,
                "verified": c.verified,
                "detail": c.detail,
            })).collect::<Vec<_>>(),
        })),
    })
}
