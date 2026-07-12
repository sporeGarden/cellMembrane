// SPDX-License-Identifier: AGPL-3.0-or-later

//! Tower HTTP gateway operations — shadow validation, health probes, config generation.
//!
//! This module orchestrates the Tower gateway (bearDog TLS + songBird mesh routing)
//! that replaces Caddy. During the shadow period, both paths run in parallel and
//! this module compares their responses.
//!
//! Dispatch commands:
//! - `gateway.health` — probe bearDog + songBird health
//! - `gateway.routes` — list active routes from config
//! - `gateway.shadow` — compare legacy (Caddy) vs Tower paths
//! - `gateway.config.validate` — validate gateway config TOML
//! - `gateway.config.generate` — generate gateway config from manifest
//! - `gateway.env` — output env vars for Tower deployment
//! - `gateway.units` — generate systemd units (songBird + bearDog)
//! - `gateway.deploy.check` — pre-deployment readiness validation
//! - `gateway.retire-caddy` — shadow validate then disable Caddy

pub mod config;
pub mod shadow;

use crate::error::{Result, ShadowError};
use crate::{ShadowConfig, ShadowOutcome};

/// Resolve gate name from CLI args, falling back to local gate identity.
fn resolve_gate_arg(args: &[&str]) -> String {
    args.first().map_or_else(crate::gate::resolve_local_gate_identity, |s| {
        (*s).to_owned()
    })
}

use config::{
    format_route_line, generate_from_manifest, load_gateway_config, parse_port,
    to_songbird_proxy_routes,
};

/// Dispatch gateway commands.
pub async fn dispatch(config: &ShadowConfig, cmd: &str, args: &[&str]) -> Result<ShadowOutcome> {
    match cmd {
        "gateway.health" => dispatch_health(config).await,
        "gateway.routes" => dispatch_routes(args),
        "gateway.shadow" => shadow::dispatch_shadow(config, args).await,
        "gateway.config.validate" => dispatch_config_validate(args),
        "gateway.config.generate" => dispatch_config_generate(args),
        "gateway.env" => dispatch_env(args),
        "gateway.units" => dispatch_units(args),
        "gateway.deploy.check" => dispatch_deploy_check(args).await,
        "gateway.retire-caddy" => dispatch_retire_caddy(config, args).await,
        "gateway.sporeprint.units" => Ok(dispatch_sporeprint_units(args)),
        "gateway.sporeprint.check" => Ok(dispatch_sporeprint_check(args)),
        _ => Ok(ShadowOutcome::fail(format!(
            "unknown gateway command: {cmd}"
        ))),
    }
}

/// Probe bearDog TLS + songBird mesh health.
async fn dispatch_health(_config: &ShadowConfig) -> Result<ShadowOutcome> {
    let gateway_bind = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_GATEWAY_BIND,
        cellmembrane_types::service::DEFAULT_GATEWAY_BIND,
    );

    let tls_port = parse_port(&gateway_bind).unwrap_or(443);
    let tls_listening = port_is_listening(tls_port).await;

    let songbird_socket = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_SONGBIRD_SOCKET,
        cellmembrane_types::service::DEFAULT_SONGBIRD_SOCKET,
    );
    let mesh_connected = tokio::fs::metadata(&songbird_socket).await.is_ok();

    let health = cellmembrane_types::gateway::GatewayHealth {
        tls_listening,
        mesh_connected,
        active_routes: 0,
        cert_status: vec![],
        backends_reachable: vec![],
    };

    if tls_listening && mesh_connected {
        Ok(ShadowOutcome::ok_with(
            format!("gateway: healthy (TLS :{tls_port}, mesh socket OK)"),
            serde_json::to_value(&health)?,
        ))
    } else {
        let mut issues = Vec::new();
        if !tls_listening {
            issues.push(format!("TLS not listening on :{tls_port}"));
        }
        if !mesh_connected {
            issues.push(format!("songBird socket missing: {songbird_socket}"));
        }
        Ok(ShadowOutcome {
            ok: false,
            message: format!("gateway: {}", issues.join(", ")),
            data: serde_json::to_value(&health).ok(),
        })
    }
}

