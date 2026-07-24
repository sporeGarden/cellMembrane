// SPDX-License-Identifier: AGPL-3.0-or-later

//! Systemd unit generation for cascade timers and Tower gateway services.
//!
//! Extracted from `nucleus.rs` to keep the NUCLEUS lifecycle orchestrator
//! focused on primal startup while this module handles systemd unit templates.

// ── Quorum cascade timer ────────────────────────────────────────────

/// Generate systemd timer + service units for autonomous cascade.
///
/// Runs `membrane temporal.cascade` periodically so the gate converges
/// without human intervention. Uses the manifest `default_source` for
/// the `--source` flag (falls back to `temporal`). This is Quorum Phase 1:
/// the gate autonomously pulls all ecosystem repos on a schedule.
///
/// The timer uses `OnCalendar` with `RandomizedDelaySec` to avoid
/// thundering-herd across gates.
pub(crate) fn generate_cascade_timer(interval_minutes: u32, gate_name: &str) -> (String, String) {
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

    let service = format!(
        r"[Unit]
Description=Membrane Autonomous Cascade ({gate_name})
After=network-online.target
Wants=network-online.target

[Service]
Type=oneshot
ExecStart={install_base}/membrane temporal.cascade --source {source}
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

    if !super::nucleus::systemctl(&["daemon-reload"]) {
        tracing::warn!("systemctl daemon-reload failed");
    }
    let enable_ok = super::nucleus::systemctl(&["enable", "--now", "membrane-cascade.timer"]);

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

/// Generate the songBird gateway systemd unit.
///
/// songBird acts as the mesh router — it listens for `capability.call` IPC
/// and routes to the correct backend. The `http.proxy` method enables it to
/// also serve as a reverse proxy (replacing Caddy's routing role).
#[must_use]
pub(crate) fn generate_songbird_unit(params: &GatewayUnitParams<'_>) -> String {
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
         UMask={umask}\n\
         ExecStart={base}/songbird server --socket {socket} --bind {bind_all} --port {federation_port}\n\
         {env_lines}\
         Restart=on-failure\n\
         RestartSec=5\n\
         StartLimitIntervalSec=120\n\
         StartLimitBurst=10\n\
         RuntimeDirectory=membrane\n\
         RuntimeDirectoryMode={rtd_mode}\n\
         RuntimeDirectoryPreserve=yes\n\n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        gate = params.gate_name,
        base = params.install_base,
        socket = params.songbird_socket,
        umask = cellmembrane_types::service::DEFAULT_SERVICE_UMASK,
        rtd_mode = cellmembrane_types::service::DEFAULT_RUNTIME_DIRECTORY_MODE,
    )
}

/// Generate the bearDog ACME gateway systemd unit.
///
/// bearDog handles TLS termination on :443 and proxies to songBird's
/// `http.proxy` method. It manages ACME certificate renewal via HTTP-01.
#[must_use]
pub(crate) fn generate_beardog_unit(params: &GatewayUnitParams<'_>) -> String {
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
         Restart=on-failure\n\
         RestartSec=5\n\
         StartLimitIntervalSec=120\n\
         StartLimitBurst=10\n\
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
pub(crate) fn generate_gateway_units(params: &GatewayUnitParams<'_>) -> (String, String) {
    (
        generate_songbird_unit(params),
        generate_beardog_unit(params),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
