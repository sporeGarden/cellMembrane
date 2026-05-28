// SPDX-License-Identifier: AGPL-3.0-or-later

//! Tests for `TransportMode`, UDS helpers, health check methods, and composition queries.

use cellmembrane_types::composition::MembraneComposition;
use cellmembrane_types::service::{HealthCheckMethod, MembraneService, TransportMode};

// --- TransportMode ---

#[test]
fn transport_mode_serde_roundtrip() {
    #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
    struct Wrapper {
        mode: TransportMode,
    }

    for mode in [
        TransportMode::UdsOnly,
        TransportMode::TcpDefault,
        TransportMode::TcpOptIn,
    ] {
        let w = Wrapper { mode };
        let toml_str = toml::to_string(&w).unwrap();
        let back: Wrapper = toml::from_str(&toml_str).unwrap();
        assert_eq!(w, back, "TransportMode roundtrip failed for {mode}");
    }
}

#[test]
fn transport_mode_display() {
    assert_eq!(TransportMode::UdsOnly.to_string(), "uds_only");
    assert_eq!(TransportMode::TcpDefault.to_string(), "tcp_default");
    assert_eq!(TransportMode::TcpOptIn.to_string(), "tcp_opt_in");
}

// --- UDS helpers on MembraneService ---

#[test]
fn internal_primals_are_uds_only() {
    // Primals that communicate only internally use UDS-only transport.
    // Songbird (TURN relay) uses TcpOptIn since it needs external reachability.
    for name in [
        "beardog",
        "skunkbat",
        "nestgate",
        "rhizocrypt",
        "loamspine",
        "sweetgrass",
    ] {
        let svc = MembraneService::for_binary(name)
            .unwrap_or_else(|| panic!("Service not found: {name}"));
        assert!(
            svc.is_uds_only(),
            "{name} should be UDS-only (VPS standard)"
        );
    }
}

#[test]
fn songbird_is_tcp_opt_in() {
    let svc = MembraneService::for_binary("songbird").unwrap();
    assert_eq!(
        svc.vps_transport,
        TransportMode::TcpOptIn,
        "Songbird (TURN relay) needs TCP for external clients"
    );
    assert!(!svc.is_uds_only());
}

#[test]
fn symbiotic_services_are_tcp_default() {
    for name in ["hbbs", "hbbr", "caddy"] {
        let svc = MembraneService::for_binary(name)
            .unwrap_or_else(|| panic!("Service not found: {name}"));
        assert!(
            svc.requires_tcp_in_uds_mode(),
            "{name} should require TCP in UDS mode"
        );
        assert_eq!(
            svc.vps_transport,
            TransportMode::TcpDefault,
            "{name} should be TcpDefault"
        );
    }
}

#[test]
fn uds_health_check_returns_socket_exists_for_primals() {
    for name in ["beardog", "skunkbat", "nestgate", "rhizocrypt"] {
        let svc = MembraneService::for_binary(name).unwrap();
        assert_eq!(
            svc.uds_health_check(),
            HealthCheckMethod::SocketExists,
            "{name} should use SocketExists in UDS mode"
        );
    }
}

#[test]
fn uds_health_check_falls_back_for_symbiotic() {
    let hbbs = MembraneService::for_binary("hbbs").unwrap();
    assert_ne!(
        hbbs.uds_health_check(),
        HealthCheckMethod::SocketExists,
        "hbbs is not UDS-only, should fall back to primary health check"
    );
}

// --- Socket paths on primals ---

#[test]
fn all_uds_primals_have_socket_paths() {
    // Only UDS-only primals declare socket paths (not songbird/TcpOptIn).
    for name in [
        "beardog",
        "skunkbat",
        "nestgate",
        "rhizocrypt",
        "loamspine",
        "sweetgrass",
    ] {
        let svc = MembraneService::for_binary(name).unwrap();
        assert!(
            svc.socket_path.is_some(),
            "{name} should declare a socket_path for UDS transport"
        );
        let path = svc.socket_path.unwrap();
        assert!(
            path.starts_with("/run/membrane/"),
            "{name} socket path should be under /run/membrane/, got: {path}"
        );
        assert!(
            std::path::Path::new(path)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("sock")),
            "{name} socket path should end with .sock, got: {path}"
        );
    }
}

// --- Composition-level UDS queries ---

#[test]
fn tower_uds_socket_paths_covers_primals() {
    let spec = MembraneComposition::Tower.spec();
    let paths = spec.uds_socket_paths();

    assert!(
        paths.iter().any(|(bin, _)| *bin == "beardog"),
        "Tower UDS paths should include beardog"
    );
    assert!(
        paths.iter().any(|(bin, _)| *bin == "skunkbat"),
        "Tower UDS paths should include skunkbat"
    );
    // Songbird uses TcpOptIn, not UDS-only — so it's NOT in UDS paths
    assert!(
        !paths.iter().any(|(bin, _)| *bin == "songbird"),
        "Songbird (TcpOptIn) should NOT be in UDS socket paths"
    );

    for (bin, path) in &paths {
        assert!(
            path.starts_with("/run/membrane/"),
            "UDS path for {bin} should be under /run/membrane/"
        );
    }
}

#[test]
fn tower_tcp_ports_uds_mode_includes_ssh() {
    let spec = MembraneComposition::Tower.spec();
    let ports = spec.tcp_ports_uds_mode();
    assert!(
        ports.contains(&22),
        "UDS-mode TCP ports should always include SSH (22)"
    );
}

#[test]
fn tower_tcp_ports_uds_mode_includes_symbiotic_ports() {
    let spec = MembraneComposition::Tower.spec();
    let ports = spec.tcp_ports_uds_mode();

    let hbbs = MembraneService::for_binary("hbbs").unwrap();
    if let Some(hbbs_port) = hbbs.port {
        assert!(
            ports.contains(&hbbs_port),
            "UDS-mode TCP ports should include hbbs port {hbbs_port}"
        );
    }
}

#[test]
fn nest_uds_socket_paths_superset_of_tower() {
    let tower_paths = MembraneComposition::Tower.spec().uds_socket_paths();
    let nest_paths = MembraneComposition::Nest.spec().uds_socket_paths();

    for (bin, _) in &tower_paths {
        assert!(
            nest_paths.iter().any(|(b, _)| b == bin),
            "Nest UDS paths should include Tower primal: {bin}"
        );
    }
    assert!(
        nest_paths.len() > tower_paths.len(),
        "Nest should have more UDS paths than Tower"
    );
}

// --- HealthCheckMethod display ---

#[test]
fn health_check_method_display() {
    assert_eq!(HealthCheckMethod::Liveness.to_string(), "health.liveness");
    assert_eq!(HealthCheckMethod::TcpConnect.to_string(), "tcp_connect");
    assert_eq!(HealthCheckMethod::HttpsProbe.to_string(), "https_probe");
    assert_eq!(HealthCheckMethod::DnsProbe.to_string(), "dns_probe");
    assert_eq!(HealthCheckMethod::SocketExists.to_string(), "socket_exists");
}