/// List routes from gateway config TOML.
fn dispatch_routes(args: &[&str]) -> Result<ShadowOutcome> {
    let config = load_gateway_config(args)?;
    let lines: Vec<String> = config.routes.iter().map(format_route_line).collect();

    Ok(ShadowOutcome::ok_with(
        format!("{} routes for {}", config.routes.len(), config.gate_name),
        serde_json::to_value(&config.routes)?,
    )
    .tap_lines(&lines))
}

/// Validate a gateway config TOML file.
fn dispatch_config_validate(args: &[&str]) -> Result<ShadowOutcome> {
    let config = load_gateway_config(args)?;
    let errors = config.validate();
    if errors.is_empty() {
        Ok(ShadowOutcome::ok(format!(
            "gateway config valid: {} routes for {}",
            config.routes.len(),
            config.gate_name
        )))
    } else {
        Ok(ShadowOutcome::fail(format!(
            "gateway config invalid:\n  {}",
            errors.join("\n  ")
        )))
    }
}

/// Generate a gateway config from the ecosystem manifest.
fn dispatch_config_generate(args: &[&str]) -> Result<ShadowOutcome> {
    let gate_name_owned = resolve_gate_arg(args);
    let gate_name = gate_name_owned.as_str();
    let config = generate_from_manifest(gate_name)?;
    let toml_str = toml::to_string_pretty(&config)
        .map_err(|e| ShadowError::Config(format!("TOML serialize: {e}")))?;
    Ok(ShadowOutcome::ok_with(
        format!("generated gateway config for {gate_name}"),
        serde_json::to_value(&config)?,
    )
    .tap_lines(&toml_str.lines().map(String::from).collect::<Vec<_>>()))
}

/// Output the environment variables needed for the Tower gateway deployment.
fn dispatch_env(args: &[&str]) -> Result<ShadowOutcome> {
    let gate_name_owned = resolve_gate_arg(args);
    let gate_name = gate_name_owned.as_str();
    let config = generate_from_manifest(gate_name)?;
    let routes_env = to_songbird_proxy_routes(&config);

    let lines = vec![
        format!(
            "{}={}",
            cellmembrane_types::service::ENV_GATEWAY_BIND,
            cellmembrane_types::service::DEFAULT_GATEWAY_BIND
        ),
        format!(
            "{}={}",
            cellmembrane_types::service::ENV_SONGBIRD_SOCKET,
            cellmembrane_types::service::DEFAULT_SONGBIRD_SOCKET
        ),
        format!(
            "{}={routes_env}",
            cellmembrane_types::service::ENV_SONGBIRD_PROXY_ROUTES
        ),
        format!("GATE_NAME={gate_name}"),
    ];

    Ok(ShadowOutcome::ok(format!(
        "gateway env for {gate_name} ({} routes)",
        config.routes.len()
    ))
    .tap_lines(&lines))
}

/// Generate systemd unit files for the Tower gateway (songBird + bearDog).
fn dispatch_units(args: &[&str]) -> Result<ShadowOutcome> {
    let gate_name_owned = resolve_gate_arg(args);
    let gate_name = gate_name_owned.as_str();
    let config = generate_from_manifest(gate_name)?;
    let routes_env = to_songbird_proxy_routes(&config);

    let mut params = crate::gate::systemd_units::GatewayUnitParams::for_gate(gate_name);
    params.proxy_routes = &routes_env;

    let (songbird_unit, beardog_unit) = crate::gate::systemd_units::generate_gateway_units(&params);

    let mut lines = vec!["--- songbird-gateway.service ---".to_owned()];
    lines.extend(songbird_unit.lines().map(String::from));
    lines.push(String::new());
    lines.push("--- beardog-gateway.service ---".to_owned());
    lines.extend(beardog_unit.lines().map(String::from));

    Ok(ShadowOutcome::ok_with(
        format!("gateway units generated for {gate_name}"),
        serde_json::json!({
            "gate": gate_name,
            "songbird_unit_lines": songbird_unit.lines().count(),
            "beardog_unit_lines": beardog_unit.lines().count(),
        }),
    )
    .tap_lines(&lines))
}

