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
//! - `gateway.retire-caddy` — shadow validate then disable Caddy

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
        "gateway.env" => dispatch_env(args),
        "gateway.units" => dispatch_units(args),
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

/// Generate the `SONGBIRD_PROXY_ROUTES` env value from a gateway config.
///
/// songBird uses a comma-separated format: `host/path=capability,host/path=capability,...`
/// This bridges our typed `GatewayConfig` to songBird's runtime route table.
#[must_use]
pub fn to_songbird_proxy_routes(config: &GatewayConfig) -> String {
    config
        .routes
        .iter()
        .map(|r| {
            let path = if r.path_prefix.is_empty() {
                "/*"
            } else {
                &r.path_prefix
            };
            format!("{}{path}={}", r.host, r.capability)
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Parse a `SONGBIRD_PROXY_ROUTES` env value back into route entries.
///
/// Inverse of [`to_songbird_proxy_routes`] — parses the runtime format back
/// into typed routes for validation or display.
#[must_use]
pub fn parse_songbird_proxy_routes(env_val: &str) -> Vec<GatewayRoute> {
    env_val
        .split(',')
        .filter(|s| !s.is_empty())
        .filter_map(|entry| {
            let (host_path, capability) = entry.split_once('=')?;
            let (host, path_prefix) = host_path.find('/').map_or((host_path, "/*"), |pos| {
                (&host_path[..pos], &host_path[pos..])
            });
            Some(GatewayRoute {
                host: host.to_owned(),
                path_prefix: path_prefix.to_owned(),
                capability: capability.to_owned(),
                timeout_secs: cellmembrane_types::service::DEFAULT_GATEWAY_TIMEOUT_SECS,
            })
        })
        .collect()
}

/// Generate default gateway routes based on a gate's roles.
///
/// Pure function: maps role strings to default route configurations.
/// Gates with `http` or `gateway` roles get the standard `JupyterHub` routes.
#[must_use]
pub fn default_routes_for_roles(roles: &[String]) -> Vec<GatewayRoute> {
    let has_http_role = roles
        .iter()
        .any(|r| r.contains("http") || r.contains("gateway"));

    if !has_http_role {
        return Vec::new();
    }

    let timeout = cellmembrane_types::service::DEFAULT_GATEWAY_TIMEOUT_SECS;
    let host = "lab.primals.eco";

    vec![
        GatewayRoute {
            host: host.into(),
            path_prefix: "/hub".into(),
            capability: "jupyter".into(),
            timeout_secs: timeout,
        },
        GatewayRoute {
            host: host.into(),
            path_prefix: "/user".into(),
            capability: "jupyter".into(),
            timeout_secs: timeout,
        },
        GatewayRoute {
            host: host.into(),
            path_prefix: "/api".into(),
            capability: "jupyter".into(),
            timeout_secs: timeout,
        },
        GatewayRoute {
            host: host.into(),
            path_prefix: "/services".into(),
            capability: "jupyter".into(),
            timeout_secs: timeout,
        },
    ]
}

/// Generate a gateway config from the ecosystem manifest for a specific gate.
pub fn generate_from_manifest(gate_name: &str) -> Result<GatewayConfig> {
    let root = crate::temporal::resolve_workspace_root()?;
    let manifest = crate::manifest::load_from_workspace(&root)?;
    let profile = manifest.gates.get(gate_name).ok_or_else(|| {
        ShadowError::Config(format!("gate '{gate_name}' not in ecosystem manifest"))
    })?;

    let routes = default_routes_for_roles(&profile.roles);

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
    fn default_routes_for_http_role() {
        let roles = vec!["http".to_owned(), "ci".to_owned()];
        let routes = default_routes_for_roles(&roles);
        assert_eq!(routes.len(), 4);
        assert!(routes.iter().all(|r| r.host == "lab.primals.eco"));
        assert!(routes.iter().any(|r| r.path_prefix == "/hub"));
        assert!(routes.iter().any(|r| r.path_prefix == "/user"));
        assert!(routes.iter().any(|r| r.path_prefix == "/api"));
        assert!(routes.iter().any(|r| r.path_prefix == "/services"));
    }

    #[test]
    fn default_routes_for_gateway_role() {
        let roles = vec!["gateway".to_owned()];
        let routes = default_routes_for_roles(&roles);
        assert_eq!(routes.len(), 4);
    }

    #[test]
    fn default_routes_for_non_http_role() {
        let roles = vec!["ci".to_owned(), "compute".to_owned()];
        let routes = default_routes_for_roles(&roles);
        assert!(routes.is_empty());
    }

    #[test]
    fn default_routes_empty_roles() {
        let routes = default_routes_for_roles(&[]);
        assert!(routes.is_empty());
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

    #[test]
    fn to_songbird_proxy_routes_generates_env() {
        let config = GatewayConfig {
            gate_name: "sporeGate".into(),
            enabled: true,
            max_connections: 100,
            default_timeout_secs: 30,
            routes: vec![
                GatewayRoute {
                    host: "lab.primals.eco".into(),
                    path_prefix: "/hub".into(),
                    capability: "jupyter".into(),
                    timeout_secs: 30,
                },
                GatewayRoute {
                    host: "lab.primals.eco".into(),
                    path_prefix: "/api".into(),
                    capability: "jupyter".into(),
                    timeout_secs: 30,
                },
            ],
        };
        let env = to_songbird_proxy_routes(&config);
        assert_eq!(
            env,
            "lab.primals.eco/hub=jupyter,lab.primals.eco/api=jupyter"
        );
    }

    #[test]
    fn to_songbird_proxy_routes_empty_path() {
        let config = GatewayConfig {
            gate_name: "test".into(),
            enabled: true,
            max_connections: 100,
            default_timeout_secs: 30,
            routes: vec![GatewayRoute {
                host: "lab.primals.eco".into(),
                path_prefix: String::new(),
                capability: "compute".into(),
                timeout_secs: 30,
            }],
        };
        let env = to_songbird_proxy_routes(&config);
        assert_eq!(env, "lab.primals.eco/*=compute");
    }

    #[test]
    fn parse_songbird_proxy_routes_roundtrip() {
        let env_val = "lab.primals.eco/hub=jupyter,lab.primals.eco/api=jupyter";
        let routes = parse_songbird_proxy_routes(env_val);
        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0].host, "lab.primals.eco");
        assert_eq!(routes[0].path_prefix, "/hub");
        assert_eq!(routes[0].capability, "jupyter");
        assert_eq!(routes[1].path_prefix, "/api");
    }

    #[test]
    fn parse_songbird_proxy_routes_empty() {
        assert!(parse_songbird_proxy_routes("").is_empty());
    }

    #[test]
    fn parse_songbird_proxy_routes_no_path() {
        let routes = parse_songbird_proxy_routes("example.com=service");
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].host, "example.com");
        assert_eq!(routes[0].path_prefix, "/*");
        assert_eq!(routes[0].capability, "service");
    }

    #[test]
    fn parse_songbird_proxy_routes_skips_invalid() {
        let routes = parse_songbird_proxy_routes("valid.host/path=cap,invalid_no_equals");
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].host, "valid.host");
    }
}
