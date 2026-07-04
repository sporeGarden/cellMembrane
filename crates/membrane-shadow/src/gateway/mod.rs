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

pub mod shadow;

use crate::error::{Result, ShadowError};
use crate::{ShadowConfig, ShadowOutcome};
use cellmembrane_types::gateway::{GatewayConfig, GatewayRoute};

/// Dispatch gateway commands.
pub async fn dispatch(config: &ShadowConfig, cmd: &str, args: &[&str]) -> Result<ShadowOutcome> {
    match cmd {
        "gateway.health" => dispatch_health(config).await,
        "gateway.routes" => dispatch_routes(args),
        "gateway.shadow" => shadow::dispatch_shadow(config, args).await,
        "gateway.config.validate" => dispatch_config_validate(args),
        "gateway.config.generate" => dispatch_config_generate(args),
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

// ── Pure helpers ─────────────────────────────────────────────────────

/// Format a route for display.
#[must_use]
pub fn format_route_line(route: &GatewayRoute) -> String {
    let path = if route.path_prefix.is_empty() {
        "/*"
    } else {
        &route.path_prefix
    };
    format!(
        "  {}{} → {} ({}s)",
        route.host, path, route.capability, route.timeout_secs
    )
}

/// Parse a port number from a bind address string (e.g. "0.0.0.0:443" → 443).
#[must_use]
pub fn parse_port(bind: &str) -> Option<u16> {
    bind.rsplit(':').next()?.parse().ok()
}

/// Generate a gateway config from the ecosystem manifest for a specific gate.
pub fn generate_from_manifest(gate_name: &str) -> Result<GatewayConfig> {
    let root = crate::temporal::resolve_workspace_root()?;
    let manifest = crate::manifest::load_from_workspace(&root)?;
    let profile = manifest.gates.get(gate_name).ok_or_else(|| {
        ShadowError::Config(format!("gate '{gate_name}' not in ecosystem manifest"))
    })?;

    let roles = &profile.roles;
    let mut routes = Vec::new();

    if roles
        .iter()
        .any(|r| r.contains("http") || r.contains("gateway"))
    {
        routes.push(GatewayRoute {
            host: "lab.primals.eco".into(),
            path_prefix: "/hub".into(),
            capability: "jupyter".into(),
            timeout_secs: cellmembrane_types::service::DEFAULT_GATEWAY_TIMEOUT_SECS,
        });
        routes.push(GatewayRoute {
            host: "lab.primals.eco".into(),
            path_prefix: "/user".into(),
            capability: "jupyter".into(),
            timeout_secs: cellmembrane_types::service::DEFAULT_GATEWAY_TIMEOUT_SECS,
        });
        routes.push(GatewayRoute {
            host: "lab.primals.eco".into(),
            path_prefix: "/api".into(),
            capability: "jupyter".into(),
            timeout_secs: cellmembrane_types::service::DEFAULT_GATEWAY_TIMEOUT_SECS,
        });
        routes.push(GatewayRoute {
            host: "lab.primals.eco".into(),
            path_prefix: "/services".into(),
            capability: "jupyter".into(),
            timeout_secs: cellmembrane_types::service::DEFAULT_GATEWAY_TIMEOUT_SECS,
        });
    }

    Ok(GatewayConfig {
        gate_name: gate_name.into(),
        enabled: true,
        max_connections: cellmembrane_types::service::DEFAULT_GATEWAY_MAX_CONNECTIONS,
        default_timeout_secs: cellmembrane_types::service::DEFAULT_GATEWAY_TIMEOUT_SECS,
        routes,
    })
}

/// Check if a port has a listener (best-effort via TCP connect to loopback).
async fn port_is_listening(port: u16) -> bool {
    tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .is_ok()
}

/// Load gateway config from a TOML file path (first arg) or default location.
fn load_gateway_config(args: &[&str]) -> Result<GatewayConfig> {
    let path = args.first().map_or_else(
        || {
            let config_dir = cellmembrane_types::service::env_or(
                cellmembrane_types::service::ENV_CONFIG_DIR,
                cellmembrane_types::service::DEFAULT_CONFIG_DIR,
            );
            format!("{config_dir}/gateway.toml")
        },
        |p| (*p).to_string(),
    );

    let content = std::fs::read_to_string(&path)
        .map_err(|e| ShadowError::Config(format!("cannot read gateway config at {path}: {e}")))?;

    toml::from_str(&content)
        .map_err(|e| ShadowError::Config(format!("invalid gateway config: {e}")))
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
    fn parse_port_from_bind() {
        assert_eq!(parse_port("0.0.0.0:443"), Some(443));
        assert_eq!(parse_port("127.0.0.1:8443"), Some(8443));
        assert_eq!(parse_port("443"), Some(443));
    }

    #[test]
    fn parse_port_invalid() {
        assert_eq!(parse_port(""), None);
        assert_eq!(parse_port("no-port"), None);
    }

    #[test]
    fn format_route_line_with_path() {
        let route = GatewayRoute {
            host: "lab.primals.eco".into(),
            path_prefix: "/hub".into(),
            capability: "jupyter".into(),
            timeout_secs: 30,
        };
        let line = format_route_line(&route);
        assert!(line.contains("lab.primals.eco/hub"));
        assert!(line.contains("jupyter"));
        assert!(line.contains("30s"));
    }

    #[test]
    fn format_route_line_empty_path() {
        let route = GatewayRoute {
            host: "lab.primals.eco".into(),
            path_prefix: String::new(),
            capability: "compute".into(),
            timeout_secs: 60,
        };
        let line = format_route_line(&route);
        assert!(line.contains("/*"));
        assert!(line.contains("compute"));
    }

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
}
