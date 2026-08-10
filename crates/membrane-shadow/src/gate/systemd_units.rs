// SPDX-License-Identifier: AGPL-3.0-or-later

//! Systemd unit generation for cascade timers and Tower gateway services.
//!
//! Extracted from `nucleus.rs` to keep the NUCLEUS lifecycle orchestrator
//! focused on primal startup while this module handles systemd unit templates.

// ── Quorum cascade timer ────────────────────────────────────────────

/// Options for cascade timer generation.
pub(crate) struct CascadeTimerOpts<'a> {
    pub interval_minutes: u32,
    pub gate_name: &'a str,
    /// Include `--with-rebuild` so stale primals auto-build after cascade.
    pub with_rebuild: bool,
    /// Include `--with-push` so rebuilt binaries are pushed to the depot server.
    pub with_push: bool,
}

/// Generate systemd timer + service units for autonomous cascade.
///
/// Runs `membrane temporal.cascade` periodically so the gate converges
/// without human intervention. Uses the manifest `default_source` for
/// the `--source` flag (falls back to `temporal`). This is Quorum Phase 1:
/// the gate autonomously pulls all ecosystem repos on a schedule.
///
/// When `with_rebuild` is set, appends `--with-rebuild` so stale primals
/// are rebuilt after each cascade. When `with_push` is also set, rebuilt
/// binaries are pushed to the remote depot server.
///
/// The timer uses `OnCalendar` with `RandomizedDelaySec` to avoid
/// thundering-herd across gates.
pub(crate) fn generate_cascade_timer(opts: &CascadeTimerOpts<'_>) -> (String, String) {
    let install_base = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_INSTALL_BASE,
        cellmembrane_types::service::DEFAULT_INSTALL_BASE,
    );

    let source = crate::temporal::resolve_workspace_root()
        .ok()
        .and_then(|r| crate::manifest::load_from_workspace(&r).ok())
        .map_or_else(cellmembrane_types::CascadeSource::default, |m| {
            m.sync.default_source
        });

    let cascade_timeout = cellmembrane_types::service::DEFAULT_CASCADE_TIMEOUT_SECS;
    let cascade_jitter = cellmembrane_types::service::DEFAULT_CASCADE_JITTER_SECS;

    let mut extra_flags = String::new();
    if opts.with_rebuild {
        extra_flags.push_str(" --with-rebuild");
    }
    if opts.with_push {
        extra_flags.push_str(" --with-push");
    }

    let gate_name = opts.gate_name;
    let interval_minutes = opts.interval_minutes;

    let service = format!(
        r"[Unit]
Description=Membrane Autonomous Cascade ({gate_name})
After=network-online.target
Wants=network-online.target

[Service]
Type=oneshot
ExecStart={install_base}/membrane temporal.cascade --source {source}{extra_flags}
Environment=MEMBRANE_GATE_NAME={gate_name}
TimeoutStartSec={cascade_timeout}
StandardOutput=journal
StandardError=journal
"
    );

    let timer = format!(
        r"[Unit]
Description=Membrane Cascade Timer ({gate_name}) — Quorum Phase 1

[Timer]
OnCalendar=*:0/{interval_minutes}
RandomizedDelaySec={cascade_jitter}
Persistent=true

[Install]
WantedBy=timers.target
"
    );

    (service, timer)
}

/// Install the cascade timer units and enable the timer.
pub fn install_cascade_timer(opts: &CascadeTimerOpts<'_>, dry_run: bool) -> super::BootstrapPhase {
    let interval_minutes = opts.interval_minutes;
    let gate_name = opts.gate_name;

    if dry_run {
        let mut detail =
            format!("dry-run: would install membrane-cascade.timer (every {interval_minutes}m)");
        if opts.with_rebuild {
            detail.push_str(" +rebuild");
        }
        if opts.with_push {
            detail.push_str(" +push");
        }
        return super::BootstrapPhase {
            name: "quorum.cascade-timer".into(),
            ok: true,
            detail,
        };
    }

    let (service_content, timer_content) = generate_cascade_timer(opts);
    let unit_dir = cellmembrane_types::service::resolve_systemd_unit_dir();
    let systemd_dir = std::path::Path::new(&unit_dir);

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

    if !super::nucleus::systemctl(&["daemon-reload"]) {
        tracing::warn!("systemctl daemon-reload failed");
    }
    let enable_ok = super::nucleus::systemctl(&["enable", "--now", "membrane-cascade.timer"]);

    let mut detail =
        format!("membrane-cascade.timer installed (every {interval_minutes}m, gate={gate_name})");
    if opts.with_rebuild {
        detail.push_str(" +rebuild");
    }
    if opts.with_push {
        detail.push_str(" +push");
    }

    super::BootstrapPhase {
        name: "quorum.cascade-timer".into(),
        ok: enable_ok,
        detail,
    }
}

// ── Forgejo GC timer ────────────────────────────────────────────

