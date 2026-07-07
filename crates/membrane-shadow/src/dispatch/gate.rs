// SPDX-License-Identifier: AGPL-3.0-or-later

//! Gate domain dispatch — local self-management, health, bootstrap, firewall, preflight.
//!
//! SRP: All commands that operate on the local gate.
//! Provisioning is in `provision_dispatch`. VPS ops are in `infra`.

use crate::cli;
use crate::{ShadowConfig, ShadowOutcome, gate, service};

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
                .map_or_else(crate::gate::resolve_local_gate_identity, |&&s| {
                    s.to_string()
                });
            let result = gate::bootstrap(config, &gate_name, dry_run, mobility).await?;
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
                crate::error::ShadowError::Config(
                    "gate.profile requires gate name: membrane gate.profile <gate>".into(),
                )
            })?;
            dispatch_profile(gate_name)
        }
        "gate.preflight" => dispatch_preflight(args).await,
        "firewall.generate" => dispatch_firewall_generate(args),
        "gate.validate" => super::gate_validate(config, args, None).await,
        "gate.quorum" => dispatch_quorum(args),
        "wireguard.generate" => dispatch_wireguard_generate(args).await,
        #[cfg(feature = "http")]
        "gate.provision" => super::provision_dispatch::dispatch_provision(args).await,
        #[cfg(feature = "http")]
        "gate.provision.status" => super::provision_dispatch::dispatch_provision_status(args).await,
        #[cfg(feature = "http")]
        "gate.provision.destroy" => {
            super::provision_dispatch::dispatch_provision_destroy(args).await
        }
        #[cfg(feature = "http")]
        "gate.provision.verify" => super::provision_dispatch::dispatch_provision_verify(args).await,
        _ => Ok(ShadowOutcome::fail(format!("unknown command: {cmd}"))),
    }
}

// ── Quorum cascade timer ─────────────────────────────────────────────

