// SPDX-License-Identifier: AGPL-3.0-or-later

//! NUCLEUS service management — cross-platform primal lifecycle.
//!
//! Dispatches on [`InitSystem::detect()`] to select the service management
//! strategy: systemd unit generation on Linux, bare process spawn on Windows,
//! macOS, containers, and other platforms. Secrets generation and service
//! registry resolution are platform-agnostic.

use super::BootstrapPhase;

/// Run a `systemctl` subcommand. Returns `true` if it exits 0.
///
/// On non-systemd platforms, returns `false` with a trace-level warning.
pub(crate) fn systemctl(args: &[&str]) -> bool {
    if !matches!(
        cellmembrane_types::InitSystem::detect(),
        cellmembrane_types::InitSystem::Systemd
    ) {
        tracing::trace!(args = ?args, "systemctl skipped — not on systemd");
        return false;
    }
    std::process::Command::new("systemctl")
        .args(args)
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Async variant for use in tokio contexts (e.g. cascade-restart).
///
/// On non-systemd platforms, returns `false` with a trace-level warning.
pub(crate) async fn systemctl_async(args: &[&str]) -> bool {
    if !matches!(
        cellmembrane_types::InitSystem::detect(),
        cellmembrane_types::InitSystem::Systemd
    ) {
        tracing::trace!(args = ?args, "systemctl_async skipped — not on systemd");
        return false;
    }
    tokio::process::Command::new("systemctl")
        .args(args)
        .output()
        .await
        .is_ok_and(|o| o.status.success())
}

/// Start all NUCLEUS primals — dispatches on [`InitSystem::detect()`].
///
/// - **Systemd**: generate secrets, write unit files, `systemctl enable --now`
/// - **Bare / Windows / macOS**: generate secrets, spawn processes, write PID files
pub(super) fn start_nucleus_primals(arch: &str) -> super::ProbeResult {
    let init = cellmembrane_types::InitSystem::detect();
    tracing::info!(init_system = %init, "starting NUCLEUS primals");

    match init {
        cellmembrane_types::InitSystem::Systemd => start_nucleus_systemd(arch),
        _ => start_nucleus_bare(arch, init),
    }
}

/// Systemd path: write unit files, daemon-reload, enable+start.
fn start_nucleus_systemd(arch: &str) -> super::ProbeResult {
    let config_dir = generate_secrets_env();

    let install_base = super::resolve_install_base();
    let dest_root = super::resolve_plasmidbin_dir();
    let bin_dir = dest_root.join("primals").join(arch);
    let paths = cellmembrane_types::service::ServicePaths::from_env();
    let systemd_dir = std::path::Path::new(cellmembrane_types::service::SYSTEMD_UNIT_DIR);

    prepare_socket_base();

    let security_socket = resolve_security_socket(&paths);

    let services = cellmembrane_types::MembraneService::all();
    let mut installed = 0u32;
    let mut failed = 0u32;

    for svc in services {
        if !svc.is_primal || !bin_dir.join(svc.binary).exists() {
            continue;
        }

        let socket_path = paths.socket_path(svc).unwrap_or_else(|| {
            format!(
                "{}/{}.sock",
                cellmembrane_types::service::DEFAULT_SOCKET_BASE,
                svc.binary
            )
        });
        let exec_start = svc.server_contract.exec_args_with_base(
            &install_base,
            svc.binary,
            &socket_path,
            &security_socket,
        );
        let extra_args = extra_exec_args(svc);
        let unit_content = generate_unit_content(svc, &exec_start, &extra_args, &config_dir);
        let unit_path = systemd_dir.join(svc.systemd_unit);

        if let Err(e) = std::fs::write(&unit_path, &unit_content) {
            tracing::warn!(
                service = %svc.binary,
                path = %unit_path.display(),
                error = %e,
                "systemd unit write failed"
            );
            failed += 1;
        } else {
            installed += 1;
        }
    }

    if installed > 0 {
        if !systemctl(&["daemon-reload"]) {
            tracing::warn!("systemctl daemon-reload failed");
        }

        for svc in services {
            if !svc.is_primal || !bin_dir.join(svc.binary).exists() {
                continue;
            }
            if !systemctl(&["enable", "--now", svc.systemd_unit]) {
                tracing::warn!(unit = %svc.systemd_unit, "systemctl enable failed");
            }
        }
    }

    if installed == 0 && failed == 0 {
        return super::ProbeResult::pass("no primal binaries found in depot — skipped");
    }

    let ok = failed == 0 && installed > 0;
    super::ProbeResult {
        ok,
        detail: format!("{installed} units installed, {failed} failed (systemd)"),
    }
}

/// Bare process path: spawn each primal binary directly, write PID files.
///
/// Used on Windows (SCM not yet implemented), macOS (launchd not yet
/// implemented), containers without systemd, and other non-systemd platforms.
/// PID files are written to `{install_base}/pids/` for lifecycle management.
fn start_nucleus_bare(arch: &str, init: cellmembrane_types::InitSystem) -> super::ProbeResult {
    let config_dir = generate_secrets_env();

    let install_base = super::resolve_install_base();
    let dest_root = super::resolve_plasmidbin_dir();
    let bin_dir = dest_root.join("primals").join(arch);
    let paths = cellmembrane_types::service::ServicePaths::from_env();
    let pid_dir = std::path::Path::new(&install_base).join("pids");

    if let Err(e) = std::fs::create_dir_all(&pid_dir) {
        tracing::warn!(error = %e, "failed to create PID directory");
    }

    prepare_socket_base();

    let security_socket = resolve_security_socket(&paths);

    let env_file = std::path::Path::new(&config_dir).join("secrets.env");
    let env_vars = load_env_file(&env_file);

    let services = cellmembrane_types::MembraneService::all();
    let mut started = 0u32;
    let mut failed = 0u32;

    for svc in services {
        let bin_path = bin_dir.join(svc.binary);
        if !svc.is_primal || !bin_path.exists() {
            continue;
        }

        let socket_path = paths.socket_path(svc).unwrap_or_else(|| {
            format!(
                "{}/{}.sock",
                cellmembrane_types::service::DEFAULT_SOCKET_BASE,
                svc.binary
            )
        });
        let exec_start = svc.server_contract.exec_args_with_base(
            &install_base,
            svc.binary,
            &socket_path,
            &security_socket,
        );
        let extra_args = extra_exec_args(svc);
        let full_cmd = format!("{exec_start}{extra_args}");

        let parts: Vec<&str> = full_cmd.split_whitespace().collect();
        let Some((&program, args)) = parts.split_first() else {
            failed += 1;
            continue;
        };

        let mut cmd = std::process::Command::new(program);
        cmd.args(args);
        for (k, v) in &env_vars {
            cmd.env(k, v);
        }

        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }

        match cmd.spawn() {
            Ok(child) => {
                let pid = child.id();
                let pid_file = pid_dir.join(format!("{}.pid", svc.binary));
                if let Err(e) = std::fs::write(&pid_file, pid.to_string()) {
                    tracing::warn!(service = %svc.binary, error = %e, "PID file write failed");
                }
                tracing::info!(service = %svc.binary, pid, "spawned (bare)");
                started += 1;
            }
            Err(e) => {
                tracing::warn!(service = %svc.binary, error = %e, "spawn failed");
                failed += 1;
            }
        }
    }

    if started == 0 && failed == 0 {
        return super::ProbeResult::pass("no primal binaries found in depot — skipped");
    }

    let ok = failed == 0 && started > 0;
    super::ProbeResult {
        ok,
        detail: format!("{started} processes spawned, {failed} failed ({init})"),
    }
}

