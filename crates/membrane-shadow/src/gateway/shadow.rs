// SPDX-License-Identifier: AGPL-3.0-or-later

//! Shadow validation — compare legacy Caddy path vs Tower gateway path.
//!
//! During the shadow period, Caddy owns :443 and bearDog listens on :8443.
//! This module probes both paths with the same requests and reports diffs.

use crate::error::Result;
use crate::{ShadowConfig, ShadowOutcome};
use cellmembrane_types::gateway::{ProbeResult, ShadowComparison, ShadowReport};
use std::time::Instant;

/// Default URLs to probe during shadow validation.
const DEFAULT_SHADOW_PATHS: &[&str] = &["/hub/login", "/api/status", "/hub/api"];

/// Dispatch `gateway.shadow` — compare Caddy (:443) vs Tower (:8443).
pub async fn dispatch_shadow(config: &ShadowConfig, args: &[&str]) -> Result<ShadowOutcome> {
    let default_host = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_DEPOT_HOSTNAME,
        "lab.primals.eco",
    );
    let host = extract_host(args).unwrap_or(default_host.as_str());
    let legacy_port = extract_port(args, "--legacy-port")
        .unwrap_or(cellmembrane_types::service::DEFAULT_HTTPS_PORT);
    let tower_port = extract_port(args, "--tower-port")
        .unwrap_or(cellmembrane_types::service::DEFAULT_SHADOW_PORT);
    let paths = extract_paths(args);

    let comparisons = run_shadow_comparisons(config, host, legacy_port, tower_port, &paths).await?;
    let report = ShadowReport::from_comparisons(comparisons);

    let summary = format!(
        "shadow: {}/{} pass (rate={:.0}%) legacy=:{legacy_port} tower=:{tower_port}",
        report.comparisons.iter().filter(|c| c.passes()).count(),
        report.comparisons.len(),
        report.pass_rate * 100.0,
    );

    if report.all_pass {
        Ok(ShadowOutcome::ok_with(
            summary,
            serde_json::to_value(&report)?,
        ))
    } else {
        Ok(ShadowOutcome {
            ok: false,
            message: summary,
            data: serde_json::to_value(&report).ok(),
        })
    }
}

/// Run shadow comparisons for all paths.
async fn run_shadow_comparisons(
    _config: &ShadowConfig,
    host: &str,
    legacy_port: u16,
    tower_port: u16,
    paths: &[&str],
) -> Result<Vec<ShadowComparison>> {
    let mut comparisons = Vec::with_capacity(paths.len());

    for path in paths {
        let legacy_url = format!("https://{host}:{legacy_port}{path}");
        let tower_url = format!("https://{host}:{tower_port}{path}");

        let legacy = probe_endpoint(&legacy_url).await;
        let tower = probe_endpoint(&tower_url).await;

        let match_status = legacy.is_ok() && tower.is_ok() && legacy.status == tower.status;

        comparisons.push(ShadowComparison {
            url: format!("{host}{path}"),
            legacy,
            tower,
            match_status,
        });
    }

    Ok(comparisons)
}

/// Probe a single HTTPS endpoint, returning timing + status.
async fn probe_endpoint(url: &str) -> ProbeResult {
    let client = match crate::http_client_insecure(std::time::Duration::from_secs(10)) {
        Ok(c) => c,
        Err(e) => return ProbeResult::err(format!("client build: {e}")),
    };

    let start = Instant::now();
    match client.get(url).send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let body_size = resp.content_length().unwrap_or(0);
            let elapsed = start.elapsed().as_millis();
            let latency_ms = u32::try_from(elapsed).unwrap_or(u32::MAX);
            ProbeResult::ok(status, latency_ms, body_size)
        }
        Err(e) => ProbeResult::err(format!("{e}")),
    }
}

// ── Argument helpers ─────────────────────────────────────────────────

fn extract_host<'a>(args: &[&'a str]) -> Option<&'a str> {
    args.iter()
        .position(|&a| a == "--host")
        .and_then(|i| args.get(i + 1).copied())
}

fn extract_port(args: &[&str], flag: &str) -> Option<u16> {
    args.iter()
        .position(|&a| a == flag)
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
}

/// Extract paths from args (positional or `--path` flags), falling back to defaults.
fn extract_paths<'a>(args: &[&'a str]) -> Vec<&'a str> {
    let mut paths = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--path" {
            if let Some(&val) = args.get(i + 1) {
                paths.push(val);
                i += 2;
                continue;
            }
        }
        i += 1;
    }
    if paths.is_empty() {
        DEFAULT_SHADOW_PATHS.to_vec()
    } else {
        paths
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_host_from_args() {
        let args = ["--host", "example.com", "--path", "/test"];
        assert_eq!(extract_host(&args), Some("example.com"));
    }

    #[test]
    fn extract_host_missing() {
        let args = ["--path", "/test"];
        assert_eq!(extract_host(&args), None);
    }

    #[test]
    fn extract_port_from_args() {
        let args = ["--tower-port", "8443"];
        assert_eq!(extract_port(&args, "--tower-port"), Some(8443));
    }

    #[test]
    fn extract_port_missing() {
        let args = ["--host", "example.com"];
        assert_eq!(extract_port(&args, "--tower-port"), None);
    }

    #[test]
    fn extract_paths_from_args() {
        let args = ["--path", "/hub", "--path", "/api"];
        let paths = extract_paths(&args);
        assert_eq!(paths, vec!["/hub", "/api"]);
    }

    #[test]
    fn extract_paths_defaults_when_empty() {
        let args: [&str; 0] = [];
        let paths = extract_paths(&args);
        assert_eq!(paths, DEFAULT_SHADOW_PATHS);
    }

    #[test]
    fn default_shadow_paths_not_empty() {
        assert!(!DEFAULT_SHADOW_PATHS.is_empty());
        for path in DEFAULT_SHADOW_PATHS {
            assert!(path.starts_with('/'));
        }
    }
}
