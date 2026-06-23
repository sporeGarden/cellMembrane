// SPDX-License-Identifier: AGPL-3.0-or-later

//! Caddy depot provisioning — WAN binary serving and checksums routes.

use crate::ShadowConfig;
use crate::error::{Result, ShadowError};

use super::{caddy_bin_path, caddy_exec, caddyfile_path};

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

    let depot_hostname = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_DEPOT_HOSTNAME,
        cellmembrane_types::service::DEFAULT_DEPOT_HOSTNAME,
    );
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
        if let Err(e) = caddy_exec(config, &rollback_cmd).await {
            tracing::warn!(error = %e, "Caddyfile rollback also failed");
        }
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

    let depot_hostname = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_DEPOT_HOSTNAME,
        cellmembrane_types::service::DEFAULT_DEPOT_HOSTNAME,
    );
    Ok(format!(
        "checksums.toml route provisioned: https://{depot_hostname}/depot/checksums.toml → {checksums_path}"
    ))
}