/// Create the socket base directory with appropriate permissions.
fn prepare_socket_base() {
    let socket_base = std::path::Path::new(cellmembrane_types::service::DEFAULT_SOCKET_BASE);
    if let Err(e) = std::fs::create_dir_all(socket_base) {
        tracing::warn!(error = %e, "failed to create socket base directory");
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        if let Err(e) = std::fs::set_permissions(socket_base, perms) {
            tracing::warn!(error = %e, "failed to set socket base directory permissions");
        }
    }
}

/// Resolve the crypto signer socket path from the service registry.
pub(crate) fn resolve_security_socket(paths: &cellmembrane_types::service::ServicePaths) -> String {
    let security_binary = cellmembrane_types::MembraneService::binary_for(
        cellmembrane_types::ServiceCapability::CryptoSigner,
    );
    cellmembrane_types::MembraneService::with_capability(
        cellmembrane_types::ServiceCapability::CryptoSigner,
    )
    .and_then(|svc| paths.socket_path(svc))
    .unwrap_or_else(|| {
        format!(
            "{}/{security_binary}.sock",
            cellmembrane_types::service::DEFAULT_SOCKET_BASE
        )
    })
}

/// Stop a bare-process primal by reading its PID file and sending SIGTERM (Unix)
/// or `taskkill` (Windows). Cleans up the PID file afterward.
pub(crate) fn stop_bare_process(binary: &str) -> bool {
    let install_base = super::resolve_install_base();
    let pid_dir = std::path::Path::new(&install_base).join("pids");
    let pid_file = pid_dir.join(format!("{binary}.pid"));

    let Ok(pid_str) = std::fs::read_to_string(&pid_file) else {
        return false;
    };
    let Ok(pid) = pid_str.trim().parse::<u32>() else {
        return false;
    };

    let killed = kill_process(pid);
    if killed {
        let _ = std::fs::remove_file(&pid_file);
    }
    killed
}

