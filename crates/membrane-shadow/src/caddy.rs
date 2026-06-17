// SPDX-License-Identifier: AGPL-3.0-or-later

//! Caddy reverse proxy management — agentic TLS and vhost operations.
//!
//! Wraps Caddy's admin API (localhost:2019) via SSH to golgiBody-ext.
//! Supports TLS certificate inspection, vhost listing, config validation,
//! and graceful reloads.
//!
//! The outer membrane (golgiBody-ext) runs Caddy for TLS termination.
//! Inner membrane calls this module through SSH, never directly.

use crate::ShadowConfig;
use crate::error::{Result, ShadowError};
use crate::ssh;
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
    ssh::exec_raw_on(&config.ssh_host, config.ssh_timeout, command).await
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

// ── Operations ──────────────────────────────────────────────────────

/// Check Caddy service health on golgiBody-ext.
pub async fn status(config: &ShadowConfig) -> Result<CaddyHealth> {
    let (active_out, active_code) = caddy_exec(
        config,
        "systemctl is-active caddy-tls 2>/dev/null || echo inactive",
    )
    .await?;
    let service_active = active_out.trim() == "active" && active_code == 0;

    let (api_out, api_code) = caddy_exec(
        config,
        &format!(
            "curl -sf http://{}/config/ 2>/dev/null | head -c 100 || echo FAIL",
            caddy_admin_endpoint()
        ),
    )
    .await?;
    let admin_api_ok = api_code == 0 && !api_out.contains("FAIL");

    let caddyfile = caddyfile_path();
    let (vhosts_out, _) = caddy_exec(
        config,
        &format!("grep -cE '^[a-zA-Z]' {caddyfile} 2>/dev/null || echo 0"),
    )
    .await?;
    let vhost_count: usize = vhosts_out.trim().parse().unwrap_or(0);

    let (listeners_out, _) = caddy_exec(
        config,
        "ss -tlnp | grep caddy | awk '{print $4}' 2>/dev/null",
    )
    .await?;
    let listeners: Vec<String> = listeners_out
        .lines()
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();

    Ok(CaddyHealth {
        service_active,
        admin_api_ok,
        config_path: caddyfile_path().into(),
        vhost_count,
        listeners,
    })
}

/// Check TLS certificate status for a domain via Caddy's admin API.
pub async fn tls_check(config: &ShadowConfig, domain: &str) -> Result<CertStatus> {
    let tls_port = cellmembrane_types::MembraneService::for_binary("caddy")
        .and_then(|s| s.port)
        .unwrap_or(443);
    let cmd = format!(
        "curl -sf 'http://{endpoint}/id/{domain}/tls' 2>/dev/null || \
         openssl s_client -connect {domain}:{tls_port} -servername {domain} </dev/null 2>/dev/null | \
         openssl x509 -noout -dates -issuer 2>/dev/null || echo ERROR",
        endpoint = caddy_admin_endpoint()
    );

    let (out, _) = caddy_exec(config, &cmd).await?;

    if out.contains("ERROR") || out.is_empty() {
        let (err_out, _) = caddy_exec(
            config,
            &format!(
                "curl -sf https://{domain}/ 2>&1 | head -3 || \
                 echo 'TLS connection failed'"
            ),
        )
        .await?;
        return Ok(CertStatus {
            domain: domain.into(),
            valid: false,
            issuer: String::new(),
            expires: String::new(),
            days_remaining: 0,
            error: Some(err_out.trim().to_string()),
        });
    }

    let issuer = out
        .lines()
        .find(|l| l.starts_with("issuer="))
        .map_or(String::new(), |l| {
            l.trim_start_matches("issuer=").to_string()
        });

    let not_after = out
        .lines()
        .find(|l| l.starts_with("notAfter="))
        .map_or(String::new(), |l| {
            l.trim_start_matches("notAfter=").to_string()
        });

    let days_remaining = parse_days_remaining(&not_after);

    Ok(CertStatus {
        domain: domain.into(),
        valid: days_remaining > 0,
        issuer,
        expires: not_after,
        days_remaining,
        error: None,
    })
}

/// List configured vhosts from the Caddyfile.
pub async fn vhosts(config: &ShadowConfig) -> Result<Vec<VhostEntry>> {
    let caddyfile = caddyfile_path();
    let cmd = format!("cat {caddyfile} 2>/dev/null");
    let (content, code) = caddy_exec(config, &cmd).await?;
    if code != 0 {
        return Err(ShadowError::Ssh("Failed to read Caddyfile".into()));
    }

    Ok(parse_caddyfile_vhosts(&content))
}

