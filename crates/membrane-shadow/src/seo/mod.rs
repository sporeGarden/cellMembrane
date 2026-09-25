// SPDX-License-Identifier: AGPL-3.0-or-later

//! Agentic SEO management — GSC + IndexNow + multi-site crawler notification.
//!
//! Wraps the GSC REST API for sitemap management, URL inspection, and
//! search analytics. Authenticates via Google service account (RS256 JWT).
//! Also provides IndexNow submission for instant crawler notification to
//! Bing, DuckDuckGo, Yandex, and Seznam.
//!
//! ## Progression
//!
//! - **Jelly string**: Python `gsc-agent.py` on golgiBody (deployed, works today)
//! - **Composition**: This Rust module — `membrane seo.*` commands
//! - **Capability**: SEO as a `petalTongue` routed capability (future)
//!
//! ## Multi-site Pattern
//!
//! All `*.primals.eco` sites share the `sc-domain:primals.eco` GSC property.
//! IndexNow is submitted per-site (each site has its own key + sitemap).
//! The `seo.notify` command is the webhook-triggered entry point:
//! push → Forgejo webhook → classify_push → seo.notify → IndexNow + GSC.
//!
//! The cascade probe (`seo.cascade`) shells out to the Python agent when
//! the Rust OAuth flow isn't available (no credentials file on this gate).
//! As primals graduate, the bridge pattern routes to the Python fallback
//! until the pure-Rust path handles all cases.

mod gsc;
mod indexnow;

use crate::error::{Result, ShadowError};

/// Environment variable for GSC service account credentials path.
const ENV_GSC_CREDENTIALS: &str = "GSC_CREDENTIALS_FILE";

/// Default credentials path on golgiBody.
const DEFAULT_CREDENTIALS_PATH: &str = "/opt/ecoPrimals/credentials/gsc-service-account.json";

/// GSC property identifier (domain property covers all subdomains).
const GSC_PROPERTY: &str = "sc-domain:primals.eco";

/// Sitemap URLs to submit/query (derived from PUBLISH_SITES).
const SITEMAP_URLS: &[&str] = &[
    "https://sporeprint.primals.eco/sitemap.xml",
    "https://detroit.primals.eco/sitemap.xml",
];

/// Configuration for a single site's SEO (IndexNow + sitemap).
pub struct SiteConfig {
    /// Hostname (e.g. "detroit.primals.eco").
    pub host: &'static str,
    /// Full sitemap URL.
    pub sitemap: &'static str,
    /// IndexNow verification key for this site.
    pub indexnow_key: &'static str,
}

/// Publishable static site — repo-to-host-to-worktree mapping.
///
/// This is the single registry for all `*.primals.eco` static sites.
/// The webhook pipeline uses this to classify pushes as `should_publish`,
/// the SEO module uses it for crawler notification, and the publish
/// pipeline uses it for zola build + deploy.
pub struct PublishSite {
    /// Forgejo repo name (case-insensitive match against push event).
    pub repo_name: &'static str,
    /// Hostname served by Caddy.
    pub host: &'static str,
    /// Git worktree path on golgiBody.
    pub worktree: &'static str,
    /// Output directory for built site (Caddy root).
    pub public_dir: &'static str,
    /// Subdirectory containing the zola project (relative to worktree).
    /// `None` if zola.toml is at the worktree root.
    pub build_subdir: Option<&'static str>,
    /// SEO config for this site.
    pub seo: SiteConfig,
    /// Local evidence depot directory (sporeGate authority path).
    /// If set, `evidence.push` will sync files to golgiBody alongside the site.
    pub evidence_dir: Option<&'static str>,
}

/// Registry of all publishable static sites.
///
/// When a push matches one of these repos, the webhook pipeline runs
/// `run_publish_pipeline` instead of harvest or cascade.
pub const PUBLISH_SITES: &[PublishSite] = &[
    PublishSite {
        repo_name: "detroit",
        host: "detroit.primals.eco",
        worktree: "/opt/ecoPrimals/detroit/worktree",
        public_dir: "/opt/ecoPrimals/detroit/public",
        build_subdir: Some("site"),
        seo: SiteConfig {
            host: "detroit.primals.eco",
            sitemap: "https://detroit.primals.eco/sitemap.xml",
            indexnow_key: "de552b8179f84854854cd2e02788a130",
        },
        evidence_dir: Some("/home/sporegate/Development/detroit/evidence"),
    },
    PublishSite {
        repo_name: "sporeprint",
        host: "sporeprint.primals.eco",
        worktree: "/opt/ecoPrimals/sporePrint",
        public_dir: "/opt/ecoPrimals/sporePrint/public",
        build_subdir: None,
        seo: SiteConfig {
            host: "sporeprint.primals.eco",
            sitemap: "https://sporeprint.primals.eco/sitemap.xml",
            indexnow_key: "de552b8179f84854854cd2e02788a130",
        },
        evidence_dir: None,
    },
];