/// Kill a process by PID — platform-aware.
fn kill_process(pid: u32) -> bool {
    #[cfg(unix)]
    {
        std::process::Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .output()
            .is_ok_and(|o| o.status.success())
    }
    #[cfg(windows)]
    {
        std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .output()
            .is_ok_and(|o| o.status.success())
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        false
    }
}

/// Restart a bare-process primal: stop then re-spawn from the current binary.
pub(crate) async fn restart_bare_process(binary: &str, arch: &str) -> bool {
    stop_bare_process(binary);
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let install_base = super::resolve_install_base();
    let dest_root = super::resolve_plasmidbin_dir();
    let bin_dir = dest_root.join("primals").join(arch);
    let bin_path = bin_dir.join(binary);

    if !bin_path.exists() {
        return false;
    }

    let paths = cellmembrane_types::service::ServicePaths::from_env();
    let Some(svc) = cellmembrane_types::MembraneService::for_binary(binary) else {
        return false;
    };
    let security_socket = resolve_security_socket(&paths);
    let socket_path = paths.socket_path(svc).unwrap_or_else(|| {
        format!(
            "{}/{}.sock",
            cellmembrane_types::service::DEFAULT_SOCKET_BASE,
            svc.binary
        )
    });
    let exec_start = svc.server_contract.exec_args_with_base(
        &install_base,
        svc.binary,
        &socket_path,
        &security_socket,
    );
    let extra_args = extra_exec_args(svc);
    let full_cmd = format!("{exec_start}{extra_args}");

    let parts: Vec<&str> = full_cmd.split_whitespace().collect();
    let Some((&program, args)) = parts.split_first() else {
        return false;
    };

    let mut cmd = std::process::Command::new(program);
    cmd.args(args);

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }

    match cmd.spawn() {
        Ok(child) => {
            let pid_dir = std::path::Path::new(&install_base).join("pids");
            let pid_file = pid_dir.join(format!("{}.pid", svc.binary));
            let _ = std::fs::write(&pid_file, child.id().to_string());
            tracing::info!(service = %binary, pid = child.id(), "respawned (bare)");
            true
        }
        Err(e) => {
            tracing::warn!(service = %binary, error = %e, "respawn failed");
            false
        }
    }
}

/// Load key=value pairs from a secrets env file (best-effort).
fn load_env_file(path: &std::path::Path) -> Vec<(String, String)> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    content
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (k, v) = line.split_once('=')?;
            Some((k.trim().to_string(), v.trim().to_string()))
        })
        .collect()
}

/// Construct the nucleus startup phase.
pub(super) fn nucleus_phase(arch: &str, dry_run: bool) -> BootstrapPhase {
    if dry_run {
        return BootstrapPhase {
            name: "nucleus.start".into(),
            ok: true,
            detail: "dry-run: would start NUCLEUS primals".into(),
        };
    }
    let probe = start_nucleus_primals(arch);
    BootstrapPhase {
        name: "nucleus.start".into(),
        ok: probe.ok,
        detail: probe.detail,
    }
}

// ── Secrets generation ──────────────────────────────────────────────

fn generate_secrets_env() -> String {
    use std::io::Write as _;

    let config_dir = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_CONFIG_DIR,
        cellmembrane_types::service::DEFAULT_CONFIG_DIR,
    );
    let env_dir = std::path::Path::new(&config_dir);
    if let Err(e) = std::fs::create_dir_all(env_dir) {
        tracing::warn!(error = %e, "failed to create config directory for secrets");
    }
    let env_file = env_dir.join("secrets.env");
    if env_file.exists() {
        return config_dir;
    }

    let Some(secret) = csprng_hex(64) else {
        tracing::warn!("CSPRNG failed — secrets.env not generated");
        return config_dir;
    };
    let content = format!("NESTGATE_JWT_SECRET={secret}\n");
    if let Ok(mut f) = std::fs::File::create(&env_file) {
        if let Err(e) = f.write_all(content.as_bytes()) {
            tracing::warn!(error = %e, "failed to write secrets.env");
        }
    }
    set_restricted_permissions(&env_file);
    config_dir
}

