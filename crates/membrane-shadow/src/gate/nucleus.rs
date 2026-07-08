// SPDX-License-Identifier: AGPL-3.0-or-later

//! NUCLEUS service management — systemd unit generation, secrets, and startup.
//!
//! Extracted from `bootstrap.rs` to keep the bootstrap orchestrator focused on
//! phase coordination while this module handles systemd unit installation,
//! secret generation, and primal service lifecycle.

use super::BootstrapPhase;

/// Run a `systemctl` subcommand. Returns `true` if it exits 0.
fn systemctl(args: &[&str]) -> bool {
    std::process::Command::new("systemctl")
        .args(args)
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Async variant for use in tokio contexts (e.g. cascade-restart).
pub async fn systemctl_async(args: &[&str]) -> bool {
    tokio::process::Command::new("systemctl")
        .args(args)
        .output()
        .await
        .is_ok_and(|o| o.status.success())
}

/// Start all NUCLEUS primals — generate secrets, write systemd units, enable+start.
pub(super) fn start_nucleus_primals(arch: &str) -> (bool, String) {
    let config_dir = generate_secrets_env();

    let install_base = super::resolve_install_base();
    let dest_root = super::resolve_plasmidbin_dir();
    let bin_dir = dest_root.join("primals").join(arch);
    let paths = cellmembrane_types::service::ServicePaths::from_env();
    let systemd_dir = std::path::Path::new(cellmembrane_types::service::SYSTEMD_UNIT_DIR);

    if let Err(e) = std::fs::create_dir_all(std::path::Path::new(
        cellmembrane_types::service::DEFAULT_SOCKET_BASE,
    )) {
        tracing::warn!(error = %e, "failed to create socket base directory");
    }

    let security_binary = cellmembrane_types::MembraneService::binary_for(
        cellmembrane_types::ServiceCapability::CryptoSigner,
    );
    let Some(crypto_svc) = cellmembrane_types::MembraneService::with_capability(
        cellmembrane_types::ServiceCapability::CryptoSigner,
    ) else {
        return (
            false,
            "CryptoSigner capability not found in service registry".into(),
        );
    };
    let security_socket = paths.socket_path(crypto_svc).unwrap_or_else(|| {
        format!(
            "{}/{security_binary}.sock",
            cellmembrane_types::service::DEFAULT_SOCKET_BASE
        )
    });

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
        let unit_path = systemd_dir.join(format!("{}-membrane.service", svc.binary));

        if std::fs::write(&unit_path, &unit_content).is_ok() {
            installed += 1;
        } else {
            failed += 1;
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
            let unit = format!("{}-membrane.service", svc.binary);
            if !systemctl(&["enable", "--now", &unit]) {
                tracing::warn!(unit = %unit, "systemctl enable failed");
            }
        }
    }

    if installed == 0 && failed == 0 {
        return (true, "no primal binaries found in depot — skipped".into());
    }

    let ok = failed == 0 && installed > 0;
    (ok, format!("{installed} units installed, {failed} failed"))
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
    let (ok, detail) = start_nucleus_primals(arch);
    BootstrapPhase {
        name: "nucleus.start".into(),
        ok,
        detail,
    }
}

// ── Quorum cascade timer ────────────────────────────────────────────

/// Generate systemd timer + service units for autonomous cascade.
///
/// Runs `membrane temporal.cascade --source forgejo` periodically so the
/// gate converges without human intervention. This is Quorum Phase 1:
/// the gate autonomously pulls all ecosystem repos on a schedule.
///
/// The timer uses `OnCalendar` with `RandomizedDelaySec` to avoid
/// thundering-herd across gates.
pub fn generate_cascade_timer(interval_minutes: u32, gate_name: &str) -> (String, String) {
    let install_base = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_INSTALL_BASE,
        cellmembrane_types::service::DEFAULT_INSTALL_BASE,
    );

    let service = format!(
        r"[Unit]
Description=Membrane Autonomous Cascade ({gate_name})
After=network-online.target
Wants=network-online.target

[Service]
Type=oneshot
ExecStart={install_base}/membrane temporal.cascade --source forgejo
Environment=GATE_NAME={gate_name}
TimeoutStartSec=300
StandardOutput=journal
StandardError=journal
"
    );

    let timer = format!(
        r"[Unit]
Description=Membrane Cascade Timer ({gate_name}) — Quorum Phase 1

[Timer]
OnCalendar=*:0/{interval_minutes}
RandomizedDelaySec=60
Persistent=true

[Install]
WantedBy=timers.target
"
    );

    (service, timer)
}

