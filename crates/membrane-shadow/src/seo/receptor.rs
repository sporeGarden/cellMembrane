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
use tracing::{debug, info};

use crate::error::Result;

// Re-export classification types from cellmembrane-types so existing
// consumers of `seo::receptor::VisitorClass` continue to work, and
// skunky-ingest can import the same types from cellmembrane-types directly.
// Re-export classification types from cellmembrane-types for downstream
// consumers and internal `use super::*` in tests.
#[allow(unused_imports)]
pub use cellmembrane_types::visitor::{
    PROBE_PREFIXES, SESSION_WINDOW_SECS, VELOCITY_THRESHOLD, VELOCITY_WINDOW_SECS, VisitorClass,
    classify_path, classify_request, classify_visitor,
};

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
    #[allow(dead_code)] // deserialized from Caddy JSON, used for future bandwidth metrics
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
    #[allow(dead_code)] // deserialized from Caddy JSON, hashed elsewhere
    #[serde(default, alias = "remote_ip")]
    pub remote_ip: String,
    /// Requested hostname.
    #[serde(default)]
    pub host: String,
    /// Request URI path.
    #[serde(default)]
    pub uri: String,
    /// HTTP method (GET, POST, etc.).
    #[allow(dead_code)] // deserialized from Caddy JSON, needed for request classification
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
    /// Scraper/recon bot hits (credential scanners, vuln probes).
    #[serde(default)]
    pub scraper_bot_hits: u64,
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

// ── Session Detection ───────────────────────────────────────────────

/// A detected visitor session (same UA within a time window).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// SHA-256 hash of the User-Agent string (never stored raw).
    pub ua_hash: String,
    /// Visitor classification for this session.
    pub classification: VisitorClass,
    /// Ordered navigation path (pages visited in sequence).
    pub pages: Vec<String>,
    /// First page hit (entry page — correlates to which link was clicked).
    pub entry_page: String,
    /// Session duration in seconds (last hit - first hit).
    pub duration_s: u64,
    /// Host this session was on.
    pub host: String,
    /// Session start timestamp (epoch seconds).
    pub start_ts: f64,
}

/// Per-host session summary (aggregated from raw sessions).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionSummary {
    /// Total sessions detected.
    pub total_sessions: u64,
    /// Human sessions.
    pub human_sessions: u64,
    /// Average pages per human session (navigation depth).
    pub avg_depth: f64,
    /// Sessions with only 1 page hit (bounces).
    pub bounces: u64,
    /// Sessions with 3+ page hits (deep readers).
    pub deep_readers: u64,
}

/// Raw hit record for session grouping (internal, not serialized in report).
#[derive(Debug, Clone)]
struct RawHit {
    host: String,
    path: String,
    ua: String,
    class: VisitorClass,
    ts: f64,
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
    /// Per-host session summaries (populated during finalize).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sessions: BTreeMap<String, SessionSummary>,
    /// Raw hits for session grouping (not serialized).
    #[serde(skip)]
    raw_hits: Vec<RawHit>,
}

impl ReceptorReport {
    /// Create an empty report.
    pub(crate) fn empty() -> Self {
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
            sessions: BTreeMap::new(),
            raw_hits: Vec::new(),
        }
    }

    /// Test helper — same as `empty()` but publicly named for cross-module tests.
    #[cfg(test)]
    pub(crate) fn empty_for_test() -> Self {
        Self::empty()
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
        let normalized = normalize_path(&entry.request.uri);
        let class = classify_request(ua, &normalized);

        let host_signal = self
            .hosts
            .entry(entry.request.host.clone())
            .or_insert_with(|| HostSignal {
                host: entry.request.host.clone(),
                ..Default::default()
            });

        host_signal.total_requests += 1;
        match class {
            VisitorClass::Human | VisitorClass::AgenticHuman => host_signal.total_human += 1,
            VisitorClass::SearchBot => host_signal.total_search_bot += 1,
            _ => {}
        }

        let page = host_signal
            .pages
            .entry(normalized.clone())
            .or_insert_with(|| PageSignal {
                path: normalized.clone(),
                ..Default::default()
            });

        page.total_hits += 1;
        match class {
            VisitorClass::Human | VisitorClass::AgenticHuman => page.human_hits += 1,
            VisitorClass::SearchBot => page.search_bot_hits += 1,
            VisitorClass::SocialBot => page.social_bot_hits += 1,
            VisitorClass::AiBot => page.ai_bot_hits += 1,
            VisitorClass::ScraperBot | VisitorClass::StealthFleet => {
                page.scraper_bot_hits += 1;
            }
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

        // Collect raw hit for session grouping.
        self.raw_hits.push(RawHit {
            host: entry.request.host.clone(),
            path: normalized,
            ua: ua.to_string(),
            class,
            ts: entry.ts,
        });
    }

    /// Finalize crawl coverage counts and build session summaries.
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

        // Build session summaries from raw hits.
        self.sessions = build_session_summaries(&self.raw_hits);
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

            // Session summary for this host.
            if let Some(sess) = self.sessions.get(host) {
                lines.push(format!(
                    "  sessions: {} total, {} human (avg depth {:.1}, {} bounces, {} deep readers)",
                    sess.total_sessions,
                    sess.human_sessions,
                    sess.avg_depth,
                    sess.bounces,
                    sess.deep_readers,
                ));
            }

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
    #[allow(dead_code)] // public API, wired when IndexNow priority scoring lands
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

/// SHA-256 hash of a string, returned as hex. Used for UA hashing.
fn sha256_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(s.as_bytes());
    hex::encode(digest)
}

