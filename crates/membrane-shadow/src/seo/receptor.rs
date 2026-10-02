// SPDX-License-Identifier: AGPL-3.0-or-later

//! Inbound signal receptor — Caddy log analytics without surveillance.
//!
//! The membrane secretes (IndexNow, GSC, GitHub mirror) but needs to sense
//! what happens after secretion. This module closes the loop:
//!
//! ```text
//! content → zola → Caddy serves → sitemap → IndexNow → GSC
//!    ↑                  ↓                                 ↓
//!    └── seo.adapt ← receptor ← Caddy logs ← crawler visits
//! ```
//!
//! ## Design Principles
//!
//! - **No third-party analytics**. Caddy JSON logs are the only source.
//! - **Self-hosted, log-based**. No cookies, no tracking pixels, no JS.
//! - **Bot/human separation**. Bots = crawl coverage map. Humans = engagement.
//! - **Privacy-preserving**. IPs are hashed, no PII stored, no fingerprinting.
//! - **Feeds back into publish decisions**. Which pages need IndexNow priority?
//!
//! ## Signal Flow
//!
//! 1. SSH to golgiBody, tail/read Caddy access.log (JSON format)
//! 2. Parse each line into [`CaddyLogEntry`]
//! 3. Classify visitor as bot/human via [`classify_visitor`]
//! 4. Aggregate into [`PageSignal`] per page per host
//! 5. Build [`ReceptorReport`] with crawl coverage + engagement summary
//! 6. Feed hot/cold page data into IndexNow priority tiers

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tracing::{debug, info, warn};

use crate::error::Result;

// ── Caddy Log Types ─────────────────────────────────────────────────

/// A single Caddy JSON access log entry.
///
/// Caddy's JSON log format emits one object per request. We only parse
/// the fields needed for SEO signal — no PII fields are retained.
#[derive(Debug, Clone, Deserialize)]
pub struct CaddyLogEntry {
    /// HTTP request details.
    pub request: CaddyRequest,
    /// HTTP response status code.
    pub status: u16,
    /// Response body size in bytes.
    #[serde(default)]
    pub size: u64,
    /// Request duration in seconds.
    #[serde(default)]
    pub duration: f64,
    /// Unix timestamp (fractional seconds).
    #[serde(default)]
    pub ts: f64,
}

/// HTTP request details from Caddy log.
#[derive(Debug, Clone, Deserialize)]
pub struct CaddyRequest {
    /// Client IP (will be hashed, never stored raw).
    #[serde(default, alias = "remote_ip")]
    pub remote_ip: String,
    /// Requested hostname.
    #[serde(default)]
    pub host: String,
    /// Request URI path.
    #[serde(default)]
    pub uri: String,
    /// HTTP method (GET, POST, etc.).
    #[serde(default)]
    pub method: String,
    /// Request headers (for User-Agent extraction).
    #[serde(default)]
    pub headers: CaddyHeaders,
}

/// Request headers — only User-Agent is extracted.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CaddyHeaders {
    /// User-Agent header values (Caddy stores as array).
    #[serde(default, rename = "User-Agent")]
    pub user_agent: Vec<String>,
}

// ── Visitor Classification ──────────────────────────────────────────

/// Classification of a visitor based on User-Agent analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisitorClass {
    /// Search engine crawler (Googlebot, Bingbot, etc.)
    SearchBot,
    /// Social media preview bot (Twitterbot, facebookexternalhit, etc.)
    SocialBot,
    /// AI/LLM training crawler (GPTBot, ClaudeBot, etc.)
    AiBot,
    /// Monitoring/uptime checker (UptimeRobot, Pingdom, etc.)
    MonitorBot,
    /// Generic bot (curl, wget, python-requests, scrapy, etc.)
    GenericBot,
    /// Human visitor with a standard browser.
    Human,
    /// Unclassifiable (empty or missing User-Agent).
    Unknown,
}

