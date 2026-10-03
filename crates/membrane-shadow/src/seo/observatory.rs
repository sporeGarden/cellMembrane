// SPDX-License-Identifier: AGPL-3.0-or-later

//! Observatory data pipeline — receptor snapshots + historical trends.
//!
//! Bridges the receptor's per-request analysis into the observatory page's
//! `data.json` format, with daily digest accumulation for trend history.
//!
//! ## Pipeline Flow
//!
//! 1. Receptor analyzes Caddy logs → `ReceptorReport`
//! 2. Load existing JSONL history file (one line per day)
//! 3. Upsert today's digest into history
//! 4. Merge report + history into `ObservatorySnapshot`
//! 5. Write `data.json` to site's `public/observatory/` dir
//!
//! ## Privacy
//!
//! The snapshot contains only aggregate counts — no IPs, no PII, no
//! per-visitor data. Safe to serve publicly from sporePrint.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use tracing::{info, warn};

use super::receptor::ReceptorReport;

// ── History Types ───────────────────────────────────────────────────

/// Daily digest — one entry per day in the JSONL history file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyDigest {
    /// Epoch seconds when this digest was generated.
    pub generated_at: u64,
    /// Date string (YYYY-MM-DD).
    pub date: String,
    /// Per-host summary for this day.
    pub hosts: BTreeMap<String, HostDigest>,
}

/// Compact per-host summary for trend history.
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

// ── Observatory Snapshot ────────────────────────────────────────────

/// Complete observatory payload — receptor report + trend history.
///
/// This is what gets written to `data.json` and consumed by the
/// observatory page's vanilla JS.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservatorySnapshot {
    /// Epoch seconds when this snapshot was generated.
    pub generated_at: u64,
    /// Per-host signal summaries (from receptor).
    pub hosts: BTreeMap<String, super::receptor::HostSignal>,
    /// Log lines processed in this snapshot.
    pub lines_processed: u64,
    /// Log lines that failed to parse.
    pub parse_errors: u64,
    /// Earliest log timestamp (epoch seconds).
    pub window_start: f64,
    /// Latest log timestamp (epoch seconds).
    pub window_end: f64,
    /// Historical daily digests (newest first, capped at 90 days).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<DailyDigest>,
}

/// Maximum number of daily digests to retain.
const MAX_HISTORY_DAYS: usize = 90;

impl ObservatorySnapshot {
    /// Build a snapshot from a receptor report and existing history.
    pub fn from_report(report: &ReceptorReport, history: Vec<DailyDigest>) -> Self {
        Self {
            generated_at: report.generated_at,
            hosts: report.hosts.clone(),
            lines_processed: report.lines_processed,
            parse_errors: report.parse_errors,
            window_start: report.window_start,
            window_end: report.window_end,
            history,
        }
    }
}

// ── History Management ──────────────────────────────────────────────

/// Extract a daily digest from a receptor report.
pub fn digest_from_report(report: &ReceptorReport) -> DailyDigest {
    let hosts: BTreeMap<String, HostDigest> = report
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

    DailyDigest {
        generated_at: report.generated_at,
        date: today_date_string(),
        hosts,
    }
}

/// Load history from a JSONL file (one `DailyDigest` per line).
pub fn load_history(path: &Path) -> Vec<DailyDigest> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|line| match serde_json::from_str::<DailyDigest>(line) {
            Ok(d) => Some(d),
            Err(e) => {
                warn!(error = %e, "observatory: skipping malformed history line");
                None
            }
        })
        .collect()
}

/// Upsert today's digest into the history and save to JSONL.
///
/// If today's date already has an entry, it's replaced. Otherwise the
/// new digest is appended. History is capped at [`MAX_HISTORY_DAYS`].
pub fn save_history(path: &Path, history: &[DailyDigest]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let lines: Vec<String> = history
        .iter()
        .filter_map(|d| serde_json::to_string(d).ok())
        .collect();

    std::fs::write(path, lines.join("\n") + "\n")
}

/// Upsert a digest into the history vec (by date), capping at max days.
pub fn upsert_digest(history: &mut Vec<DailyDigest>, digest: DailyDigest) {
    if let Some(existing) = history.iter_mut().find(|d| d.date == digest.date) {
        *existing = digest;
    } else {
        history.push(digest);
    }

    // Sort by date descending (newest first for the observatory JS).
    history.sort_by(|a, b| b.date.cmp(&a.date));

    // Cap at MAX_HISTORY_DAYS.
    history.truncate(MAX_HISTORY_DAYS);
}

// ── Snapshot Generation ─────────────────────────────────────────────

