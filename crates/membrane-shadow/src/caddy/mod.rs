// SPDX-License-Identifier: AGPL-3.0-or-later

//! Caddy reverse proxy management — agentic TLS and vhost operations.
//!
//! Wraps Caddy's admin API (localhost:2019) via SSH to golgiBody-ext.
//! Supports TLS certificate inspection, vhost listing, config validation,
//! and graceful reloads.
//!
//! The outer membrane (golgiBody-ext) runs Caddy for TLS termination.
//! Inner membrane calls this module through SSH, never directly.

pub mod depot;
pub mod tls;

use crate::ShadowConfig;
use crate::error::{Result, ShadowError};
use serde::{Deserialize, Serialize};

fn caddy_admin_endpoint() -> &'static str {
    static ENDPOINT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ENDPOINT.get_or_init(|| {
        std::env::var(cellmembrane_types::service::ENV_CADDY_ADMIN_ENDPOINT)
            .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_CADDY_ADMIN_ENDPOINT.into())
    })
}

fn caddy_bin_path() -> &'static str {
    static BIN: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    BIN.get_or_init(|| {
        let vps_bin_dir = std::env::var(cellmembrane_types::service::ENV_VPS_BIN_DIR)
            .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_INSTALL_BASE.into());
        format!("{vps_bin_dir}/caddy")
    })
}

fn caddyfile_path() -> &'static str {
    static PATH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    PATH.get_or_init(|| {
        let config_dir = std::env::var(cellmembrane_types::service::ENV_CONFIG_DIR)
            .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_CONFIG_DIR.into());
        format!("{config_dir}/Caddyfile")
    })
}

/// Execute a command on the Caddy host (golgiBody inner membrane), returning stdout and exit code.
async fn caddy_exec(config: &ShadowConfig, command: &str) -> Result<(String, i32)> {
    crate::ssh::exec_raw_on(&config.ssh_host, config.ssh_timeout, command).await
}

// ── Types ───────────────────────────────────────────────────────────

/// TLS certificate status for a domain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertStatus {
    /// Domain name.
    pub domain: String,
    /// Whether the certificate is valid.
    pub valid: bool,
    /// Certificate issuer (e.g. "Let's Encrypt").
    pub issuer: String,
    /// Expiry date (ISO 8601).
    pub expires: String,
    /// Days until expiry.
    pub days_remaining: i64,
    /// Any error during check.
    pub error: Option<String>,
}

/// Caddy vhost entry parsed from the Caddyfile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VhostEntry {
    /// Domain/address block.
    pub address: String,
    /// Whether it has TLS configured.
    pub tls_enabled: bool,
    /// Upstream targets (`reverse_proxy` destinations).
    pub upstreams: Vec<String>,
}

/// Caddy service health.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaddyHealth {
    /// Whether the Caddy systemd unit is active.
    pub service_active: bool,
    /// Whether the admin API responds.
    pub admin_api_ok: bool,
    /// Caddyfile path on VPS.
    pub config_path: String,
    /// Number of configured vhosts.
    pub vhost_count: usize,
    /// Listening addresses.
    pub listeners: Vec<String>,
}

// ── Core Operations ─────────────────────────────────────────────────

/// Check Caddy service health on golgiBody-ext.
pub async fn status(config: &ShadowConfig) -> Result<CaddyHealth> {
    let (active_out, active_code) = caddy_exec(
        config,
        &format!(
            "systemctl is-active {} 2>/dev/null || echo inactive",
            cellmembrane_types::service::CADDY_SERVICE_UNIT
        ),
    )
    .await?;
    let service_active = active_out.trim() == "active" && active_code == 0;

    let endpoint = caddy_admin_endpoint();
    let (api_out, api_code) =
        caddy_exec(config, &format!("curl -sf {endpoint}/config/ 2>/dev/null")).await?;
    let admin_api_ok = api_code == 0 && !api_out.trim().is_empty();

    let caddyfile = caddyfile_path();
    let (vhost_out, _) = caddy_exec(config, &format!("cat {caddyfile} 2>/dev/null")).await?;
    let entries = parse_caddyfile_vhosts(&vhost_out);

    let (listen_out, _) = caddy_exec(
        config,
        &format!(
            "curl -sf {endpoint}/config/apps/http/servers/ 2>/dev/null | \
                  grep -oP '\"listen\":\\s*\\[\\K[^\\]]+' 2>/dev/null || echo ''"
        ),
    )
    .await?;
    let listeners: Vec<String> = listen_out
        .split(',')
        .map(|s| s.trim().trim_matches('"').to_string())
        .filter(|s| !s.is_empty())
        .collect();

    Ok(CaddyHealth {
        service_active,
        admin_api_ok,
        config_path: caddyfile.to_string(),
        vhost_count: entries.len(),
        listeners,
    })
}

