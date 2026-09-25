// SPDX-License-Identifier: AGPL-3.0-or-later

//! Caddy reverse proxy management — agentic TLS and vhost operations.
//!
//! **DEPRECATED (Wave 132):** This module is being superseded by the Tower HTTP
//! gateway (`gateway` module). During the shadow validation period, both paths
//! run in parallel. Once shadow validation passes, Caddy will be retired and
//! this module archived.
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
        cellmembrane_types::service::env_or(
            cellmembrane_types::service::ENV_CADDY_ADMIN_ENDPOINT,
            cellmembrane_types::service::DEFAULT_CADDY_ADMIN_ENDPOINT,
        )
    })
}

fn caddy_bin_path() -> &'static str {
    static BIN: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    BIN.get_or_init(|| {
        let vps_bin_dir = cellmembrane_types::service::env_or(
            cellmembrane_types::service::ENV_VPS_BIN_DIR,
            cellmembrane_types::service::DEFAULT_INSTALL_BASE,
        );
        format!("{vps_bin_dir}/caddy")
    })
}

fn caddyfile_path() -> &'static str {
    static PATH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    PATH.get_or_init(|| {
        let config_dir = cellmembrane_types::service::env_or(
            cellmembrane_types::service::ENV_CONFIG_DIR,
            cellmembrane_types::service::DEFAULT_CONFIG_DIR,
        );
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
        return Err(ShadowError::ssh(format!(
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
        return Err(ShadowError::ssh(format!(
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
                ShadowError::config("domain required: membrane caddy.tls.check <domain>")
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
                ShadowError::config("domain required: membrane caddy.tls.external <domain>")
            })?;
            let msg = tls::tls_external(config, domain).await?;
            Ok(crate::ShadowOutcome::ok(msg))
        }
        "caddy.tls.revert" => {
            let domain = args.first().ok_or_else(|| {
                ShadowError::config("domain required: membrane caddy.tls.revert <domain>")
            })?;
            let msg = tls::tls_revert_acme(config, domain).await?;
            Ok(crate::ShadowOutcome::ok(msg))
        }
        "caddy.generate" => dispatch_caddy_generate(args).await,
        "caddy.deploy" => dispatch_caddy_deploy(config, args).await,
        _ => Ok(crate::ShadowOutcome::fail(format!(
            "unknown caddy command: {cmd}"
        ))),
    }
}

// ── Static site Caddy block generation ──────────────────────────────

/// Generate the Caddy vhost block for a static site from `PublishSite` config.
///
/// Produces a complete, self-contained vhost block with:
/// - `root * {public_dir}` + `encode gzip`
/// - Security headers (inline)
/// - SEO handles (`/sitemap.xml`, `/robots.txt`)
/// - Evidence routes (if `evidence_dir` is set): landing page + file browser
/// - SPA fallback (`try_files`)
/// - 404 error page
pub fn generate_static_site_block(site: &crate::seo::PublishSite) -> String {
    let host = site.host;
    let public_dir = site.public_dir;

    // Evidence route block (if configured)
    let evidence_block = if site.evidence_dir.is_some() {
        // Evidence dir on golgiBody is always {parent_of_public}/evidence
        let evidence_remote = public_dir
            .rsplit_once('/')
            .map(|(parent, _)| format!("{parent}/evidence"))
            .unwrap_or_else(|| format!("{public_dir}/../evidence"));

        format!(
            r#"
    # Evidence landing page — serve the Zola HTML page (not the file browser)
    handle /evidence/ {{
        try_files /evidence/index.html
        file_server
    }}

    # Evidence depot — separate from Zola build output (survives zola --force)
    handle /evidence/* {{
        uri strip_prefix /evidence
        root * {evidence_remote}
        file_server browse
    }}
"#
        )
    } else {
        String::new()
    };

    format!(
        r#"{host} {{
    root * {public_dir}
    encode gzip

    # Static files first
    handle /sitemap.xml {{
        file_server
    }}
    handle /robots.txt {{
        file_server
    }}
{evidence_block}
    # SPA fallback for Zola pages (must be after handle blocks)
    handle {{
        try_files {{path}} {{path}}/index.html /index.html
        file_server
    }}

    handle_errors {{
        @404 expression `{{http.error.status_code}} == 404`
        rewrite @404 /404.html
        file_server
    }}

    header {{
        Strict-Transport-Security "max-age=63072000; includeSubDomains; preload"
        X-Content-Type-Options "nosniff"
        X-Frame-Options "DENY"
        Referrer-Policy "strict-origin-when-cross-origin"
        Permissions-Policy "camera=(), microphone=(), geolocation=(), interest-cohort=()"
    }}
}}
"#
    )
}

/// Deploy a static site vhost to golgiBody's Caddyfile.
///
/// Uses block replacement: finds the existing `{host} {` block and replaces it,
/// or appends if not found. Validates + reloads, with rollback on failure.
async fn dispatch_caddy_deploy(
    config: &ShadowConfig,
    args: &[&str],
) -> Result<crate::ShadowOutcome> {
    let caddy_bin = caddy_bin_path();
    let caddyfile = caddyfile_path();

    let dry_run = args.contains(&"--dry-run");

    // Determine which sites to deploy
    let sites: Vec<&crate::seo::PublishSite> = if let Some(site_name) =
        args.iter().find(|a| !a.starts_with('-'))
    {
        let site = crate::seo::find_publish_site(site_name).ok_or_else(|| {
            ShadowError::config(format!("unknown publish site: {site_name}"))
        })?;
        vec![site]
    } else if args.contains(&"--all") {
        crate::seo::PUBLISH_SITES.iter().collect()
    } else {
        return Ok(crate::ShadowOutcome::fail(
            "usage: membrane caddy.deploy <site|--all> [--dry-run]".to_string(),
        ));
    };

    let mut results = Vec::new();

    for site in &sites {
        let block = generate_static_site_block(site);

        if dry_run {
            results.push(format!("--- {} (dry-run) ---\n{}", site.host, block));
            continue;
        }

        // Backup + replace/append on golgiBody
        let host_escaped = site.host.replace('.', r"\.");
        let replace_script = format!(
            r#"python3 -c "
import re, shutil, sys
path = '{caddyfile}'
shutil.copy2(path, path + '.bak')
with open(path) as f:
    content = f.read()

# Find existing block: '{host} {{' ... matching '}}'
pattern = r'^{host_escaped}\s*\{{.*?^\}}\n?'
new_block = sys.stdin.read()

if re.search(pattern, content, re.MULTILINE | re.DOTALL):
    content = re.sub(pattern, new_block, content, count=1, flags=re.MULTILINE | re.DOTALL)
    action = 'replaced'
else:
    content += '\n' + new_block
    action = 'appended'

with open(path, 'w') as f:
    f.write(content)
print(action)
""#,
            host = site.host,
        );

        // Write block via stdin to the python script
        let full_cmd = format!("echo '{}' | {}", block.replace('\'', "'\\''"), replace_script);
        let (action_out, code) = caddy_exec(config, &full_cmd).await?;

        if code != 0 {
            results.push(format!("FAILED {}: {}", site.host, action_out.trim()));
            continue;
        }

        let action = action_out.trim();

        // Validate + reload
        let validate_reload = format!(
            "{caddy_bin} validate --config {caddyfile} 2>&1 && \
             {caddy_bin} reload --config {caddyfile} --force 2>&1"
        );
        let (reload_out, reload_code) = caddy_exec(config, &validate_reload).await?;

        if reload_code != 0 {
            // Rollback
            let rollback = format!("cp {caddyfile}.bak {caddyfile} && {caddy_bin} reload --config {caddyfile} --force 2>&1");
            let _ = caddy_exec(config, &rollback).await;
            results.push(format!(
                "FAILED {} (rolled back): {}",
                site.host,
                reload_out.trim()
            ));
        } else {
            results.push(format!("{}: {} vhost block", site.host, action));
        }
    }

    let msg = results.join("\n");
    let ok = !msg.contains("FAILED");
    Ok(crate::ShadowOutcome {
        ok,
        message: msg,
        data: None,
    })
}

/// Check if a site's vhost block exists in the golgiBody Caddyfile.
pub async fn vhost_exists(config: &ShadowConfig, host: &str) -> bool {
    let caddyfile = caddyfile_path();
    let check_cmd = format!("grep -q '{host} {{' {caddyfile} 2>/dev/null && echo EXISTS || echo MISSING");
    match caddy_exec(config, &check_cmd).await {
        Ok((out, _)) => out.trim() == "EXISTS",
        Err(_) => false,
    }
}

// ── Manifest-driven Caddyfile generation ────────────────────────────

#[allow(
    clippy::too_many_lines,
    reason = "Caddyfile generation builds structured output in one pass"
)]
async fn dispatch_caddy_generate(args: &[&str]) -> Result<crate::ShadowOutcome> {
    use cellmembrane_types::caddy::{CaddyConfig, CaddySubRoute, CaddyVhost};
    use cellmembrane_types::service;

    let root = crate::temporal::resolve_workspace_root()?;
    let m = crate::manifest::load_from_workspace_async(&root).await?;

    let gate_name = crate::cli::extract_flag_value(args, "--gate")
        .map_or_else(crate::gate::resolve_local_gate_identity, str::to_string);

    let acme_email = crate::cli::extract_flag_value(args, "--email").map(String::from);

    let topo = m.topology.as_ref();
    let mut vhosts = Vec::new();

    let caddy_gates = {
        let mut gates = m.gates_for_role("caddy");
        if gates.is_empty() {
            gates = m.gates_for_role("caddy_tls");
        }
        if gates.is_empty() {
            gates = m.gates_for_role("tls_terminator");
        }
        gates
    };
    let is_caddy_gate = caddy_gates.iter().any(|(name, _)| *name == gate_name);

    if !is_caddy_gate && caddy_gates.is_empty() {
        return Ok(crate::ShadowOutcome::ok(String::from(
            "no gate has role 'caddy' or 'caddy_tls' — add roles = [\"caddy_tls\"] to a gate profile",
        )));
    }

    if let Some(topo_data) = topo
        && let Some(inner_ip) = topo_data.hosts.get(&topo_data.inner_membrane)
    {
        let simple_vhosts: &[(&str, &str, u16)] = &[
            (
                "forgejo",
                service::GIT_DOMAIN,
                service::DEFAULT_FORGEJO_HTTP_PORT,
            ),
            (
                "depot",
                service::DEPOT_DOMAIN,
                service::DEFAULT_DEPOT_HTTP_PORT,
            ),
            (
                "relay",
                service::MESH_DOMAIN,
                service::DEFAULT_FEDERATION_PORT,
            ),
        ];

        for &(role, domain, port) in simple_vhosts {
            let gates = m.gates_for_role(role);
            if let Some((gate_name_ref, profile)) = gates.first() {
                let ip = resolve_upstream_ip(gate_name_ref, profile, inner_ip);
                vhosts.push(CaddyVhost {
                    domain: domain.into(),
                    upstream: format!("{ip}:{port}"),
                    path: None,
                    tls: true,
                    extra_directives: vec![],
                    sub_routes: vec![],
                });
            }
        }

        if let Some((gate_name_ref, profile)) = m.gates_for_role("footprint").first() {
            let ip = resolve_upstream_ip(gate_name_ref, profile, inner_ip);
            vhosts.push(CaddyVhost {
                domain: service::FOOTPRINT_DOMAIN.into(),
                upstream: String::new(),
                path: None,
                tls: true,
                extra_directives: vec![
                    "header Content-Security-Policy \"default-src 'self'; img-src 'self' data: blob: https://*.arcgisonline.com https://*.tile.openstreetmap.org; style-src 'self' 'unsafe-inline'; script-src 'self' 'unsafe-inline'; connect-src 'self' https://*.arcgisonline.com https://geocode-api.arcgis.com wss:;\"".into(),
                ],
                sub_routes: vec![
                    CaddySubRoute {
                        path_prefix: "/ws".into(),
                        upstream: format!(
                            "{ip}:{}",
                            service::DEFAULT_PETALTONGUE_PORT
                        ),
                    },
                    CaddySubRoute {
                        path_prefix: String::new(),
                        upstream: format!(
                            "{ip}:{}",
                            service::DEFAULT_FOOTPRINT_PORT
                        ),
                    },
                ],
            });
        }

        if let Some((gate_name_ref, profile)) = m.gates_for_role("tideglass").first() {
            let ip = resolve_upstream_ip(gate_name_ref, profile, inner_ip);
            vhosts.push(CaddyVhost {
                domain: service::TIDEGLASS_DOMAIN.into(),
                upstream: format!("{ip}:{}", service::DEFAULT_PETALTONGUE_PORT),
                path: None,
                tls: true,
                extra_directives: vec![],
                sub_routes: vec![],
            });
        }

        if let Some((gate_name_ref, profile)) = m.gates_for_role("esotericwebb").first() {
            let ip = resolve_upstream_ip(gate_name_ref, profile, inner_ip);
            vhosts.push(CaddyVhost {
                domain: service::WEBB_DOMAIN.into(),
                upstream: format!("{ip}:{}", service::DEFAULT_ESOTERICWEBB_PORT),
                path: None,
                tls: true,
                extra_directives: vec![],
                sub_routes: vec![],
            });
        }

        // Root domain redirects to sporePrint documentation site.
        vhosts.push(CaddyVhost {
            domain: service::SURFACE_DOMAIN.into(),
            upstream: String::new(),
            path: None,
            tls: true,
            extra_directives: vec![format!(
                "redir https://{} permanent",
                service::SPOREPRINT_DOMAIN
            )],
            sub_routes: vec![],
        });
    }

    if vhosts.is_empty() {
        return Ok(crate::ShadowOutcome::ok(String::from(
            "no services resolved from manifest — ensure topology.hosts and gate roles are configured",
        )));
    }

    let config = CaddyConfig {
        gate_name,
        acme_email,
        vhosts,
    };

    let output = config.to_caddyfile();
    let data = serde_json::to_value(&config)?;

    Ok(crate::ShadowOutcome::ok_with(output, data))
}

fn resolve_upstream_ip<'a>(
    gate_name_ref: &str,
    profile: &'a crate::manifest::GateProfile,
    fallback: &'a str,
) -> &'a str {
    profile
        .wg_ip
        .as_deref()
        .or_else(|| cellmembrane_types::cytoplasm::mesh_address(gate_name_ref))
        .unwrap_or(fallback)
}

// ── Helpers ─────────────────────────────────────────────────────────

pub(crate) fn parse_days_remaining(not_after: &str) -> i64 {
    if not_after.is_empty() {
        return 0;
    }
    parse_openssl_date(not_after).map_or(0, |expiry| {
        let now = time::OffsetDateTime::now_utc();
        (expiry - now).whole_days()
    })
}

/// Parse OpenSSL-style date: `"Jul 28 18:00:00 2026"` or `"Jul  1 18:00:00 2026"`.
fn parse_openssl_date(s: &str) -> Option<time::OffsetDateTime> {
    let s = s.trim().trim_end_matches(" GMT");
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() != 4 {
        return None;
    }
    let month = match parts[0] {
        "Jan" => time::Month::January,
        "Feb" => time::Month::February,
        "Mar" => time::Month::March,
        "Apr" => time::Month::April,
        "May" => time::Month::May,
        "Jun" => time::Month::June,
        "Jul" => time::Month::July,
        "Aug" => time::Month::August,
        "Sep" => time::Month::September,
        "Oct" => time::Month::October,
        "Nov" => time::Month::November,
        "Dec" => time::Month::December,
        _ => return None,
    };
    let day: u8 = parts[1].parse().ok()?;
    let year: i32 = parts[3].parse().ok()?;
    let time_parts: Vec<&str> = parts[2].split(':').collect();
    if time_parts.len() != 3 {
        return None;
    }
    let hour: u8 = time_parts[0].parse().ok()?;
    let minute: u8 = time_parts[1].parse().ok()?;
    let second: u8 = time_parts[2].parse().ok()?;
    let date = time::Date::from_calendar_date(year, month, day).ok()?;
    let t = time::Time::from_hms(hour, minute, second).ok()?;
    Some(time::PrimitiveDateTime::new(date, t).assume_utc())
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