/// Group raw hits into sessions and compute per-host summaries.
///
/// A session is: same User-Agent string, hits within [`SESSION_WINDOW_SECS`],
/// ordered by timestamp. Different hosts = separate sessions.
fn build_session_summaries(raw_hits: &[RawHit]) -> BTreeMap<String, SessionSummary> {
    if raw_hits.is_empty() {
        return BTreeMap::new();
    }

    // Group by (host, UA) and sort by timestamp.
    let mut grouped: BTreeMap<(String, String), Vec<&RawHit>> = BTreeMap::new();
    for hit in raw_hits {
        grouped
            .entry((hit.host.clone(), hit.ua.clone()))
            .or_default()
            .push(hit);
    }

    // Build sessions from each group.
    let mut all_sessions: Vec<Session> = Vec::new();
    for ((host, _ua), hits) in &grouped {
        let mut sorted: Vec<&&RawHit> = hits.iter().collect();
        sorted.sort_by(|a, b| a.ts.partial_cmp(&b.ts).unwrap_or(std::cmp::Ordering::Equal));

        // Split into sessions at gaps > SESSION_WINDOW_SECS.
        let mut session_start = 0;
        for i in 1..sorted.len() {
            if sorted[i].ts - sorted[i - 1].ts > SESSION_WINDOW_SECS {
                all_sessions.push(build_session(host, &sorted[session_start..i]));
                session_start = i;
            }
        }
        all_sessions.push(build_session(host, &sorted[session_start..]));
    }

    // Aggregate into per-host summaries.
    let mut summaries: BTreeMap<String, SessionSummary> = BTreeMap::new();
    for session in &all_sessions {
        let summary = summaries.entry(session.host.clone()).or_default();
        summary.total_sessions += 1;
        if matches!(
            session.classification,
            VisitorClass::Human | VisitorClass::AgenticHuman
        ) {
            summary.human_sessions += 1;
            let depth = session.pages.len() as f64;
            let n = summary.human_sessions as f64;
            summary.avg_depth = summary.avg_depth * ((n - 1.0) / n) + depth / n;
            if session.pages.len() == 1 {
                summary.bounces += 1;
            }
            if session.pages.len() >= 3 {
                summary.deep_readers += 1;
            }
        }
    }

    summaries
}

/// Build a single session from a sorted slice of hits, applying behavioral reclassification.
fn build_session(host: &str, hits: &[&&RawHit]) -> Session {
    let first = hits[0];
    let last = hits[hits.len() - 1];
    let pages: Vec<String> = hits.iter().map(|h| h.path.clone()).collect();
    let duration_s = if last.ts > first.ts {
        (last.ts - first.ts) as u64
    } else {
        0
    };

    // Start with the UA-based classification from the first hit.
    let mut classification = first.class;

    // Apply behavioral reclassification for "Human" sessions.
    if classification == VisitorClass::Human || classification == VisitorClass::Unknown {
        classification = reclassify_session(hits, classification);
    }

    Session {
        ua_hash: sha256_hex(&first.ua),
        classification,
        pages: pages.clone(),
        entry_page: pages.first().cloned().unwrap_or_default(),
        duration_s,
        host: host.to_string(),
        start_ts: first.ts,
    }
}

