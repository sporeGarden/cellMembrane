// SPDX-License-Identifier: AGPL-3.0-or-later

//! Dispatch for `gate.keys` and `gate.keys.renew` — SSH certificate management.

use crate::ShadowOutcome;

pub(super) async fn dispatch_keys() -> crate::Result<ShadowOutcome> {
    use std::fmt::Write;

    let lifetime_str = std::env::var(cellmembrane_types::service::ENV_STEP_CA_SSH_LIFETIME)
        .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_SSH_CERT_LIFETIME.into());
    let lifetime_secs = crate::gate::key_portal::parse_lifetime_secs(&lifetime_str);

    let statuses = crate::gate::key_portal::inspect_certificates().await;
    let mut msg = String::from("SSH Certificate Status\n");
    for s in &statuses {
        let _ = write!(msg, "\n  {}: {} — {}", s.label, s.cert_path, s.status);
        if let Some(ref cert) = s.certificate {
            let _ = write!(msg, "\n    principals: {}", cert.principals.join(", "));
            let _ = write!(msg, "\n    serial: {}", cert.serial);
            let _ = write!(msg, "\n    ca: {}", cert.ca_fingerprint);
            if cert.needs_renewal(lifetime_secs) {
                let _ = write!(msg, "\n    ⚠ renewal recommended");
            }
        }
    }
    Ok(ShadowOutcome::ok_with(
        msg,
        serde_json::to_value(&statuses)?,
    ))
}

pub(super) async fn dispatch_keys_renew(args: &[&str]) -> crate::Result<ShadowOutcome> {
    let dry_run = args.contains(&"--dry-run");

    if args.contains(&"--bootstrap") {
        let msg = crate::gate::key_portal::bootstrap_ca(dry_run).await?;
        return Ok(ShadowOutcome::ok(msg));
    }

    if args.contains(&"--host") {
        let hostname = args
            .iter()
            .find(|a| !a.starts_with("--"))
            .copied()
            .unwrap_or("localhost");
        let cert = crate::gate::key_portal::install_host_certificate(hostname, dry_run).await?;
        return Ok(ShadowOutcome::ok_with(
            format!(
                "host cert issued for {} (serial {})",
                cert.principals.join(", "),
                cert.serial
            ),
            serde_json::to_value(&cert)?,
        ));
    }

    let cert = crate::gate::key_portal::renew_ssh_certificate(dry_run).await?;
    Ok(ShadowOutcome::ok_with(
        format!(
            "user cert renewed for {} (serial {})",
            cert.principals.join(", "),
            cert.serial
        ),
        serde_json::to_value(&cert)?,
    ))
}
