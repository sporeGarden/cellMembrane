// SPDX-License-Identifier: AGPL-3.0-or-later

//! Gate domain dispatch — local self-management, health, bootstrap, provisioning.
//!
//! SRP: All commands that operate on the local gate or spawn new gates.
//! Contrast with `infra.rs` which operates on the remote VPS via SSH/Forgejo API.

use crate::cli;
use crate::{ShadowConfig, ShadowOutcome, gate, service};
use tracing::info;

#[allow(
    clippy::too_many_lines,
    reason = "match dispatch hub — each arm is trivial"
)]
pub(super) async fn dispatch(
    config: &ShadowConfig,
    cmd: &str,
    args: &[&str],
) -> crate::Result<ShadowOutcome> {
    match cmd {
        "gate.info" => dispatch_info(config).await,
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
        "gate.health" => dispatch_health(config).await,
        "health.audit" => dispatch_health_audit(config, args).await,
        "gate.bootstrap" => {
            let dry_run = args.contains(&"--dry-run");
            let mobility = if args.contains(&"--mobile") {
                cellmembrane_types::GateMobility::Mobile
            } else {
                cellmembrane_types::GateMobility::Fixed
            };
            let positional: Vec<&&str> = args.iter().filter(|a| !a.starts_with("--")).collect();
            let gate_name = positional
                .first()
                .copied()
                .copied()
                .unwrap_or_else(|| crate::gate::resolve_local_gate_identity().leak());
            let result = gate::bootstrap(config, gate_name, dry_run, mobility).await?;
            let msg = format!(
                "bootstrap {}: {}/{} phases passed{}{}{}",
                result.gate_name,
                result.phases.iter().filter(|p| p.ok).count(),
                result.phases.len(),
                if result.all_pass { " — ENROLLED" } else { "" },
                if dry_run { " (dry-run)" } else { "" },
                if mobility == cellmembrane_types::GateMobility::Mobile {
                    " [mobile]"
                } else {
                    ""
                },
            );
            Ok(ShadowOutcome::ok_with(msg, serde_json::to_value(&result)?))
        }
        "gate.status" => dispatch_status().await,
        "gate.profile" => {
            let gate_name = args.first().ok_or_else(|| {
                crate::error::ShadowError::Parse(
                    "gate.profile requires gate name: membrane gate.profile <gate>".into(),
                )
            })?;
            dispatch_profile(gate_name)
        }
        "gate.preflight" => dispatch_preflight(args).await,
        "firewall.generate" => dispatch_firewall_generate(args),
        #[cfg(feature = "http")]
        "gate.provision" => dispatch_provision(args).await,
        #[cfg(feature = "http")]
        "gate.provision.status" => dispatch_provision_status(args).await,
        #[cfg(feature = "http")]
        "gate.provision.destroy" => dispatch_provision_destroy(args).await,
        #[cfg(feature = "http")]
        "gate.provision.verify" => dispatch_provision_verify(args).await,
        _ => Ok(ShadowOutcome::fail(format!("unknown command: {cmd}"))),
    }
}

// ── Gate info ────────────────────────────────────────────────────────

