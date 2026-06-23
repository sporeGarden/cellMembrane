// SPDX-License-Identifier: AGPL-3.0-or-later

//! Content domain dispatch — S3 sporePrint content integrity verification.

use crate::{ShadowConfig, ShadowOutcome};

pub(super) async fn dispatch_content(
    config: &ShadowConfig,
    cmd: &str,
    _args: &[&str],
) -> crate::Result<ShadowOutcome> {
    match cmd {
        "content.verify" => dispatch_content_verify(config).await,
        _ => Ok(ShadowOutcome::fail(format!(
            "unknown content command: {cmd}"
        ))),
    }
}

async fn dispatch_content_verify(config: &ShadowConfig) -> crate::Result<ShadowOutcome> {
    let (caddy_out, caddy_code) =
        crate::ssh::exec_raw(config, "systemctl is-active caddy-tls").await?;
    let caddy_active = caddy_code == 0;

    let content_binary = cellmembrane_types::MembraneService::binary_for(
        cellmembrane_types::ServiceCapability::ContentServing,
    );
    let content_unit = format!("{content_binary}-membrane");
    let (svc_out, svc_code) =
        crate::ssh::exec_raw(config, &format!("systemctl is-active {content_unit}")).await?;
    let svc_active = svc_code == 0;

    let content_path = std::env::var(cellmembrane_types::service::ENV_NESTGATE_CONTENT_PATH)
        .unwrap_or_else(|_| {
            let install_base = cellmembrane_types::service::env_or(
                cellmembrane_types::service::ENV_INSTALL_BASE,
                cellmembrane_types::service::DEFAULT_INSTALL_BASE,
            );
            format!("{install_base}/{content_binary}/content")
        });
    let (content_count_out, _) = crate::ssh::exec_raw(
        config,
        &format!("find {content_path} -type f 2>/dev/null | wc -l"),
    )
    .await?;
    let content_files: u32 = content_count_out.trim().parse().unwrap_or(0);

    let content_svc = cellmembrane_types::MembraneService::with_capability(
        cellmembrane_types::ServiceCapability::ContentServing,
    );
    let content_port = std::env::var(cellmembrane_types::service::ENV_NESTGATE_PORT)
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or_else(|| {
            content_svc
                .and_then(|s| s.port)
                .unwrap_or(cellmembrane_types::service::DEFAULT_NESTGATE_PORT)
        });
    let bind = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_NUCLEUS_BIND,
        cellmembrane_types::service::BIND_LOOPBACK,
    );
    let (curl_out, curl_code) = crate::ssh::exec_raw(
        config,
        &format!("curl -s -o /dev/null -w '%{{http_code}}' http://{bind}:{content_port}/health 2>/dev/null"),
    )
    .await?;
    let http_status = curl_out.trim().to_string();
    let http_ok = curl_code == 0 && http_status == "200";

    let status = if caddy_active && svc_active && http_ok {
        "READY"
    } else {
        "NOT READY"
    };

    let msg = format!(
        "=== S3 Content Verification ===\n\
         Status:         {status}\n\
         Caddy TLS:      {} ({})\n\
         {content_binary}:       {} ({})\n\
         {content_binary} HTTP:  {} ({bind}:{content_port}/health)\n\
         Content files:  {content_files}",
        if caddy_active { "active" } else { "inactive" },
        caddy_out.trim(),
        if svc_active { "active" } else { "inactive" },
        svc_out.trim(),
        if http_ok { "200 OK" } else { &http_status },
    );

    let ok = caddy_active && svc_active && http_ok;
    Ok(if ok {
        ShadowOutcome::ok_with(
            msg,
            serde_json::json!({
                "status": status,
                "caddy": caddy_active,
                "content_service": content_binary,
                "content_active": svc_active,
                "content_http": http_status,
                "content_files": content_files,
            }),
        )
    } else {
        ShadowOutcome {
            ok: false,
            message: msg,
            data: Some(serde_json::json!({
                "status": status,
                "caddy": caddy_active,
                "content_service": content_binary,
                "content_active": svc_active,
                "content_http": http_status,
                "content_files": content_files,
            })),
        }
    })
}
