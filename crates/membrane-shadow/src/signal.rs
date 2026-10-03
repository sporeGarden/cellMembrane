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

use serde::Serialize;
use std::collections::BTreeMap;
use tracing::warn;

/// Signal log path on golgiBody.
const SIGNAL_LOG_PATH: &str = "/var/log/membrane/signal.jsonl";

// ── Signal Event Types ──────────────────────────────────────────────

/// A single event in the signal log (one JSON line).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SignalEvent {
    /// LuxI: outbound publish completed.
    Publish(PublishSignal),
    /// LuxR: inbound receptor snapshot taken.
    Receptor(ReceptorSignal),
}

/// LuxI publish event — records what was secreted outward.
#[derive(Debug, Clone, Serialize)]
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
#[derive(Debug, Clone, Serialize)]
pub struct ReceptorSignal {
    /// Unix epoch seconds.
    pub ts: u64,
    /// Date string (YYYY-MM-DD).
    pub date: String,
    /// Per-host summary digests.
    pub hosts: BTreeMap<String, HostDigest>,
}

/// Compact per-host summary for the signal log.
#[derive(Debug, Clone, Serialize)]
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
}