/// Apply behavioral signals to reclassify a session.
///
/// Catches bots that use clean browser UAs but exhibit non-human behavior:
/// - Velocity: >5 hits in <10 seconds → ScraperBot
/// - Probe paths: any hit to a known probe path → ScraperBot
/// - All 404s: session of only 404 responses → ScraperBot (path scanning)
fn reclassify_session(hits: &[&&RawHit], initial_class: VisitorClass) -> VisitorClass {
    // Check 1: Any probe path in the session → entire session is ScraperBot.
    // (Individual hits already classified by classify_request, but a session
    // mixing normal + probe paths should be flagged too.)
    let has_probe = hits.iter().any(|h| h.class == VisitorClass::ScraperBot);
    if has_probe {
        return VisitorClass::ScraperBot;
    }

    // Check 2: Velocity — too many hits too fast.
    if hits.len() > VELOCITY_THRESHOLD {
        let first_ts = hits[0].ts;
        let window_end_ts = first_ts + VELOCITY_WINDOW_SECS;
        let hits_in_window = hits.iter().filter(|h| h.ts <= window_end_ts).count();
        if hits_in_window > VELOCITY_THRESHOLD {
            return VisitorClass::ScraperBot;
        }
    }

    initial_class
}

// ── Log Retrieval + Parsing ─────────────────────────────────────────

/// Default number of Caddy log lines to analyze.
const DEFAULT_LOG_LINES: u32 = 5000;

/// Caddy access log path on golgiBody.
const CADDY_LOG_PATH: &str = "/var/log/caddy/access.log";

/// Rotated log file (uncompressed, previous rotation).
const CADDY_LOG_ROTATED: &str = "/var/log/caddy/access.log.1";

/// Compressed rotated log file (rotation before previous).
const CADDY_LOG_ROTATED_GZ: &str = "/var/log/caddy/access.log.2.gz";

/// Build an SSH command that reads across log rotation boundaries.
///
/// Combines current + rotated logs (newest last) so `tail -n N` gets
/// the most recent N lines regardless of when rotation happened.
///
/// Without `include_rotated`, reads only the current log file.
fn build_log_command(lines: u32, include_rotated: bool) -> String {
    if include_rotated {
        // Cat rotated files (oldest first) + current, then tail.
        // .2.gz needs zcat; .1 and current are plain text.
        // Use subshell with conditional existence checks.
        format!(
            "{{ [ -f {CADDY_LOG_ROTATED_GZ} ] && zcat {CADDY_LOG_ROTATED_GZ} 2>/dev/null; \
             [ -f {CADDY_LOG_ROTATED} ] && cat {CADDY_LOG_ROTATED} 2>/dev/null; \
             cat {CADDY_LOG_PATH} 2>/dev/null; }} | tail -n {lines}"
        )
    } else {
        format!("tail -n {lines} {CADDY_LOG_PATH} 2>/dev/null")
    }
}

/// Retrieve and analyze Caddy access logs from golgiBody via SSH.
///
/// Fetches the last `lines` entries from the Caddy JSON access log,
/// parses each into a [`CaddyLogEntry`], classifies visitors, and
/// aggregates into a [`ReceptorReport`].
///
/// When `include_rotated` is true, reads across rotation boundaries
/// (`access.log.1` and `access.log.2.gz`) to prevent data loss during
/// Caddy reloads.
pub async fn analyze(config: &crate::ShadowConfig, lines: u32) -> Result<ReceptorReport> {
    analyze_opts(config, lines, false).await
}