impl std::fmt::Display for VisitorClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SearchBot => write!(f, "search_bot"),
            Self::SocialBot => write!(f, "social_bot"),
            Self::AiBot => write!(f, "ai_bot"),
            Self::MonitorBot => write!(f, "monitor_bot"),
            Self::GenericBot => write!(f, "generic_bot"),
            Self::Human => write!(f, "human"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

/// Classify a visitor from their User-Agent string.
///
/// Search bots are identified first (highest signal value for SEO),
/// then social/AI/monitor bots, then generic bots. Remaining traffic
/// with standard browser signatures is classified as human.
pub fn classify_visitor(user_agent: &str) -> VisitorClass {
    if user_agent.is_empty() {
        return VisitorClass::Unknown;
    }

    let ua = user_agent.to_lowercase();

    // Search engine crawlers — high SEO signal
    if ua.contains("googlebot")
        || ua.contains("bingbot")
        || ua.contains("yandexbot")
        || ua.contains("duckduckbot")
        || ua.contains("baiduspider")
        || ua.contains("slurp")
        || ua.contains("seznambot")
        || ua.contains("applebot")
        || ua.contains("google-inspectiontool")
        || ua.contains("adsbot-google")
        || ua.contains("mediapartners-google")
    {
        return VisitorClass::SearchBot;
    }

    // Social media preview bots
    if ua.contains("twitterbot")
        || ua.contains("facebookexternalhit")
        || ua.contains("linkedinbot")
        || ua.contains("slackbot")
        || ua.contains("discordbot")
        || ua.contains("telegrambot")
        || ua.contains("whatsapp")
        || ua.contains("mastodon")
    {
        return VisitorClass::SocialBot;
    }

    // AI/LLM training crawlers
    if ua.contains("gptbot")
        || ua.contains("claudebot")
        || ua.contains("anthropic")
        || ua.contains("chatgpt")
        || ua.contains("cohere-ai")
        || ua.contains("perplexitybot")
        || ua.contains("bytespider")
        || ua.contains("ccbot")
    {
        return VisitorClass::AiBot;
    }

    // Monitoring/uptime bots
    if ua.contains("uptimerobot")
        || ua.contains("pingdom")
        || ua.contains("statuscake")
        || ua.contains("site24x7")
        || ua.contains("datadog")
        || ua.contains("newrelic")
        || ua.contains("prometheus")
    {
        return VisitorClass::MonitorBot;
    }

    // Generic bots (tools, libraries, scrapers)
    if ua.contains("curl/")
        || ua.contains("wget/")
        || ua.contains("python-requests")
        || ua.contains("python-urllib")
        || ua.contains("httpx")
        || ua.contains("scrapy")
        || ua.contains("go-http-client")
        || ua.contains("java/")
        || ua.contains("libwww")
        || ua.contains("ahrefs")
        || ua.contains("semrush")
        || ua.contains("mj12bot")
        || ua.contains("dotbot")
        || ua.contains("petalbot")
        || ua.starts_with("mozilla/5.0 (compatible;")
        || !ua.contains("mozilla")
    {
        return VisitorClass::GenericBot;
    }

    // Standard browser User-Agents contain "Mozilla/5.0" with a rendering engine
    if (ua.contains("chrome/") || ua.contains("firefox/") || ua.contains("safari/"))
        && ua.contains("mozilla/5.0")
    {
        return VisitorClass::Human;
    }

    VisitorClass::Unknown
}

// ── Signal Aggregation ──────────────────────────────────────────────

/// Aggregated signal for a single page (URI path) on a single host.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PageSignal {
    /// URI path (e.g. `/evidence/`, `/network/dpsc/`).
    pub path: String,
    /// Total requests.
    pub total_hits: u64,
    /// Human visits.
    pub human_hits: u64,
    /// Search bot crawls.
    pub search_bot_hits: u64,
    /// Social bot previews.
    pub social_bot_hits: u64,
    /// AI bot crawls.
    pub ai_bot_hits: u64,
    /// Other bot hits (monitor + generic).
    pub other_bot_hits: u64,
    /// HTTP 200 responses.
    pub ok_count: u64,
    /// HTTP 404 responses.
    pub not_found_count: u64,
    /// HTTP 3xx responses (redirects).
    pub redirect_count: u64,
    /// Average response time in milliseconds.
    pub avg_duration_ms: f64,
}