/// List configured vhosts from the Caddyfile.
pub async fn vhosts(config: &ShadowConfig) -> Result<Vec<VhostEntry>> {
    let caddyfile = caddyfile_path();
    let (out, _) = caddy_exec(config, &format!("cat {caddyfile} 2>/dev/null")).await?;
    Ok(parse_caddyfile_vhosts(&out))
}

/// Reload Caddy configuration (graceful — zero downtime).
pub async fn reload(config: &ShadowConfig) -> Result<String> {
    let caddy_bin = caddy_bin_path();
    let caddyfile = caddyfile_path();
    let cmd = format!(
        "{caddy_bin} validate --config {caddyfile} 2>&1 && \
         {caddy_bin} reload --config {caddyfile} --force 2>&1"
    );
    let (out, code) = caddy_exec(config, &cmd).await?;
    if code != 0 {
        return Err(ShadowError::Ssh(format!(
            "Caddy reload failed: {}",
            out.trim()
        )));
    }
    Ok(format!("Caddy reloaded: {}", out.trim()))
}

/// Validate Caddyfile syntax without applying.
pub async fn validate(config: &ShadowConfig) -> Result<String> {
    let caddy_bin = caddy_bin_path();
    let caddyfile = caddyfile_path();
    let (out, code) = caddy_exec(
        config,
        &format!("{caddy_bin} validate --config {caddyfile} 2>&1"),
    )
    .await?;
    if code != 0 {
        return Err(ShadowError::Ssh(format!(
            "Caddyfile validation failed: {}",
            out.trim()
        )));
    }
    Ok(format!("Caddyfile valid: {}", out.trim()))
}

// ── Dispatch ────────────────────────────────────────────────────────

/// Dispatch `caddy.*` CLI commands.
pub async fn dispatch(
    config: &ShadowConfig,
    cmd: &str,
    args: &[&str],
) -> Result<crate::ShadowOutcome> {
    match cmd {
        "caddy.status" => {
            let health = status(config).await?;
            let json = serde_json::to_string_pretty(&health)?;
            Ok(crate::ShadowOutcome::ok(json))
        }
        "caddy.tls.check" => {
            let domain = args.first().ok_or_else(|| {
                ShadowError::Config("domain required: membrane caddy.tls.check <domain>".into())
            })?;
            let cert = tls::tls_check(config, domain).await?;
            let json = serde_json::to_string_pretty(&cert)?;
            Ok(crate::ShadowOutcome::ok(json))
        }
        "caddy.vhosts" => {
            let entries = vhosts(config).await?;
            let json = serde_json::to_string_pretty(&entries)?;
            Ok(crate::ShadowOutcome::ok(json))
        }
        "caddy.reload" => {
            let msg = reload(config).await?;
            Ok(crate::ShadowOutcome::ok(msg))
        }
        "caddy.validate" => {
            let msg = validate(config).await?;
            Ok(crate::ShadowOutcome::ok(msg))
        }
        "caddy.acme.log" => {
            let lines: u32 = args.first().and_then(|v| v.parse().ok()).unwrap_or(50);
            let log = tls::acme_log(config, lines).await?;
            Ok(crate::ShadowOutcome::ok(log))
        }
        "caddy.depot.provision" => {
            let msg = depot::depot_provision(config).await?;
            Ok(crate::ShadowOutcome::ok(msg))
        }
        "caddy.depot.checksums" => {
            let msg = depot::depot_checksums_provision(config).await?;
            Ok(crate::ShadowOutcome::ok(msg))
        }
        "caddy.tls.external" => {
            let domain = args.first().ok_or_else(|| {
                ShadowError::Config("domain required: membrane caddy.tls.external <domain>".into())
            })?;
            let msg = tls::tls_external(config, domain).await?;
            Ok(crate::ShadowOutcome::ok(msg))
        }
        "caddy.tls.revert" => {
            let domain = args.first().ok_or_else(|| {
                ShadowError::Config("domain required: membrane caddy.tls.revert <domain>".into())
            })?;
            let msg = tls::tls_revert_acme(config, domain).await?;
            Ok(crate::ShadowOutcome::ok(msg))
        }
        "caddy.generate" => dispatch_caddy_generate(args).await,
        _ => Ok(crate::ShadowOutcome::fail(format!(
            "unknown caddy command: {cmd}"
        ))),
    }
}