/// Analyze with full options including rotated log support.
pub async fn analyze_opts(
    config: &crate::ShadowConfig,
    lines: u32,
    include_rotated: bool,
) -> Result<ReceptorReport> {
    let cmd = build_log_command(lines, include_rotated);
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
    include_rotated: bool,
) -> Result<ReceptorReport> {
    let source = if include_rotated {
        format!(
            "{{ [ -f {CADDY_LOG_ROTATED_GZ} ] && zcat {CADDY_LOG_ROTATED_GZ} 2>/dev/null; \
             [ -f {CADDY_LOG_ROTATED} ] && cat {CADDY_LOG_ROTATED} 2>/dev/null; \
             cat {CADDY_LOG_PATH} 2>/dev/null; }}"
        )
    } else {
        format!("cat {CADDY_LOG_PATH} 2>/dev/null")
    };
    let cmd = format!("{source} | grep '\"host\":\"{host}\"' | tail -n {lines}");
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
    let include_rotated = args.contains(&"--include-rotated");

    if with_feedback {
        let report = analyze_opts(config, lines, include_rotated).await?;
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
        let report = analyze_host(config, host, lines, include_rotated).await?;
        Ok(crate::ShadowOutcome::ok_with(
            report.summary(),
            serde_json::to_value(&report)?,
        ))
    } else {
        let report = analyze_opts(config, lines, include_rotated).await?;
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
        assert_eq!(VisitorClass::ScraperBot.to_string(), "scraper_bot");
        assert_eq!(VisitorClass::AgenticHuman.to_string(), "agentic_human");
    }

    #[test]
    fn classify_path_dotenv() {
        assert_eq!(classify_path("/.env"), Some(VisitorClass::ScraperBot));
        assert_eq!(
            classify_path("/.env.backup"),
            Some(VisitorClass::ScraperBot)
        );
        assert_eq!(classify_path("/.env.prod"), Some(VisitorClass::ScraperBot));
    }

    #[test]
    fn classify_path_aws_credentials() {
        assert_eq!(
            classify_path("/.aws/credentials"),
            Some(VisitorClass::ScraperBot)
        );
    }

    #[test]
    fn classify_path_git() {
        assert_eq!(
            classify_path("/.git/config"),
            Some(VisitorClass::ScraperBot)
        );
    }

    #[test]
    fn classify_path_wordpress() {
        assert_eq!(classify_path("/wp-admin"), Some(VisitorClass::ScraperBot));
        assert_eq!(
            classify_path("/wp-login.php"),
            Some(VisitorClass::ScraperBot)
        );
        assert_eq!(classify_path("/xmlrpc.php"), Some(VisitorClass::ScraperBot));
    }

    #[test]
    fn classify_path_php_on_static_site() {
        assert_eq!(classify_path("/index.php"), Some(VisitorClass::ScraperBot));
        assert_eq!(classify_path("/admin.php"), Some(VisitorClass::ScraperBot));
    }

    #[test]
    fn classify_path_normal_content() {
        assert_eq!(classify_path("/"), None);
        assert_eq!(classify_path("/evidence/"), None);
        assert_eq!(classify_path("/network/dpsc/"), None);
        assert_eq!(classify_path("/analysis/rico-pattern/"), None);
    }

    #[test]
    fn classify_request_probe_overrides_ua() {
        // Clean Chrome UA but requesting /.env → ScraperBot, not Human
        assert_eq!(
            classify_request(
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/120.0.0.0 Safari/537.36",
                "/.env"
            ),
            VisitorClass::ScraperBot
        );
    }

    #[test]
    fn classify_request_normal_path_uses_ua() {
        assert_eq!(
            classify_request(
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/120.0.0.0 Safari/537.36",
                "/evidence/"
            ),
            VisitorClass::Human
        );
    }

    #[test]
    fn scraper_session_not_counted_as_human() {
        let mut report = ReceptorReport::empty();

        let ua = "Mozilla/5.0 Chrome/152.0.0.0 Safari/537.36";

        // Scanner hitting 28 dotfiles in 30 seconds
        let probes = [
            "/.env",
            "/.env.backup",
            "/.env.prod",
            "/.env.local",
            "/.aws/credentials",
            "/.git/config",
            "/wp-admin",
            "/xmlrpc.php",
        ];
        for (i, path) in probes.iter().enumerate() {
            report.ingest(&CaddyLogEntry {
                request: CaddyRequest {
                    remote_ip: "5.6.7.8".into(),
                    host: "detroit.primals.eco".into(),
                    uri: path.to_string(),
                    method: "GET".into(),
                    headers: CaddyHeaders {
                        user_agent: vec![ua.into()],
                    },
                },
                status: 404,
                size: 0,
                duration: 0.001,
                ts: 1727800000.0 + (i as f64 * 1.0), // 1 second apart
            });
        }

        report.finalize();

        let host = report.hosts.get("detroit.primals.eco").unwrap();
        // Zero human hits — all probe paths classified as ScraperBot
        assert_eq!(host.total_human, 0, "probe hits should not count as human");
        assert_eq!(host.total_requests, 8);

        // Session should also be ScraperBot, not Human
        let sess = report.sessions.get("detroit.primals.eco").unwrap();
        assert_eq!(
            sess.human_sessions, 0,
            "scraper session should not be counted as human"
        );
    }

    #[test]
    fn velocity_reclassifies_fast_session() {
        let mut report = ReceptorReport::empty();

        let ua = "Mozilla/5.0 Chrome/120.0.0.0 Safari/537.36";

        // 8 legitimate-looking pages in 5 seconds — too fast for a human
        let pages = [
            "/",
            "/evidence/",
            "/network/",
            "/analysis/",
            "/timeline/",
            "/sources/",
            "/about/",
            "/contact/",
        ];
        for (i, path) in pages.iter().enumerate() {
            report.ingest(&CaddyLogEntry {
                request: CaddyRequest {
                    remote_ip: "1.2.3.4".into(),
                    host: "detroit.primals.eco".into(),
                    uri: path.to_string(),
                    method: "GET".into(),
                    headers: CaddyHeaders {
                        user_agent: vec![ua.into()],
                    },
                },
                status: 200,
                size: 1000,
                duration: 0.01,
                ts: 1727800000.0 + (i as f64 * 0.5), // 0.5 seconds apart
            });
        }

        report.finalize();

        let sess = report.sessions.get("detroit.primals.eco").unwrap();
        assert_eq!(
            sess.human_sessions, 0,
            "velocity-flagged session should not be human"
        );
        assert_eq!(sess.total_sessions, 1);
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

    #[test]
    fn build_log_command_single() {
        let cmd = build_log_command(5000, false);
        assert!(cmd.contains("tail -n 5000"));
        assert!(!cmd.contains("access.log.1"));
    }

    #[test]
    fn build_log_command_rotated() {
        let cmd = build_log_command(50000, true);
        assert!(cmd.contains("access.log.1"));
        assert!(cmd.contains("access.log.2.gz"));
        assert!(cmd.contains("zcat"));
        assert!(cmd.contains("tail -n 50000"));
    }

    #[test]
    fn session_detection_groups_by_ua() {
        let mut report = ReceptorReport::empty();

        let ua = "Mozilla/5.0 Chrome/120.0.0.0 Safari/537.36";

        // Three hits from same UA within 30 minutes = one session
        for (i, path) in ["/", "/evidence/", "/network/"].iter().enumerate() {
            report.ingest(&CaddyLogEntry {
                request: CaddyRequest {
                    remote_ip: "1.2.3.4".into(),
                    host: "detroit.primals.eco".into(),
                    uri: path.to_string(),
                    method: "GET".into(),
                    headers: CaddyHeaders {
                        user_agent: vec![ua.into()],
                    },
                },
                status: 200,
                size: 1000,
                duration: 0.01,
                ts: 1727800000.0 + (i as f64 * 300.0), // 5 min apart
            });
        }

        report.finalize();

        let sess = report.sessions.get("detroit.primals.eco").unwrap();
        assert_eq!(sess.total_sessions, 1);
        assert_eq!(sess.human_sessions, 1);
        assert_eq!(sess.deep_readers, 1); // 3 pages = deep reader
        assert_eq!(sess.bounces, 0);
        assert!((sess.avg_depth - 3.0).abs() < 0.01);
    }

    #[test]
    fn session_detection_splits_at_gap() {
        let mut report = ReceptorReport::empty();

        let ua = "Mozilla/5.0 Chrome/120.0.0.0 Safari/537.36";

        // Hit 1: time T
        report.ingest(&CaddyLogEntry {
            request: CaddyRequest {
                remote_ip: "1.2.3.4".into(),
                host: "detroit.primals.eco".into(),
                uri: "/".into(),
                method: "GET".into(),
                headers: CaddyHeaders {
                    user_agent: vec![ua.into()],
                },
            },
            status: 200,
            size: 1000,
            duration: 0.01,
            ts: 1727800000.0,
        });

        // Hit 2: T + 2 hours (beyond 30min window)
        report.ingest(&CaddyLogEntry {
            request: CaddyRequest {
                remote_ip: "1.2.3.4".into(),
                host: "detroit.primals.eco".into(),
                uri: "/evidence/".into(),
                method: "GET".into(),
                headers: CaddyHeaders {
                    user_agent: vec![ua.into()],
                },
            },
            status: 200,
            size: 1000,
            duration: 0.01,
            ts: 1727800000.0 + 7200.0, // 2 hours later
        });

        report.finalize();

        let sess = report.sessions.get("detroit.primals.eco").unwrap();
        assert_eq!(sess.total_sessions, 2); // Two separate sessions
        assert_eq!(sess.human_sessions, 2);
        assert_eq!(sess.bounces, 2); // Both are single-page
    }

    #[test]
    fn session_summary_in_report_summary() {
        let mut report = ReceptorReport::empty();

        report.ingest(&CaddyLogEntry {
            request: CaddyRequest {
                remote_ip: "1.2.3.4".into(),
                host: "detroit.primals.eco".into(),
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
        });

        report.finalize();

        let summary = report.summary();
        assert!(summary.contains("sessions:"));
        assert!(summary.contains("1 total"));
        assert!(summary.contains("1 human"));
    }
}
