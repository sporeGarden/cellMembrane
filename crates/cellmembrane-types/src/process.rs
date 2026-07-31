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
    ///
    /// Honors `MEMBRANE_INIT_SCOPE` env override for user-space deploy:
    /// `bare` forces bare-process management (no systemd/launchd), enabling
    /// deployment on read-only rootfs (`SteamOS`), non-root users, or
    /// environments where system services are unavailable.
    #[must_use]
    pub fn detect() -> Self {
        if let Ok(scope) = std::env::var(crate::service::ENV_INIT_SCOPE) {
            match scope.to_lowercase().as_str() {
                "bare" => return Self::Bare,
                "systemd" => return Self::Systemd,
                "launchd" => return Self::Launchd,
                _ => {}
            }
        }

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

// ── Service specification ─────────────────────────────────────────────

/// Platform-agnostic service configuration — the unified input for all
/// init-system backends (systemd units, launchd plists, Windows SCM,
/// bare process manifests).
///
/// Built from `MembraneService` + `ServerContract` + gate profile overrides.
/// Renderers consume this to produce platform-specific config files.
#[derive(Debug, Clone)]
pub struct ServiceSpec {
    /// Binary name (e.g. "songbird", "beardog").
    pub binary: String,
    /// Canonical systemd unit name from the service registry.
    pub systemd_unit: String,
    /// Human-readable description.
    pub description: String,
    /// Full `ExecStart` command line.
    pub exec_start: String,
    /// Extra CLI arguments appended to `exec_start`.
    pub extra_args: String,
    /// Environment variables (key=value pairs).
    pub environment: Vec<(String, String)>,
    /// Path to an environment file (optional, e.g. `secrets.env`).
    pub env_file: Option<String>,
    /// Restart policy.
    pub restart_policy: RestartPolicy,
    /// Services this unit should start after (systemd `After=`).
    pub after: Vec<String>,
    /// Working directory (optional).
    pub working_directory: Option<String>,
    /// `UMask` for socket accessibility.
    pub umask: String,
    /// Runtime directory name (e.g. "membrane").
    pub runtime_directory: Option<String>,
    /// Runtime directory mode (e.g. "0755").
    pub runtime_directory_mode: String,
}

/// Restart policy for managed services.
#[derive(Debug, Clone)]
pub struct RestartPolicy {
    /// When to restart: "on-failure", "always", "no".
    pub condition: String,
    /// Seconds to wait before restarting.
    pub restart_sec: u32,
    /// Window for burst detection (seconds).
    pub start_limit_interval_sec: u32,
    /// Max restarts within the interval before giving up.
    pub start_limit_burst: u32,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self {
            condition: "on-failure".into(),
            restart_sec: 5,
            start_limit_interval_sec: 120,
            start_limit_burst: 10,
        }
    }
}

impl ServiceSpec {
    /// Build a `ServiceSpec` from a `MembraneService` registry entry.
    #[must_use]
    pub fn from_membrane_service(
        svc: &crate::MembraneService,
        install_base: &str,
        socket_base: &str,
        security_socket: &str,
        config_dir: &str,
    ) -> Self {
        let paths = crate::service::ServicePaths::new(install_base, socket_base);
        let socket_path = paths
            .socket_path(svc)
            .unwrap_or_else(|| format!("{socket_base}/{}.sock", svc.binary));
        let exec_start = svc.server_contract.exec_args_with_base(
            install_base,
            svc.binary,
            &socket_path,
            security_socket,
        );

        let content_binary =
            crate::MembraneService::binary_for(crate::ServiceCapability::ContentServing);
        let env_file = if svc.binary == content_binary {
            Some(format!("{config_dir}/secrets.env"))
        } else {
            None
        };

        let mut after = vec!["network.target".to_string()];
        let nucleus_spec = crate::MembraneComposition::Nucleus.spec();
        for dep_unit in nucleus_spec.systemd_after_deps(svc.binary) {
            after.push(dep_unit.to_string());
        }

        Self {
            binary: svc.binary.to_string(),
            systemd_unit: svc.systemd_unit.to_string(),
            description: format!("{} primal (membrane NUCLEUS)", svc.binary),
            exec_start,
            extra_args: String::new(),
            environment: Vec::new(),
            env_file,
            restart_policy: RestartPolicy::default(),
            after,
            working_directory: None,
            umask: crate::service::DEFAULT_SERVICE_UMASK.into(),
            runtime_directory: Some(crate::service::DEFAULT_RUNTIME_DIRECTORY.into()),
            runtime_directory_mode: crate::service::DEFAULT_RUNTIME_DIRECTORY_MODE.into(),
        }
    }