/// Install the cascade timer units and enable the timer.
pub fn install_cascade_timer(
    interval_minutes: u32,
    gate_name: &str,
    dry_run: bool,
) -> super::BootstrapPhase {
    if dry_run {
        return super::BootstrapPhase {
            name: "quorum.cascade-timer".into(),
            ok: true,
            detail: format!(
                "dry-run: would install membrane-cascade.timer (every {interval_minutes}m)"
            ),
        };
    }

    let (service_content, timer_content) = generate_cascade_timer(interval_minutes, gate_name);
    let systemd_dir = std::path::Path::new(cellmembrane_types::service::SYSTEMD_UNIT_DIR);

    let service_path = systemd_dir.join("membrane-cascade.service");
    let timer_path = systemd_dir.join("membrane-cascade.timer");

    let write_ok = std::fs::write(&service_path, &service_content).is_ok()
        && std::fs::write(&timer_path, &timer_content).is_ok();

    if !write_ok {
        return super::BootstrapPhase {
            name: "quorum.cascade-timer".into(),
            ok: false,
            detail: "failed to write systemd units".into(),
        };
    }

    if !systemctl(&["daemon-reload"]) {
        tracing::warn!("systemctl daemon-reload failed");
    }
    let enable_ok = systemctl(&["enable", "--now", "membrane-cascade.timer"]);

    super::BootstrapPhase {
        name: "quorum.cascade-timer".into(),
        ok: enable_ok,
        detail: format!(
            "membrane-cascade.timer installed (every {interval_minutes}m, gate={gate_name})"
        ),
    }
}

// ── Tower gateway systemd units ──────────────────────────────────

/// Parameters for Tower HTTP gateway systemd unit generation.
pub struct GatewayUnitParams<'a> {
    pub gate_name: &'a str,
    pub install_base: &'a str,
    pub songbird_socket: &'a str,
    pub gateway_bind: &'a str,
    pub proxy_routes: &'a str,
}

impl<'a> GatewayUnitParams<'a> {
    /// Create params with defaults from constants, requiring only the gate name.
    #[must_use]
    pub const fn for_gate(gate_name: &'a str) -> Self {
        Self {
            gate_name,
            install_base: cellmembrane_types::service::DEFAULT_INSTALL_BASE,
            songbird_socket: cellmembrane_types::service::DEFAULT_SONGBIRD_SOCKET,
            gateway_bind: cellmembrane_types::service::DEFAULT_GATEWAY_BIND,
            proxy_routes: "",
        }
    }
}

/// Generate the songBird gateway systemd unit.
///
/// songBird acts as the mesh router — it listens for `capability.call` IPC
/// and routes to the correct backend. The `http.proxy` method enables it to
/// also serve as a reverse proxy (replacing Caddy's routing role).
#[must_use]
pub fn generate_songbird_unit(params: &GatewayUnitParams<'_>) -> String {
    use std::fmt::Write as _;

    let mut env_lines = format!("Environment=GATE_NAME={}\n", params.gate_name);
    if !params.proxy_routes.is_empty() {
        let _ = writeln!(
            env_lines,
            "Environment={}={}",
            cellmembrane_types::service::ENV_SONGBIRD_PROXY_ROUTES,
            params.proxy_routes,
        );
    }

    let federation_port = cellmembrane_types::service::DEFAULT_FEDERATION_PORT;
    let bind_all = cellmembrane_types::service::BIND_ALL;

    format!(
        "[Unit]\n\
         Description=songBird mesh hub ({gate})\n\
         After=network-online.target\n\
         Wants=network-online.target\n\n\
         [Service]\n\
         Type=simple\n\
         ExecStart={base}/songbird server --socket {socket} --bind {bind_all} --port {federation_port}\n\
         {env_lines}\
         Restart=always\n\
         RestartSec=3\n\
         RuntimeDirectory=membrane\n\
         RuntimeDirectoryPreserve=yes\n\n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        gate = params.gate_name,
        base = params.install_base,
        socket = params.songbird_socket,
    )
}

