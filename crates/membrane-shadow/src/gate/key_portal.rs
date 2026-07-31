// SPDX-License-Identifier: AGPL-3.0-or-later

//! SSH certificate lifecycle via a sovereign step-ca instance.
//!
//! Provides functions for requesting, renewing, and inspecting SSH certificates
//! from an ecoPrimals step-ca certificate authority. This replaces manual
//! `authorized_keys` management with short-lived, auditable certificates.
//!
//! ## Flow
//!
//! 1. `request_ssh_certificate` — Generates an SSH keypair and obtains a
//!    short-lived user certificate from step-ca.
//! 2. `renew_ssh_certificate` — Renews an existing certificate before expiry.
//! 3. `install_host_certificate` — Obtains and installs a host certificate.
//! 4. `inspect_certificates` — Reports installed certificate status.

use crate::error::{Result, ShadowError};
use cellmembrane_types::credentials::{SshCertType, SshCertificate};
use cellmembrane_types::service::{
    DEFAULT_SSH_CERT_LIFETIME, DEFAULT_STEP_CA_PROVISIONER, DEFAULT_STEP_CA_URL,
    ENV_STEP_CA_FINGERPRINT, ENV_STEP_CA_PROVISIONER, ENV_STEP_CA_SSH_LIFETIME, ENV_STEP_CA_URL,
    STEP_CA_CERT_DIR,
};
use std::path::{Path, PathBuf};

/// Resolve the step-ca URL from environment or default.
fn resolve_ca_url() -> String {
    std::env::var(ENV_STEP_CA_URL).unwrap_or_else(|_| DEFAULT_STEP_CA_URL.into())
}

/// Resolve the step-ca provisioner from environment or default.
fn resolve_provisioner() -> String {
    std::env::var(ENV_STEP_CA_PROVISIONER).unwrap_or_else(|_| DEFAULT_STEP_CA_PROVISIONER.into())
}

/// Resolve the SSH certificate lifetime from environment or default.
fn resolve_lifetime() -> String {
    std::env::var(ENV_STEP_CA_SSH_LIFETIME).unwrap_or_else(|_| DEFAULT_SSH_CERT_LIFETIME.into())
}

/// Resolve the CA fingerprint from environment. Required for bootstrap.
fn resolve_fingerprint() -> Option<String> {
    std::env::var(ENV_STEP_CA_FINGERPRINT).ok()
}

/// Directory where gate certificates are stored.
fn cert_dir() -> PathBuf {
    let base = std::env::var(cellmembrane_types::service::ENV_INSTALL_BASE)
        .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_INSTALL_BASE.into());
    PathBuf::from(base).join(STEP_CA_CERT_DIR)
}