// ── Manifest-driven Caddyfile generation ────────────────────────────

async fn dispatch_caddy_generate(args: &[&str]) -> Result<crate::ShadowOutcome> {
    use cellmembrane_types::caddy::{CaddyConfig, CaddyVhost};

    let root = crate::temporal::resolve_workspace_root()?;
    let m = crate::manifest::load_from_workspace_async(&root).await?;

    let gate_name = crate::cli::extract_flag_value(args, "--gate")
        .unwrap_or_else(|| crate::gate::resolve_local_gate_identity().leak());

    let acme_email = crate::cli::extract_flag_value(args, "--email").map(String::from);

    let topo = m.topology.as_ref();
    let mut vhosts = Vec::new();

    let caddy_gates = m.gates_for_role("caddy");
    let is_caddy_gate = caddy_gates.iter().any(|(name, _)| *name == gate_name);

    if !is_caddy_gate && caddy_gates.is_empty() {
        return Ok(crate::ShadowOutcome::ok(String::from(
            "no gate has role 'caddy' — add roles = [\"caddy\"] to a gate profile",
        )));
    }

    if let Some(topo_data) = topo {
        if let Some(inner_ip) = topo_data.hosts.get(&topo_data.inner_membrane) {
            let forgejo_gates = m.gates_for_role("forgejo");
            if let Some((_, forgejo_profile)) = forgejo_gates.first() {
                let forgejo_ip = forgejo_profile
                    .wg_ip
                    .as_deref()
                    .or_else(|| cellmembrane_types::cytoplasm::mesh_address(forgejo_gates[0].0))
                    .unwrap_or(inner_ip.as_str());
                vhosts.push(CaddyVhost {
                    domain: "git.primals.eco".into(),
                    upstream: format!("{forgejo_ip}:3000"),
                    path: None,
                    tls: true,
                    extra_directives: vec![],
                });
            }

            let depot_gates = m.gates_for_role("depot");
            if !depot_gates.is_empty() {
                let depot_ip = depot_gates[0]
                    .1
                    .wg_ip
                    .as_deref()
                    .or_else(|| cellmembrane_types::cytoplasm::mesh_address(depot_gates[0].0))
                    .unwrap_or(inner_ip.as_str());
                vhosts.push(CaddyVhost {
                    domain: "depot.primals.eco".into(),
                    upstream: format!("{depot_ip}:8080"),
                    path: None,
                    tls: true,
                    extra_directives: vec![],
                });
            }

            let relay_gates = m.gates_for_role("relay");
            if !relay_gates.is_empty() {
                let relay_ip = relay_gates[0]
                    .1
                    .wg_ip
                    .as_deref()
                    .or_else(|| cellmembrane_types::cytoplasm::mesh_address(relay_gates[0].0))
                    .unwrap_or(inner_ip.as_str());
                vhosts.push(CaddyVhost {
                    domain: "mesh.primal.eco".into(),
                    upstream: format!("{relay_ip}:7700"),
                    path: None,
                    tls: true,
                    extra_directives: vec![],
                });
            }
        }
    }

    if vhosts.is_empty() {
        return Ok(crate::ShadowOutcome::ok(String::from(
            "no services resolved from manifest — ensure topology.hosts and gate roles are configured",
        )));
    }

    let config = CaddyConfig {
        gate_name: gate_name.into(),
        acme_email,
        vhosts,
    };

    let output = config.to_caddyfile();
    let data = serde_json::to_value(&config)?;

    Ok(crate::ShadowOutcome::ok_with(output, data))
}