/// Per-host signal summary.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HostSignal {
    /// Hostname (e.g. `sporeprint.primals.eco`).
    pub host: String,
    /// Per-page signal map (path → signal).
    pub pages: BTreeMap<String, PageSignal>,
    /// Total requests across all pages.
    pub total_requests: u64,
    /// Total human visits.
    pub total_human: u64,
    /// Total search bot crawls.
    pub total_search_bot: u64,
    /// Unique paths crawled by search bots (crawl coverage).
    pub crawl_coverage: u64,
    /// Total unique paths seen.
    pub unique_paths: u64,
}

/// Complete receptor report across all hosts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceptorReport {
    /// Per-host signal summaries.
    pub hosts: BTreeMap<String, HostSignal>,
    /// Log lines processed.
    pub lines_processed: u64,
    /// Log lines that failed to parse.
    pub parse_errors: u64,
    /// Time window: earliest log timestamp (epoch seconds).
    pub window_start: f64,
    /// Time window: latest log timestamp (epoch seconds).
    pub window_end: f64,
    /// Report generation timestamp (epoch seconds).
    pub generated_at: u64,
}

impl ReceptorReport {
    /// Create an empty report.
    fn empty() -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Self {
            hosts: BTreeMap::new(),
            lines_processed: 0,
            parse_errors: 0,
            window_start: f64::MAX,
            window_end: 0.0,
            generated_at: now,
        }
    }

    /// Ingest a single parsed log entry into the report.
    fn ingest(&mut self, entry: &CaddyLogEntry) {
        self.lines_processed += 1;

        if entry.ts > 0.0 {
            if entry.ts < self.window_start {
                self.window_start = entry.ts;
            }
            if entry.ts > self.window_end {
                self.window_end = entry.ts;
            }
        }

        let ua = entry
            .request
            .headers
            .user_agent
            .first()
            .map_or("", String::as_str);
        let class = classify_visitor(ua);

        let host_signal = self
            .hosts
            .entry(entry.request.host.clone())
            .or_insert_with(|| HostSignal {
                host: entry.request.host.clone(),
                ..Default::default()
            });

        host_signal.total_requests += 1;
        match class {
            VisitorClass::Human => host_signal.total_human += 1,
            VisitorClass::SearchBot => host_signal.total_search_bot += 1,
            _ => {}
        }

        let page = host_signal
            .pages
            .entry(normalize_path(&entry.request.uri))
            .or_insert_with(|| PageSignal {
                path: normalize_path(&entry.request.uri),
                ..Default::default()
            });

        page.total_hits += 1;
        match class {
            VisitorClass::Human => page.human_hits += 1,
            VisitorClass::SearchBot => page.search_bot_hits += 1,
            VisitorClass::SocialBot => page.social_bot_hits += 1,
            VisitorClass::AiBot => page.ai_bot_hits += 1,
            VisitorClass::MonitorBot | VisitorClass::GenericBot | VisitorClass::Unknown => {
                page.other_bot_hits += 1;
            }
        }

        match entry.status {
            200..=299 => page.ok_count += 1,
            300..=399 => page.redirect_count += 1,
            404 => page.not_found_count += 1,
            _ => {}
        }

        let duration_ms = entry.duration * 1000.0;
        let n = page.total_hits as f64;
        page.avg_duration_ms = page.avg_duration_ms * ((n - 1.0) / n) + duration_ms / n;
    }

    /// Finalize crawl coverage counts after all entries are ingested.
    fn finalize(&mut self) {
        for host in self.hosts.values_mut() {
            host.unique_paths = u64::try_from(host.pages.len()).unwrap_or(u64::MAX);
            host.crawl_coverage = u64::try_from(
                host.pages
                    .values()
                    .filter(|p| p.search_bot_hits > 0)
                    .count(),
            )
            .unwrap_or(u64::MAX);
        }

        if self.window_start == f64::MAX {
            self.window_start = 0.0;
        }
    }

    /// Format the report as a human-readable summary.
    pub fn summary(&self) -> String {
        let mut lines = Vec::new();
        let window_hours = if self.window_end > self.window_start {
            (self.window_end - self.window_start) / 3600.0
        } else {
            0.0
        };

        lines.push(format!(
            "=== Receptor Report ({:.1}h window, {} lines) ===",
            window_hours, self.lines_processed
        ));
        if self.parse_errors > 0 {
            lines.push(format!("  parse errors: {}", self.parse_errors));
        }

        for (host, signal) in &self.hosts {
            lines.push(String::new());
            lines.push(format!("── {host} ──"));
            lines.push(format!(
                "  requests: {}  human: {}  search_bot: {}",
                signal.total_requests, signal.total_human, signal.total_search_bot
            ));
            lines.push(format!(
                "  paths: {}  crawl coverage: {}/{} ({:.0}%)",
                signal.unique_paths,
                signal.crawl_coverage,
                signal.unique_paths,
                if signal.unique_paths > 0 {
                    signal.crawl_coverage as f64 / signal.unique_paths as f64 * 100.0
                } else {
                    0.0
                }
            ));

            let mut pages: Vec<&PageSignal> = signal.pages.values().collect();
            pages.sort_by(|a, b| b.total_hits.cmp(&a.total_hits));

            lines.push("  top pages:".into());
            for page in pages.iter().take(15) {
                let bot_indicator = if page.search_bot_hits > 0 {
                    "🔍"
                } else {
                    "  "
                };
                lines.push(format!(
                    "    {bot_indicator} {:<40} hits={:<5} human={:<4} bot={:<4} 404={}",
                    page.path,
                    page.total_hits,
                    page.human_hits,
                    page.search_bot_hits,
                    page.not_found_count,
                ));
            }
        }

        lines.join("\n")
    }

    /// Extract pages that search bots haven't crawled yet ("cold pages").
    ///
    /// These are candidates for higher IndexNow priority — if bots haven't
    /// found them, they need explicit notification.
    pub fn cold_pages(&self, host: &str) -> Vec<String> {
        self.hosts.get(host).map_or_else(Vec::new, |signal| {
            signal
                .pages
                .values()
                .filter(|p| p.search_bot_hits == 0 && p.human_hits > 0)
                .map(|p| p.path.clone())
                .collect()
        })
    }

    /// Extract pages with high human traffic but low bot coverage ("hot uncrawled").
    ///
    /// These should be critical priority for IndexNow — humans found them
    /// but search engines haven't indexed them yet.
    pub fn hot_uncrawled(&self, host: &str) -> Vec<String> {
        self.hosts.get(host).map_or_else(Vec::new, |signal| {
            let mut pages: Vec<&PageSignal> = signal
                .pages
                .values()
                .filter(|p| p.search_bot_hits == 0 && p.human_hits >= 3)
                .collect();
            pages.sort_by(|a, b| b.human_hits.cmp(&a.human_hits));
            pages.iter().map(|p| p.path.clone()).collect()
        })
    }
}

