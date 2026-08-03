// SPDX-License-Identifier: AGPL-3.0-or-later

//! Harvest scheduler — ingest → debounce → batch build.
//!
//! CI-EVO-01: Replaces immediate per-push builds with a queue-based scheduler.
//!
//! Two layers of intentionality:
//! - **Team-driven**: A gate pushes code and optionally signals `[harvest]` in the
//!   commit message or calls `harvest.request`. This promotes the primal to
//!   `build_requested` status, triggering a build on the next scheduler tick.
//! - **Pipeline-driven**: If a primal stays dirty beyond `staleness_threshold`,
//!   the scheduler auto-promotes it. Regular scheduler ticks batch all ready
//!   primals into a single harvest call.
//!
//! Queue file: `$XDG_STATE_HOME/membrane/harvest_queue.toml`
//! (default: `~/.local/state/membrane/harvest_queue.toml`)

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

/// Default debounce window: 5 minutes after last push before auto-building.
const DEFAULT_DEBOUNCE_SECS: u64 = 300;

/// Default staleness threshold: auto-promote after 24 hours dirty.
const DEFAULT_STALENESS_THRESHOLD_SECS: u64 = 86400;

/// Queue entry status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueStatus {
    /// Push received, waiting for debounce window or team signal.
    Dirty,
    /// Team explicitly requested a build (commit tag or `harvest.request`).
    BuildRequested,
    /// Currently being built.
    Building,
}

/// A single primal's entry in the harvest queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueEntry {
    pub status: QueueStatus,
    /// ISO-8601 timestamp of the first push that dirtied this primal.
    pub first_dirty: String,
    /// ISO-8601 timestamp of the most recent push.
    pub last_push: String,
    /// Latest commit SHA from the push event.
    pub commit: String,
    /// Which gate pushed (Forgejo username or gate name).
    pub pusher: String,
    /// Number of pushes received while dirty.
    pub push_count: u32,
}

/// The full harvest queue persisted to disk.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HarvestQueue {
    /// Scheduler configuration.
    #[serde(default)]
    pub config: SchedulerConfig,
    /// Per-primal queue entries. Key is lowercase primal name.
    #[serde(default)]
    pub primals: BTreeMap<String, QueueEntry>,
}

/// Scheduler tuning parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerConfig {
    /// Seconds of silence after last push before auto-building dirty primals.
    #[serde(default = "default_debounce")]
    pub debounce_secs: u64,
    /// Seconds a primal can stay dirty before auto-promotion to `build_requested`.
    #[serde(default = "default_staleness")]
    pub staleness_threshold_secs: u64,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            debounce_secs: DEFAULT_DEBOUNCE_SECS,
            staleness_threshold_secs: DEFAULT_STALENESS_THRESHOLD_SECS,
        }
    }
}

fn default_debounce() -> u64 {
    DEFAULT_DEBOUNCE_SECS
}
fn default_staleness() -> u64 {
    DEFAULT_STALENESS_THRESHOLD_SECS
}

/// Result of a scheduler tick.
#[derive(Debug)]
pub struct SchedulerDecision {
    /// Primals that should be built now.
    pub build_now: Vec<String>,
    /// Primals still waiting (debounce not elapsed).
    pub waiting: Vec<String>,
    /// Primals auto-promoted from dirty to build_requested (staleness).
    pub auto_promoted: Vec<String>,
    /// Human-readable reason for the decision.
    pub reason: String,
}

// ── Queue file I/O ──────────────────────────────────────────────────────

fn queue_path() -> PathBuf {
    let state_home = std::env::var("XDG_STATE_HOME").map_or_else(
        |_| {
            PathBuf::from(cellmembrane_types::service::env_or(
                cellmembrane_types::service::ENV_HOME,
                "/tmp",
            ))
            .join(".local")
            .join("state")
        },
        PathBuf::from,
    );
    state_home.join("membrane").join("harvest_queue.toml")
}

pub fn load_queue() -> HarvestQueue {
    let path = queue_path();
    match std::fs::read_to_string(&path) {
        Ok(content) => toml::from_str(&content).unwrap_or_else(|e| {
            warn!(error = %e, path = %path.display(), "harvest queue parse error, starting fresh");
            HarvestQueue::default()
        }),
        Err(_) => HarvestQueue::default(),
    }
}