/// Generate the bearDog ACME gateway systemd unit.
///
/// bearDog handles TLS termination on :443 and proxies to songBird's
/// `http.proxy` method. It manages ACME certificate renewal via HTTP-01.
#[must_use]
pub fn generate_beardog_unit(params: &GatewayUnitParams<'_>) -> String {
    format!(
        "[Unit]\n\
         Description=bearDog ACME gateway ({gate})\n\
         After=network-online.target songbird-gateway.service\n\
         Wants=network-online.target\n\
         Requires=songbird-gateway.service\n\n\
         [Service]\n\
         Type=simple\n\
         ExecStart={base}/beardog serve-https \
         --upstream {socket} \
         --bind {bind}\n\
         Environment=GATE_NAME={gate}\n\
         Restart=always\n\
         RestartSec=5\n\
         AmbientCapabilities=CAP_NET_BIND_SERVICE\n\n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        gate = params.gate_name,
        base = params.install_base,
        socket = params.songbird_socket,
        bind = params.gateway_bind,
    )
}

/// Generate both gateway units (songBird + bearDog) as a tuple.
#[must_use]
pub fn generate_gateway_units(params: &GatewayUnitParams<'_>) -> (String, String) {
    (
        generate_songbird_unit(params),
        generate_beardog_unit(params),
    )
}

// ── Secrets generation ──────────────────────────────────────────────

fn generate_secrets_env() -> String {
    use std::io::Write as _;
    use std::os::unix::fs::PermissionsExt;

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
        tracing::warn!("failed to read /dev/urandom — secrets.env not generated");
        return config_dir;
    };
    let content = format!("NESTGATE_JWT_SECRET={secret}\n");
    if let Ok(mut f) = std::fs::File::create(&env_file) {
        if let Err(e) = f.write_all(content.as_bytes()) {
            tracing::warn!(error = %e, "failed to write secrets.env");
        }
    }
    if let Err(e) = std::fs::set_permissions(&env_file, std::fs::Permissions::from_mode(0o600)) {
        tracing::warn!(error = %e, "failed to set secrets.env permissions");
    }
    config_dir
}

/// Read `n` bytes from `/dev/urandom` and return as hex string.
fn csprng_hex(n: usize) -> Option<String> {
    use std::io::Read as _;
    let mut buf = vec![0u8; n];
    std::fs::File::open("/dev/urandom")
        .ok()?
        .read_exact(&mut buf)
        .ok()?;
    let mut hex = String::with_capacity(n * 2);
    for b in &buf {
        use std::fmt::Write;
        let _ = write!(hex, "{b:02x}");
    }
    Some(hex)
}

// ── Systemd unit generation ─────────────────────────────────────────