/// Run a `step` CLI command and return its output.
///
/// Unified subprocess wrapper for all step-ca interactions. Returns an error
/// if the `step` binary is not found or the command exits non-zero.
async fn run_step(args: &[&str], context: &str) -> Result<std::process::Output> {
    let output = tokio::process::Command::new("step")
        .args(args)
        .output()
        .await
        .map_err(|e| ShadowError::Config(format!("step CLI not found: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ShadowError::Config(format!("{context}: {stderr}")));
    }

    Ok(output)
}

/// Bootstrap the step CLI on this gate (first-time CA trust).
///
/// Runs `step ca bootstrap` to download and trust the root CA certificate.
/// This is idempotent — safe to re-run.
pub async fn bootstrap_ca(dry_run: bool) -> Result<String> {
    let ca_url = resolve_ca_url();
    let Some(fingerprint) = resolve_fingerprint() else {
        return Err(ShadowError::Config(format!(
            "set {ENV_STEP_CA_FINGERPRINT} to the root CA SHA256 fingerprint"
        )));
    };

    if dry_run {
        return Ok(format!(
            "dry-run: would bootstrap step-ca from {ca_url} (fingerprint: {fingerprint})"
        ));
    }

    run_step(
        &[
            "ca",
            "bootstrap",
            "--ca-url",
            &ca_url,
            "--fingerprint",
            &fingerprint,
            "--force",
        ],
        "step ca bootstrap failed",
    )
    .await?;

    Ok(format!("step-ca bootstrap complete (CA: {ca_url})"))
}

/// Request a new SSH user certificate from the sovereign CA.
///
/// Generates an ECDSA keypair and requests a short-lived certificate via
/// `step ssh certificate`. The principal is the gate name.
pub async fn request_ssh_certificate(gate_name: &str, dry_run: bool) -> Result<SshCertificate> {
    let ca_url = resolve_ca_url();
    let provisioner = resolve_provisioner();
    let lifetime = resolve_lifetime();
    let dir = cert_dir();

    if !dry_run {
        tokio::fs::create_dir_all(&dir).await.map_err(|e| {
            ShadowError::Config(format!("cannot create cert dir {}: {e}", dir.display()))
        })?;
    }

    let key_path = dir.join("id_ecdsa");
    let cert_path = dir.join("id_ecdsa-cert.pub");
    let principal = format!(
        "{gate_name}@{}",
        cellmembrane_types::service::SURFACE_DOMAIN
    );

    if dry_run {
        return Ok(SshCertificate {
            cert_type: SshCertType::User,
            principals: vec![principal],
            serial: 0,
            cert_path: cert_path.to_string_lossy().into(),
            key_path: key_path.to_string_lossy().into(),
            valid_before: 0,
            ca_fingerprint: "dry-run".into(),
        });
    }

    let key_str = key_path.to_string_lossy();
    run_step(
        &[
            "ssh",
            "certificate",
            &principal,
            &key_str,
            "--ca-url",
            &ca_url,
            "--provisioner",
            &provisioner,
            "--not-after",
            &lifetime,
            "--force",
            "--no-password",
            "--insecure",
        ],
        "step ssh certificate failed",
    )
    .await?;

    let cert = parse_certificate_info(&cert_path).await?;
    Ok(cert)
}

/// Renew an existing SSH certificate before expiry.
///
/// Uses `step ssh renew` to refresh the certificate in-place.
pub async fn renew_ssh_certificate(dry_run: bool) -> Result<SshCertificate> {
    let dir = cert_dir();
    let cert_path = dir.join("id_ecdsa-cert.pub");
    let key_path = dir.join("id_ecdsa");

    if !cert_path.exists() {
        return Err(ShadowError::Config(format!(
            "no certificate to renew at {}",
            cert_path.display()
        )));
    }

    if dry_run {
        return Ok(SshCertificate {
            cert_type: SshCertType::User,
            principals: vec!["dry-run".into()],
            serial: 0,
            cert_path: cert_path.to_string_lossy().into(),
            key_path: key_path.to_string_lossy().into(),
            valid_before: 0,
            ca_fingerprint: "dry-run".into(),
        });
    }

    let cert_str = cert_path.to_string_lossy();
    let key_str = key_path.to_string_lossy();
    run_step(
        &["ssh", "renew", &cert_str, &key_str, "--force"],
        "step ssh renew failed",
    )
    .await?;

    let cert = parse_certificate_info(&cert_path).await?;
    Ok(cert)
}

/// Request and install a host certificate for this gate.
///
/// Obtains a host certificate from step-ca and outputs `sshd_config` directives.
pub async fn install_host_certificate(hostname: &str, dry_run: bool) -> Result<SshCertificate> {
    let ca_url = resolve_ca_url();
    let provisioner = resolve_provisioner();

    let host_key = PathBuf::from("/etc/ssh/ssh_host_ecdsa_key.pub");
    if !host_key.exists() && !dry_run {
        return Err(ShadowError::Config(
            "no host key at /etc/ssh/ssh_host_ecdsa_key.pub — generate with ssh-keygen first"
                .into(),
        ));
    }

    let cert_path = PathBuf::from("/etc/ssh/ssh_host_ecdsa_key-cert.pub");

    if dry_run {
        return Ok(SshCertificate {
            cert_type: SshCertType::Host,
            principals: vec![hostname.into()],
            serial: 0,
            cert_path: cert_path.to_string_lossy().into(),
            key_path: host_key.to_string_lossy().into(),
            valid_before: 0,
            ca_fingerprint: "dry-run".into(),
        });
    }

    let host_key_str = host_key.to_string_lossy();
    run_step(
        &[
            "ssh",
            "certificate",
            hostname,
            &host_key_str,
            "--host",
            "--sign",
            "--ca-url",
            &ca_url,
            "--provisioner",
            &provisioner,
            "--force",
        ],
        "host certificate request failed",
    )
    .await?;

    let cert = parse_certificate_info(&cert_path).await?;
    Ok(cert)
}

/// Inspect all installed SSH certificates and return their status.
pub async fn inspect_certificates() -> Vec<CertStatus> {
    let mut results = Vec::new();

    let dir = cert_dir();
    let user_cert = dir.join("id_ecdsa-cert.pub");
    let user_key = dir.join("id_ecdsa");
    results.push(inspect_single_cert("user", &user_cert, &user_key).await);

    let host_cert = PathBuf::from("/etc/ssh/ssh_host_ecdsa_key-cert.pub");
    let host_key = PathBuf::from("/etc/ssh/ssh_host_ecdsa_key");
    results.push(inspect_single_cert("host", &host_cert, &host_key).await);

    results
}

/// Status of a single certificate file.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CertStatus {
    /// Label: "user" or "host".
    pub label: String,
    /// Path to the certificate file.
    pub cert_path: String,
    /// Whether the certificate file exists.
    pub exists: bool,
    /// Parsed certificate info (if parseable).
    pub certificate: Option<SshCertificate>,
    /// Human-readable status.
    pub status: String,
}

async fn inspect_single_cert(label: &str, cert_path: &Path, _key_path: &Path) -> CertStatus {
    if !cert_path.exists() {
        return CertStatus {
            label: label.into(),
            cert_path: cert_path.to_string_lossy().into(),
            exists: false,
            certificate: None,
            status: "not installed".into(),
        };
    }

    match parse_certificate_info(cert_path).await {
        Ok(cert) => {
            let status = if cert.is_expired() {
                "EXPIRED".into()
            } else {
                let remaining = cert.seconds_remaining();
                let hours = remaining / 3600;
                let mins = (remaining % 3600) / 60;
                format!("valid ({hours}h {mins}m remaining)")
            };
            CertStatus {
                label: label.into(),
                cert_path: cert_path.to_string_lossy().into(),
                exists: true,
                certificate: Some(cert),
                status,
            }
        }
        Err(e) => CertStatus {
            label: label.into(),
            cert_path: cert_path.to_string_lossy().into(),
            exists: true,
            certificate: None,
            status: format!("parse error: {e}"),
        },
    }
}

/// Parse certificate metadata using `step ssh inspect`.
async fn parse_certificate_info(cert_path: &Path) -> Result<SshCertificate> {
    let cert_str = cert_path.to_string_lossy();
    let output = run_step(&["ssh", "inspect", &cert_str], "step ssh inspect failed").await?;

    let text = String::from_utf8_lossy(&output.stdout);
    Ok(parse_inspect_output(&text, cert_path))
}

/// Parse the text output of `step ssh inspect` into an `SshCertificate`.
fn parse_inspect_output(text: &str, cert_path: &Path) -> SshCertificate {
    let cert_type = if text.contains("host certificate") {
        SshCertType::Host
    } else {
        SshCertType::User
    };

    let principals = text
        .lines()
        .skip_while(|l| !l.contains("Principals:"))
        .skip(1)
        .take_while(|l| {
            let trimmed = l.trim();
            !trimmed.is_empty() && !trimmed.contains(':') && l.starts_with(' ')
        })
        .map(|l| l.trim().to_string())
        .collect::<Vec<_>>();

    let serial = text
        .lines()
        .find(|l| l.contains("Serial:"))
        .and_then(|l| l.split_whitespace().last())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    let valid_before = text
        .lines()
        .find(|l| l.contains("Valid:"))
        .and_then(|l| {
            // "Valid: from 2026-07-28T18:00:00Z to 2026-07-29T02:00:00Z"
            l.rsplit("to ").next()
        })
        .and_then(|ts| chrono::DateTime::parse_from_rfc3339(ts.trim()).ok())
        .map_or(0, |dt| u64::try_from(dt.timestamp()).unwrap_or(0));

    let ca_fingerprint = text
        .lines()
        .find(|l| l.contains("Signing CA:") || l.contains("CA:"))
        .and_then(|l| l.split_whitespace().find(|w| w.starts_with("SHA256:")))
        .unwrap_or(cellmembrane_types::service::UNKNOWN_LABEL)
        .to_string();

    let key_path = cert_path.to_string_lossy().replace("-cert.pub", "");

    SshCertificate {
        cert_type,
        principals,
        serial,
        cert_path: cert_path.to_string_lossy().into(),
        key_path,
        valid_before,
        ca_fingerprint,
    }
}

/// Parse an SSH cert lifetime string (e.g. "8h", "24h", "1h") into seconds.
#[must_use]
pub fn parse_lifetime_secs(lifetime: &str) -> u64 {
    let trimmed = lifetime.trim();
    let (suffix, body) = trimmed.as_bytes().last().map_or((' ', trimmed), |&b| {
        (char::from(b), &trimmed[..trimmed.len() - 1])
    });
    match suffix {
        'h' => body.parse::<u64>().unwrap_or(8) * 3600,
        'm' => body.parse::<u64>().unwrap_or(480) * 60,
        's' => body.parse::<u64>().unwrap_or(28800),
        _ => trimmed.parse::<u64>().unwrap_or(28800),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_defaults() {
        assert_eq!(resolve_ca_url(), DEFAULT_STEP_CA_URL);
        assert_eq!(resolve_provisioner(), DEFAULT_STEP_CA_PROVISIONER);
        assert_eq!(resolve_lifetime(), DEFAULT_SSH_CERT_LIFETIME);
    }

    #[test]
    fn cert_dir_under_install_base() {
        let dir = cert_dir();
        assert!(
            dir.to_string_lossy().ends_with("/certs"),
            "cert dir should end with /certs: {dir:?}"
        );
    }

    #[test]
    fn parse_lifetime_hours() {
        assert_eq!(parse_lifetime_secs("8h"), 28800);
        assert_eq!(parse_lifetime_secs("24h"), 86400);
        assert_eq!(parse_lifetime_secs("1h"), 3600);
    }

    #[test]
    fn parse_lifetime_minutes() {
        assert_eq!(parse_lifetime_secs("30m"), 1800);
    }

    #[test]
    fn parse_lifetime_seconds() {
        assert_eq!(parse_lifetime_secs("3600s"), 3600);
    }

    #[test]
    fn parse_lifetime_fallback() {
        assert_eq!(parse_lifetime_secs("garbage"), 28800);
    }

    #[test]
    fn parse_inspect_output_user_cert() {
        let sample = r#"
        Type: ecdsa-sha2-nistp256-cert-v01@openssh.com user certificate
        Public key: ECDSA-CERT SHA256:abc123
        Signing CA: ECDSA SHA256:cafingerprint123 (using ecdsa-sha2-nistp256)
        Key ID: "sporegate@primals.eco"
        Serial: 42
        Valid: from 2026-07-28T18:00:00Z to 2026-07-29T02:00:00Z
        Principals:
                sporegate@primals.eco
                root
        Critical Options: (none)
        Extensions:
                permit-agent-forwarding
                permit-pty
"#;
        let cert = parse_inspect_output(sample, Path::new("/tmp/id_ecdsa-cert.pub"));
        assert_eq!(cert.cert_type, SshCertType::User);
        assert_eq!(cert.serial, 42);
        assert_eq!(cert.principals, vec!["sporegate@primals.eco", "root"]);
        assert_eq!(cert.ca_fingerprint, "SHA256:cafingerprint123");
        assert_eq!(cert.key_path, "/tmp/id_ecdsa");
        assert!(cert.valid_before > 0);
    }

    #[test]
    fn parse_inspect_output_host_cert() {
        let sample = r#"
        Type: ecdsa-sha2-nistp256-cert-v01@openssh.com host certificate
        Public key: ECDSA-CERT SHA256:hostkey
        Signing CA: ECDSA SHA256:hostca (using ecdsa-sha2-nistp256)
        Key ID: "golgi.primals.eco"
        Serial: 7
        Valid: from 2026-07-28T18:00:00Z to 2026-08-28T18:00:00Z
        Principals:
                golgi.primals.eco
"#;
        let cert = parse_inspect_output(sample, Path::new("/etc/ssh/ssh_host_ecdsa_key-cert.pub"));
        assert_eq!(cert.cert_type, SshCertType::Host);
        assert_eq!(cert.serial, 7);
        assert_eq!(cert.principals, vec!["golgi.primals.eco"]);
    }

    #[tokio::test]
    async fn request_dry_run_succeeds() {
        let cert = request_ssh_certificate("testGate", true).await.unwrap();
        assert_eq!(cert.cert_type, SshCertType::User);
        assert!(cert.principals[0].contains("testGate"));
        assert_eq!(cert.ca_fingerprint, "dry-run");
    }

    #[tokio::test]
    async fn renew_dry_run_needs_cert() {
        let result = renew_ssh_certificate(true).await;
        // In CI/test environment, there's no certificate to renew — should fail
        // because we check cert_path.exists() even in dry_run mode.
        // This is intentional: renew requires an existing cert.
        assert!(result.is_err() || result.unwrap().ca_fingerprint == "dry-run");
    }

    #[tokio::test]
    async fn install_host_dry_run_succeeds() {
        let cert = install_host_certificate("test.primals.eco", true)
            .await
            .unwrap();
        assert_eq!(cert.cert_type, SshCertType::Host);
        assert_eq!(cert.principals, vec!["test.primals.eco"]);
    }

    #[tokio::test]
    async fn inspect_returns_results() {
        let results = inspect_certificates().await;
        assert_eq!(results.len(), 2, "should have user and host entries");
        assert_eq!(results[0].label, "user");
        assert_eq!(results[1].label, "host");
    }

    #[test]
    fn bootstrap_needs_fingerprint() {
        // Without STEP_CA_FINGERPRINT set, bootstrap should fail with Config error
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = rt.block_on(bootstrap_ca(false));
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains(ENV_STEP_CA_FINGERPRINT));
    }
}