/// Reload Caddy configuration (graceful — zero downtime).
pub async fn reload(config: &ShadowConfig) -> Result<String> {
    let caddy_bin = caddy_bin_path();
    let caddyfile = caddyfile_path();
    let cmd = format!(
        "{caddy_bin} reload --config {caddyfile} --force 2>&1 || \
         systemctl reload caddy-tls 2>&1"
    );
    let (out, code) = caddy_exec(config, &cmd).await?;

    if code == 0 || out.contains("reloaded") {
        Ok("Caddy reloaded successfully".into())
    } else {
        Err(ShadowError::Ssh(format!(
            "Caddy reload failed: {}",
            out.trim()
        )))
    }
}

/// Validate Caddyfile syntax without applying.
pub async fn validate(config: &ShadowConfig) -> Result<String> {
    let caddy_bin = caddy_bin_path();
    let caddyfile = caddyfile_path();
    let cmd = format!("{caddy_bin} validate --config {caddyfile} 2>&1");
    let (out, code) = caddy_exec(config, &cmd).await?;

    if code == 0 {
        Ok("Caddyfile valid".into())
    } else {
        Err(ShadowError::Ssh(format!(
            "Caddyfile invalid: {}",
            out.trim()
        )))
    }
}

/// Provision the `/depot/` route on the outer membrane Caddyfile.
///
/// Adds a `file_server` block serving depot binaries over HTTPS,
/// enabling WAN gates to fetch via `plasmid.fetch --source wan`.
/// The route serves `{depot_root}/primals/` at `https://{outer_domain}/depot/`.
pub async fn depot_provision(config: &ShadowConfig) -> Result<String> {
    let caddy_bin = caddy_bin_path();
    let caddyfile = caddyfile_path();
    let depot_root = format!(
        "{}/plasmidBin/primals",
        cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT
    );

    let check_cmd =
        format!("grep -q '/depot/' {caddyfile} 2>/dev/null && echo EXISTS || echo MISSING");
    let (check_out, _) = caddy_exec(config, &check_cmd).await?;

    if check_out.trim() == "EXISTS" {
        return Ok("depot route already provisioned in Caddyfile".into());
    }

    let depot_hostname = std::env::var(cellmembrane_types::service::ENV_DEPOT_HOSTNAME)
        .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_DEPOT_HOSTNAME.into());
    let escaped_hostname = depot_hostname.replace('.', r"\.");
    let inject_cmd = format!(
        r"cat > /tmp/depot-snippet.caddy << 'SNIPPET'
    handle /depot/* {{
        uri strip_prefix /depot
        root * {depot_root}
        file_server browse
    }}
SNIPPET
sed -i '/^{escaped_hostname} {{/r /tmp/depot-snippet.caddy' {caddyfile} && rm -f /tmp/depot-snippet.caddy"
    );

    let (out, code) = caddy_exec(config, &inject_cmd).await?;
    if code != 0 {
        return Err(ShadowError::Ssh(format!(
            "Failed to inject depot route: {}",
            out.trim()
        )));
    }

    let validate_reload = format!(
        "{caddy_bin} validate --config {caddyfile} 2>&1 && \
         {caddy_bin} reload --config {caddyfile} --force 2>&1"
    );
    let (reload_out, reload_code) = caddy_exec(config, &validate_reload).await?;

    if reload_code != 0 {
        let rollback_cmd = format!(
            "cd {}/infra/plasmidBin && \
             git checkout -- {caddyfile} 2>/dev/null; \
             {caddy_bin} reload --config {caddyfile} --force 2>/dev/null",
            cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT
        );
        let _ = caddy_exec(config, &rollback_cmd).await;
        return Err(ShadowError::Ssh(format!(
            "Caddyfile validation/reload failed (rolled back): {}",
            reload_out.trim()
        )));
    }

    Ok(format!(
        "depot route provisioned: https://{depot_hostname}/depot/ → {depot_root}"
    ))
}

/// Provision a route to serve `checksums.toml` at `/depot/checksums.toml`.
///
/// Enables zero-git verification: gates can fetch the authoritative checksum
/// file directly from the WAN endpoint without cloning plasmidBin.
pub async fn depot_checksums_provision(config: &ShadowConfig) -> Result<String> {
    let caddy_bin = caddy_bin_path();
    let caddyfile = caddyfile_path();
    let checksums_path = format!(
        "{}/plasmidBin/checksums.toml",
        cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT
    );

    let check_cmd =
        format!("grep -q 'checksums.toml' {caddyfile} 2>/dev/null && echo EXISTS || echo MISSING");
    let (check_out, _) = caddy_exec(config, &check_cmd).await?;

    if check_out.trim() == "EXISTS" {
        return Ok("checksums.toml route already provisioned".into());
    }

    let snippet = format!(
        "    handle /depot/checksums.toml {{\n        root * /\n        rewrite * {checksums_path}\n        file_server\n    }}"
    );
    let inject_cmd = format!(
        "printf '%s\\n' '{snippet}' > /tmp/checksums-snippet.caddy && \
         sed -i '/handle \\/depot\\/\\*/r /tmp/checksums-snippet.caddy' {caddyfile} && \
         rm -f /tmp/checksums-snippet.caddy"
    );

    let (out, code) = caddy_exec(config, &inject_cmd).await?;
    if code != 0 {
        return Err(ShadowError::Ssh(format!(
            "Failed to inject checksums route: {}",
            out.trim()
        )));
    }

    let validate_reload = format!(
        "{caddy_bin} validate --config {caddyfile} 2>&1 && \
         {caddy_bin} reload --config {caddyfile} --force 2>&1"
    );
    let (reload_out, reload_code) = caddy_exec(config, &validate_reload).await?;

    if reload_code != 0 {
        return Err(ShadowError::Ssh(format!(
            "Caddyfile validation/reload failed after checksums route: {}",
            reload_out.trim()
        )));
    }

    let depot_hostname = std::env::var(cellmembrane_types::service::ENV_DEPOT_HOSTNAME)
        .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_DEPOT_HOSTNAME.into());
    Ok(format!(
        "checksums.toml route provisioned: https://{depot_hostname}/depot/checksums.toml → {checksums_path}"
    ))
}