/// Pre-deployment readiness check for Tower gateway.
///
/// Validates that all prerequisites are met before deploying:
/// - songBird binary exists in depot
/// - bearDog binary exists in depot
/// - Gateway config generates and validates
/// - songBird socket path is writable
///
/// Returns a structured checklist. Does NOT perform any mutations.
async fn dispatch_deploy_check(args: &[&str]) -> Result<ShadowOutcome> {
    let gate_name_owned = resolve_gate_arg(args);
    let gate_name = gate_name_owned.as_str();
    let arch = crate::plasmid::detect_target_triple();

    let depot_dir = crate::gate::resolve_plasmidbin_dir();
    let bin_dir = depot_dir.join("primals").join(&arch);

    let mut checks: Vec<DeployCheck> = Vec::new();

    for svc in cellmembrane_types::service::MembraneService::gateway_primals() {
        let bin_path = bin_dir.join(svc.binary);
        checks.push(DeployCheck {
            name: format!("{} binary", svc.binary),
            ok: bin_path.is_file(),
            detail: if bin_path.is_file() {
                format!("{}", bin_path.display())
            } else {
                format!("missing: {}", bin_path.display())
            },
        });
    }

    let config_ok = generate_from_manifest(gate_name).is_ok();
    checks.push(DeployCheck {
        name: "gateway config".into(),
        ok: config_ok,
        detail: if config_ok {
            format!("generates for {gate_name}")
        } else {
            "manifest parse failed".into()
        },
    });

    let songbird_socket = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_SONGBIRD_SOCKET,
        cellmembrane_types::service::DEFAULT_SONGBIRD_SOCKET,
    );
    let socket_parent = std::path::Path::new(&songbird_socket)
        .parent()
        .is_some_and(std::path::Path::is_dir);
    checks.push(DeployCheck {
        name: "socket directory".into(),
        ok: socket_parent,
        detail: if socket_parent {
            songbird_socket
        } else {
            format!("parent dir missing: {songbird_socket}")
        },
    });

    let songbird_running = tokio::net::TcpStream::connect((
        cellmembrane_types::service::BIND_LOOPBACK,
        cellmembrane_types::service::DEFAULT_FEDERATION_PORT,
    ))
    .await
    .is_ok();
    checks.push(DeployCheck {
        name: "songbird reachable".into(),
        ok: songbird_running,
        detail: if songbird_running {
            format!("TCP :{} open", cellmembrane_types::service::DEFAULT_FEDERATION_PORT)
        } else {
            format!("TCP :{} closed", cellmembrane_types::service::DEFAULT_FEDERATION_PORT)
        },
    });

    let all_pass = checks.iter().all(|c| c.ok);
    let passed = checks.iter().filter(|c| c.ok).count();
    let total = checks.len();

    let lines: Vec<String> = checks
        .iter()
        .map(|c| {
            let mark = if c.ok { "✓" } else { "✗" };
            format!("  [{mark}] {}: {}", c.name, c.detail)
        })
        .collect();

    let summary = format!("deploy check: {passed}/{total} pass ({gate_name}, {arch})");

    if all_pass {
        Ok(ShadowOutcome::ok(summary).tap_lines(&lines))
    } else {
        Ok(ShadowOutcome {
            ok: false,
            message: format!("{summary}\n{}", lines.join("\n")),
            data: serde_json::to_value(&checks).ok(),
        })
    }
}

/// A single deployment readiness check result.
#[derive(Debug, Clone, serde::Serialize)]
struct DeployCheck {
    name: String,
    ok: bool,
    detail: String,
}

/// Orchestrate Caddy retirement — shadow validate then disable.
///
/// Steps:
/// 1. Run shadow comparison (Caddy :443 vs Tower :8443)
/// 2. If all routes pass → stop + disable Caddy systemd unit
/// 3. If `--dry-run` flag is present, report without acting
///
/// This command is idempotent: if Caddy is already stopped, it reports success.
async fn dispatch_retire_caddy(config: &ShadowConfig, args: &[&str]) -> Result<ShadowOutcome> {
    let dry_run = args.contains(&"--dry-run");

    let shadow_result = shadow::dispatch_shadow(config, args).await?;

    if !shadow_result.ok {
        return Ok(ShadowOutcome {
            ok: false,
            message: format!(
                "retirement blocked: shadow validation failed\n{}",
                shadow_result.message
            ),
            data: shadow_result.data,
        });
    }

    if dry_run {
        return Ok(ShadowOutcome::ok(format!(
            "dry-run: shadow passes — would disable caddy.service\n{}",
            shadow_result.message
        )));
    }

    let stopped = crate::gate::nucleus::systemctl_async(&["stop", "caddy"]).await;
    let disabled = crate::gate::nucleus::systemctl_async(&["disable", "caddy"]).await;

    let detail = match (stopped, disabled) {
        (true, true) => "caddy.service stopped + disabled".to_owned(),
        (true, false) => {
            "caddy.service stopped (disable failed — may already be disabled)".to_owned()
        }
        (false, _) => "caddy.service stop failed (may already be stopped)".to_owned(),
    };

    Ok(ShadowOutcome::ok(format!(
        "retirement complete: {detail}\n{}",
        shadow_result.message
    )))
}

