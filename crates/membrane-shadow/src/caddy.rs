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

/// SSH host for Caddy operations. Resolved from `ShadowConfig::ssh_host_ext`.
fn caddy_host(config: &ShadowConfig) -> String {
    config.ssh_host_ext.clone()
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
    /// Whether the admin API responds on :2019.
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
    let host = caddy_host(config);
    let host_config = ShadowConfig {
        ssh_host: host,
        ..config.clone()
    };

    let (active_out, active_code) = ssh::exec_raw(
        &host_config,
        "systemctl is-active caddy 2>/dev/null || echo inactive",
    )
    .await?;
    let service_active = active_out.trim() == "active" && active_code == 0;

    let (api_out, api_code) = ssh::exec_raw(
        &host_config,
        "curl -sf http://localhost:2019/config/ 2>/dev/null | head -c 100 || echo FAIL",
    )
    .await?;
    let admin_api_ok = api_code == 0 && !api_out.contains("FAIL");

    let (vhosts_out, _) = ssh::exec_raw(
        &host_config,
        "grep -cE '^[a-zA-Z]' /etc/caddy/Caddyfile 2>/dev/null || echo 0",
    )
    .await?;
    let vhost_count: usize = vhosts_out.trim().parse().unwrap_or(0);

    let (listeners_out, _) = ssh::exec_raw(
        &host_config,
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
        config_path: "/etc/caddy/Caddyfile".into(),
        vhost_count,
        listeners,
    })
}

/// Check TLS certificate status for a domain via Caddy's admin API.
pub async fn tls_check(config: &ShadowConfig, domain: &str) -> Result<CertStatus> {
    let host = caddy_host(config);
    let host_config = ShadowConfig {
        ssh_host: host,
        ..config.clone()
    };

    let cmd = format!(
        "curl -sf 'http://localhost:2019/id/{domain}/tls' 2>/dev/null || \
         openssl s_client -connect {domain}:443 -servername {domain} </dev/null 2>/dev/null | \
         openssl x509 -noout -dates -issuer 2>/dev/null || echo ERROR"
    );

    let (out, _) = ssh::exec_raw(&host_config, &cmd).await?;

    if out.contains("ERROR") || out.is_empty() {
        let (err_out, _) = ssh::exec_raw(
            &host_config,
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
    let host = caddy_host(config);
    let host_config = ShadowConfig {
        ssh_host: host,
        ..config.clone()
    };

    let (caddyfile, code) =
        ssh::exec_raw(&host_config, "cat /etc/caddy/Caddyfile 2>/dev/null").await?;
    if code != 0 {
        return Err(ShadowError::Ssh("Failed to read Caddyfile".into()));
    }

    Ok(parse_caddyfile_vhosts(&caddyfile))
}

/// Reload Caddy configuration (graceful — zero downtime).
pub async fn reload(config: &ShadowConfig) -> Result<String> {
    let host = caddy_host(config);
    let host_config = ShadowConfig {
        ssh_host: host,
        ..config.clone()
    };

    let (out, code) = ssh::exec_raw(
        &host_config,
        "caddy reload --config /etc/caddy/Caddyfile --force 2>&1 || \
         systemctl reload caddy 2>&1",
    )
    .await?;

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
    let host = caddy_host(config);
    let host_config = ShadowConfig {
        ssh_host: host,
        ..config.clone()
    };

    let (out, code) = ssh::exec_raw(
        &host_config,
        "caddy validate --config /etc/caddy/Caddyfile 2>&1",
    )
    .await?;

    if code == 0 {
        Ok("Caddyfile valid".into())
    } else {
        Err(ShadowError::Ssh(format!(
            "Caddyfile invalid: {}",
            out.trim()
        )))
    }
}

/// Check ACME certificate provisioning logs for recent errors.
pub async fn acme_log(config: &ShadowConfig, lines: u32) -> Result<String> {
    let host = caddy_host(config);
    let host_config = ShadowConfig {
        ssh_host: host,
        ..config.clone()
    };

    let cmd = format!(
        "journalctl -u caddy --no-pager -n {lines} --grep='acme\\|tls\\|certificate' 2>/dev/null || \
         journalctl -u caddy --no-pager -n {lines} 2>/dev/null"
    );
    let (out, _) = ssh::exec_raw(&host_config, &cmd).await?;
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
            let json = serde_json::to_string_pretty(&health)
                .map_err(|e| ShadowError::Parse(e.to_string()))?;
            Ok(crate::ShadowOutcome::ok(json))
        }
        "caddy.tls.check" => {
            let domain = args.first().ok_or_else(|| {
                ShadowError::Parse("domain required: membrane caddy.tls.check <domain>".into())
            })?;
            let cert = tls_check(config, domain).await?;
            let json = serde_json::to_string_pretty(&cert)
                .map_err(|e| ShadowError::Parse(e.to_string()))?;
            Ok(crate::ShadowOutcome::ok(json))
        }
        "caddy.vhosts" => {
            let entries = vhosts(config).await?;
            let json = serde_json::to_string_pretty(&entries)
                .map_err(|e| ShadowError::Parse(e.to_string()))?;
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
