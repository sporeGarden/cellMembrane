// SPDX-License-Identifier: AGPL-3.0-or-later

//! Content domain dispatch — sporePrint static site build + integrity verification.

use crate::{ShadowConfig, ShadowOutcome};
use tracing::{info, warn};

pub(super) async fn dispatch_content(
    config: &ShadowConfig,
    cmd: &str,
    args: &[&str],
) -> crate::Result<ShadowOutcome> {
    match cmd {
        "content.rebuild" => dispatch_content_rebuild(args).await,
        "content.verify" => dispatch_content_verify(config).await,
        _ => Ok(ShadowOutcome::fail(format!(
            "unknown content command: {cmd}"
        ))),
    }
}

/// `content.rebuild` — run `zola build` in the sporePrint directory.
///
/// Intended to be chained after cascade on gates that serve the static site
/// (currently golgi). Finds sporePrint via workspace root + manifest path,
/// or falls back to `ECOPRIMALS_ROOT/infra/sporePrint`.
async fn dispatch_content_rebuild(args: &[&str]) -> crate::Result<ShadowOutcome> {
    let site_dir = resolve_sporeprint_dir(args);

    if !site_dir.join("config.toml").exists() {
        return Ok(ShadowOutcome::fail(format!(
            "content.rebuild: no config.toml in {} — not a Zola site",
            site_dir.display()
        )));
    }

    info!(path = %site_dir.display(), "content.rebuild: running zola build");

    let result = tokio::process::Command::new("zola")
        .arg("build")
        .current_dir(&site_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await;

    match result {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let line_count = stdout.lines().count();
            info!(pages = line_count, "content.rebuild: zola build succeeded");
            Ok(ShadowOutcome::ok(format!(
                "content.rebuild: OK — zola build in {} ({line_count} output lines)",
                site_dir.display()
            )))
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let first_line = stderr.lines().next().unwrap_or("unknown error");
            warn!(error = %first_line, "content.rebuild: zola build failed");
            Ok(ShadowOutcome::fail(format!(
                "content.rebuild: FAIL — {first_line}"
            )))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Ok(ShadowOutcome::fail(
                "content.rebuild: zola binary not found — install with: cargo install zola"
            ))
        }
        Err(e) => Ok(ShadowOutcome::fail(format!(
            "content.rebuild: execution error — {e}"
        ))),
    }
}

/// Resolve the sporePrint directory from args, env, or workspace.
fn resolve_sporeprint_dir(args: &[&str]) -> std::path::PathBuf {
    if let Some(path) = crate::cli::extract_flag_value(args, "--path") {
        return std::path::PathBuf::from(path);
    }

    if let Ok(root) = crate::temporal::resolve_workspace_root() {
        let manifest_path = root.join(cellmembrane_types::service::SPOREPRINT_CONTENT_DIR);
        if manifest_path.exists() {
            return manifest_path;
        }
    }

    let eco_root = std::env::var(cellmembrane_types::service::ENV_ECOPRIMALS_ROOT)
        .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT.to_string());
    std::path::PathBuf::from(eco_root).join(cellmembrane_types::service::SPOREPRINT_CONTENT_DIR)
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