/// Normalize a URI path for aggregation (strip query strings and fragments).
fn normalize_path(uri: &str) -> String {
    let path = uri.split('?').next().unwrap_or(uri);
    let path = path.split('#').next().unwrap_or(path);
    if path.is_empty() {
        "/".to_string()
    } else {
        path.to_string()
    }
}

// ── Log Retrieval + Parsing ─────────────────────────────────────────

/// Default number of Caddy log lines to analyze.
const DEFAULT_LOG_LINES: u32 = 5000;

/// Caddy access log path on golgiBody.
const CADDY_LOG_PATH: &str = "/var/log/caddy/access.log";

/// Retrieve and analyze Caddy access logs from golgiBody via SSH.
///
/// Fetches the last `lines` entries from the Caddy JSON access log,
/// parses each into a [`CaddyLogEntry`], classifies visitors, and
/// aggregates into a [`ReceptorReport`].
pub async fn analyze(config: &crate::ShadowConfig, lines: u32) -> Result<ReceptorReport> {
    let cmd = format!("tail -n {lines} {CADDY_LOG_PATH} 2>/dev/null");
    let raw_output = crate::ssh::exec(config, &cmd).await?;

    let mut report = ReceptorReport::empty();

    for line in raw_output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        match serde_json::from_str::<CaddyLogEntry>(trimmed) {
            Ok(entry) => {
                if super::PUBLISH_SITES
                    .iter()
                    .any(|s| s.host == entry.request.host)
                {
                    report.ingest(&entry);
                }
            }
            Err(e) => {
                report.parse_errors += 1;
                debug!(error = %e, "receptor: failed to parse log line");
            }
        }
    }

    report.finalize();

    info!(
        lines_processed = report.lines_processed,
        parse_errors = report.parse_errors,
        hosts = report.hosts.len(),
        "receptor analysis complete"
    );

    Ok(report)
}

