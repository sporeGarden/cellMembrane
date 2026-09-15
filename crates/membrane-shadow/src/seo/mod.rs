// SPDX-License-Identifier: AGPL-3.0-or-later

//! Google Search Console API client — agentic SEO management.
//!
//! Wraps the GSC REST API for sitemap management, URL inspection, and
//! search analytics. Authenticates via Google service account (RS256 JWT).
//!
//! ## Progression
//!
//! - **Jelly string**: Python `gsc-agent.py` on golgiBody (deployed, works today)
//! - **Composition**: This Rust module — `membrane seo.*` commands
//! - **Capability**: SEO as a `petalTongue` routed capability (future)
//!
//! The cascade probe (`seo.cascade`) shells out to the Python agent when
//! the Rust OAuth flow isn't available (no credentials file on this gate).
//! As primals graduate, the bridge pattern routes to the Python fallback
//! until the pure-Rust path handles all cases.

mod gsc;

use crate::error::{Result, ShadowError};

/// Environment variable for GSC service account credentials path.
const ENV_GSC_CREDENTIALS: &str = "GSC_CREDENTIALS_FILE";

/// Default credentials path on golgiBody.
const DEFAULT_CREDENTIALS_PATH: &str = "/opt/ecoPrimals/credentials/gsc-service-account.json";

/// GSC property identifier (domain property covers all subdomains).
const GSC_PROPERTY: &str = "sc-domain:primals.eco";

/// Sitemap URL to submit/query.
const SITEMAP_URL: &str = "https://sporeprint.primals.eco/sitemap.xml";

/// Python agent path (jelly string fallback).
const PYTHON_AGENT: &str = "/opt/ecoPrimals/bin/gsc-agent.py";

/// Python venv interpreter.
const PYTHON_VENV: &str = "/opt/ecoPrimals/venv-gsc/bin/python3";

// ── Dispatch ────────────────────────────────────────────────────────

/// Dispatch `seo.*` CLI commands.
pub async fn dispatch(cmd: &str, args: &[&str]) -> Result<crate::ShadowOutcome> {
    match cmd {
        "seo.status" => {
            let days: u32 = crate::cli::extract_flag_value(args, "--days")
                .and_then(|v| v.parse().ok())
                .unwrap_or(7);
            let report = status(days).await?;
            Ok(crate::ShadowOutcome::ok(report))
        }
        "seo.submit" => {
            let msg = submit_sitemap().await?;
            Ok(crate::ShadowOutcome::ok(msg))
        }
        "seo.inspect" => {
            let url = crate::cli::require_arg(args, 0, "URL")?;
            let report = inspect_url(url).await?;
            Ok(crate::ShadowOutcome::ok(report))
        }
        "seo.cascade" => {
            let report = cascade_probe().await;
            Ok(crate::ShadowOutcome::ok(report))
        }
        _ => Ok(crate::ShadowOutcome::fail(format!(
            "unknown seo command: {cmd}"
        ))),
    }
}

// ── Public API ──────────────────────────────────────────────────────

/// Full status report: sitemap, analytics, homepage inspection.
async fn status(days: u32) -> Result<String> {
    // Try pure-Rust GSC client first
    if let Ok(client) = gsc::GscClient::from_env().await {
        return gsc::status_report(&client, days).await;
    }
    // Fallback to Python agent
    python_agent(&["status", &days.to_string()]).await
}

/// Resubmit sitemap to Google.
async fn submit_sitemap() -> Result<String> {
    if let Ok(client) = gsc::GscClient::from_env().await {
        return gsc::submit_sitemap(&client).await;
    }
    python_agent(&["submit"]).await
}

/// Inspect a specific URL's indexing status.
async fn inspect_url(url: &str) -> Result<String> {
    if let Ok(client) = gsc::GscClient::from_env().await {
        return gsc::inspect_url(&client, url).await;
    }
    python_agent(&["inspect", url]).await
}

/// Cascade probe: compact status line for cascade-sense output.
/// Always succeeds (returns error as status text, not Err).
pub async fn cascade_probe() -> String {
    // Try Python agent first (most reliable on golgiBody)
    match python_agent(&["cascade"]).await {
        Ok(output) => output,
        Err(e) => {
            // Try pure-Rust fallback
            match try_rust_cascade_probe().await {
                Ok(output) => output,
                Err(_) => format!("  [seo] SKIP: {e}"),
            }
        }
    }
}

async fn try_rust_cascade_probe() -> Result<String> {
    let client = gsc::GscClient::from_env().await?;
    gsc::cascade_probe(&client).await
}

// ── Python Agent Fallback ───────────────────────────────────────────

/// Shell out to the Python GSC agent (jelly string bridge).
async fn python_agent(args: &[&str]) -> Result<String> {
    let python = if tokio::fs::metadata(PYTHON_VENV).await.is_ok() {
        PYTHON_VENV
    } else {
        "python3"
    };

    if tokio::fs::metadata(PYTHON_AGENT).await.is_err() {
        return Err(ShadowError::config(format!(
            "GSC agent not found at {PYTHON_AGENT} — deploy gsc-agent.py first"
        )));
    }

    let output = tokio::process::Command::new(python)
        .arg(PYTHON_AGENT)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .map_err(|e| ShadowError::Io(e))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(ShadowError::config(format!(
            "GSC agent failed: {}",
            stderr.lines().next().unwrap_or("unknown error")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_are_correct() {
        assert!(GSC_PROPERTY.starts_with("sc-domain:"));
        assert!(SITEMAP_URL.ends_with("sitemap.xml"));
        assert!(PYTHON_AGENT.ends_with(".py"));
    }

    #[tokio::test]
    async fn dispatch_unknown_returns_fail() {
        let result = dispatch("seo.nonexistent", &[]).await.unwrap();
        assert!(!result.ok);
        assert!(result.message.contains("unknown seo command"));
    }
}