/// Set owner-only permissions on a sensitive file.
///
/// Unix: `chmod 0o600`. Windows: `icacls` to restrict to current user.
/// Other: trace log only.
fn set_restricted_permissions(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
            tracing::warn!(path = %path.display(), error = %e, "failed to set restricted permissions");
        }
    }
    #[cfg(windows)]
    {
        let path_str = path.display().to_string();
        let result = std::process::Command::new("icacls")
            .args([&path_str, "/inheritance:r", "/grant:r", "%USERNAME%:F"])
            .output();
        if let Err(e) = result {
            tracing::warn!(path = %path_str, error = %e, "icacls failed");
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        tracing::trace!(path = %path.display(), "restricted permissions: no platform method");
        let _ = path;
    }
}

/// Generate `n` cryptographically random bytes and return as hex string.
///
/// Platform-aware — uses `getrandom` crate which delegates to the OS
/// CSPRNG on all platforms (`urandom` on Linux, `BCryptGenRandom` on Windows,
/// `SecRandomCopyBytes` on macOS/iOS, etc.).
fn csprng_hex(n: usize) -> Option<String> {
    let mut buf = vec![0u8; n];
    getrandom::fill(&mut buf).ok()?;
    let mut hex = String::with_capacity(n * 2);
    for b in &buf {
        use std::fmt::Write;
        let _ = write!(hex, "{b:02x}");
    }
    Some(hex)
}

// ── Systemd unit generation ─────────────────────────────────────────

/// Resolve extra CLI args for a primal's systemd `ExecStart`, based on capability.
pub(crate) fn extra_exec_args(svc: &cellmembrane_types::MembraneService) -> String {
    let relay_binary = cellmembrane_types::MembraneService::binary_for(
        cellmembrane_types::ServiceCapability::MeshRelay,
    );
    let content_binary = cellmembrane_types::MembraneService::binary_for(
        cellmembrane_types::ServiceCapability::ContentServing,
    );
    let identity_binary = cellmembrane_types::MembraneService::binary_for(
        cellmembrane_types::ServiceCapability::Identity,
    );

    if svc.binary == relay_binary {
        format!(
            " --federation-port {} --bind {}",
            cellmembrane_types::service::DEFAULT_FEDERATION_PORT,
            cellmembrane_types::service::BIND_ALL,
        )
    } else if svc.binary == content_binary {
        let port = cellmembrane_types::MembraneService::for_binary(content_binary)
            .and_then(|s| s.port)
            .unwrap_or(cellmembrane_types::service::DEFAULT_FEDERATION_PORT);
        format!(
            " --port {} --bind {}",
            port,
            cellmembrane_types::service::BIND_LOOPBACK,
        )
    } else if svc.binary == identity_binary {
        format!(
            " --http-address {}:0",
            cellmembrane_types::service::BIND_LOOPBACK,
        )
    } else {
        String::new()
    }
}