async fn dispatch_info(config: &ShadowConfig) -> crate::Result<ShadowOutcome> {
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

// ── Gate status ──────────────────────────────────────────────────────

async fn dispatch_status() -> crate::Result<ShadowOutcome> {
    let result = gate::status().await?;
    let probe_lines: Vec<String> = result
        .probes
        .iter()
        .map(|p| {
            let tag = if p.ok { "OK" } else { "DEGRADED" };
            format!("  [{tag}] {}: {}", p.name, p.detail)
        })
        .collect();
    let health = if result.healthy {
        "HEALTHY"
    } else {
        "DEGRADED"
    };
    let msg = format!(
        "{} ({}) — {health}\n{}",
        result.gate_name,
        result.arch,
        probe_lines.join("\n"),
    );
    Ok(ShadowOutcome::ok_with(msg, serde_json::to_value(&result)?))
}

// ── Gate profile ─────────────────────────────────────────────────────

fn dispatch_profile(gate_name: &str) -> crate::Result<ShadowOutcome> {
    let workspace_root = crate::temporal::resolve_workspace_root()?;
    let manifest = crate::manifest::load_from_workspace(&workspace_root)?;

    let Some(profile) = manifest.gates.get(gate_name) else {
        let available: Vec<&String> = manifest.gates.keys().collect();
        return Ok(ShadowOutcome::fail(format!(
            "gate '{gate_name}' not in ecosystem_manifest.toml. Available: {available:?}"
        )));
    };

    let msg = format!(
        "gate.profile: {gate_name}\n  target: {}\n  mobility: {}\n  bind_mode: {}\n  \
         composition: {}\n  transport: {}\n  mesh_peer: {}\n  repos: {}",
        profile.target.as_deref().unwrap_or("(default)"),
        profile.mobility.as_deref().unwrap_or("fixed"),
        profile.bind_mode.as_deref().unwrap_or("(auto)"),
        profile.composition.as_deref().unwrap_or("full"),
        profile.transport.as_deref().unwrap_or("(auto)"),
        profile.mesh_peer.as_deref().unwrap_or("(default relay)"),
        profile.repos.len(),
    );

    Ok(ShadowOutcome::ok_with(msg, serde_json::to_value(profile)?))
}

// ── Gate health ──────────────────────────────────────────────────────

async fn dispatch_health(config: &ShadowConfig) -> crate::Result<ShadowOutcome> {
    let services = service::list(config).await?;
    let total = services.len();
    let healthy = services.iter().filter(|s| s.sub_state == "running").count();
    let degraded: Vec<&str> = services
        .iter()
        .filter(|s| s.sub_state != "running")
        .map(|s| s.unit.as_str())
        .collect();

    let disk = crate::ssh::exec(config, "df --output=pcent / | tail -1")
        .await
        .unwrap_or_default()
        .trim()
        .to_string();

    let status = if degraded.is_empty() {
        "HEALTHY"
    } else {
        "DEGRADED"
    };

    let msg = format!(
        "=== Gate Health ===\n\
         Status:   {status}\n\
         Services: {healthy}/{total} running\n\
         Disk:     {disk}\n\
         {}",
        if degraded.is_empty() {
            String::new()
        } else {
            format!("Degraded: {}", degraded.join(", "))
        }
    );

    let ok = degraded.is_empty();
    Ok(if ok {
        ShadowOutcome::ok_with(
            msg,
            serde_json::json!({
                "status": status,
                "services_total": total,
                "services_healthy": healthy,
                "disk": disk,
            }),
        )
    } else {
        ShadowOutcome {
            ok: false,
            message: msg,
            data: Some(serde_json::json!({
                "status": status,
                "services_total": total,
                "services_healthy": healthy,
                "degraded": degraded,
                "disk": disk,
            })),
        }
    })
}

// ── Health audit ─────────────────────────────────────────────────────

/// Cross-gate version skew report.
///
/// Probes the local depot provenance and compares commit versions.
/// With `--mesh`, queries the VPS depot checksums for remote comparison.
async fn dispatch_health_audit(
    config: &ShadowConfig,
    args: &[&str],
) -> crate::Result<ShadowOutcome> {
    use crate::plasmid;

    let include_mesh = args.contains(&"--mesh");

    let local_report = tokio::task::spawn_blocking(|| {
        let depot_dir = plasmid::depot::resolve_depot(None)?;
        plasmid::depot::detect_stale_primals(&depot_dir)
    })
    .await
    .map_err(|e| crate::error::ShadowError::Parse(format!("spawn failed: {e}")))??;

    let mut entries: Vec<serde_json::Value> = Vec::new();
    for entry in &local_report.entries {
        entries.push(serde_json::json!({
            "primal": entry.name,
            "binary_exists": entry.binary_exists,
            "commit": entry.provenance_commit.as_deref().unwrap_or("none"),
            "stale": entry.stale,
        }));
    }

    let mut vps_skew: Vec<serde_json::Value> = Vec::new();
    if include_mesh {
        let vps_provenance = crate::ssh::exec(
            config,
            "cat /opt/ecoPrimals/plasmidBin/provenance.toml 2>/dev/null || echo ''",
        )
        .await
        .unwrap_or_default();

        if !vps_provenance.is_empty() {
            for entry in &local_report.entries {
                let local_commit = entry.provenance_commit.as_deref().unwrap_or("");
                if local_commit.is_empty() {
                    continue;
                }
                let vps_has = vps_provenance.contains(local_commit);
                if !vps_has {
                    vps_skew.push(serde_json::json!({
                        "primal": entry.name,
                        "local_commit": local_commit,
                        "vps_match": false,
                    }));
                }
            }
        }
    }

    let total = local_report.total;
    let stale = local_report.stale_count;
    let msg = if stale == 0 && vps_skew.is_empty() {
        format!("health.audit: {total} primals — NO SKEW (all current)")
    } else {
        let mut parts = Vec::new();
        if stale > 0 {
            parts.push(format!("{stale} stale in depot"));
        }
        if !vps_skew.is_empty() {
            parts.push(format!("{} local/VPS version mismatch", vps_skew.len()));
        }
        format!("health.audit: {total} primals — {}", parts.join(", "))
    };

    let ok = stale == 0 && vps_skew.is_empty();
    let data = serde_json::json!({
        "total": total,
        "stale": stale,
        "current": local_report.current_count,
        "vps_skew_count": vps_skew.len(),
        "entries": entries,
        "vps_skew": vps_skew,
    });

    Ok(if ok {
        ShadowOutcome::ok_with(msg, data)
    } else {
        ShadowOutcome {
            ok: false,
            message: msg,
            data: Some(data),
        }
    })
}

// ── Gate provisioning ────────────────────────────────────────────────

#[cfg(feature = "http")]
async fn dispatch_provision(args: &[&str]) -> crate::Result<ShadowOutcome> {
    use crate::provision::{self, ProvisionRequest, digitalocean};

    let provider_str = cli::extract_flag_value(args, "--provider").unwrap_or("digitalocean");
    let _provider: provision::Provider = provider_str
        .parse()
        .map_err(|e: String| crate::ShadowError::Parse(e))?;

    let name = cli::extract_flag_value(args, "--name")
        .unwrap_or("membrane-canary")
        .to_string();
    let region = cli::extract_flag_value(args, "--region")
        .unwrap_or("nyc1")
        .to_string();
    let size = cli::extract_flag_value(args, "--size")
        .unwrap_or("s-1vcpu-2gb")
        .to_string();
    let profile = cli::extract_flag_value(args, "--profile")
        .unwrap_or("canary-fieldmouse")
        .to_string();
    let dry_run = args.contains(&"--dry-run");

    if dry_run {
        return Ok(ShadowOutcome::ok(format!(
            "dry-run: would provision {name} ({size}) in {region} with profile {profile}"
        )));
    }

    info!("resolving SSH keys");
    let keys = digitalocean::list_ssh_keys().await?;
    let ssh_key_ids: Vec<String> = keys.iter().map(|k| k.id.to_string()).collect();

    if ssh_key_ids.is_empty() {
        return Ok(ShadowOutcome::fail(
            "no SSH keys found on DO account — add one first",
        ));
    }

    info!(
        name = %name,
        size = %size,
        region = %region,
        ssh_keys = ssh_key_ids.len(),
        "creating droplet"
    );

    let req = ProvisionRequest {
        name: name.clone(),
        region,
        size,
        image: "debian-12-x64".into(),
        profile: profile.clone(),
        ssh_keys: ssh_key_ids,
        tags: vec!["membrane".into(), "canary".into(), "ecoprimals".into()],
    };

    let droplet = digitalocean::create_droplet(&req).await?;
    info!(
        name = %droplet.name,
        id = droplet.id,
        "droplet created, waiting for active"
    );

    let active = digitalocean::wait_until_active(droplet.id, &profile).await?;
    let ip = active.ip.as_deref().unwrap_or("").to_string();
    info!(ip = %ip, "droplet active");

    info!("bootstrapping");
    let outcome = provision::bootstrap::bootstrap_droplet(&active, &name).await;

    let msg = format!(
        "provision: {} (id={}, ip={ip})\n  phases:\n    {}",
        outcome.message,
        active.id,
        outcome.phases.join("\n    ")
    );

    Ok(ShadowOutcome::ok_with(msg, serde_json::to_value(&outcome)?))
}

#[cfg(feature = "http")]
async fn dispatch_provision_status(args: &[&str]) -> crate::Result<ShadowOutcome> {
    use crate::plasmid::canary;
    use crate::provision::digitalocean;

    if let Some(id_str) = cli::extract_flag_value(args, "--id") {
        let id: u64 = id_str.parse().map_err(|e| {
            crate::ShadowError::Parse(format!("invalid droplet id '{id_str}': {e}"))
        })?;
        let state = digitalocean::get_droplet(id).await?;
        let msg = format!(
            "{} (id={}) — {} @ {}",
            state.name,
            state.id,
            state.status,
            state.ip.as_deref().unwrap_or("no-ip")
        );
        Ok(ShadowOutcome::ok_with(msg, serde_json::to_value(&state)?))
    } else {
        let droplets = digitalocean::list_membrane_droplets().await?;
        let remote_canaries = canary::list_remote_canaries().await;

        if droplets.is_empty() && remote_canaries.is_empty() {
            return Ok(ShadowOutcome::ok("no membrane droplets found"));
        }

        let mut lines: Vec<String> = droplets
            .iter()
            .map(|d| {
                format!(
                    "  {} (id={}) — {} @ {} [{}]",
                    d.name,
                    d.id,
                    d.status,
                    d.ip.as_deref().unwrap_or("no-ip"),
                    d.region
                )
            })
            .collect();

        if !remote_canaries.is_empty() {
            lines.push(String::new());
            lines.push("  remote canary registry:".into());
            for rc in &remote_canaries {
                lines.push(format!(
                    "    {} @ {} (id={:?}, primals={})",
                    rc.gate_name,
                    rc.ip,
                    rc.droplet_id,
                    rc.primals.len()
                ));
            }
        }

        let msg = format!(
            "{} droplet(s), {} registered canary(ies):\n{}",
            droplets.len(),
            remote_canaries.len(),
            lines.join("\n")
        );
        Ok(ShadowOutcome::ok_with(
            msg,
            serde_json::to_value(&droplets)?,
        ))
    }
}

#[cfg(feature = "http")]
async fn dispatch_provision_destroy(args: &[&str]) -> crate::Result<ShadowOutcome> {
    use crate::plasmid::canary;
    use crate::provision::digitalocean;

    let id_str = cli::extract_flag_value(args, "--id").ok_or_else(|| {
        crate::ShadowError::Parse("gate.provision.destroy requires --id <droplet-id>".into())
    })?;
    let id: u64 = id_str
        .parse()
        .map_err(|e| crate::ShadowError::Parse(format!("invalid droplet id '{id_str}': {e}")))?;

    let gate_name = cli::extract_flag_value(args, "--gate");

    let dry_run = args.contains(&"--dry-run");
    if dry_run {
        return Ok(ShadowOutcome::ok(format!(
            "dry-run: would destroy droplet id={id}"
        )));
    }

    digitalocean::destroy_droplet(id).await?;

    if let Some(name) = gate_name {
        canary::deregister_remote_canary(name).await;
    } else {
        let registry = canary::load_remote_canaries().await;
        if let Some(entry) = registry.entries.iter().find(|e| e.droplet_id == Some(id)) {
            canary::deregister_remote_canary(&entry.gate_name).await;
        }
    }

    Ok(ShadowOutcome::ok(format!("DESTROYED droplet id={id}")))
}

#[cfg(feature = "http")]
async fn dispatch_provision_verify(args: &[&str]) -> crate::Result<ShadowOutcome> {
    use crate::plasmid::canary;
    use crate::provision::bootstrap;

    let ip = cli::extract_flag_value(args, "--ip");
    let gate = cli::extract_flag_value(args, "--gate");

    let (target_ip, gate_name) = match (ip, gate) {
        (Some(ip), name) => (ip.to_string(), name.unwrap_or("unknown").to_string()),
        (None, Some(name)) => {
            let registry = canary::load_remote_canaries().await;
            let entry = registry
                .entries
                .iter()
                .find(|e| e.gate_name == name)
                .ok_or_else(|| {
                    crate::ShadowError::Parse(format!(
                        "gate '{name}' not found in remote canary registry"
                    ))
                })?;
            (entry.ip.clone(), name.to_string())
        }
        (None, None) => {
            return Err(crate::ShadowError::Parse(
                "gate.provision.verify requires --ip <addr> or --gate <name>".into(),
            ));
        }
    };

    let profile = cli::extract_flag_value(args, "--profile");
    let outcome = bootstrap::verify_remote_gate(&target_ip, &gate_name, profile).await;
    let summary = outcome
        .phases
        .iter()
        .map(|p| format!("  {p}"))
        .collect::<Vec<_>>()
        .join("\n");

    let status = if outcome.success { "PASS" } else { "FAIL" };
    let msg = format!("gate.provision.verify [{status}] {gate_name}@{target_ip}\n{summary}");

    if outcome.success {
        Ok(ShadowOutcome::ok(msg))
    } else {
        Ok(ShadowOutcome::fail(msg))
    }
}

// ── Firewall generation ─────────────────────────────────────────────

fn dispatch_firewall_generate(args: &[&str]) -> crate::Result<ShadowOutcome> {
    use cellmembrane_types::composition::MembraneComposition;
    use cellmembrane_types::firewall::{FirewallRuleset, NftablesConfig};

    let comp_str = cli::extract_flag_value(args, "--composition")
        .or_else(|| args.first().filter(|a| !a.starts_with("--")).copied())
        .unwrap_or("relay");
    let composition = MembraneComposition::parse_name(comp_str).ok_or_else(|| {
        crate::error::ShadowError::Parse(format!(
            "unknown composition: {comp_str} (expected: {})",
            MembraneComposition::all()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ))
    })?;

    let fw = FirewallRuleset::for_composition(composition);

    let format = cli::extract_flag_value(args, "--format").unwrap_or("nftables");

    let nft_config = if args.contains(&"--plasma-membrane") {
        let wan = cli::extract_flag_value(args, "--wan").unwrap_or("enp1s0");
        let lan = cli::extract_flag_value(args, "--lan").unwrap_or("eno1");
        let subnet = cli::extract_flag_value(args, "--subnet").unwrap_or("192.168.4.0/22");
        let gate_name = cli::extract_flag_value(args, "--gate-name")
            .unwrap_or_else(|| crate::gate::resolve_local_gate_identity().leak());
        Some(NftablesConfig {
            wan_interface: wan.into(),
            lan_interface: lan.into(),
            lan_subnet: subnet.into(),
            gate_name: gate_name.into(),
            enable_nat: !args.contains(&"--no-nat"),
            enable_dhcp: !args.contains(&"--no-dhcp"),
            trust_lan_input: args.contains(&"--trust-lan"),
            wireguard_interface: cli::extract_flag_value(args, "--wg-iface").map(Into::into),
            wireguard_port: cli::extract_flag_value(args, "--wg-port")
                .and_then(|p| p.parse().ok())
                .unwrap_or(51820),
            drop_ipv6_forward: !args.contains(&"--allow-ipv6-forward"),
        })
    } else {
        None
    };

    let script = match format {
        "ufw" => fw.to_ufw_script(),
        "nftables" | "nft" => fw.to_nftables_script(nft_config.as_ref()),
        other => {
            return Err(crate::error::ShadowError::Parse(format!(
                "unknown format: {other} (expected: nftables, ufw)"
            )));
        }
    };

    Ok(ShadowOutcome::ok(script))
}

// ── Pre-flight scanner ──────────────────────────────────────────────

async fn dispatch_preflight(args: &[&str]) -> crate::Result<ShadowOutcome> {
    let target_ip = cli::extract_flag_value(args, "--ip");
    let report = gate::preflight::run_preflight(target_ip).await;

    let mut lines = vec![
        "╔══════════════════════════════════════════╗".to_string(),
        "║   Gate Pre-flight — System Readiness     ║".to_string(),
        "╚══════════════════════════════════════════╝".to_string(),
        String::new(),
    ];

    if !report.interfaces.is_empty() {
        lines.push("Interfaces:".to_string());
        for iface in &report.interfaces {
            let speed = iface
                .speed_mbps
                .map_or_else(|| "?".into(), |s| format!("{s}Mbps"));
            let carrier = if iface.carrier { "UP" } else { "DOWN" };
            let ips = if iface.ipv4.is_empty() {
                "no-ip".into()
            } else {
                iface.ipv4.join(", ")
            };
            lines.push(format!(
                "  {:<12} {:>8} {:>6}  {:<15}  {:?}  [{}]",
                iface.name, speed, carrier, ips, iface.role_hint, iface.driver
            ));
        }
        lines.push(String::new());
    }

    lines.push("Checks:".to_string());
    for check in &report.checks {
        let icon = if check.passed { "✓" } else { "✗" };
        lines.push(format!("  {icon} {:<28} {}", check.name, check.detail));
    }
    lines.push(String::new());

    let status = if report.all_pass {
        "ALL CHECKS PASSED — ready for deployment"
    } else {
        "SOME CHECKS FAILED — review above before proceeding"
    };
    lines.push(status.to_string());

    let msg = lines.join("\n");
    let outcome = ShadowOutcome {
        ok: report.all_pass,
        message: msg,
        data: Some(serde_json::to_value(&report)?),
    };
    Ok(outcome)
}