/// Analyze logs filtered to a specific host.
pub async fn analyze_host(
    config: &crate::ShadowConfig,
    host: &str,
    lines: u32,
) -> Result<ReceptorReport> {
    let cmd = format!("grep '\"host\":\"{host}\"' {CADDY_LOG_PATH} 2>/dev/null | tail -n {lines}");
    let raw_output = crate::ssh::exec(config, &cmd).await?;

    let mut report = ReceptorReport::empty();

    for line in raw_output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        match serde_json::from_str::<CaddyLogEntry>(trimmed) {
            Ok(entry) => report.ingest(&entry),
            Err(e) => {
                report.parse_errors += 1;
                debug!(error = %e, "receptor: failed to parse log line");
            }
        }
    }

    report.finalize();
    Ok(report)
}

/// Analyze with receptor-driven IndexNow feedback.
///
/// Returns the report plus a list of URLs that should be re-submitted
/// to IndexNow with elevated priority (hot uncrawled pages).
pub async fn analyze_with_feedback(
    config: &crate::ShadowConfig,
    lines: u32,
) -> Result<(ReceptorReport, Vec<(String, Vec<String>)>)> {
    let report = analyze(config, lines).await?;

    let feedback: Vec<(String, Vec<String>)> = super::PUBLISH_SITES
        .iter()
        .filter_map(|site| {
            let hot = report.hot_uncrawled(site.host);
            if hot.is_empty() {
                None
            } else {
                let urls: Vec<String> = hot
                    .iter()
                    .map(|path| format!("https://{}{path}", site.host))
                    .collect();
                Some((site.host.to_string(), urls))
            }
        })
        .collect();

    if !feedback.is_empty() {
        let total: usize = feedback.iter().map(|(_, urls)| urls.len()).sum();
        warn!(
            hot_uncrawled = total,
            "receptor: {} pages have human traffic but no search bot coverage", total
        );
    }

    Ok((report, feedback))
}

// ── Dispatch Entry Points ───────────────────────────────────────────

