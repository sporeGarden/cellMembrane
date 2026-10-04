// SPDX-License-Identifier: AGPL-3.0-or-later

//! Dual signal log for skunkBat anomaly detection.
//!
//! Records both LuxI (outbound publish) and LuxR (inbound receptor) events
//! to `/var/log/membrane/signal.jsonl` — a single JSONL file that skunkBat's
//! `baseline.observe` can tail for trend analysis.
//!
//! ## Signal Types
//!
//! | Type | Source | Trigger |
//! |------|--------|---------|
//! | `publish` | webhook pipeline | successful site.publish |
//! | `receptor` | observatory snapshot | receptor analysis during publish |
//!
//! ## Anomaly Signals (for skunkBat)
//!
//! - Sudden drop in `total_search_bot` → crawlers stopped coming
//! - Spike in `total_human` after publish → Reddit repeater effect
//! - `indexnow_ok` = false → IndexNow key invalidated
//! - `crawl_coverage` regression → pages dropping out of bot reach
//! - `url_notify_quota_exhausted` persistent → need to optimize tiers

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tracing::{info, warn};

/// Signal log path on golgiBody.
const SIGNAL_LOG_PATH: &str = "/var/log/membrane/signal.jsonl";

// ── Signal Event Types ──────────────────────────────────────────────

/// A single event in the signal log (one JSON line).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SignalEvent {
    /// LuxI: outbound publish completed.
    Publish(PublishSignal),
    /// LuxR: inbound receptor snapshot taken.
    Receptor(ReceptorSignal),
    /// LuxI: manual outreach emission (email, press, social).
    Outreach(OutreachSignal),
    /// Correlation: outreach → arrival propagation measurement.
    Correlation(CorrelationSignal),
}

/// LuxI publish event — records what was secreted outward.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishSignal {
    /// Unix epoch seconds.
    pub ts: u64,
    /// Site repo name (e.g. "detroit", "sporeprint").
    pub site: String,
    /// Number of HTML pages in the built site.
    pub pages: usize,
    /// Number of URLs submitted to IndexNow.
    pub indexnow_urls: usize,
    /// Whether IndexNow submission succeeded.
    pub indexnow_ok: bool,
    /// Whether GSC sitemap was resubmitted.
    pub gsc_submitted: bool,
    /// Number of URLs sent to Google URL Notification API.
    pub url_notify_count: usize,
    /// Whether the URL notification hit the daily quota limit.
    pub url_notify_quota_exhausted: bool,
    /// Whether an observatory snapshot was written.
    pub observatory_snapshot: bool,
    /// Number of Caddy log lines the observatory analyzed (0 if skipped).
    pub observatory_lines: u64,
}

/// LuxR receptor event — records what was sensed inward.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceptorSignal {
    /// Unix epoch seconds.
    pub ts: u64,
    /// Date string (YYYY-MM-DD).
    pub date: String,
    /// Per-host summary digests.
    pub hosts: BTreeMap<String, HostDigest>,
}

/// Compact per-host summary for the signal log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostDigest {
    /// Total requests seen.
    pub total_requests: u64,
    /// Total human visits.
    pub total_human: u64,
    /// Total search bot crawls.
    pub total_search_bot: u64,
    /// Unique URI paths seen.
    pub unique_paths: u64,
    /// Paths crawled by search bots.
    pub crawl_coverage: u64,
}

/// LuxI outreach event — manual signal emission (email, press, social).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutreachSignal {
    /// Unix epoch seconds when the outreach was sent.
    pub ts: u64,
    /// Channel: "email", "reddit", "press", "social", etc.
    pub channel: String,
    /// Number of recipients (email count, subreddit subscribers, etc.).
    pub recipients: u32,
    /// Pages linked in the outreach (URI paths).
    pub pages_linked: Vec<String>,
    /// Free-text note describing the outreach.
    pub note: String,
}