/// Generate systemd timer + service units for weekly Forgejo repo maintenance.
///
/// Runs `git gc --aggressive` on all Forgejo-managed repos to compact objects
/// and reduce disk footprint on the golgi pepti relay.
pub(crate) fn generate_forgejo_gc_timer() -> (String, String) {
    let forgejo_data = format!(
        "{}/gitea-repositories",
        cellmembrane_types::service::DEFAULT_FORGEJO_DATA_DIR
    );

    let service = format!(
        "[Unit]\n\
         Description=Forgejo Repository GC (weekly)\n\
         After=forgejo.service\n\
         \n\
         [Service]\n\
         Type=oneshot\n\
         ExecStart=/bin/bash -c 'for repo in {forgejo_data}/*/*; do [ -d \"$repo\" ] && \
         git -C \"$repo\" gc --aggressive --prune=now 2>&1; done'\n\
         User=git\n\
         TimeoutStartSec=3600\n\
         StandardOutput=journal\n\
         StandardError=journal\n"
    );

    let timer = r"[Unit]
Description=Forgejo Repository GC Timer — weekly maintenance

[Timer]
OnCalendar=Sun *-*-* 04:00:00
RandomizedDelaySec=1800
Persistent=true

[Install]
WantedBy=timers.target
"
    .to_string();

    (service, timer)
}

/// Install the Forgejo GC timer on this gate.
pub fn install_forgejo_gc_timer(dry_run: bool) -> super::BootstrapPhase {
    if dry_run {
        return super::BootstrapPhase {
            name: "forgejo.gc-timer".into(),
            ok: true,
            detail: "dry-run: would install membrane-forgejo-gc.timer (weekly Sunday 04:00)".into(),
        };
    }

    let (service_content, timer_content) = generate_forgejo_gc_timer();
    let unit_dir = cellmembrane_types::service::resolve_systemd_unit_dir();
    let systemd_dir = std::path::Path::new(&unit_dir);

    let service_path = systemd_dir.join("membrane-forgejo-gc.service");
    let timer_path = systemd_dir.join("membrane-forgejo-gc.timer");

    let write_ok = std::fs::write(&service_path, &service_content).is_ok()
        && std::fs::write(&timer_path, &timer_content).is_ok();

    if !write_ok {
        return super::BootstrapPhase {
            name: "forgejo.gc-timer".into(),
            ok: false,
            detail: "failed to write Forgejo GC systemd units".into(),
        };
    }

    if !super::nucleus::systemctl(&["daemon-reload"]) {
        tracing::warn!("systemctl daemon-reload failed for Forgejo GC timer");
    }
    let enable_ok = super::nucleus::systemctl(&["enable", "--now", "membrane-forgejo-gc.timer"]);

    super::BootstrapPhase {
        name: "forgejo.gc-timer".into(),
        ok: enable_ok,
        detail: "membrane-forgejo-gc.timer installed (weekly Sunday 04:00)".into(),
    }
}

// ── Tower gateway systemd units ──────────────────────────────────

/// Parameters for Tower HTTP gateway systemd unit generation.
pub(crate) struct GatewayUnitParams<'a> {
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

/// Generate the mesh relay gateway systemd unit.
///
/// The mesh relay (songBird) acts as the mesh router — it listens for
/// `capability.call` IPC and routes to the correct backend. The `http.proxy`
/// method enables it to also serve as a reverse proxy.
#[must_use]
pub(super) fn generate_songbird_unit(params: &GatewayUnitParams<'_>) -> String {
    use std::fmt::Write as _;

    let relay_binary = cellmembrane_types::MembraneService::binary_for(
        cellmembrane_types::ServiceCapability::MeshRelay,
    );
    let mut env_lines = format!("Environment=MEMBRANE_GATE_NAME={}\n", params.gate_name);
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
         Description={relay_binary} mesh hub ({gate})\n\
         After=network-online.target\n\
         Wants=network-online.target\n\n\
         [Service]\n\
         Type=simple\n\
         UMask={umask}\n\
         ExecStart={base}/{relay_binary} server --socket {socket} --bind {bind_all} --port {federation_port}\n\
         {env_lines}\
         Restart=on-failure\n\
         RestartSec={restart_delay}\n\
         StartLimitIntervalSec={start_limit_interval}\n\
         StartLimitBurst={start_limit_burst}\n\
         RuntimeDirectory={rtd}\n\
         RuntimeDirectoryMode={rtd_mode}\n\
         RuntimeDirectoryPreserve=yes\n\n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        gate = params.gate_name,
        base = params.install_base,
        socket = params.songbird_socket,
        umask = cellmembrane_types::service::DEFAULT_SERVICE_UMASK,
        rtd = cellmembrane_types::service::DEFAULT_RUNTIME_DIRECTORY,
        rtd_mode = cellmembrane_types::service::DEFAULT_RUNTIME_DIRECTORY_MODE,
        restart_delay = cellmembrane_types::service::DEFAULT_RESTART_DELAY_SECS,
        start_limit_interval = cellmembrane_types::service::DEFAULT_START_LIMIT_INTERVAL_SECS,
        start_limit_burst = cellmembrane_types::service::DEFAULT_START_LIMIT_BURST,
    )
}