pub fn save_queue(queue: &HarvestQueue) -> crate::Result<()> {
    let path = queue_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(crate::error::ShadowError::Io)?;
    }
    let content = toml::to_string_pretty(queue).map_err(crate::error::ShadowError::Serialize)?;
    std::fs::write(&path, content).map_err(crate::error::ShadowError::Io)?;
    info!(path = %path.display(), primals = queue.primals.len(), "harvest queue saved");
    Ok(())
}

// ── Ingest: record a push event ─────────────────────────────────────────

/// Record a push event in the harvest queue without triggering a build.
///
/// If the primal is already dirty, updates `last_push` and increments `push_count`.
/// If it's currently building, logs a warning but still updates the queue
/// (the next scheduler tick will see the newer commit).
pub fn ingest(primal: &str, commit: &str, pusher: &str) -> crate::Result<HarvestQueue> {
    let mut queue = load_queue();
    let now = crate::utc_now_iso8601();
    let lower = primal.to_lowercase();

    match queue.primals.get_mut(&lower) {
        Some(entry) => {
            if entry.status == QueueStatus::Building {
                warn!(
                    primal = %lower,
                    "push received while building — will rebuild on next tick"
                );
            }
            entry.last_push = now;
            entry.commit = commit.to_string();
            entry.pusher = pusher.to_string();
            entry.push_count += 1;
            if entry.status != QueueStatus::Building {
                entry.status = QueueStatus::Dirty;
            }
            info!(
                primal = %lower,
                push_count = entry.push_count,
                "harvest queue: push #{} (batching)",
                entry.push_count
            );
        }
        None => {
            queue.primals.insert(
                lower.clone(),
                QueueEntry {
                    status: QueueStatus::Dirty,
                    first_dirty: now.clone(),
                    last_push: now,
                    commit: commit.to_string(),
                    pusher: pusher.to_string(),
                    push_count: 1,
                },
            );
            info!(primal = %lower, "harvest queue: new dirty primal");
        }
    }

    save_queue(&queue)?;
    Ok(queue)
}

// ── Request: team-driven intentional build ──────────────────────────────

/// Explicitly request a build for a primal (team-driven signal).
///
/// If the primal isn't in the queue, adds it as `build_requested` with
/// the current source HEAD. If already dirty, promotes to `build_requested`.
pub fn request_build(primal: &str) -> crate::Result<HarvestQueue> {
    let mut queue = load_queue();
    let now = crate::utc_now_iso8601();
    let lower = primal.to_lowercase();

    match queue.primals.get_mut(&lower) {
        Some(entry) => {
            entry.status = QueueStatus::BuildRequested;
            info!(primal = %lower, "harvest queue: build requested (promoted)");
        }
        None => {
            queue.primals.insert(
                lower.clone(),
                QueueEntry {
                    status: QueueStatus::BuildRequested,
                    first_dirty: now.clone(),
                    last_push: now,
                    commit: String::new(),
                    pusher: "operator".to_string(),
                    push_count: 0,
                },
            );
            info!(primal = %lower, "harvest queue: build requested (new)");
        }
    }

    save_queue(&queue)?;
    Ok(queue)
}

// ── Schedule: evaluate queue and decide what to build ───────────────────