/// Generate all 4 sporePrint NUCLEUS systemd units (petalTongue + nestGate + songBird + bearDog).
fn dispatch_sporeprint_units(args: &[&str]) -> ShadowOutcome {
    let resolved_gate = crate::gate::resolve_local_gate_identity();
    let gate_name = args.first().copied().unwrap_or(resolved_gate.as_str());
    let resolved_domain = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_DEPOT_HOSTNAME,
        "primals.eco",
    );
    let domain = crate::cli::extract_flag_value(args, "--domain")
        .unwrap_or(resolved_domain.as_str());

    let params = crate::gate::sporeprint::SporePrintDeployParams::new(gate_name, domain);
    let units = crate::gate::sporeprint::generate_sporeprint_units(&params);

    let filenames = crate::gate::sporeprint::SporePrintUnits::filenames();
    let mut lines = Vec::new();
    for (filename, content) in units.iter() {
        lines.push(format!("--- {filename} ---"));
        lines.extend(content.lines().map(String::from));
        lines.push(String::new());
    }

    ShadowOutcome::ok_with(
        format!(
            "sporePrint NUCLEUS units for {gate_name} (domain={domain}, {} units)",
            filenames.len()
        ),
        serde_json::json!({
            "gate": gate_name,
            "domain": domain,
            "units": filenames,
            "composition": "sporePrint",
        }),
    )
    .tap_lines(&lines)
}

/// Pre-deployment readiness check for sporePrint NUCLEUS on a target gate.
fn dispatch_sporeprint_check(args: &[&str]) -> ShadowOutcome {
    let resolved_gate = crate::gate::resolve_local_gate_identity();
    let gate_name = args.first().copied().unwrap_or(resolved_gate.as_str());
    let arch = crate::plasmid::detect_target_triple();

    let depot_dir = crate::gate::resolve_plasmidbin_dir();
    let bin_dir = depot_dir.join("primals").join(&arch);

    let mut checks: Vec<DeployCheck> = Vec::new();

    for &binary in cellmembrane_types::service::SPOREPRINT_NUCLEUS_BINARIES {
        let bin_path = bin_dir.join(binary);
        checks.push(DeployCheck {
            name: format!("{binary} binary"),
            ok: bin_path.is_file(),
            detail: if bin_path.is_file() {
                format!("{}", bin_path.display())
            } else {
                format!("missing: {}", bin_path.display())
            },
        });
    }

    let root = crate::temporal::resolve_workspace_root().ok();
    let manifest_ok = root.as_ref().is_some_and(|r| {
        crate::manifest::EcosystemManifest::find_in_workspace(r).is_some()
    });
    checks.push(DeployCheck {
        name: "ecosystem manifest".into(),
        ok: manifest_ok,
        detail: if manifest_ok {
            "found".into()
        } else {
            "not found".into()
        },
    });

    if let Some(ref r) = root {
        let sporeprint_dir = r.join(cellmembrane_types::service::SPOREPRINT_CONTENT_DIR);
        let has_config = sporeprint_dir.join("config.toml").is_file();
        checks.push(DeployCheck {
            name: "sporePrint site".into(),
            ok: has_config,
            detail: if has_config {
                format!("{}", sporeprint_dir.display())
            } else {
                format!("no config.toml in {}", sporeprint_dir.display())
            },
        });

        let has_public = sporeprint_dir.join("public").is_dir();
        checks.push(DeployCheck {
            name: "sporePrint public/".into(),
            ok: has_public,
            detail: if has_public {
                "built (zola build output exists)".into()
            } else {
                "missing — run `zola build` in sporePrint dir".into()
            },
        });
    }

    if let Some(ref r) = root {
        let manifest = crate::manifest::load_from_workspace(r).ok();
        let gate_in_manifest = manifest
            .as_ref()
            .is_some_and(|m| m.gates.contains_key(gate_name));
        checks.push(DeployCheck {
            name: format!("{gate_name} gate profile"),
            ok: gate_in_manifest,
            detail: if gate_in_manifest {
                "in manifest".into()
            } else {
                "gate not in ecosystem_manifest.toml".into()
            },
        });

        let ssh_target = manifest
            .as_ref()
            .and_then(|m| m.ssh_target_for(gate_name));
        checks.push(DeployCheck {
            name: format!("{gate_name} SSH target"),
            ok: ssh_target.is_some(),
            detail: ssh_target.unwrap_or("no routable address").to_string(),
        });
    }

    let all_pass = checks.iter().all(|c| c.ok);
    let passed = checks.iter().filter(|c| c.ok).count();
    let total = checks.len();

    let lines: Vec<String> = checks
        .iter()
        .map(|c| {
            let mark = if c.ok { "\u{2713}" } else { "\u{2717}" };
            format!("  [{mark}] {}: {}", c.name, c.detail)
        })
        .collect();

    let summary = format!(
        "sporePrint NUCLEUS deploy check: {passed}/{total} pass ({gate_name}, {arch})"
    );

    if all_pass {
        ShadowOutcome::ok(summary).tap_lines(&lines)
    } else {
        ShadowOutcome {
            ok: false,
            message: format!("{summary}\n{}", lines.join("\n")),
            data: serde_json::to_value(&checks).ok(),
        }
    }
}