#[allow(
    clippy::unnecessary_wraps,
    reason = "dispatch adapter — must match arm signature"
)]
fn dispatch_quorum(args: &[&str]) -> crate::Result<ShadowOutcome> {
    let dry_run = args.contains(&"--dry-run");
    let interval: u32 = cli::extract_flag_value(args, "--interval")
        .and_then(|v| v.parse().ok())
        .unwrap_or(15);
    let gate_name = crate::gate::resolve_local_gate_identity();

    if args.contains(&"--generate") {
        let (service, timer) = crate::gate::nucleus::generate_cascade_timer(interval, &gate_name);
        let data = serde_json::json!({
            "service": service,
            "timer": timer,
            "interval_minutes": interval,
            "gate": gate_name,
        });
        return Ok(ShadowOutcome::ok_with(
            format!(
                "=== membrane-cascade.service ===\n{service}\n=== membrane-cascade.timer ===\n{timer}"
            ),
            data,
        ));
    }

    let phase = crate::gate::nucleus::install_cascade_timer(interval, &gate_name, dry_run);
    let data = serde_json::json!({
        "phase": phase.name,
        "ok": phase.ok,
        "detail": &phase.detail,
        "interval_minutes": interval,
        "gate": gate_name,
    });
    Ok(ShadowOutcome::ok_with(phase.detail, data))
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
        let available: Vec<&str> = manifest.gates.keys().map(String::as_str).collect();
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
        profile
            .transport
            .map_or_else(|| "(auto)".to_string(), |t| t.to_string()),
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
    .map_err(|e| crate::error::ShadowError::Config(format!("spawn failed: {e}")))??;

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
            &format!(
                "cat {}/{}/provenance.toml 2>/dev/null || echo ''",
                cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT,
                cellmembrane_types::service::INFRA_PLASMID_BIN,
            ),
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

// ── Firewall generation ─────────────────────────────────────────────

fn dispatch_firewall_generate(args: &[&str]) -> crate::Result<ShadowOutcome> {
    use cellmembrane_types::composition::MembraneComposition;
    use cellmembrane_types::firewall::{FirewallRuleset, NftablesConfig};

    let gate_name_owned = cli::extract_flag_value(args, "--gate-name")
        .or_else(|| cli::extract_flag_value(args, "--gate"))
        .map_or_else(crate::gate::resolve_local_gate_identity, String::from);
    let gate_name: &str = &gate_name_owned;

    let manifest = crate::temporal::resolve_workspace_root()
        .ok()
        .and_then(|root| crate::manifest::load_from_workspace(&root).ok());

    let profile = manifest.as_ref().and_then(|m| m.gates.get(gate_name));

    let comp_str = cli::extract_flag_value(args, "--composition")
        .or_else(|| args.first().filter(|a| !a.starts_with("--")).copied())
        .or_else(|| profile.and_then(|p| p.composition.as_deref()))
        .unwrap_or("relay");
    let composition = MembraneComposition::parse_name(comp_str).ok_or_else(|| {
        crate::error::ShadowError::Config(format!(
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

    let is_plasma_membrane = args.contains(&"--plasma-membrane")
        || profile.is_some_and(|p| p.roles.iter().any(|r| r == "nat_firewall"));

    let nft_config = if is_plasma_membrane {
        let wan = cli::extract_flag_value(args, "--wan")
            .or_else(|| profile.and_then(|p| p.wan_interface.as_deref()))
            .unwrap_or(cellmembrane_types::service::DEFAULT_WAN_IFACE);
        let lan = cli::extract_flag_value(args, "--lan")
            .or_else(|| profile.and_then(|p| p.lan_interface.as_deref()))
            .unwrap_or(cellmembrane_types::service::DEFAULT_LAN_IFACE);
        let subnet = cli::extract_flag_value(args, "--subnet")
            .or_else(|| profile.and_then(|p| p.lan_subnet.as_deref()))
            .unwrap_or(cellmembrane_types::service::DEFAULT_LAN_SUBNET);
        let has_wg = profile.is_some_and(|p| p.wg_ip.is_some());
        Some(NftablesConfig {
            wan_interface: wan.into(),
            lan_interface: lan.into(),
            lan_subnet: subnet.into(),
            gate_name: gate_name.into(),
            enable_nat: !args.contains(&"--no-nat"),
            enable_dhcp: !args.contains(&"--no-dhcp")
                && profile.is_some_and(|p| p.roles.iter().any(|r| r == "dhcp")),
            trust_lan_input: args.contains(&"--trust-lan")
                || profile.is_some_and(|p| p.roles.iter().any(|r| r == "nat_firewall")),
            wireguard_interface: cli::extract_flag_value(args, "--wg-iface")
                .map(Into::into)
                .or_else(|| if has_wg { Some("wg0".into()) } else { None }),
            wireguard_port: cli::extract_flag_value(args, "--wg-port")
                .and_then(|p| p.parse().ok())
                .unwrap_or(cellmembrane_types::firewall::default_wg_port()),
            drop_ipv6_forward: !args.contains(&"--allow-ipv6-forward"),
        })
    } else {
        None
    };

    let script = match format {
        "ufw" => fw.to_ufw_script(),
        "nftables" | "nft" => fw.to_nftables_script(nft_config.as_ref()),
        other => {
            return Err(crate::error::ShadowError::Config(format!(
                "unknown format: {other} (expected: nftables, ufw)"
            )));
        }
    };

    Ok(ShadowOutcome::ok(script))
}

// ── WireGuard config generation ─────────────────────────────────────

async fn dispatch_wireguard_generate(args: &[&str]) -> crate::Result<ShadowOutcome> {
    use cellmembrane_types::wireguard::{DEFAULT_WG_PORT, WgConfig, WgPeer};

    let root = crate::temporal::resolve_workspace_root()?;
    let m = crate::manifest::load_from_workspace_async(&root).await?;

    let gate_name = cli::extract_flag_value(args, "--gate")
        .map_or_else(crate::gate::resolve_local_gate_identity, str::to_string);

    let listen_port: u16 = cli::extract_flag_value(args, "--port")
        .and_then(|p| p.parse().ok())
        .unwrap_or(DEFAULT_WG_PORT);

    let subnet = cli::extract_flag_value(args, "--subnet")
        .unwrap_or(cellmembrane_types::service::DEFAULT_WG_MESH_SUBNET);

    let local_ip = m.mesh_ip_for(&gate_name).ok_or_else(|| {
        crate::error::ShadowError::Config(format!(
            "gate '{gate_name}' has no WG mesh IP — add wg_ip to its manifest profile"
        ))
    })?;

    let keepalive: u16 = cli::extract_flag_value(args, "--keepalive")
        .and_then(|k| k.parse().ok())
        .unwrap_or(25);

    let hub_mode = cli::extract_flag_value(args, "--hub").unwrap_or_else(|| {
        m.gates_for_role("wg_hub")
            .first()
            .map_or("", |&(name, _)| name)
    });

    let is_local_hub = m
        .gates
        .get(&gate_name)
        .is_some_and(|p| p.roles.iter().any(|r| r == "wg_hub"));

    let mut peers = Vec::new();
    for (name, profile) in &m.gates {
        if *name == gate_name {
            continue;
        }
        let Some(mesh_ip) = m.mesh_ip_for(name) else {
            continue;
        };

        let is_hub = profile.roles.iter().any(|r| r == "wg_hub");

        let endpoint = profile
            .wan_endpoint
            .as_deref()
            .or(profile.host.as_deref())
            .map(String::from);

        let allowed_ips = if is_hub && !is_local_hub {
            vec![format!("{subnet}")]
        } else if is_local_hub {
            let mut ips = vec![format!("{mesh_ip}/32")];
            if let Some(ref lan) = profile.lan_subnet {
                ips.push(lan.clone());
            }
            ips
        } else {
            vec![format!("{mesh_ip}/32")]
        };

        peers.push(WgPeer {
            name: name.clone(),
            mesh_ip,
            public_key: profile.wg_pubkey.clone(),
            endpoint,
            allowed_ips,
            keepalive,
        });
    }

    if !is_local_hub && !hub_mode.is_empty() && peers.iter().any(|p| p.name != hub_mode) {
        peers.retain(|p| p.name == hub_mode);
    }

    peers.sort_by(|a, b| a.name.cmp(&b.name));

    let config = WgConfig {
        gate_name,
        address: local_ip,
        listen_port,
        subnet: subnet.into(),
        peers,
    };

    let output = config.to_wg_quick();
    let data = serde_json::to_value(&config)?;

    Ok(ShadowOutcome::ok_with(output, data))
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
                "  {:<12} {:>8} {:>6}  {:<15}  {}  [{}]",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unknown_command_fails() {
        let config = ShadowConfig::default();
        let result = dispatch(&config, "gate.nonexistent_xyz", &[]).await;
        assert!(result.is_ok());
        let outcome = result.unwrap();
        assert!(!outcome.ok);
        assert!(outcome.message.contains("unknown command"));
    }

    #[test]
    fn quorum_dispatch_dry_run() {
        let result = dispatch_quorum(&["--generate", "--dry-run"]);
        assert!(result.is_ok());
        let outcome = result.unwrap();
        assert!(outcome.ok);
        assert!(outcome.message.contains("timer"));
    }

    #[test]
    fn quorum_dispatch_custom_interval() {
        let result = dispatch_quorum(&["--generate", "--interval", "30"]);
        assert!(result.is_ok());
        let outcome = result.unwrap();
        let data = outcome.data.unwrap();
        assert_eq!(data["interval_minutes"], 30);
    }

    #[test]
    fn profile_dispatch_missing_gate() {
        let result = dispatch_profile("nonexistent_gate_xyz");
        match result {
            Ok(outcome) => assert!(!outcome.ok, "unknown gate should fail"),
            Err(_) => {} // workspace not found is acceptable in test
        }
    }
}