/// Look up a publish site by repo name (case-insensitive).
pub fn find_publish_site(repo_name: &str) -> Option<&'static PublishSite> {
    let lower = repo_name.to_lowercase();
    PUBLISH_SITES
        .iter()
        .find(|s| s.repo_name.to_lowercase() == lower)
}

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
        "seo.indexnow" => {
            let site = crate::cli::extract_flag_value(args, "--site");
            let report = submit_indexnow(site.as_deref()).await?;
            Ok(crate::ShadowOutcome::ok(report))
        }
        "seo.notify" => {
            let site = crate::cli::extract_flag_value(args, "--site");
            let report = notify_crawlers(site.as_deref()).await?;
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

/// Resubmit all sitemaps to Google.
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

/// Submit URLs to IndexNow (Bing/DuckDuckGo/Yandex/Seznam).
///
/// If `site` is specified, only submits for that host.
/// Otherwise submits for all known sites.
async fn submit_indexnow(site: Option<&str>) -> Result<String> {
    let seo_configs: Vec<&SiteConfig> = match site {
        Some(host) => {
            let s = PUBLISH_SITES
                .iter()
                .find(|s| s.seo.host == host)
                .ok_or_else(|| ShadowError::config(format!("unknown site: {host}")))?;
            vec![&s.seo]
        }
        None => PUBLISH_SITES.iter().map(|s| &s.seo).collect(),
    };

    // Try pure-Rust first
    #[cfg(feature = "http")]
    {
        let mut results = Vec::new();
        for s in &seo_configs {
            match indexnow::submit(s).await {
                Ok(msg) => results.push(msg),
                Err(e) => results.push(format!("FAILED {}: {e}", s.host)),
            }
        }
        return Ok(results.join("; "));
    }

    // Fallback to Python agent
    #[cfg(not(feature = "http"))]
    python_agent(&["indexnow"]).await
}

/// Notify all crawlers about site changes (IndexNow + GSC sitemap resubmit).
///
/// This is the agentic entry point — called by webhook handlers after deploy.
/// Combines IndexNow (instant) + GSC sitemap resubmit (authoritative).
pub async fn notify_crawlers(site: Option<&str>) -> Result<String> {
    let mut lines = Vec::new();

    // IndexNow — instant notification
    match submit_indexnow(site).await {
        Ok(msg) => lines.push(format!("  [seo] indexnow: {msg}")),
        Err(e) => lines.push(format!("  [seo] indexnow: FAILED — {e}")),
    }

    // GSC sitemap resubmit — authoritative
    match submit_sitemap().await {
        Ok(msg) => lines.push(format!("  [seo] gsc: {msg}")),
        Err(e) => lines.push(format!("  [seo] gsc: FAILED — {e}")),
    }

    Ok(lines.join("\n"))
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
        assert!(SITEMAP_URLS.iter().all(|u| u.ends_with("sitemap.xml")));
        assert!(PYTHON_AGENT.ends_with(".py"));
    }

    #[test]
    fn publish_sites_have_consistent_seo() {
        for site in PUBLISH_SITES {
            assert!(site.seo.sitemap.contains(site.seo.host));
            assert!(site.seo.sitemap.ends_with("sitemap.xml"));
            assert!(!site.seo.indexnow_key.is_empty());
            assert_eq!(site.host, site.seo.host, "host mismatch for {}", site.repo_name);
        }
    }

    #[test]
    fn publish_sites_have_valid_paths() {
        for site in PUBLISH_SITES {
            assert!(site.worktree.starts_with('/'), "worktree must be absolute: {}", site.repo_name);
            assert!(site.public_dir.starts_with('/'), "public_dir must be absolute: {}", site.repo_name);
            assert!(!site.repo_name.is_empty());
        }
    }

    #[test]
    fn sitemap_urls_match_publish_sites() {
        for site in PUBLISH_SITES {
            assert!(
                SITEMAP_URLS.contains(&site.seo.sitemap),
                "PUBLISH_SITES entry {} not in SITEMAP_URLS",
                site.host
            );
        }
    }

    #[test]
    fn find_publish_site_case_insensitive() {
        assert!(find_publish_site("detroit").is_some());
        assert!(find_publish_site("Detroit").is_some());
        assert!(find_publish_site("DETROIT").is_some());
        assert!(find_publish_site("sporePrint").is_some());
        assert!(find_publish_site("nonexistent").is_none());
    }

    #[tokio::test]
    async fn dispatch_unknown_returns_fail() {
        let result = dispatch("seo.nonexistent", &[]).await.unwrap();
        assert!(!result.ok);
        assert!(result.message.contains("unknown seo command"));
    }
}