/// Generate the crypto-signer ACME gateway systemd unit.
///
/// The crypto signer (bearDog) handles TLS termination on :443 and proxies to
/// the mesh relay's `http.proxy` method. It manages ACME certificate renewal
/// via HTTP-01.
#[must_use]
fn generate_beardog_unit(params: &GatewayUnitParams<'_>) -> String {
    let crypto_binary = cellmembrane_types::MembraneService::binary_for(
        cellmembrane_types::ServiceCapability::CryptoSigner,
    );
    let relay_svc = cellmembrane_types::MembraneService::require_capability(
        cellmembrane_types::ServiceCapability::MeshRelay,
    );
    let relay_gateway_unit = format!("{}-gateway.service", relay_svc.binary);

    format!(
        "[Unit]\n\
         Description={crypto_binary} ACME gateway ({gate})\n\
         After=network-online.target {relay_gateway_unit}\n\
         Wants=network-online.target\n\
         Requires={relay_gateway_unit}\n\n\
         [Service]\n\
         Type=simple\n\
         ExecStart={base}/{crypto_binary} serve-https \
         --upstream {socket} \
         --bind {bind}\n\
         Environment=MEMBRANE_GATE_NAME={gate}\n\
         Restart=on-failure\n\
         RestartSec={restart_delay}\n\
         StartLimitIntervalSec={start_limit_interval}\n\
         StartLimitBurst={start_limit_burst}\n\
         AmbientCapabilities=CAP_NET_BIND_SERVICE\n\n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        gate = params.gate_name,
        base = params.install_base,
        socket = params.songbird_socket,
        bind = params.gateway_bind,
        restart_delay = cellmembrane_types::service::DEFAULT_RESTART_DELAY_SECS,
        start_limit_interval = cellmembrane_types::service::DEFAULT_START_LIMIT_INTERVAL_SECS,
        start_limit_burst = cellmembrane_types::service::DEFAULT_START_LIMIT_BURST,
    )
}

/// Generate both gateway units (songBird + bearDog) as a tuple.
#[must_use]
pub(crate) fn generate_gateway_units(params: &GatewayUnitParams<'_>) -> (String, String) {
    (
        generate_songbird_unit(params),
        generate_beardog_unit(params),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_opts(gate: &str) -> CascadeTimerOpts<'_> {
        CascadeTimerOpts {
            interval_minutes: cellmembrane_types::service::DEFAULT_CASCADE_INTERVAL_MINUTES,
            gate_name: gate,
            with_rebuild: false,
            with_push: false,
        }
    }

    #[test]
    fn cascade_timer_generates_valid_units() {
        let opts = test_opts("golgi");
        let (service, timer) = generate_cascade_timer(&opts);
        assert!(service.contains("[Unit]"));
        assert!(service.contains("[Service]"));
        assert!(service.contains("temporal.cascade"));
        assert!(service.contains("golgi"));
        assert!(service.contains("Type=oneshot"));
        assert!(!service.contains("--with-rebuild"));
        assert!(!service.contains("--with-push"));

        assert!(timer.contains("[Timer]"));
        assert!(timer.contains("OnCalendar=*:0/15"));
        assert!(timer.contains(&format!(
            "RandomizedDelaySec={}",
            cellmembrane_types::service::DEFAULT_CASCADE_JITTER_SECS
        )));
        assert!(timer.contains("Persistent=true"));
        assert!(timer.contains("timers.target"));
    }

    #[test]
    fn cascade_timer_custom_interval() {
        let mut opts = test_opts("sporeGate");
        opts.interval_minutes = 30;
        let (_, timer) = generate_cascade_timer(&opts);
        assert!(timer.contains("OnCalendar=*:0/30"));
        assert!(timer.contains("sporeGate"));
    }

    #[test]
    fn cascade_timer_with_rebuild_and_push() {
        let opts = CascadeTimerOpts {
            interval_minutes: 15,
            gate_name: "sporeGate",
            with_rebuild: true,
            with_push: true,
        };
        let (service, _) = generate_cascade_timer(&opts);
        assert!(
            service.contains("--with-rebuild"),
            "should include --with-rebuild flag"
        );
        assert!(
            service.contains("--with-push"),
            "should include --with-push flag"
        );
    }

    #[test]
    fn cascade_timer_dry_run() {
        let opts = test_opts("test-gate");
        let phase = install_cascade_timer(&opts, true);
        assert!(phase.ok);
        assert_eq!(phase.name, "quorum.cascade-timer");
        assert!(phase.detail.contains("dry-run"));
        assert!(phase.detail.contains("15m"));
    }

    #[test]
    fn cascade_timer_dry_run_with_flags() {
        let opts = CascadeTimerOpts {
            interval_minutes: 15,
            gate_name: "test-gate",
            with_rebuild: true,
            with_push: true,
        };
        let phase = install_cascade_timer(&opts, true);
        assert!(phase.ok);
        assert!(phase.detail.contains("+rebuild"));
        assert!(phase.detail.contains("+push"));
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
        assert!(unit.contains("Restart=on-failure"));
        assert!(unit.contains("StartLimitBurst=10"));
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