/// Correlation event — outreach → arrival propagation measurement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorrelationSignal {
    /// Unix epoch seconds when this correlation was computed.
    pub ts: u64,
    /// Timestamp of the outreach event this correlates to.
    pub outreach_ts: u64,
    /// Seconds between outreach and first human arrival on any linked page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub propagation_delay_s: Option<u64>,
    /// Seconds between outreach and first search bot arrival on any linked page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bot_propagation_delay_s: Option<u64>,
    /// Number of linked pages that received human visits within the window.
    pub pages_activated: u32,
    /// Total pages that were linked in the outreach.
    pub pages_linked_total: u32,
    /// Number of detected human sessions on linked pages.
    pub sessions: u32,
    /// Average navigation depth of human sessions on linked pages.
    pub depth_avg: f64,
    /// Activation rate: pages_activated / pages_linked_total.
    pub activation_rate: f64,
}

// ── Append ──────────────────────────────────────────────────────────

/// Append a signal event to the JSONL log file.
///
/// Non-fatal by design — if the write fails (permissions, missing dir),
/// it logs a warning but never returns an error. The publish/receptor
/// pipeline must not fail because of signal logging.
pub fn append_signal(event: &SignalEvent) {
    let line = match serde_json::to_string(event) {
        Ok(json) => json,
        Err(e) => {
            warn!(error = %e, "signal: failed to serialize event");
            return;
        }
    };

    append_line(&line);
}

/// Low-level JSONL line append with directory creation.
fn append_line(line: &str) {
    use std::io::Write;

    let path = std::path::Path::new(SIGNAL_LOG_PATH);

    // Ensure directory exists.
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                warn!(error = %e, path = %parent.display(), "signal: cannot create log dir");
                return;
            }
        }
    }

    // Open for append (create if missing).
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path);

    match file {
        Ok(mut f) => {
            if let Err(e) = writeln!(f, "{line}") {
                warn!(error = %e, "signal: failed to write to signal log");
            }
        }
        Err(e) => {
            warn!(error = %e, path = %path.display(), "signal: cannot open signal log");
        }
    }
}

// ── Builder Helpers ─────────────────────────────────────────────────

/// Current Unix epoch timestamp.
pub fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Build a publish signal with defaults (fields filled in by pipeline).
pub fn publish_signal(site: &str, pages: usize) -> PublishSignal {
    PublishSignal {
        ts: now_epoch(),
        site: site.to_string(),
        pages,
        indexnow_urls: 0,
        indexnow_ok: false,
        gsc_submitted: false,
        url_notify_count: 0,
        url_notify_quota_exhausted: false,
        observatory_snapshot: false,
        observatory_lines: 0,
    }
}

/// Build a receptor signal from a receptor report.
pub fn receptor_signal(report: &crate::seo::receptor::ReceptorReport) -> ReceptorSignal {
    let hosts = report
        .hosts
        .iter()
        .map(|(name, signal)| {
            (
                name.clone(),
                HostDigest {
                    total_requests: signal.total_requests,
                    total_human: signal.total_human,
                    total_search_bot: signal.total_search_bot,
                    unique_paths: signal.unique_paths,
                    crawl_coverage: signal.crawl_coverage,
                },
            )
        })
        .collect();

    ReceptorSignal {
        ts: now_epoch(),
        date: crate::utc_today(),
        hosts,
    }
}

// ── SEO Message Parsing ─────────────────────────────────────────────