/// Default number of log lines for observatory snapshots.
///
/// 50k lines ≈ 24h of traffic at current levels, giving a good
/// daily window for the observatory dashboard.
const OBSERVATORY_LOG_LINES: u32 = 50_000;

/// JSONL history file path relative to the site worktree.
const HISTORY_FILENAME: &str = "observatory-history.jsonl";

/// Generate and write an observatory snapshot for a publish site.
///
/// Called from the publish pipeline after zola build. Writes:
/// 1. `{public_dir}/observatory/data.json` — full snapshot for the page
/// 2. `{worktree}/{HISTORY_FILENAME}` — append-only JSONL history
///
/// Returns `Ok(Some(msg))` on success, `Ok(None)` if skipped (not sporePrint),
/// or `Err` on failure. Designed to be non-fatal in the pipeline.
pub async fn generate_snapshot(
    config: &crate::ShadowConfig,
    site: &super::PublishSite,
) -> crate::error::Result<Option<String>> {
    // Only generate for sporePrint — detroit shouldn't expose analytics publicly.
    if site.host != "sporeprint.primals.eco" {
        return Ok(None);
    }

    // Run the receptor analysis.
    let report = super::receptor::analyze(config, OBSERVATORY_LOG_LINES).await?;

    let lines_processed = report.lines_processed;
    let host_count = report.hosts.len();

    // Load existing history.
    let history_path = Path::new(site.worktree).join(HISTORY_FILENAME);
    let mut history = load_history(&history_path);

    // Create today's digest and upsert.
    let digest = digest_from_report(&report);
    upsert_digest(&mut history, digest);

    // Save updated history JSONL.
    if let Err(e) = save_history(&history_path, &history) {
        warn!(error = %e, "observatory: failed to save history JSONL");
        // Non-fatal — continue with snapshot even if history can't be saved.
    }

    // Build the full snapshot (report + history).
    let snapshot = ObservatorySnapshot::from_report(&report, history);

    // Write to public/observatory/data.json (post-build, overrides Zola's static copy).
    let observatory_dir = Path::new(site.public_dir).join("observatory");
    std::fs::create_dir_all(&observatory_dir).map_err(crate::error::ShadowError::Io)?;

    let data_path = observatory_dir.join("data.json");
    let json = serde_json::to_string_pretty(&snapshot)
        .map_err(|e| crate::error::ShadowError::config(format!("JSON serialize: {e}")))?;

    std::fs::write(&data_path, &json).map_err(crate::error::ShadowError::Io)?;

    info!(
        path = %data_path.display(),
        lines = lines_processed,
        hosts = host_count,
        history_days = snapshot.history.len(),
        "observatory snapshot written"
    );

    // Append receptor signal to dual signal log (non-fatal).
    let receptor_sig = crate::signal::receptor_signal(&report);
    crate::signal::append_signal(&crate::signal::SignalEvent::Receptor(receptor_sig));

    Ok(Some(format!(
        "observatory: {lines_processed} lines, {host_count} hosts, {} history days → {}",
        snapshot.history.len(),
        data_path.display()
    )))
}

/// Current date as YYYY-MM-DD string (UTC).
fn today_date_string() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // 86400 seconds per day, epoch is 1970-01-01
    let days = now / 86400;
    // Convert days since epoch to YYYY-MM-DD using a simple algorithm.
    epoch_days_to_date(days)
}