/// Generate the systemd unit file content for a NUCLEUS primal.
///
/// Delegates to `ServiceSpec::to_systemd_unit()` — the unified cross-platform
/// service config model (J6). The `extra_args` are appended to the exec line.
pub(crate) fn generate_unit_content(
    svc: &cellmembrane_types::MembraneService,
    exec_start: &str,
    extra_args: &str,
    config_dir: &str,
) -> String {
    let content_binary = cellmembrane_types::MembraneService::binary_for(
        cellmembrane_types::ServiceCapability::ContentServing,
    );
    let env_file = if svc.binary == content_binary {
        Some(format!("{config_dir}/secrets.env"))
    } else {
        None
    };

    let spec = cellmembrane_types::ServiceSpec {
        binary: svc.binary.to_string(),
        systemd_unit: svc.systemd_unit.to_string(),
        description: format!("{} primal (membrane NUCLEUS)", svc.binary),
        exec_start: exec_start.to_string(),
        extra_args: extra_args.to_string(),
        environment: Vec::new(),
        env_file,
        restart_policy: cellmembrane_types::RestartPolicy::default(),
        after: vec!["network.target".into()],
        working_directory: None,
        umask: cellmembrane_types::service::DEFAULT_SERVICE_UMASK.into(),
        runtime_directory: Some("membrane".into()),
        runtime_directory_mode: cellmembrane_types::service::DEFAULT_RUNTIME_DIRECTORY_MODE.into(),
    };
    spec.to_systemd_unit()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellmembrane_types::{MembraneService, ServiceCapability};

    #[test]
    fn extra_exec_args_relay_contains_federation_port_and_bind_all() {
        let svc = MembraneService::with_capability(ServiceCapability::MeshRelay)
            .expect("MeshRelay must exist in registry");
        let args = extra_exec_args(svc);
        assert!(
            args.contains("--federation-port"),
            "relay should have --federation-port, got: {args}"
        );
        assert!(
            args.contains(cellmembrane_types::service::BIND_ALL),
            "relay should bind 0.0.0.0, got: {args}"
        );
    }

    #[test]
    fn extra_exec_args_content_contains_port_and_loopback() {
        let svc = MembraneService::with_capability(ServiceCapability::ContentServing)
            .expect("ContentServing must exist in registry");
        let args = extra_exec_args(svc);
        assert!(
            args.contains("--port"),
            "content server should have --port, got: {args}"
        );
        assert!(
            args.contains(cellmembrane_types::service::BIND_LOOPBACK),
            "content server should bind loopback, got: {args}"
        );
    }

    #[test]
    fn extra_exec_args_identity_contains_http_address() {
        let svc = MembraneService::with_capability(ServiceCapability::Identity)
            .expect("Identity must exist in registry");
        let args = extra_exec_args(svc);
        assert!(
            args.contains("--http-address"),
            "identity should have --http-address, got: {args}"
        );
        assert!(
            args.contains(cellmembrane_types::service::BIND_LOOPBACK),
            "identity should bind loopback, got: {args}"
        );
    }

    #[test]
    fn extra_exec_args_crypto_signer_is_empty() {
        let svc = MembraneService::with_capability(ServiceCapability::CryptoSigner)
            .expect("CryptoSigner must exist in registry");
        let relay = MembraneService::binary_for(ServiceCapability::MeshRelay);
        let content = MembraneService::binary_for(ServiceCapability::ContentServing);
        let identity = MembraneService::binary_for(ServiceCapability::Identity);
        if svc.binary != relay && svc.binary != content && svc.binary != identity {
            let args = extra_exec_args(svc);
            assert!(args.is_empty(), "crypto signer should have no extra args");
        }
    }

    #[test]
    fn generate_unit_content_has_systemd_sections() {
        let svc = MembraneService::with_capability(ServiceCapability::CryptoSigner)
            .expect("CryptoSigner must exist");
        let content = generate_unit_content(
            svc,
            "/usr/bin/beardog server --socket /run/x",
            "",
            "/etc/membrane",
        );
        assert!(content.contains("[Unit]"), "missing [Unit]");
        assert!(content.contains("[Service]"), "missing [Service]");
        assert!(content.contains("[Install]"), "missing [Install]");
        assert!(content.contains("After=network.target"));
        assert!(content.contains("Restart=on-failure"));
        assert!(content.contains("WantedBy=multi-user.target"));
    }

    #[test]
    fn generate_unit_content_includes_exec_start_and_extra_args() {
        let svc = MembraneService::with_capability(ServiceCapability::MeshRelay)
            .expect("MeshRelay must exist");
        let exec = "/opt/membrane/primals/x86_64/songbird server --socket /run/s.sock";
        let extra = format!(
            " --federation-port {} --bind {}",
            cellmembrane_types::service::DEFAULT_FEDERATION_PORT,
            cellmembrane_types::service::BIND_ALL,
        );
        let content = generate_unit_content(svc, exec, &extra, "/etc/membrane");
        let exec_line = format!("ExecStart={exec}{extra}");
        assert!(
            content.contains(&exec_line),
            "should embed ExecStart with extra args"
        );
    }

    #[test]
    fn generate_unit_content_env_file_only_for_content_serving() {
        let content_svc = MembraneService::with_capability(ServiceCapability::ContentServing)
            .expect("ContentServing must exist");
        let unit = generate_unit_content(content_svc, "/bin/x", "", "/etc/membrane");
        assert!(
            unit.contains("EnvironmentFile"),
            "content serving primal should have EnvironmentFile"
        );

        let crypto_svc = MembraneService::with_capability(ServiceCapability::CryptoSigner)
            .expect("CryptoSigner must exist");
        let unit2 = generate_unit_content(crypto_svc, "/bin/x", "", "/etc/membrane");
        assert!(
            !unit2.contains("EnvironmentFile"),
            "non-content primal should NOT have EnvironmentFile"
        );
    }

    #[test]
    fn generate_unit_content_env_file_uses_config_dir() {
        let content_svc = MembraneService::with_capability(ServiceCapability::ContentServing)
            .expect("ContentServing must exist");
        let unit = generate_unit_content(content_svc, "/bin/x", "", "/custom/config");
        assert!(
            unit.contains("EnvironmentFile=-/custom/config/secrets.env"),
            "env file path should use config_dir, got: {unit}"
        );
    }

    #[test]
    fn generate_unit_content_description_includes_binary_name() {
        let svc = MembraneService::with_capability(ServiceCapability::MeshRelay).unwrap();
        let content = generate_unit_content(svc, "/bin/x", "", "/etc/membrane");
        assert!(
            content.contains(&format!("Description={} primal", svc.binary)),
            "description should include binary name"
        );
    }

    #[test]
    fn generate_unit_content_includes_socket_permissions() {
        let svc = MembraneService::with_capability(ServiceCapability::CryptoSigner)
            .expect("CryptoSigner must exist");
        let content = generate_unit_content(svc, "/bin/x", "", "/etc/membrane");
        assert!(
            content.contains("UMask=0002"),
            "unit should set UMask=0002 for socket accessibility"
        );
        assert!(
            content.contains("RuntimeDirectoryMode=0755"),
            "unit should set RuntimeDirectoryMode=0755"
        );
    }

    #[test]
    fn csprng_hex_produces_correct_length() {
        let hex = csprng_hex(32).expect("/dev/urandom should be readable");
        assert_eq!(hex.len(), 64, "32 bytes should produce 64 hex chars");
        assert!(
            hex.chars().all(|c| c.is_ascii_hexdigit()),
            "output should be hex only, got: {hex}"
        );
    }

    #[test]
    fn csprng_hex_produces_varied_output() {
        let a = csprng_hex(16).unwrap();
        let b = csprng_hex(16).unwrap();
        assert_ne!(a, b, "two CSPRNG reads should differ");
    }

    #[test]
    fn nucleus_phase_dry_run_returns_ok() {
        let phase = nucleus_phase("x86_64-unknown-linux-musl", true);
        assert!(phase.ok, "dry-run should always succeed");
        assert_eq!(phase.name, "nucleus.start");
        assert!(phase.detail.contains("dry-run"));
    }

    #[test]
    fn init_system_detect_is_valid() {
        let init = cellmembrane_types::InitSystem::detect();
        let valid = matches!(
            init,
            cellmembrane_types::InitSystem::Systemd
                | cellmembrane_types::InitSystem::Launchd
                | cellmembrane_types::InitSystem::WindowsSCM
                | cellmembrane_types::InitSystem::Bare
        );
        assert!(
            valid,
            "detect() must return a known init system, got: {init}"
        );
    }

    #[test]
    fn load_env_file_parses_key_value_pairs() {
        let dir = std::env::temp_dir().join(format!("nucleus_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("test.env");
        std::fs::write(&path, "FOO=bar\n# comment\nBAZ=qux\n\n").expect("write");
        let pairs = load_env_file(&path);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0], ("FOO".to_string(), "bar".to_string()));
        assert_eq!(pairs[1], ("BAZ".to_string(), "qux".to_string()));
    }

    #[test]
    fn load_env_file_returns_empty_on_missing() {
        let pairs = load_env_file(std::path::Path::new("/nonexistent/secrets.env"));
        assert!(pairs.is_empty());
    }

    #[test]
    fn resolve_security_socket_returns_valid_path() {
        let paths = cellmembrane_types::service::ServicePaths::from_env();
        let socket = resolve_security_socket(&paths);
        assert!(
            socket.contains(".sock"),
            "should be a .sock path, got: {socket}"
        );
    }

    #[test]
    fn stop_bare_process_returns_false_on_missing_pid() {
        assert!(!stop_bare_process("nonexistent-binary-xyz"));
    }
}