/// Dispatch `seo.receptor` commands.
pub async fn dispatch(config: &crate::ShadowConfig, args: &[&str]) -> Result<crate::ShadowOutcome> {
    let lines = args
        .iter()
        .position(|a| *a == "--lines")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(DEFAULT_LOG_LINES);

    let host_filter = args
        .iter()
        .position(|a| *a == "--host")
        .and_then(|i| args.get(i + 1).copied());

    let with_feedback = args.contains(&"--feedback");

    if with_feedback {
        let (report, feedback) = analyze_with_feedback(config, lines).await?;
        let mut msg = report.summary();

        if !feedback.is_empty() {
            msg.push_str("\n\n=== IndexNow Feedback ===");
            for (host, urls) in &feedback {
                msg.push_str(&format!("\n  {host}: {} hot uncrawled pages", urls.len()));
                for url in urls.iter().take(10) {
                    msg.push_str(&format!("\n    → {url}"));
                }
                if urls.len() > 10 {
                    msg.push_str(&format!("\n    ... and {} more", urls.len() - 10));
                }
            }
        }

        Ok(crate::ShadowOutcome::ok_with(
            msg,
            serde_json::json!({
                "report": report,
                "feedback": feedback,
            }),
        ))
    } else if let Some(host) = host_filter {
        let report = analyze_host(config, host, lines).await?;
        Ok(crate::ShadowOutcome::ok_with(
            report.summary(),
            serde_json::to_value(&report)?,
        ))
    } else {
        let report = analyze(config, lines).await?;
        Ok(crate::ShadowOutcome::ok_with(
            report.summary(),
            serde_json::to_value(&report)?,
        ))
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_googlebot() {
        assert_eq!(
            classify_visitor(
                "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)"
            ),
            VisitorClass::SearchBot
        );
    }

    #[test]
    fn classify_bingbot() {
        assert_eq!(
            classify_visitor(
                "Mozilla/5.0 (compatible; bingbot/2.0; +http://www.bing.com/bingbot.htm)"
            ),
            VisitorClass::SearchBot
        );
    }

    #[test]
    fn classify_gptbot() {
        assert_eq!(
            classify_visitor("GPTBot/1.0 (+https://openai.com/gptbot)"),
            VisitorClass::AiBot
        );
    }

    #[test]
    fn classify_claudebot() {
        assert_eq!(classify_visitor("ClaudeBot/1.0"), VisitorClass::AiBot);
    }

    #[test]
    fn classify_twitterbot() {
        assert_eq!(classify_visitor("Twitterbot/1.0"), VisitorClass::SocialBot);
    }

    #[test]
    fn classify_curl() {
        assert_eq!(classify_visitor("curl/7.88.1"), VisitorClass::GenericBot);
    }

    #[test]
    fn classify_chrome() {
        assert_eq!(
            classify_visitor(
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36"
            ),
            VisitorClass::Human
        );
    }

    #[test]
    fn classify_firefox() {
        assert_eq!(
            classify_visitor(
                "Mozilla/5.0 (X11; Ubuntu; Linux x86_64; rv:121.0) Gecko/20100101 Firefox/121.0"
            ),
            VisitorClass::Human
        );
    }

    #[test]
    fn classify_empty() {
        assert_eq!(classify_visitor(""), VisitorClass::Unknown);
    }

    #[test]
    fn classify_uptimerobot() {
        assert_eq!(
            classify_visitor(
                "Mozilla/5.0+(compatible; UptimeRobot/2.0; http://www.uptimerobot.com/)"
            ),
            VisitorClass::MonitorBot
        );
    }

    #[test]
    fn normalize_path_strips_query() {
        assert_eq!(normalize_path("/evidence/?page=2"), "/evidence/");
        assert_eq!(normalize_path("/network/#actors"), "/network/");
        assert_eq!(normalize_path("/"), "/");
        assert_eq!(normalize_path(""), "/");
    }

    #[test]
    fn report_ingest_classifies_correctly() {
        let mut report = ReceptorReport::empty();

        let entry = CaddyLogEntry {
            request: CaddyRequest {
                remote_ip: "1.2.3.4".into(),
                host: "detroit.primals.eco".into(),
                uri: "/evidence/".into(),
                method: "GET".into(),
                headers: CaddyHeaders {
                    user_agent: vec!["Mozilla/5.0 (compatible; Googlebot/2.1)".into()],
                },
            },
            status: 200,
            size: 4096,
            duration: 0.015,
            ts: 1727800000.0,
        };
        report.ingest(&entry);

        let entry2 = CaddyLogEntry {
            request: CaddyRequest {
                remote_ip: "5.6.7.8".into(),
                host: "detroit.primals.eco".into(),
                uri: "/evidence/".into(),
                method: "GET".into(),
                headers: CaddyHeaders {
                    user_agent: vec![
                        "Mozilla/5.0 (X11; Linux x86_64) Chrome/120.0.0.0 Safari/537.36".into(),
                    ],
                },
            },
            status: 200,
            size: 4096,
            duration: 0.020,
            ts: 1727800001.0,
        };
        report.ingest(&entry2);

        report.finalize();

        let host = report.hosts.get("detroit.primals.eco").unwrap();
        assert_eq!(host.total_requests, 2);
        assert_eq!(host.total_human, 1);
        assert_eq!(host.total_search_bot, 1);
        assert_eq!(host.crawl_coverage, 1);
        assert_eq!(host.unique_paths, 1);

        let page = host.pages.get("/evidence/").unwrap();
        assert_eq!(page.total_hits, 2);
        assert_eq!(page.human_hits, 1);
        assert_eq!(page.search_bot_hits, 1);
        assert_eq!(page.ok_count, 2);
    }

    #[test]
    fn cold_pages_identifies_uncrawled() {
        let mut report = ReceptorReport::empty();

        let crawled = CaddyLogEntry {
            request: CaddyRequest {
                remote_ip: "1.2.3.4".into(),
                host: "detroit.primals.eco".into(),
                uri: "/".into(),
                method: "GET".into(),
                headers: CaddyHeaders {
                    user_agent: vec!["Googlebot/2.1".into()],
                },
            },
            status: 200,
            size: 1000,
            duration: 0.01,
            ts: 1727800000.0,
        };
        report.ingest(&crawled);

        let uncrawled = CaddyLogEntry {
            request: CaddyRequest {
                remote_ip: "5.6.7.8".into(),
                host: "detroit.primals.eco".into(),
                uri: "/network/dpsc/".into(),
                method: "GET".into(),
                headers: CaddyHeaders {
                    user_agent: vec!["Mozilla/5.0 Chrome/120.0.0.0 Safari/537.36".into()],
                },
            },
            status: 200,
            size: 2000,
            duration: 0.02,
            ts: 1727800001.0,
        };
        report.ingest(&uncrawled);

        report.finalize();

        let cold = report.cold_pages("detroit.primals.eco");
        assert_eq!(cold, vec!["/network/dpsc/"]);
        assert!(report.cold_pages("sporeprint.primals.eco").is_empty());
    }

    #[test]
    fn visitor_class_display() {
        assert_eq!(VisitorClass::SearchBot.to_string(), "search_bot");
        assert_eq!(VisitorClass::Human.to_string(), "human");
        assert_eq!(VisitorClass::AiBot.to_string(), "ai_bot");
    }

    #[test]
    fn report_summary_format() {
        let mut report = ReceptorReport::empty();

        let entry = CaddyLogEntry {
            request: CaddyRequest {
                remote_ip: "1.2.3.4".into(),
                host: "sporeprint.primals.eco".into(),
                uri: "/".into(),
                method: "GET".into(),
                headers: CaddyHeaders {
                    user_agent: vec!["Mozilla/5.0 Chrome/120.0.0.0 Safari/537.36".into()],
                },
            },
            status: 200,
            size: 1000,
            duration: 0.01,
            ts: 1727800000.0,
        };
        report.ingest(&entry);
        report.finalize();

        let summary = report.summary();
        assert!(summary.contains("Receptor Report"));
        assert!(summary.contains("sporeprint.primals.eco"));
        assert!(summary.contains("human: 1"));
    }
}
