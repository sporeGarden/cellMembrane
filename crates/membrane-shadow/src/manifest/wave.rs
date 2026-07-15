// SPDX-License-Identifier: AGPL-3.0-or-later

//! Wave lifecycle tracking — evolves raw `meta.wave` u32 into a typed state machine.

use serde::{Deserialize, Serialize};

use super::ManifestMeta;

/// Typed representation of a wave's lifecycle state.
///
/// Evolves the raw `meta.wave` u32 into a domain object that can track
/// lifecycle progression. Freshness and cascade become derived views of
/// the wave state rather than hand-crafted TOML fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaveState {
    /// Wave numeric identifier.
    pub id: u32,
    /// ISO-8601 date when the wave was opened (first cascade at this ID).
    #[serde(default)]
    pub opened: Option<String>,
    /// ISO-8601 date when exit criteria were met and wave was closed.
    #[serde(default)]
    pub closed: Option<String>,
    /// Exit criteria with their satisfaction state.
    #[serde(default)]
    pub exit_criteria: Vec<ExitCriterion>,
    /// Last rootPulse session committed for this wave (sovereignty proof).
    #[serde(default)]
    pub last_rootpulse_session: Option<String>,
}

/// A single exit criterion for wave closure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExitCriterion {
    /// Human-readable description of the criterion.
    pub description: String,
    /// Whether this criterion has been satisfied.
    #[serde(default)]
    pub satisfied: bool,
}

impl WaveState {
    /// Create a new open wave.
    #[must_use]
    pub fn open(id: u32) -> Self {
        Self {
            id,
            opened: Some(chrono::Utc::now().format(cellmembrane_types::service::ISO8601_UTC).to_string()),
            closed: None,
            exit_criteria: Vec::new(),
            last_rootpulse_session: None,
        }
    }

    /// Construct from manifest meta (backward compatible with raw wave ID).
    #[must_use]
    pub const fn from_manifest(meta: &ManifestMeta) -> Self {
        Self {
            id: meta.wave,
            opened: None,
            closed: None,
            exit_criteria: Vec::new(),
            last_rootpulse_session: None,
        }
    }

    /// Whether all exit criteria are satisfied.
    #[must_use]
    pub fn is_closeable(&self) -> bool {
        !self.exit_criteria.is_empty() && self.exit_criteria.iter().all(|c| c.satisfied)
    }

    /// Mark the wave as closed with the current timestamp.
    pub fn close(&mut self) {
        self.closed = Some(chrono::Utc::now().format(cellmembrane_types::service::ISO8601_UTC).to_string());
    }

    /// Record a rootpulse session.
    pub fn record_rootpulse(&mut self, session_id: &str) {
        self.last_rootpulse_session = Some(session_id.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wave_open_sets_timestamp() {
        let w = WaveState::open(42);
        assert_eq!(w.id, 42);
        assert!(w.opened.is_some());
        assert!(w.closed.is_none());
    }

    #[test]
    fn wave_from_manifest_meta() {
        let meta = ManifestMeta {
            version: String::new(),
            generated: String::new(),
            wave: 99,
            total_repos: 0,
        };
        let w = WaveState::from_manifest(&meta);
        assert_eq!(w.id, 99);
        assert!(w.opened.is_none());
    }

    #[test]
    fn wave_closeable_requires_all_satisfied() {
        let mut w = WaveState::open(1);
        assert!(!w.is_closeable(), "empty criteria not closeable");

        w.exit_criteria.push(ExitCriterion {
            description: "tests pass".into(),
            satisfied: true,
        });
        w.exit_criteria.push(ExitCriterion {
            description: "reviewed".into(),
            satisfied: false,
        });
        assert!(!w.is_closeable(), "partial criteria not closeable");

        w.exit_criteria[1].satisfied = true;
        assert!(w.is_closeable(), "all satisfied → closeable");
    }

    #[test]
    fn wave_close_sets_timestamp() {
        let mut w = WaveState::open(1);
        assert!(w.closed.is_none());
        w.close();
        assert!(w.closed.is_some());
    }

    #[test]
    fn wave_record_rootpulse() {
        let mut w = WaveState::open(1);
        assert!(w.last_rootpulse_session.is_none());
        w.record_rootpulse("session-abc");
        assert_eq!(w.last_rootpulse_session.as_deref(), Some("session-abc"));
    }
}
