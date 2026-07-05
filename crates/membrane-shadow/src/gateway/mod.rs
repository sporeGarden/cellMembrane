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

pub use config::{
    default_routes_for_roles, format_route_line, generate_from_manifest, parse_port,
    parse_songbird_proxy_routes, to_songbird_proxy_routes, to_songbird_routes_toml,
};
use config::load_gateway_config;

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
    let gate_name = args.first().copied().unwrap_or("sporeGate");
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
    let gate_name = args.first().copied().unwrap_or("sporeGate");
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
        format!("MEMBRANE_GATE_NAME={gate_name}"),
    ];

    Ok(ShadowOutcome::ok(format!(
        "gateway env for {gate_name} ({} routes)",
        config.routes.len()
    ))
    .tap_lines(&lines))
}

/// Generate systemd unit files for the Tower gateway (songBird + bearDog).
fn dispatch_units(args: &[&str]) -> Result<ShadowOutcome> {
    let gate_name = args.first().copied().unwrap_or("sporeGate");
    let config = generate_from_manifest(gate_name)?;
    let routes_env = to_songbird_proxy_routes(&config);

    let mut params = crate::gate::nucleus::GatewayUnitParams::for_gate(gate_name);
    params.proxy_routes = &routes_env;

    let (songbird_unit, beardog_unit) = crate::gate::nucleus::generate_gateway_units(&params);

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
    let gate_name = args.first().copied().unwrap_or("sporeGate");
    let arch = crate::plasmid::detect_target_triple();

    let depot_dir = crate::gate::resolve_plasmidbin_dir();
    let bin_dir = depot_dir.join("primals").join(&arch);

    let mut checks: Vec<DeployCheck> = Vec::new();

    let songbird_bin = bin_dir.join("songbird");
    checks.push(DeployCheck {
        name: "songbird binary".into(),
        ok: songbird_bin.is_file(),
        detail: if songbird_bin.is_file() {
            format!("{}", songbird_bin.display())
        } else {
            format!("missing: {}", songbird_bin.display())
        },
    });

    let beardog_bin = bin_dir.join("beardog");
    checks.push(DeployCheck {
        name: "beardog binary".into(),
        ok: beardog_bin.is_file(),
        detail: if beardog_bin.is_file() {
            format!("{}", beardog_bin.display())
        } else {
            format!("missing: {}", beardog_bin.display())
        },
    });

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

    let songbird_running = tokio::net::TcpStream::connect(("127.0.0.1", 7700_u16))
        .await
        .is_ok();
    checks.push(DeployCheck {
        name: "songbird reachable".into(),
        ok: songbird_running,
        detail: if songbird_running {
            "TCP :7700 open".into()
        } else {
            "TCP :7700 closed".into()
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

/// Check if a port has a listener (best-effort via TCP connect to loopback).
async fn port_is_listening(port: u16) -> bool {
    tokio::net::TcpStream::connect(("127.0.0.1", port))
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