/// Switch a domain's TLS from Caddy-managed ACME to external cert files.
///
/// Rewrites the `tls` directive in the domain's Caddyfile block from implicit
/// (Caddy manages ACME) to explicit file paths:
///   `tls /etc/membrane/tls/{domain}/fullchain.pem /etc/membrane/tls/{domain}/privkey.pem`
///
/// `BearDog` (or any external provisioner) writes certs to that path.
/// Caddy reads them on reload — no ACME interaction required.
pub async fn tls_external(config: &ShadowConfig, domain: &str) -> Result<String> {
    let caddy_bin = caddy_bin_path();
    let caddyfile = caddyfile_path();
    let cert_dir = cellmembrane_types::TlsProvider::default_cert_dir();
    let fullchain = format!("{cert_dir}/{domain}/fullchain.pem");
    let privkey = format!("{cert_dir}/{domain}/privkey.pem");

    let check_cmd = format!(
        "grep -A5 '^{domain}' {caddyfile} 2>/dev/null | grep -q 'tls /' && echo EXTERNAL || echo ACME"
    );
    let (check_out, _) = caddy_exec(config, &check_cmd).await?;

    if check_out.trim() == "EXTERNAL" {
        return Ok(format!("{domain}: already using external TLS certs"));
    }

    let setup_cmd = format!(
        "mkdir -p {cert_dir}/{domain} && \
         test -f {fullchain} && test -f {privkey} || \
         {{ echo 'MISSING_CERTS'; exit 0; }}"
    );
    let (setup_out, _) = caddy_exec(config, &setup_cmd).await?;

    if setup_out.trim() == "MISSING_CERTS" {
        return Err(ShadowError::Ssh(format!(
            "cert files not found at {cert_dir}/{domain}/. \
             BearDog must provision fullchain.pem + privkey.pem before cutover."
        )));
    }

    let sed_cmd = format!(
        "sed -i '/^{domain}/,/^}}/ {{ \
            /^[[:space:]]*tls[[:space:]]/d; \
            /^{domain}/a\\    tls {fullchain} {privkey} \
         }}' {caddyfile}"
    );
    let (out, code) = caddy_exec(config, &sed_cmd).await?;
    if code != 0 {
        return Err(ShadowError::Ssh(format!(
            "failed to rewrite TLS directive: {}",
            out.trim()
        )));
    }

    let validate_reload = format!(
        "{caddy_bin} validate --config {caddyfile} 2>&1 && \
         {caddy_bin} reload --config {caddyfile} --force 2>&1"
    );
    let (reload_out, reload_code) = caddy_exec(config, &validate_reload).await?;

    if reload_code != 0 {
        let rollback = format!(
            "sed -i '/^{domain}/,/^}}/ {{ /tls \\//d; }}' {caddyfile}; \
             {caddy_bin} reload --config {caddyfile} --force 2>/dev/null"
        );
        let _ = caddy_exec(config, &rollback).await;
        return Err(ShadowError::Ssh(format!(
            "Caddyfile invalid after TLS cutover (rolled back): {}",
            reload_out.trim()
        )));
    }

    Ok(format!(
        "{domain}: switched to external TLS — {fullchain} + {privkey}"
    ))
}