/// Convert days since Unix epoch to YYYY-MM-DD string.
fn epoch_days_to_date(days: u64) -> String {
    // Algorithm from Howard Hinnant's chrono-compatible date conversion.
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_days_known_dates() {
        // 1970-01-01 = day 0
        assert_eq!(epoch_days_to_date(0), "1970-01-01");
        // 2000-01-01 = day 10957
        assert_eq!(epoch_days_to_date(10957), "2000-01-01");
        // 2026-10-02 = day 20728
        assert_eq!(epoch_days_to_date(20728), "2026-10-02");
    }

    #[test]
    fn today_date_string_format() {
        let date = today_date_string();
        assert_eq!(date.len(), 10);
        assert_eq!(&date[4..5], "-");
        assert_eq!(&date[7..8], "-");
    }

    #[test]
    fn digest_from_report_extracts_correctly() {
        let mut report = ReceptorReport::empty_for_test();
        report.hosts.insert(
            "sporeprint.primals.eco".into(),
            super::super::receptor::HostSignal {
                host: "sporeprint.primals.eco".into(),
                total_requests: 142,
                total_human: 65,
                total_search_bot: 43,
                crawl_coverage: 27,
                unique_paths: 68,
                ..Default::default()
            },
        );

        let digest = digest_from_report(&report);
        assert_eq!(digest.hosts.len(), 1);
        let h = &digest.hosts["sporeprint.primals.eco"];
        assert_eq!(h.total_requests, 142);
        assert_eq!(h.total_human, 65);
        assert_eq!(h.total_search_bot, 43);
    }

    #[test]
    fn upsert_digest_replaces_same_day() {
        let mut history = vec![DailyDigest {
            generated_at: 1000,
            date: "2026-10-01".into(),
            hosts: BTreeMap::new(),
        }];

        let new = DailyDigest {
            generated_at: 2000,
            date: "2026-10-01".into(),
            hosts: BTreeMap::new(),
        };
        upsert_digest(&mut history, new);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].generated_at, 2000);
    }

    #[test]
    fn upsert_digest_appends_new_day() {
        let mut history = vec![DailyDigest {
            generated_at: 1000,
            date: "2026-10-01".into(),
            hosts: BTreeMap::new(),
        }];

        let new = DailyDigest {
            generated_at: 2000,
            date: "2026-10-02".into(),
            hosts: BTreeMap::new(),
        };
        upsert_digest(&mut history, new);
        assert_eq!(history.len(), 2);
        // Newest first
        assert_eq!(history[0].date, "2026-10-02");
        assert_eq!(history[1].date, "2026-10-01");
    }

    #[test]
    fn upsert_caps_at_max() {
        let mut history: Vec<DailyDigest> = (0..95)
            .map(|i| DailyDigest {
                generated_at: i,
                date: format!("2026-{:02}-{:02}", (i / 28) + 1, (i % 28) + 1),
                hosts: BTreeMap::new(),
            })
            .collect();

        let new = DailyDigest {
            generated_at: 100,
            date: "2026-12-31".into(),
            hosts: BTreeMap::new(),
        };
        upsert_digest(&mut history, new);
        assert!(history.len() <= MAX_HISTORY_DAYS);
    }

    #[test]
    fn history_jsonl_roundtrip() {
        let dir = std::env::temp_dir().join("observatory-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test-history.jsonl");

        let mut hosts = BTreeMap::new();
        hosts.insert(
            "sporeprint.primals.eco".into(),
            HostDigest {
                total_requests: 100,
                total_human: 50,
                total_search_bot: 30,
                unique_paths: 40,
                crawl_coverage: 15,
            },
        );

        let history = vec![DailyDigest {
            generated_at: 1790887146,
            date: "2026-10-01".into(),
            hosts,
        }];

        save_history(&path, &history).unwrap();
        let loaded = load_history(&path);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].date, "2026-10-01");
        assert_eq!(
            loaded[0].hosts["sporeprint.primals.eco"].total_requests,
            100
        );

        // Cleanup
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn snapshot_serializes_to_expected_schema() {
        let mut hosts = BTreeMap::new();
        hosts.insert(
            "sporeprint.primals.eco".into(),
            super::super::receptor::HostSignal {
                host: "sporeprint.primals.eco".into(),
                total_requests: 142,
                total_human: 65,
                total_search_bot: 43,
                crawl_coverage: 27,
                unique_paths: 68,
                ..Default::default()
            },
        );

        let snapshot = ObservatorySnapshot {
            generated_at: 1790973546,
            hosts,
            lines_processed: 216,
            parse_errors: 0,
            window_start: 1_790_950_000.0,
            window_end: 1_790_973_546.0,
            history: vec![],
        };

        let json = serde_json::to_value(&snapshot).unwrap();
        assert!(json["generated_at"].is_u64());
        assert!(json["hosts"]["sporeprint.primals.eco"]["total_requests"].is_u64());
        assert!(json["lines_processed"].is_u64());
        // Empty history should be omitted (skip_serializing_if)
        assert!(json.get("history").is_none());
    }

    #[test]
    fn snapshot_with_history_serializes() {
        let snapshot = ObservatorySnapshot {
            generated_at: 1790973546,
            hosts: BTreeMap::new(),
            lines_processed: 0,
            parse_errors: 0,
            window_start: 0.0,
            window_end: 0.0,
            history: vec![DailyDigest {
                generated_at: 1790887146,
                date: "2026-10-01".into(),
                hosts: BTreeMap::new(),
            }],
        };

        let json = serde_json::to_value(&snapshot).unwrap();
        assert!(json["history"].is_array());
        assert_eq!(json["history"].as_array().unwrap().len(), 1);
        assert_eq!(json["history"][0]["date"], "2026-10-01");
    }
}