/// Extract signal metadata from the SEO notification result message.
///
/// The `notify_crawlers()` function returns a multi-line string like:
/// ```text
///   [seo] indexnow: submitted 232 URLs to bing (200)
///   [seo] gsc: sitemap resubmitted
///   [seo] url_notify: notified 50/233 URLs (batch 50/233)
/// ```
///
/// We parse this best-effort — if format changes, fields stay at defaults.
pub fn parse_seo_msg(signal: &mut PublishSignal, seo_msg: &str) {
    for line in seo_msg.lines() {
        let line = line.trim();

        if line.contains("[seo] indexnow:") {
            signal.indexnow_ok = !line.contains("FAILED");
            // Try to extract URL count: "submitted N URLs"
            if let Some(after) = line.strip_suffix(')') {
                if let Some(idx) = after.rfind('(') {
                    if let Ok(status) = after[idx + 1..].trim().parse::<u16>() {
                        signal.indexnow_ok = status == 200;
                    }
                }
            }
            // Extract count: "submitted N URLs" or "N URLs"
            for word_pair in line.split_whitespace().collect::<Vec<_>>().windows(2) {
                if word_pair[1].starts_with("URL") {
                    if let Ok(n) = word_pair[0].parse::<usize>() {
                        signal.indexnow_urls = n;
                    }
                }
            }
        }

        if line.contains("[seo] gsc:") {
            signal.gsc_submitted = !line.contains("FAILED");
        }

        if line.contains("[seo] url_notify:") {
            signal.url_notify_quota_exhausted = line.contains("remaining");
            // Extract count: "notified N/" or "batch N/"
            for word_pair in line.split_whitespace().collect::<Vec<_>>().windows(2) {
                if word_pair[0] == "batch" || word_pair[0] == "notified" {
                    if let Some(slash_idx) = word_pair[1].find('/') {
                        if let Ok(n) = word_pair[1][..slash_idx].parse::<usize>() {
                            signal.url_notify_count = n;
                        }
                    }
                }
            }
        }
    }
}

/// Extract observatory metadata from the observatory step message.
///
/// Format: `[observatory] observatory: N lines, M hosts, D history days → path`
pub fn parse_observatory_msg(signal: &mut PublishSignal, observatory_msg: &str) {
    if observatory_msg.contains("[observatory]") && !observatory_msg.contains("FAILED") {
        signal.observatory_snapshot = true;
        // Extract line count: "N lines"
        for word_pair in observatory_msg
            .split_whitespace()
            .collect::<Vec<_>>()
            .windows(2)
        {
            if word_pair[1] == "lines," || word_pair[1] == "lines" {
                if let Ok(n) = word_pair[0].parse::<u64>() {
                    signal.observatory_lines = n;
                }
            }
        }
    }
}

// ── Outreach Builder ────────────────────────────────────────────────

/// Build and emit an outreach signal from CLI arguments.
pub fn emit_outreach(
    channel: &str,
    recipients: u32,
    pages: &[String],
    note: &str,
) -> OutreachSignal {
    let sig = OutreachSignal {
        ts: now_epoch(),
        channel: channel.to_string(),
        recipients,
        pages_linked: pages.to_vec(),
        note: note.to_string(),
    };
    append_signal(&SignalEvent::Outreach(sig.clone()));
    info!(
        channel = %channel,
        recipients,
        pages = pages.len(),
        "signal: outreach event emitted"
    );
    sig
}

// ── Signal Log Reading ──────────────────────────────────────────────

/// Load all signal events from the JSONL log file.
pub fn load_signals() -> Vec<SignalEvent> {
    let path = std::path::Path::new(SIGNAL_LOG_PATH);
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<SignalEvent>(line).ok())
        .collect()
}

/// Find the most recent outreach event.
pub fn latest_outreach() -> Option<OutreachSignal> {
    load_signals().into_iter().rev().find_map(|e| match e {
        SignalEvent::Outreach(o) => Some(o),
        _ => None,
    })
}

/// Find outreach events within a time window (epoch seconds).
pub fn outreach_in_window(after_ts: u64, before_ts: u64) -> Vec<OutreachSignal> {
    load_signals()
        .into_iter()
        .filter_map(|e| match e {
            SignalEvent::Outreach(o) if o.ts >= after_ts && o.ts <= before_ts => Some(o),
            _ => None,
        })
        .collect()
}

// ── Propagation Delay ───────────────────────────────────────────────

