// SPDX-License-Identifier: AGPL-3.0-or-later

//! Caddy TLS operations — certificate check, external cutover, ACME revert.

use crate::ShadowConfig;
use crate::error::{Result, ShadowError};

use super::{CertStatus, caddy_bin_path, caddy_exec, caddyfile_path, parse_days_remaining};

/// Check TLS certificate status for a domain via Caddy's admin API.
pub async fn tls_check(config: &ShadowConfig, domain: &str) -> Result<CertStatus> {
    let endpoint = super::caddy_admin_endpoint();
    let cmd = format!(
        "curl -sf {endpoint}/reverse_proxy/upstreams 2>/dev/null; \
         echo '---CERT_CHECK---'; \
         echo | openssl s_client -servername {domain} -connect {domain}:443 2>/dev/null | \
         openssl x509 -noout -issuer -dates -subject 2>/dev/null || echo 'TLS_PROBE_FAILED'"
    );
    let (out, _) = caddy_exec(config, &cmd).await?;

    let cert_section = out.split("---CERT_CHECK---").nth(1).unwrap_or("").trim();

    if cert_section.contains("TLS_PROBE_FAILED") || cert_section.is_empty() {
        return Ok(CertStatus {
            domain: domain.to_string(),
            valid: false,
            issuer: String::new(),
            expires: String::new(),
            days_remaining: 0,
            error: Some("TLS probe failed — cert not reachable or not issued".into()),
        });
    }

    let (issuer, not_after) = parse_cert_fields(cert_section);
    let days = parse_days_remaining(&not_after);

    Ok(CertStatus {
        domain: domain.to_string(),
        valid: days > 0,
        issuer,
        expires: not_after,
        days_remaining: days,
        error: None,
    })
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
        if let Err(e) = caddy_exec(config, &rollback).await {
            tracing::warn!(error = %e, "TLS cutover rollback also failed");
        }
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

/// Parse issuer and expiry from an openssl x509 output block.
///
/// Expects lines like `issuer=...` and `notAfter=...`. Returns `(issuer, not_after)`.
fn parse_cert_fields(cert_section: &str) -> (String, String) {
    let mut issuer = String::new();
    let mut not_after = String::new();
    for line in cert_section.lines() {
        let trimmed = line.trim();
        if let Some(val) = trimmed.strip_prefix("issuer=") {
            issuer = val.trim().to_string();
        } else if let Some(val) = trimmed.strip_prefix("notAfter=") {
            not_after = val.trim().to_string();
        }
    }
    (issuer, not_after)
}

/// Check ACME certificate provisioning logs for recent errors.
pub async fn acme_log(config: &ShadowConfig, lines: u32) -> Result<String> {
    let unit = cellmembrane_types::service::CADDY_SERVICE_UNIT;
    let cmd = format!(
        "journalctl -u {unit} --no-pager -n {lines} --grep='acme\\|tls\\|certificate' 2>/dev/null || \
         journalctl -u {unit} --no-pager -n {lines} 2>/dev/null"
    );
    let (out, _) = caddy_exec(config, &cmd).await?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cert_fields_standard_output() {
        let section = "\
            issuer=C = US, O = Let's Encrypt, CN = E8\n\
            notBefore=Jun  1 00:00:00 2026 GMT\n\
            notAfter=Aug 30 23:59:59 2026 GMT\n\
            subject=CN = membrane.primals.eco";
        let (issuer, not_after) = parse_cert_fields(section);
        assert_eq!(issuer, "C = US, O = Let's Encrypt, CN = E8");
        assert_eq!(not_after, "Aug 30 23:59:59 2026 GMT");
    }

    #[test]
    fn parse_cert_fields_empty_input() {
        let (issuer, not_after) = parse_cert_fields("");
        assert!(issuer.is_empty());
        assert!(not_after.is_empty());
    }

    #[test]
    fn parse_cert_fields_missing_fields() {
        let (issuer, not_after) = parse_cert_fields("subject=CN = example.com");
        assert!(issuer.is_empty());
        assert!(not_after.is_empty());
    }

    #[test]
    fn parse_cert_fields_whitespace_handling() {
        let section = "  issuer=Test CA  \n  notAfter=Dec 31 23:59:59 2030 GMT  ";
        let (issuer, not_after) = parse_cert_fields(section);
        assert_eq!(issuer, "Test CA");
        assert_eq!(not_after, "Dec 31 23:59:59 2030 GMT");
    }
}
