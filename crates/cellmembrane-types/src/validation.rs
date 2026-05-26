// SPDX-License-Identifier: AGPL-3.0-or-later

//! Validation report types.
//!
//! Follows the `plasmidbin-types` pattern: accumulate pass/fail/warn entries,
//! never panic, return a structured report.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Severity of a validation finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Informational — not an issue.
    Info,
    /// Potential issue that does not block deployment.
    Warn,
    /// Issue that blocks deployment.
    Fail,
    /// Check passed.
    Pass,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Info => write!(f, "INFO"),
            Self::Warn => write!(f, "WARN"),
            Self::Fail => write!(f, "FAIL"),
            Self::Pass => write!(f, "PASS"),
        }
    }
}

/// A single validation finding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportEntry {
    /// Severity of this finding.
    pub severity: Severity,
    /// Check identifier (e.g. "composition.primals_present").
    pub check: String,
    /// Human-readable message.
    pub message: String,
}

impl fmt::Display for ReportEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}: {}", self.severity, self.check, self.message)
    }
}

/// Accumulated validation report for a membrane configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Report {
    /// All findings.
    pub entries: Vec<ReportEntry>,
}

impl Report {
    /// Create an empty report.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a PASS entry.
    pub fn pass(&mut self, check: impl Into<String>, message: impl Into<String>) {
        self.entries.push(ReportEntry {
            severity: Severity::Pass,
            check: check.into(),
            message: message.into(),
        });
    }

    /// Add a FAIL entry.
    pub fn fail(&mut self, check: impl Into<String>, message: impl Into<String>) {
        self.entries.push(ReportEntry {
            severity: Severity::Fail,
            check: check.into(),
            message: message.into(),
        });
    }

    /// Add a WARN entry.
    pub fn warn(&mut self, check: impl Into<String>, message: impl Into<String>) {
        self.entries.push(ReportEntry {
            severity: Severity::Warn,
            check: check.into(),
            message: message.into(),
        });
    }

    /// Add an INFO entry.
    pub fn info(&mut self, check: impl Into<String>, message: impl Into<String>) {
        self.entries.push(ReportEntry {
            severity: Severity::Info,
            check: check.into(),
            message: message.into(),
        });
    }

    /// Whether all checks passed (no FAIL entries).
    pub fn is_ok(&self) -> bool {
        !self.entries.iter().any(|e| e.severity == Severity::Fail)
    }

    /// Count of entries by severity.
    pub fn count(&self, severity: Severity) -> usize {
        self.entries.iter().filter(|e| e.severity == severity).count()
    }

    /// Total number of checks (PASS + FAIL).
    pub fn total_checks(&self) -> usize {
        self.count(Severity::Pass) + self.count(Severity::Fail)
    }

    /// Merge another report into this one.
    pub fn merge(&mut self, other: Report) {
        self.entries.extend(other.entries);
    }

    /// Summary line (e.g. "12 passed, 2 failed, 1 warning").
    pub fn summary(&self) -> String {
        format!(
            "{} passed, {} failed, {} warnings",
            self.count(Severity::Pass),
            self.count(Severity::Fail),
            self.count(Severity::Warn),
        )
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for entry in &self.entries {
            writeln!(f, "{entry}")?;
        }
        writeln!(f, "--- {}", self.summary())
    }
}