/// Compute propagation delay for an outreach event against a receptor report.
///
/// Scans the report's page signals for the linked pages to find:
/// - First human arrival timestamp after the outreach
/// - First search bot arrival timestamp after the outreach
/// - Number of linked pages that received human visits
/// - Session depth on linked pages
pub fn compute_correlation(
    outreach: &OutreachSignal,
    report: &crate::seo::receptor::ReceptorReport,
) -> CorrelationSignal {
    let pages_linked_total = u32::try_from(outreach.pages_linked.len()).unwrap_or(u32::MAX);
    let mut pages_activated: u32 = 0;
    let mut total_sessions: u32 = 0;
    let mut total_depth: f64 = 0.0;

    // Check each linked page across all hosts.
    for page_path in &outreach.pages_linked {
        let mut page_got_human = false;
        for (_host, signal) in &report.hosts {
            if let Some(page) = signal.pages.get(page_path.as_str()) {
                if page.human_hits > 0 {
                    page_got_human = true;
                }
            }
        }
        if page_got_human {
            pages_activated += 1;
        }
    }

    // Count sessions on linked pages from session summaries.
    for (_host, summary) in &report.sessions {
        total_sessions += u32::try_from(summary.human_sessions).unwrap_or(u32::MAX);
        total_depth += summary.avg_depth * summary.human_sessions as f64;
    }

    let depth_avg = if total_sessions > 0 {
        total_depth / total_sessions as f64
    } else {
        0.0
    };

    let activation_rate = if pages_linked_total > 0 {
        pages_activated as f64 / pages_linked_total as f64
    } else {
        0.0
    };

    // Propagation delay: outreach.ts → window_start of first human hit.
    // This is approximate — we use the report's window_start as a proxy.
    // A more precise measurement would require per-hit timestamps on linked pages.
    let propagation_delay_s = if pages_activated > 0 && report.window_start > outreach.ts as f64 {
        Some((report.window_start - outreach.ts as f64) as u64)
    } else {
        None
    };

    CorrelationSignal {
        ts: now_epoch(),
        outreach_ts: outreach.ts,
        propagation_delay_s,
        bot_propagation_delay_s: None, // Requires per-page bot timestamps (future)
        pages_activated,
        pages_linked_total,
        sessions: total_sessions,
        depth_avg,
        activation_rate,
    }
}

// ── Dispatch ────────────────────────────────────────────────────────

/// Dispatch `signal.*` CLI commands.
pub async fn dispatch(
    config: &crate::ShadowConfig,
    cmd: &str,
    args: &[&str],
) -> crate::error::Result<crate::ShadowOutcome> {
    match cmd {
        "signal.emit" => dispatch_emit(args),
        "signal.correlate" => dispatch_correlate(config, args).await,
        "signal.log" => dispatch_log(args),
        _ => Ok(crate::ShadowOutcome::fail(format!(
            "unknown signal command: {cmd}"
        ))),
    }
}

/// `membrane signal.emit --channel email --recipients 15 --pages /p1 /p2 --note "..."`
fn dispatch_emit(args: &[&str]) -> crate::error::Result<crate::ShadowOutcome> {
    let channel = crate::cli::extract_flag_value(args, "--channel").unwrap_or("unknown");

    let recipients: u32 = crate::cli::extract_flag_value(args, "--recipients")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    let note = crate::cli::extract_flag_value(args, "--note").unwrap_or("");

    // Collect pages: everything after --pages until next flag or end.
    let pages = extract_multi_value(args, "--pages");

    if pages.is_empty() && channel == "unknown" && recipients == 0 {
        return Ok(crate::ShadowOutcome::fail(
            "signal.emit requires --channel and --pages".to_string(),
        ));
    }

    let sig = emit_outreach(&channel, recipients, &pages, &note);

    Ok(crate::ShadowOutcome::ok_with(
        format!(
            "outreach emitted: {} via {} to {} recipients, {} pages linked",
            note,
            sig.channel,
            sig.recipients,
            sig.pages_linked.len()
        ),
        serde_json::to_value(&sig).unwrap_or_default(),
    ))
}