/// Evaluate the harvest queue and decide which primals to build.
///
/// Returns a `SchedulerDecision` describing:
/// - `build_now`: primals that should be built (requested + debounce-elapsed + auto-promoted)
/// - `waiting`: primals still within debounce window
/// - `auto_promoted`: primals promoted from dirty due to staleness threshold
pub fn evaluate(queue: &mut HarvestQueue) -> SchedulerDecision {
    let now_epoch = now_epoch_secs();
    let mut build_now = Vec::new();
    let mut waiting = Vec::new();
    let mut auto_promoted = Vec::new();

    for (name, entry) in &mut queue.primals {
        match entry.status {
            QueueStatus::BuildRequested => {
                build_now.push(name.clone());
            }
            QueueStatus::Dirty => {
                let last_push_epoch = parse_iso_epoch(&entry.last_push).unwrap_or(0);
                let first_dirty_epoch = parse_iso_epoch(&entry.first_dirty).unwrap_or(0);
                let since_last_push = now_epoch.saturating_sub(last_push_epoch);
                let since_first_dirty = now_epoch.saturating_sub(first_dirty_epoch);

                if since_first_dirty >= queue.config.staleness_threshold_secs {
                    entry.status = QueueStatus::BuildRequested;
                    auto_promoted.push(name.clone());
                    build_now.push(name.clone());
                } else if since_last_push >= queue.config.debounce_secs {
                    build_now.push(name.clone());
                } else {
                    waiting.push(name.clone());
                }
            }
            QueueStatus::Building => {
                // Still building from a previous tick — skip.
            }
        }
    }

    let reason = if build_now.is_empty() && waiting.is_empty() {
        "queue empty".to_string()
    } else if build_now.is_empty() {
        format!("{} primals waiting (debounce)", waiting.len())
    } else {
        format!(
            "{} primals ready to build, {} waiting",
            build_now.len(),
            waiting.len()
        )
    };

    SchedulerDecision {
        build_now,
        waiting,
        auto_promoted,
        reason,
    }
}

/// Mark primals as building (call before starting harvest).
pub fn mark_building(queue: &mut HarvestQueue, primals: &[String]) {
    for name in primals {
        if let Some(entry) = queue.primals.get_mut(name) {
            entry.status = QueueStatus::Building;
        }
    }
}

/// Remove successfully built primals from the queue.
pub fn mark_complete(queue: &mut HarvestQueue, primals: &[String]) {
    for name in primals {
        queue.primals.remove(name);
    }
}

/// Mark primals as dirty again (build failed, will retry).
pub fn mark_failed(queue: &mut HarvestQueue, primals: &[String]) {
    for name in primals {
        if let Some(entry) = queue.primals.get_mut(name) {
            entry.status = QueueStatus::Dirty;
        }
    }
}

// ── List: show queue contents ───────────────────────────────────────────

/// Format the queue for human display.
pub fn format_queue(queue: &HarvestQueue) -> String {
    if queue.primals.is_empty() {
        return "harvest queue: empty (no dirty primals)".to_string();
    }

    let mut lines = vec![format!(
        "harvest queue: {} primals (debounce={}s, staleness={}h)",
        queue.primals.len(),
        queue.config.debounce_secs,
        queue.config.staleness_threshold_secs / 3600,
    )];

    for (name, entry) in &queue.primals {
        let status = match entry.status {
            QueueStatus::Dirty => "dirty",
            QueueStatus::BuildRequested => "BUILD_REQUESTED",
            QueueStatus::Building => "BUILDING",
        };
        lines.push(format!(
            "  {name}: {status} (pushes={}, last={}, by={})",
            entry.push_count,
            &entry.last_push[..19.min(entry.last_push.len())],
            entry.pusher,
        ));
    }

    lines.join("\n")
}

// ── Commit message signal detection ─────────────────────────────────────

/// Check if any commit message in a push contains the `[harvest]` signal.
pub fn has_harvest_signal(commits: &[super::super::webhook::CommitPayload]) -> bool {
    commits
        .iter()
        .any(|c| c.message.contains("[harvest]") || c.message.contains("[build]"))
}

// ── Helpers ─────────────────────────────────────────────────────────────

