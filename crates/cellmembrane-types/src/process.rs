// SPDX-License-Identifier: AGPL-3.0-or-later

//! Process and service lifecycle types — OS Atheism Phase 2.
//!
//! Platform-agnostic types for service management, process termination,
//! and CSPRNG. These types define the trait boundary that init-system
//! implementations (`SystemdManager`, `LaunchdManager`, `BareProcessManager`)
//! must satisfy.
//!
//! No async runtime — implementations live in `membrane-shadow`.

use std::fmt;

/// Status of a managed service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceStatus {
    /// Service is running and healthy.
    Running,
    /// Service is stopped (exited cleanly or never started).
    Stopped,
    /// Service has failed (non-zero exit or crash).
    Failed,
    /// Status cannot be determined (init system unavailable).
    Unknown,
}

impl fmt::Display for ServiceStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Running => write!(f, "running"),
            Self::Stopped => write!(f, "stopped"),
            Self::Failed => write!(f, "failed"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

/// Outcome of a service lifecycle operation (install, enable, restart, etc.).
#[derive(Debug, Clone)]
pub struct ServiceOutcome {
    /// Whether the operation succeeded.
    pub ok: bool,
    /// Human-readable detail (e.g. "3 units installed", "daemon-reload failed").
    pub detail: String,
}

impl ServiceOutcome {
    /// Successful outcome with a detail message.
    #[must_use]
    pub fn success(detail: impl Into<String>) -> Self {
        Self {
            ok: true,
            detail: detail.into(),
        }
    }

    /// Failed outcome with a detail message.
    #[must_use]
    pub fn failure(detail: impl Into<String>) -> Self {
        Self {
            ok: false,
            detail: detail.into(),
        }
    }
}

/// Init system flavor — selects the `ServiceManager` implementation.
///
/// Derived from `TargetOs::has_systemd()` and runtime detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitSystem {
    /// Linux systemd — full unit file generation and `systemctl` management.
    Systemd,
    /// macOS launchd — plist generation (future).
    Launchd,
    /// Windows Service Control Manager (future).
    WindowsSCM,
    /// No init system — bare process spawn/kill. Used for dev, containers,
    /// and platforms without a supported init system.
    Bare,
}

impl InitSystem {
    /// Detect the init system for the current platform.
    #[must_use]
    pub fn detect() -> Self {
        if cfg!(target_os = "linux") {
            if std::path::Path::new("/run/systemd/system").exists() {
                Self::Systemd
            } else {
                Self::Bare
            }
        } else if cfg!(target_os = "macos") {
            Self::Launchd
        } else if cfg!(target_os = "windows") {
            Self::WindowsSCM
        } else {
            Self::Bare
        }
    }

    /// Whether this init system supports unit/service file generation.
    #[must_use]
    pub const fn supports_units(&self) -> bool {
        matches!(self, Self::Systemd | Self::Launchd | Self::WindowsSCM)
    }
}

impl fmt::Display for InitSystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Systemd => write!(f, "systemd"),
            Self::Launchd => write!(f, "launchd"),
            Self::WindowsSCM => write!(f, "windows-scm"),
            Self::Bare => write!(f, "bare"),
        }
    }
}

// ── Crash-loop detection ──────────────────────────────────────────────

/// Default restart count above which a service is considered crash-looping.
pub const CRASH_LOOP_RESTART_THRESHOLD: u32 = 5;

/// Action taken when a crash-loop is detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrashLoopAction {
    /// Service was disabled to stop the loop.
    Disabled,
    /// Service was only logged (dry-run or threshold not met).
    Logged,
    /// Could not disable (permission denied, etc.).
    FailedToDisable,
}

impl fmt::Display for CrashLoopAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disabled => write!(f, "disabled"),
            Self::Logged => write!(f, "logged"),
            Self::FailedToDisable => write!(f, "failed-to-disable"),
        }
    }
}

/// A single service found to be crash-looping.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CrashLoopEntry {
    /// Systemd unit name.
    pub unit: String,
    /// Number of restarts observed.
    pub restart_count: u32,
    /// Current sub-state (e.g. "failed", "activating").
    pub sub_state: String,
    /// Action taken.
    pub action: CrashLoopAction,
}