/// `membrane signal.correlate [--outreach-ts EPOCH]`
///
/// Runs a receptor snapshot and correlates against the most recent outreach
/// (or a specific one by timestamp).
async fn dispatch_correlate(
    config: &crate::ShadowConfig,
    args: &[&str],
) -> crate::error::Result<crate::ShadowOutcome> {
    let outreach = if let Some(ts_str) = crate::cli::extract_flag_value(args, "--outreach-ts") {
        let ts: u64 = ts_str.parse().map_err(|_| {
            crate::error::ShadowError::config("--outreach-ts must be a Unix epoch timestamp")
        })?;
        outreach_in_window(ts, ts)
            .into_iter()
            .next()
            .ok_or_else(|| {
                crate::error::ShadowError::config(format!(
                    "no outreach event found at timestamp {ts}"
                ))
            })?
    } else {
        latest_outreach().ok_or_else(|| {
            crate::error::ShadowError::config(
                "no outreach events in signal log — run signal.emit first",
            )
        })?
    };

    // Run receptor with rotated logs to span the full window since outreach.
    let report = crate::seo::receptor::analyze_opts(config, 100_000, true).await?;

    let correlation = compute_correlation(&outreach, &report);
    append_signal(&SignalEvent::Correlation(correlation.clone()));

    let mut msg = format!(
        "=== Signal Correlation ===\n\
         outreach: {} via {} at {}\n\
         linked pages: {}\n\
         activated: {}/{} ({:.0}%)\n\
         sessions: {} (avg depth {:.1})",
        outreach.note,
        outreach.channel,
        outreach.ts,
        outreach.pages_linked.join(", "),
        correlation.pages_activated,
        correlation.pages_linked_total,
        correlation.activation_rate * 100.0,
        correlation.sessions,
        correlation.depth_avg,
    );

    if let Some(delay) = correlation.propagation_delay_s {
        let hours = delay / 3600;
        let mins = (delay % 3600) / 60;
        msg.push_str(&format!("\npropagation delay: {hours}h {mins}m"));
    } else {
        msg.push_str("\npropagation delay: not yet measurable (no arrivals in window)");
    }

    Ok(crate::ShadowOutcome::ok_with(
        msg,
        serde_json::to_value(&correlation).unwrap_or_default(),
    ))
}