/// Revert a domain from external cert files back to Caddy-managed ACME.
///
/// Removes the explicit `tls /path/...` directive, letting Caddy resume
/// automatic certificate management.
pub async fn tls_revert_acme(config: &ShadowConfig, domain: &str) -> Result<String> {
    let caddy_bin = caddy_bin_path();
    let caddyfile = caddyfile_path();
    let check_cmd = format!(
        "grep -A5 '^{domain}' {caddyfile} 2>/dev/null | grep -q 'tls /' && echo EXTERNAL || echo ACME"
    );
    let (check_out, _) = caddy_exec(config, &check_cmd).await?;

    if check_out.trim() == "ACME" {
        return Ok(format!("{domain}: already using Caddy ACME"));
    }

    let sed_cmd = format!("sed -i '/^{domain}/,/^}}/ {{ /^[[:space:]]*tls \\//d; }}' {caddyfile}");
    let (out, code) = caddy_exec(config, &sed_cmd).await?;
    if code != 0 {
        return Err(ShadowError::Ssh(format!(
            "failed to remove external TLS directive: {}",
            out.trim()
        )));
    }

    let validate_reload = format!(
        "{caddy_bin} validate --config {caddyfile} 2>&1 && \
         {caddy_bin} reload --config {caddyfile} --force 2>&1"
    );
    let (reload_out, reload_code) = caddy_exec(config, &validate_reload).await?;

    if reload_code != 0 {
        return Err(ShadowError::Ssh(format!(
            "Caddyfile validation/reload failed after revert: {}",
            reload_out.trim()
        )));
    }

    Ok(format!("{domain}: reverted to Caddy-managed ACME"))
}

/// Check ACME certificate provisioning logs for recent errors.
pub async fn acme_log(config: &ShadowConfig, lines: u32) -> Result<String> {
    let cmd = format!(
        "journalctl -u caddy-tls --no-pager -n {lines} --grep='acme\\|tls\\|certificate' 2>/dev/null || \
         journalctl -u caddy-tls --no-pager -n {lines} 2>/dev/null"
    );
    let (out, _) = caddy_exec(config, &cmd).await?;
    Ok(out)
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
                ShadowError::Parse("domain required: membrane caddy.tls.check <domain>".into())
            })?;
            let cert = tls_check(config, domain).await?;
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
            let log = acme_log(config, lines).await?;
            Ok(crate::ShadowOutcome::ok(log))
        }
        "caddy.depot.provision" => {
            let msg = depot_provision(config).await?;
            Ok(crate::ShadowOutcome::ok(msg))
        }
        "caddy.depot.checksums" => {
            let msg = depot_checksums_provision(config).await?;
            Ok(crate::ShadowOutcome::ok(msg))
        }
        "caddy.tls.external" => {
            let domain = args.first().ok_or_else(|| {
                ShadowError::Parse("domain required: membrane caddy.tls.external <domain>".into())
            })?;
            let msg = tls_external(config, domain).await?;
            Ok(crate::ShadowOutcome::ok(msg))
        }
        "caddy.tls.revert" => {
            let domain = args.first().ok_or_else(|| {
                ShadowError::Parse("domain required: membrane caddy.tls.revert <domain>".into())
            })?;
            let msg = tls_revert_acme(config, domain).await?;
            Ok(crate::ShadowOutcome::ok(msg))
        }
        _ => Ok(crate::ShadowOutcome::fail(format!(
            "unknown caddy command: {cmd}"
        ))),
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

fn parse_days_remaining(not_after: &str) -> i64 {
    if not_after.is_empty() {
        return 0;
    }
    // openssl x509 -noout -dates outputs: notAfter=Jun 10 12:00:00 2026 GMT
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
        // Far future date
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