    /// Render as a systemd unit file.
    #[must_use]
    pub fn to_systemd_unit(&self) -> String {
        let after = self.after.join(" ");
        let env_file_line = self
            .env_file
            .as_ref()
            .map_or(String::new(), |f| format!("EnvironmentFile=-{f}\n"));

        let mut env_lines = String::new();
        for (k, v) in &self.environment {
            use std::fmt::Write;
            let _ = writeln!(env_lines, "Environment={k}={v}");
        }

        let rtd = self.runtime_directory.as_ref().map_or(String::new(), |d| {
            format!(
                "RuntimeDirectory={d}\nRuntimeDirectoryMode={}\nRuntimeDirectoryPreserve=yes\n",
                self.runtime_directory_mode
            )
        });

        let wd = self
            .working_directory
            .as_ref()
            .map_or(String::new(), |d| format!("WorkingDirectory={d}\n"));

        format!(
            "[Unit]\n\
             Description={desc}\n\
             After={after}\n\n\
             [Service]\n\
             Type=simple\n\
             UMask={umask}\n\
             {env_file_line}\
             {env_lines}\
             {wd}\
             ExecStart={exec}{extra}\n\
             Restart={restart}\n\
             RestartSec={restart_sec}\n\
             StartLimitIntervalSec={sli}\n\
             StartLimitBurst={slb}\n\
             {rtd}\n\
             [Install]\n\
             WantedBy={wanted_by}\n",
            desc = self.description,
            umask = self.umask,
            exec = self.exec_start,
            extra = self.extra_args,
            restart = self.restart_policy.condition,
            restart_sec = self.restart_policy.restart_sec,
            sli = self.restart_policy.start_limit_interval_sec,
            slb = self.restart_policy.start_limit_burst,
            wanted_by = Self::resolve_wanted_by(),
        )
    }

    /// User-space deploy uses `default.target`; system scope uses `multi-user.target`.
    fn resolve_wanted_by() -> &'static str {
        if let Ok(scope) = std::env::var(crate::service::ENV_INIT_SCOPE) {
            if scope == "user" || scope == "bare" {
                return "default.target";
            }
        }
        "multi-user.target"
    }

    /// Render as a systemd drop-in override file (`override.conf`).
    ///
    /// Only includes sections that differ from the base unit. Callers
    /// specify which fields to override via the builder pattern on the spec.
    #[must_use]
    pub fn to_systemd_override(&self) -> String {
        let mut sections = Vec::new();

        if !self.environment.is_empty() || self.env_file.is_some() {
            let mut service_lines = Vec::new();
            if let Some(f) = &self.env_file {
                service_lines.push(format!("EnvironmentFile=-{f}"));
            }
            for (k, v) in &self.environment {
                service_lines.push(format!("Environment={k}={v}"));
            }
            sections.push(format!("[Service]\n{}", service_lines.join("\n")));
        }

        sections.join("\n\n")
    }

    /// Render as a launchd plist (XML).
    #[must_use]
    pub fn to_launchd_plist(&self) -> String {
        let mut args: Vec<&str> = self.exec_start.split_whitespace().collect();
        let program = if args.is_empty() { "" } else { args.remove(0) };

        let mut arg_entries = String::new();
        for a in &args {
            use std::fmt::Write;
            let _ = writeln!(arg_entries, "    <string>{a}</string>");
        }

        let mut env_entries = String::new();
        for (k, v) in &self.environment {
            use std::fmt::Write;
            let _ = writeln!(env_entries, "    <key>{k}</key>");
            let _ = writeln!(env_entries, "    <string>{v}</string>");
        }

        let env_section = if env_entries.is_empty() {
            String::new()
        } else {
            format!("  <key>EnvironmentVariables</key>\n  <dict>\n{env_entries}  </dict>\n")
        };

        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
             \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\">\n\
             <dict>\n\
             \x20 <key>Label</key>\n\
             \x20 <string>eco.primals.{binary}</string>\n\
             \x20 <key>Program</key>\n\
             \x20 <string>{program}</string>\n\
             \x20 <key>ProgramArguments</key>\n\
             \x20 <array>\n\
             \x20   <string>{program}</string>\n\
             {arg_entries}\
             \x20 </array>\n\
             {env_section}\
             \x20 <key>RunAtLoad</key>\n\
             \x20 <true/>\n\
             \x20 <key>KeepAlive</key>\n\
             \x20 <true/>\n\
             </dict>\n\
             </plist>\n",
            binary = self.binary,
        )
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