fn now_epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn parse_iso_epoch(iso: &str) -> Option<u64> {
    let date_part = iso.split('T').next()?;
    let parts: Vec<&str> = date_part.split('-').collect();
    if parts.len() < 3 {
        return None;
    }
    let year: u64 = parts[0].parse().ok()?;
    let month: u64 = parts[1].parse().ok()?;
    let day: u64 = parts[2].parse().ok()?;

    let time_part = iso.split('T').nth(1).unwrap_or("00:00:00");
    let time_parts: Vec<&str> = time_part
        .trim_end_matches('Z')
        .split(':')
        .collect();
    let hour: u64 = time_parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
    let min: u64 = time_parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let sec: u64 = time_parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);

    // Rough epoch calculation (ignoring leap years for scheduling purposes)
    let days = (year - 1970) * 365 + (year - 1969) / 4
        + match month {
            1 => 0, 2 => 31, 3 => 59, 4 => 90, 5 => 120, 6 => 151,
            7 => 181, 8 => 212, 9 => 243, 10 => 273, 11 => 304, 12 => 334,
            _ => 0,
        }
        + day - 1;

    Some(days * 86400 + hour * 3600 + min * 60 + sec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ingest_creates_dirty_entry() {
        let mut queue = HarvestQueue::default();
        let now = crate::utc_now_iso8601();
        queue.primals.insert(
            "beardog".to_string(),
            QueueEntry {
                status: QueueStatus::Dirty,
                first_dirty: now.clone(),
                last_push: now,
                commit: "abc12345".to_string(),
                pusher: "biomeGate".to_string(),
                push_count: 1,
            },
        );
        assert_eq!(queue.primals.len(), 1);
        assert_eq!(queue.primals["beardog"].status, QueueStatus::Dirty);
    }

    #[test]
    fn request_promotes_to_build_requested() {
        let mut queue = HarvestQueue::default();
        let now = crate::utc_now_iso8601();
        queue.primals.insert(
            "beardog".to_string(),
            QueueEntry {
                status: QueueStatus::Dirty,
                first_dirty: now.clone(),
                last_push: now,
                commit: "abc12345".to_string(),
                pusher: "biomeGate".to_string(),
                push_count: 3,
            },
        );
        queue.primals.get_mut("beardog").unwrap().status = QueueStatus::BuildRequested;
        assert_eq!(
            queue.primals["beardog"].status,
            QueueStatus::BuildRequested
        );
    }

    #[test]
    fn evaluate_builds_requested() {
        let mut queue = HarvestQueue::default();
        let now = crate::utc_now_iso8601();
        queue.primals.insert(
            "beardog".to_string(),
            QueueEntry {
                status: QueueStatus::BuildRequested,
                first_dirty: now.clone(),
                last_push: now,
                commit: "abc12345".to_string(),
                pusher: "biomeGate".to_string(),
                push_count: 1,
            },
        );
        let decision = evaluate(&mut queue);
        assert_eq!(decision.build_now, vec!["beardog".to_string()]);
        assert!(decision.waiting.is_empty());
    }

    #[test]
    fn evaluate_waits_during_debounce() {
        let mut queue = HarvestQueue::default();
        let now = crate::utc_now_iso8601();
        queue.primals.insert(
            "beardog".to_string(),
            QueueEntry {
                status: QueueStatus::Dirty,
                first_dirty: now.clone(),
                last_push: now, // Just pushed — within debounce window
                commit: "abc12345".to_string(),
                pusher: "biomeGate".to_string(),
                push_count: 1,
            },
        );
        let decision = evaluate(&mut queue);
        assert!(decision.build_now.is_empty());
        assert_eq!(decision.waiting, vec!["beardog".to_string()]);
    }

    #[test]
    fn mark_complete_removes_from_queue() {
        let mut queue = HarvestQueue::default();
        let now = crate::utc_now_iso8601();
        queue.primals.insert(
            "beardog".to_string(),
            QueueEntry {
                status: QueueStatus::Building,
                first_dirty: now.clone(),
                last_push: now,
                commit: "abc12345".to_string(),
                pusher: "biomeGate".to_string(),
                push_count: 1,
            },
        );
        mark_complete(&mut queue, &["beardog".to_string()]);
        assert!(queue.primals.is_empty());
    }

    #[test]
    fn iso_epoch_parse() {
        let epoch = parse_iso_epoch("2026-08-03T14:30:00Z");
        assert!(epoch.is_some());
        assert!(epoch.unwrap() > 0);
    }

    #[test]
    fn harvest_signal_detection() {
        let commits = vec![crate::webhook::CommitPayload {
            id: "abc123".to_string(),
            message: "fix: resolve race condition [harvest]".to_string(),
        }];
        assert!(has_harvest_signal(&commits));

        let commits = vec![crate::webhook::CommitPayload {
            id: "def456".to_string(),
            message: "wip: still debugging".to_string(),
        }];
        assert!(!has_harvest_signal(&commits));
    }
}