/// `membrane signal.log` — dump recent signal log entries.
fn dispatch_log(args: &[&str]) -> crate::error::Result<crate::ShadowOutcome> {
    let limit: usize = crate::cli::extract_flag_value(args, "--limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);

    let signals = load_signals();
    let recent: Vec<&SignalEvent> = signals.iter().rev().take(limit).collect();

    if recent.is_empty() {
        return Ok(crate::ShadowOutcome::ok("signal log is empty".to_string()));
    }

    let mut lines = vec![format!("=== Signal Log (last {}) ===", recent.len())];
    for event in recent.iter().rev() {
        match event {
            SignalEvent::Publish(p) => {
                lines.push(format!(
                    "  [publish] ts={} site={} pages={} indexnow={}",
                    p.ts, p.site, p.pages, p.indexnow_ok
                ));
            }
            SignalEvent::Receptor(r) => {
                let host_summary: String = r
                    .hosts
                    .iter()
                    .map(|(h, d)| format!("{h}:{}", d.total_requests))
                    .collect::<Vec<_>>()
                    .join(" ");
                lines.push(format!(
                    "  [receptor] ts={} date={} {host_summary}",
                    r.ts, r.date
                ));
            }
            SignalEvent::Outreach(o) => {
                lines.push(format!(
                    "  [outreach] ts={} channel={} recipients={} pages={} note=\"{}\"",
                    o.ts,
                    o.channel,
                    o.recipients,
                    o.pages_linked.len(),
                    o.note
                ));
            }
            SignalEvent::Correlation(c) => {
                lines.push(format!(
                    "  [correlation] ts={} outreach_ts={} activated={}/{} sessions={} delay={:?}",
                    c.ts,
                    c.outreach_ts,
                    c.pages_activated,
                    c.pages_linked_total,
                    c.sessions,
                    c.propagation_delay_s
                ));
            }
        }
    }

    Ok(crate::ShadowOutcome::ok(lines.join("\n")))
}

/// Extract multiple values after a flag (until next `--` flag or end of args).
fn extract_multi_value(args: &[&str], flag: &str) -> Vec<String> {
    let Some(pos) = args.iter().position(|a| *a == flag) else {
        return Vec::new();
    };
    args[pos + 1..]
        .iter()
        .take_while(|a| !a.starts_with("--"))
        .map(|a| a.to_string())
        .collect()
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_signal_serializes_with_type_tag() {
        let sig = publish_signal("detroit", 233);
        let event = SignalEvent::Publish(sig);
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"publish""#));
        assert!(json.contains(r#""site":"detroit""#));
        assert!(json.contains(r#""pages":233"#));
    }

    #[test]
    fn receptor_signal_serializes_with_type_tag() {
        let sig = ReceptorSignal {
            ts: 1790980000,
            date: "2026-10-03".into(),
            hosts: BTreeMap::new(),
        };
        let event = SignalEvent::Receptor(sig);
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"receptor""#));
        assert!(json.contains(r#""date":"2026-10-03""#));
    }

    #[test]
    fn parse_seo_msg_extracts_indexnow() {
        let mut sig = publish_signal("detroit", 100);
        let msg = "  [seo] indexnow: submitted 232 URLs to bing (200)\n  [seo] gsc: sitemap resubmitted\n  [seo] url_notify: notified 50/233 URLs (batch 50/233)";
        parse_seo_msg(&mut sig, msg);

        assert!(sig.indexnow_ok);
        assert_eq!(sig.indexnow_urls, 232);
        assert!(sig.gsc_submitted);
        assert_eq!(sig.url_notify_count, 50);
        assert!(!sig.url_notify_quota_exhausted);
    }

    #[test]
    fn parse_seo_msg_handles_failures() {
        let mut sig = publish_signal("detroit", 100);
        let msg = "  [seo] indexnow: FAILED — connection refused\n  [seo] gsc: FAILED — no credentials\n  [seo] url_notify: FAILED — timeout";
        parse_seo_msg(&mut sig, msg);

        assert!(!sig.indexnow_ok);
        assert!(!sig.gsc_submitted);
        assert_eq!(sig.url_notify_count, 0);
    }

    #[test]
    fn parse_seo_msg_detects_quota_exhaustion() {
        let mut sig = publish_signal("detroit", 100);
        let msg = "  [seo] url_notify: notified 50/233 URLs (batch 50/233 — 183 remaining, use --tier or increase --limit)";
        parse_seo_msg(&mut sig, msg);

        assert!(sig.url_notify_quota_exhausted);
        assert_eq!(sig.url_notify_count, 50);
    }

    #[test]
    fn parse_observatory_msg_extracts_lines() {
        let mut sig = publish_signal("sporeprint", 100);
        let msg = "  [observatory] observatory: 275 lines, 2 hosts, 5 history days → /opt/ecoPrimals/sporePrint/public/observatory/data.json";
        parse_observatory_msg(&mut sig, msg);

        assert!(sig.observatory_snapshot);
        assert_eq!(sig.observatory_lines, 275);
    }

    #[test]
    fn parse_observatory_msg_handles_failure() {
        let mut sig = publish_signal("sporeprint", 100);
        let msg = "  [observatory] FAILED: SSH connection timeout";
        parse_observatory_msg(&mut sig, msg);

        assert!(!sig.observatory_snapshot);
        assert_eq!(sig.observatory_lines, 0);
    }

    #[test]
    fn parse_observatory_msg_handles_empty() {
        let mut sig = publish_signal("detroit", 100);
        parse_observatory_msg(&mut sig, "");

        assert!(!sig.observatory_snapshot);
    }

    #[test]
    fn now_epoch_is_reasonable() {
        let ts = now_epoch();
        // Should be after 2026-01-01
        assert!(ts > 1_767_225_600, "timestamp too old: {ts}");
    }

    #[test]
    fn signal_log_roundtrip() {
        let dir = std::env::temp_dir().join("signal-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test-signal.jsonl");

        // Write two events
        let publish = SignalEvent::Publish(publish_signal("detroit", 233));
        let receptor = SignalEvent::Receptor(ReceptorSignal {
            ts: 1790980000,
            date: "2026-10-03".into(),
            hosts: BTreeMap::new(),
        });

        let line1 = serde_json::to_string(&publish).unwrap();
        let line2 = serde_json::to_string(&receptor).unwrap();

        // Simulate append
        std::fs::write(&path, format!("{line1}\n{line2}\n")).unwrap();

        // Read back and verify
        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);

        let parsed1: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(parsed1["type"], "publish");
        assert_eq!(parsed1["site"], "detroit");

        let parsed2: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(parsed2["type"], "receptor");

        // Cleanup
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn outreach_signal_serializes_with_type_tag() {
        let sig = OutreachSignal {
            ts: 1791060000,
            channel: "email".into(),
            recipients: 15,
            pages_linked: vec![
                "/evidence/tcr-22-12/".into(),
                "/analysis/rico-pattern/".into(),
            ],
            note: "Belle Isle nuclear email".into(),
        };
        let event = SignalEvent::Outreach(sig);
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"outreach""#));
        assert!(json.contains(r#""channel":"email""#));
        assert!(json.contains(r#""recipients":15"#));
        assert!(json.contains("tcr-22-12"));
    }

    #[test]
    fn correlation_signal_serializes() {
        let sig = CorrelationSignal {
            ts: 1791070000,
            outreach_ts: 1791060000,
            propagation_delay_s: Some(3600),
            bot_propagation_delay_s: None,
            pages_activated: 2,
            pages_linked_total: 3,
            sessions: 5,
            depth_avg: 2.4,
            activation_rate: 0.667,
        };
        let event = SignalEvent::Correlation(sig);
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"correlation""#));
        assert!(json.contains(r#""propagation_delay_s":3600"#));
        assert!(json.contains(r#""activation_rate":0.667"#));
        // bot_propagation_delay_s should be absent (skip_serializing_if None)
        assert!(!json.contains("bot_propagation_delay_s"));
    }

    #[test]
    fn outreach_signal_deserializes() {
        let json = r#"{"type":"outreach","ts":1791060000,"channel":"email","recipients":15,"pages_linked":["/evidence/"],"note":"test"}"#;
        let event: SignalEvent = serde_json::from_str(json).unwrap();
        match event {
            SignalEvent::Outreach(o) => {
                assert_eq!(o.channel, "email");
                assert_eq!(o.recipients, 15);
                assert_eq!(o.pages_linked, vec!["/evidence/"]);
            }
            _ => panic!("expected Outreach variant"),
        }
    }

    #[test]
    fn extract_multi_value_works() {
        let args = [
            "--channel",
            "email",
            "--pages",
            "/p1",
            "/p2",
            "--note",
            "test",
        ];
        let pages = extract_multi_value(&args, "--pages");
        assert_eq!(pages, vec!["/p1", "/p2"]);
    }

    #[test]
    fn extract_multi_value_empty_when_missing() {
        let args = ["--channel", "email"];
        let pages = extract_multi_value(&args, "--pages");
        assert!(pages.is_empty());
    }

    #[test]
    fn all_four_event_types_roundtrip() {
        let events = vec![
            SignalEvent::Publish(publish_signal("detroit", 100)),
            SignalEvent::Receptor(ReceptorSignal {
                ts: 1791000000,
                date: "2026-10-03".into(),
                hosts: BTreeMap::new(),
            }),
            SignalEvent::Outreach(OutreachSignal {
                ts: 1791060000,
                channel: "email".into(),
                recipients: 15,
                pages_linked: vec!["/evidence/".into()],
                note: "test".into(),
            }),
            SignalEvent::Correlation(CorrelationSignal {
                ts: 1791070000,
                outreach_ts: 1791060000,
                propagation_delay_s: Some(600),
                bot_propagation_delay_s: None,
                pages_activated: 1,
                pages_linked_total: 1,
                sessions: 3,
                depth_avg: 2.0,
                activation_rate: 1.0,
            }),
        ];

        for event in &events {
            let json = serde_json::to_string(event).unwrap();
            let parsed: SignalEvent = serde_json::from_str(&json).unwrap();
            let json2 = serde_json::to_string(&parsed).unwrap();
            assert_eq!(json, json2, "roundtrip failed for event type");
        }
    }
}