/// Report from a crash-loop scan.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CrashLoopReport {
    /// Services detected as crash-looping (restart count > threshold).
    pub loops: Vec<CrashLoopEntry>,
    /// Threshold used for detection.
    pub threshold: u32,
    /// Total membrane services scanned.
    pub scanned: u32,
}

impl CrashLoopReport {
    /// Whether any crash loops were found.
    #[must_use]
    pub fn has_loops(&self) -> bool {
        !self.loops.is_empty()
    }

    /// Count of loops that were successfully disabled.
    #[must_use]
    pub fn disabled_count(&self) -> usize {
        self.loops
            .iter()
            .filter(|e| e.action == CrashLoopAction::Disabled)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_status_display() {
        assert_eq!(ServiceStatus::Running.to_string(), "running");
        assert_eq!(ServiceStatus::Stopped.to_string(), "stopped");
        assert_eq!(ServiceStatus::Failed.to_string(), "failed");
        assert_eq!(ServiceStatus::Unknown.to_string(), "unknown");
    }

    #[test]
    fn service_outcome_constructors() {
        let ok = ServiceOutcome::success("3 units installed");
        assert!(ok.ok);
        assert_eq!(ok.detail, "3 units installed");

        let fail = ServiceOutcome::failure("daemon-reload failed");
        assert!(!fail.ok);
        assert_eq!(fail.detail, "daemon-reload failed");
    }

    #[test]
    fn init_system_detect_returns_valid() {
        let init = InitSystem::detect();
        if cfg!(target_os = "linux") {
            assert!(
                init == InitSystem::Systemd || init == InitSystem::Bare,
                "Linux should detect systemd or bare, got: {init}"
            );
        }
    }

    #[test]
    fn init_system_display() {
        assert_eq!(InitSystem::Systemd.to_string(), "systemd");
        assert_eq!(InitSystem::Launchd.to_string(), "launchd");
        assert_eq!(InitSystem::WindowsSCM.to_string(), "windows-scm");
        assert_eq!(InitSystem::Bare.to_string(), "bare");
    }

    #[test]
    fn init_system_units_support() {
        assert!(InitSystem::Systemd.supports_units());
        assert!(InitSystem::Launchd.supports_units());
        assert!(InitSystem::WindowsSCM.supports_units());
        assert!(!InitSystem::Bare.supports_units());
    }

    #[test]
    fn crash_loop_action_display() {
        assert_eq!(CrashLoopAction::Disabled.to_string(), "disabled");
        assert_eq!(CrashLoopAction::Logged.to_string(), "logged");
        assert_eq!(
            CrashLoopAction::FailedToDisable.to_string(),
            "failed-to-disable"
        );
    }

    #[test]
    fn crash_loop_report_empty() {
        let report = CrashLoopReport {
            loops: vec![],
            threshold: 5,
            scanned: 10,
        };
        assert!(!report.has_loops());
        assert_eq!(report.disabled_count(), 0);
    }

    #[test]
    fn crash_loop_report_with_entries() {
        let report = CrashLoopReport {
            loops: vec![
                CrashLoopEntry {
                    unit: "nestgate-membrane.service".into(),
                    restart_count: 17920,
                    sub_state: "failed".into(),
                    action: CrashLoopAction::Disabled,
                },
                CrashLoopEntry {
                    unit: "biomeos-beacon.service".into(),
                    restart_count: 11161,
                    sub_state: "activating".into(),
                    action: CrashLoopAction::FailedToDisable,
                },
            ],
            threshold: 5,
            scanned: 15,
        };
        assert!(report.has_loops());
        assert_eq!(report.disabled_count(), 1);
    }

    #[test]
    fn crash_loop_report_serialization() {
        let report = CrashLoopReport {
            loops: vec![CrashLoopEntry {
                unit: "test.service".into(),
                restart_count: 100,
                sub_state: "failed".into(),
                action: CrashLoopAction::Disabled,
            }],
            threshold: 5,
            scanned: 1,
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"action\":\"disabled\""));
        let parsed: CrashLoopReport = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.loops[0].restart_count, 100);
    }
}