/// Check if a port has a listener (best-effort via TCP connect to loopback).
async fn port_is_listening(port: u16) -> bool {
    tokio::net::TcpStream::connect((cellmembrane_types::service::BIND_LOOPBACK, port))
        .await
        .is_ok()
}

// ── Extension trait for outcome display ─────────────────────────────

trait TapLines {
    fn tap_lines(self, lines: &[String]) -> Self;
}

impl TapLines for ShadowOutcome {
    fn tap_lines(mut self, lines: &[String]) -> Self {
        if !lines.is_empty() {
            self.message = format!("{}\n{}", self.message, lines.join("\n"));
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_unknown_command() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let config = crate::ShadowConfig::from_env().await;
            let result = dispatch(&config, "gateway.nonexistent", &[]).await.unwrap();
            assert!(!result.ok);
            assert!(result.message.contains("unknown gateway command"));
        });
    }

    #[tokio::test]
    async fn deploy_check_returns_structured_result() {
        let result = dispatch_deploy_check(&["testGate"]).await.unwrap();
        assert!(
            result.message.contains("deploy check"),
            "message should mention deploy check, got: {}",
            result.message
        );
        assert!(result.message.contains("testGate"));
    }

    #[tokio::test]
    async fn deploy_check_default_gate() {
        let result = dispatch_deploy_check(&[]).await.unwrap();
        assert!(result.message.contains("sporeGate"));
    }

    #[test]
    fn sporeprint_units_generates_output() {
        let result = dispatch_sporeprint_units(&["golgiBody", "--domain", "primals.eco"]);
        assert!(result.ok);
        assert!(result.message.contains("golgiBody"));
        assert!(result.message.contains("primals.eco"));
        assert!(result.message.contains("sporePrint NUCLEUS"));
    }

    #[test]
    fn sporeprint_units_default_gate() {
        let result = dispatch_sporeprint_units(&[]);
        assert!(result.ok);
        assert!(result.message.contains("sporePrint NUCLEUS"));
    }

    #[test]
    fn sporeprint_check_returns_structured_result() {
        let result = dispatch_sporeprint_check(&["golgiBody"]);
        assert!(result.message.contains("sporePrint NUCLEUS deploy check"));
        assert!(result.message.contains("golgiBody"));
    }

    #[test]
    fn sporeprint_check_default_gate() {
        let result = dispatch_sporeprint_check(&[]);
        assert!(result.message.contains("sporePrint NUCLEUS deploy check"));
    }

    #[test]
    fn deploy_check_struct_serializes() {
        let check = DeployCheck {
            name: "songbird binary".into(),
            ok: true,
            detail: "/opt/membrane/primals/x86_64-unknown-linux-musl/songbird".into(),
        };
        let json = serde_json::to_value(&check).unwrap();
        assert_eq!(json["name"], "songbird binary");
        assert_eq!(json["ok"], true);
    }
}

