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
}