// ── Helpers ─────────────────────────────────────────────────────────

fn parse_days_remaining(not_after: &str) -> i64 {
    if not_after.is_empty() {
        return 0;
    }
    chrono::NaiveDateTime::parse_from_str(not_after.trim_end_matches(" GMT"), "%b %d %H:%M:%S %Y")
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(
                not_after.trim_end_matches(" GMT").trim(),
                "%b  %d %H:%M:%S %Y",
            )
        })
        .map_or(0, |expiry| {
            let now = chrono::Utc::now().naive_utc();
            (expiry - now).num_days()
        })
}

fn parse_caddyfile_vhosts(content: &str) -> Vec<VhostEntry> {
    let mut entries = Vec::new();
    let mut current_address: Option<String> = None;
    let mut current_upstreams: Vec<String> = Vec::new();
    let mut brace_depth: u32 = 0;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if brace_depth == 0 && !trimmed.starts_with('}') && trimmed.contains('{') {
            if let Some(addr) = current_address.take() {
                entries.push(VhostEntry {
                    address: addr,
                    tls_enabled: true,
                    upstreams: std::mem::take(&mut current_upstreams),
                });
            }
            let addr = trimmed.split('{').next().unwrap_or("").trim().to_string();
            if !addr.is_empty() {
                current_address = Some(addr);
            }
            brace_depth += 1;
        } else if trimmed.contains('{') {
            brace_depth += 1;
        } else if trimmed.contains('}') {
            brace_depth = brace_depth.saturating_sub(1);
            if brace_depth == 0 {
                if let Some(addr) = current_address.take() {
                    entries.push(VhostEntry {
                        address: addr,
                        tls_enabled: true,
                        upstreams: std::mem::take(&mut current_upstreams),
                    });
                }
            }
        } else if trimmed.starts_with("reverse_proxy") {
            let upstream = trimmed
                .trim_start_matches("reverse_proxy")
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string();
            if !upstream.is_empty() {
                current_upstreams.push(upstream);
            }
        }
    }

    if let Some(addr) = current_address {
        entries.push(VhostEntry {
            address: addr,
            tls_enabled: true,
            upstreams: current_upstreams,
        });
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_caddyfile_extracts_vhosts() {
        let caddyfile = r"
primals.eco {
    reverse_proxy localhost:8080
}

mesh.primal.eco {
    reverse_proxy 157.230.3.183:7700
}

nestgate.io {
    reverse_proxy /content/* localhost:3000
}
";
        let entries = parse_caddyfile_vhosts(caddyfile);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].address, "primals.eco");
        assert_eq!(entries[0].upstreams, vec!["localhost:8080"]);
        assert_eq!(entries[1].address, "mesh.primal.eco");
        assert_eq!(entries[1].upstreams, vec!["157.230.3.183:7700"]);
        assert_eq!(entries[2].address, "nestgate.io");
        assert_eq!(entries[2].upstreams, vec!["/content/*"]);
    }

    #[test]
    fn parse_caddyfile_handles_nested_blocks() {
        let caddyfile = r"
primal.eco {
    handle /api/* {
        reverse_proxy localhost:9000
    }
    reverse_proxy localhost:8000
}
";
        let entries = parse_caddyfile_vhosts(caddyfile);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].address, "primal.eco");
        assert_eq!(entries[0].upstreams.len(), 2);
    }

    #[test]
    fn parse_days_remaining_valid() {
        let days = parse_days_remaining("Dec 31 23:59:59 2030 GMT");
        assert!(days > 0);
    }

    #[test]
    fn parse_days_remaining_empty() {
        assert_eq!(parse_days_remaining(""), 0);
    }

    #[test]
    fn parse_days_remaining_past() {
        let days = parse_days_remaining("Jan  1 00:00:00 2020 GMT");
        assert!(days < 0);
    }

    #[test]
    fn parse_caddyfile_empty() {
        let entries = parse_caddyfile_vhosts("");
        assert!(entries.is_empty());
    }
}