/// Resolve extra CLI args for a primal's systemd `ExecStart`, based on capability.
fn extra_exec_args(svc: &cellmembrane_types::MembraneService) -> String {
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
fn generate_unit_content(
    svc: &cellmembrane_types::MembraneService,
    exec_start: &str,
    extra_args: &str,
    config_dir: &str,
) -> String {
    let content_binary = cellmembrane_types::MembraneService::binary_for(
        cellmembrane_types::ServiceCapability::ContentServing,
    );
    let env_file_line = if svc.binary == content_binary {
        format!("EnvironmentFile=-{config_dir}/secrets.env\n")
    } else {
        String::new()
    };

    format!(
        "[Unit]\n\
         Description={binary} primal (membrane NUCLEUS)\n\
         After=network.target\n\n\
         [Service]\n\
         Type=simple\n\
         {env_file_line}\
         ExecStart={exec_start}{extra_args}\n\
         Restart=on-failure\n\
         RestartSec=3\n\
         RuntimeDirectory=membrane\n\
         RuntimeDirectoryPreserve=yes\n\n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        binary = svc.binary,
    )
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
    fn cascade_timer_generates_valid_units() {
        let interval = cellmembrane_types::service::DEFAULT_CASCADE_INTERVAL_MINUTES;
        let (service, timer) = generate_cascade_timer(interval, "golgi");
        assert!(service.contains("[Unit]"));
        assert!(service.contains("[Service]"));
        assert!(service.contains("temporal.cascade"));
        assert!(service.contains("golgi"));
        assert!(service.contains("Type=oneshot"));

        assert!(timer.contains("[Timer]"));
        assert!(timer.contains("OnCalendar=*:0/15"));
        assert!(timer.contains("RandomizedDelaySec=60"));
        assert!(timer.contains("Persistent=true"));
        assert!(timer.contains("timers.target"));
    }

    #[test]
    fn cascade_timer_custom_interval() {
        let (_, timer) = generate_cascade_timer(30, "sporeGate");
        assert!(timer.contains("OnCalendar=*:0/30"));
        assert!(timer.contains("sporeGate"));
    }

    #[test]
    fn cascade_timer_dry_run() {
        let phase = install_cascade_timer(15, "test-gate", true);
        assert!(phase.ok);
        assert_eq!(phase.name, "quorum.cascade-timer");
        assert!(phase.detail.contains("dry-run"));
        assert!(phase.detail.contains("15m"));
    }

    #[test]
    fn songbird_unit_has_systemd_sections() {
        let params = GatewayUnitParams::for_gate("sporeGate");
        let unit = generate_songbird_unit(&params);
        assert!(unit.contains("[Unit]"));
        assert!(unit.contains("[Service]"));
        assert!(unit.contains("[Install]"));
        assert!(unit.contains("sporeGate"));
        assert!(unit.contains("songbird server"));
        assert!(unit.contains("--port 7700"));
        assert!(unit.contains("Restart=always"));
    }

    #[test]
    fn songbird_unit_includes_proxy_routes() {
        let mut params = GatewayUnitParams::for_gate("sporeGate");
        params.proxy_routes = "lab.primals.eco/hub=jupyter,lab.primals.eco/api=jupyter";
        let unit = generate_songbird_unit(&params);
        assert!(
            unit.contains("SONGBIRD_PROXY_ROUTES=lab.primals.eco/hub=jupyter"),
            "should embed proxy routes env, got: {unit}"
        );
    }

    #[test]
    fn songbird_unit_omits_routes_when_empty() {
        let params = GatewayUnitParams::for_gate("test");
        let unit = generate_songbird_unit(&params);
        assert!(
            !unit.contains("SONGBIRD_PROXY_ROUTES"),
            "empty routes should not emit env var"
        );
    }

    #[test]
    fn beardog_unit_has_systemd_sections() {
        let params = GatewayUnitParams::for_gate("sporeGate");
        let unit = generate_beardog_unit(&params);
        assert!(unit.contains("[Unit]"));
        assert!(unit.contains("[Service]"));
        assert!(unit.contains("[Install]"));
        assert!(unit.contains("sporeGate"));
        assert!(unit.contains("beardog serve-https"));
        assert!(unit.contains("--upstream"));
        assert!(unit.contains("CAP_NET_BIND_SERVICE"));
    }

    #[test]
    fn beardog_unit_requires_songbird() {
        let params = GatewayUnitParams::for_gate("sporeGate");
        let unit = generate_beardog_unit(&params);
        assert!(
            unit.contains("Requires=songbird-gateway.service"),
            "bearDog should depend on songBird"
        );
        assert!(
            unit.contains("After=network-online.target songbird-gateway.service"),
            "bearDog should start after songBird"
        );
    }

    #[test]
    fn generate_gateway_units_returns_both() {
        let params = GatewayUnitParams::for_gate("sporeGate");
        let (songbird, beardog) = generate_gateway_units(&params);
        assert!(songbird.contains("songbird server"));
        assert!(beardog.contains("beardog serve-https"));
    }

    #[test]
    fn gateway_unit_params_defaults() {
        let params = GatewayUnitParams::for_gate("eastGate");
        assert_eq!(params.gate_name, "eastGate");
        assert_eq!(
            params.install_base,
            cellmembrane_types::service::DEFAULT_INSTALL_BASE
        );
        assert_eq!(
            params.songbird_socket,
            cellmembrane_types::service::DEFAULT_SONGBIRD_SOCKET
        );
        assert_eq!(
            params.gateway_bind,
            cellmembrane_types::service::DEFAULT_GATEWAY_BIND
        );
        assert!(params.proxy_routes.is_empty());
    }
}
