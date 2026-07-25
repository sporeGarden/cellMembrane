use super::*;

fn sample_config() -> GatewayConfig {
    GatewayConfig {
        gate_name: "sporeGate".into(),
        enabled: true,
        max_connections: 100,
        default_timeout_secs: 30,
        routes: vec![
            GatewayRoute {
                host: "lab.primals.eco".into(),
                path_prefix: "/hub".into(),
                capability: "jupyter".into(),
                timeout_secs: 30,
            },
            GatewayRoute {
                host: "lab.primals.eco".into(),
                path_prefix: "/user".into(),
                capability: "jupyter".into(),
                timeout_secs: 30,
            },
            GatewayRoute {
                host: "lab.primals.eco".into(),
                path_prefix: "/api".into(),
                capability: "jupyter".into(),
                timeout_secs: 30,
            },
        ],
    }
}

#[test]
fn validate_good_config() {
    let cfg = sample_config();
    assert!(cfg.validate().is_empty());
}

#[test]
fn validate_catches_empty_routes() {
    let cfg = GatewayConfig {
        gate_name: "test".into(),
        enabled: true,
        max_connections: 100,
        default_timeout_secs: 30,
        routes: vec![],
    };
    let errors = cfg.validate();
    assert!(errors.iter().any(|e| e.contains("no routes")));
}

#[test]
fn validate_catches_empty_host() {
    let cfg = GatewayConfig {
        gate_name: "test".into(),
        enabled: true,
        max_connections: 100,
        default_timeout_secs: 30,
        routes: vec![GatewayRoute {
            host: String::new(),
            path_prefix: "/x".into(),
            capability: "cap".into(),
            timeout_secs: 10,
        }],
    };
    let errors = cfg.validate();
    assert!(errors.iter().any(|e| e.contains("host is empty")));
}

#[test]
fn validate_catches_empty_capability() {
    let cfg = GatewayConfig {
        gate_name: "test".into(),
        enabled: true,
        max_connections: 100,
        default_timeout_secs: 30,
        routes: vec![GatewayRoute {
            host: "lab.primals.eco".into(),
            path_prefix: "/x".into(),
            capability: String::new(),
            timeout_secs: 10,
        }],
    };
    let errors = cfg.validate();
    assert!(errors.iter().any(|e| e.contains("capability is empty")));
}

#[test]
fn validate_catches_zero_timeout() {
    let cfg = GatewayConfig {
        gate_name: "test".into(),
        enabled: true,
        max_connections: 100,
        default_timeout_secs: 30,
        routes: vec![GatewayRoute {
            host: "lab.primals.eco".into(),
            path_prefix: "/x".into(),
            capability: "cap".into(),
            timeout_secs: 0,
        }],
    };
    let errors = cfg.validate();
    assert!(
        errors
            .iter()
            .any(|e| e.contains("timeout_secs must be > 0"))
    );
}

#[test]
fn validate_catches_zero_max_connections() {
    let cfg = GatewayConfig {
        gate_name: "test".into(),
        enabled: true,
        max_connections: 0,
        default_timeout_secs: 30,
        routes: vec![GatewayRoute {
            host: "lab.primals.eco".into(),
            path_prefix: "/x".into(),
            capability: "cap".into(),
            timeout_secs: 10,
        }],
    };
    let errors = cfg.validate();
    assert!(errors.iter().any(|e| e.contains("max_connections")));
}

#[test]
fn routes_for_host_filters() {
    let cfg = sample_config();
    let routes = cfg.routes_for_host("lab.primals.eco");
    assert_eq!(routes.len(), 3);
    assert!(cfg.routes_for_host("other.host").is_empty());
}

#[test]
fn probe_result_ok() {
    let p = ProbeResult::ok(200, 15, 4096);
    assert!(p.is_ok());
    assert_eq!(p.status, 200);
    assert_eq!(p.latency_ms, 15);
}

#[test]
fn probe_result_err() {
    let p = ProbeResult::err("connection refused");
    assert!(!p.is_ok());
    assert_eq!(p.status, 0);
    assert_eq!(p.error.as_deref(), Some("connection refused"));
}

#[test]
fn shadow_comparison_passes_when_matching() {
    let cmp = ShadowComparison {
        url: "https://lab.primals.eco/hub/login".into(),
        legacy: ProbeResult::ok(200, 50, 8192),
        tower: ProbeResult::ok(200, 12, 8192),
        match_status: true,
    };
    assert!(cmp.passes());
}

#[test]
fn shadow_comparison_fails_on_mismatch() {
    let cmp = ShadowComparison {
        url: "https://lab.primals.eco/hub/login".into(),
        legacy: ProbeResult::ok(200, 50, 8192),
        tower: ProbeResult::ok(502, 12, 0),
        match_status: false,
    };
    assert!(!cmp.passes());
}

#[test]
fn shadow_comparison_fails_on_error() {
    let cmp = ShadowComparison {
        url: "https://lab.primals.eco/hub/login".into(),
        legacy: ProbeResult::ok(200, 50, 8192),
        tower: ProbeResult::err("connection refused"),
        match_status: false,
    };
    assert!(!cmp.passes());
}

#[test]
fn shadow_report_computes_pass_rate() {
    let comparisons = vec![
        ShadowComparison {
            url: "/hub".into(),
            legacy: ProbeResult::ok(200, 10, 100),
            tower: ProbeResult::ok(200, 5, 100),
            match_status: true,
        },
        ShadowComparison {
            url: "/api".into(),
            legacy: ProbeResult::ok(200, 10, 200),
            tower: ProbeResult::ok(500, 5, 0),
            match_status: false,
        },
    ];
    let report = ShadowReport::from_comparisons(comparisons);
    assert!(!report.all_pass);
    assert!((report.pass_rate - 0.5).abs() < f64::EPSILON);
}

#[test]
fn shadow_report_all_pass() {
    let comparisons = vec![ShadowComparison {
        url: "/hub".into(),
        legacy: ProbeResult::ok(200, 10, 100),
        tower: ProbeResult::ok(200, 5, 100),
        match_status: true,
    }];
    let report = ShadowReport::from_comparisons(comparisons);
    assert!(report.all_pass);
    assert!((report.pass_rate - 1.0).abs() < f64::EPSILON);
}

#[test]
fn shadow_report_empty_is_not_pass() {
    let report = ShadowReport::from_comparisons(vec![]);
    assert!(!report.all_pass);
    assert!((report.pass_rate - 0.0).abs() < f64::EPSILON);
}

#[test]
fn gateway_config_serde_roundtrip() {
    let cfg = sample_config();
    let json = serde_json::to_string(&cfg).unwrap();
    let parsed: GatewayConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.gate_name, "sporeGate");
    assert_eq!(parsed.routes.len(), 3);
}

#[test]
fn gateway_config_toml_roundtrip() {
    let cfg = sample_config();
    let toml_str = toml::to_string_pretty(&cfg).unwrap();
    let parsed: GatewayConfig = toml::from_str(&toml_str).unwrap();
    assert_eq!(parsed.gate_name, "sporeGate");
    assert_eq!(parsed.routes.len(), 3);
    assert_eq!(parsed.routes[0].host, "lab.primals.eco");
    assert_eq!(parsed.routes[0].capability, "jupyter");
    assert_eq!(parsed.max_connections, 100);
    assert_eq!(parsed.default_timeout_secs, 30);
}

#[test]
fn tls_config_toml_roundtrip() {
    let cfg = TlsGatewayConfig {
        bind: "0.0.0.0:443".into(),
        domains: vec!["lab.primals.eco".into()],
        acme_directory: "https://acme-v02.api.letsencrypt.org/directory".into(),
        acme_contacts: vec!["mailto:ops@primals.eco".into()],
        challenge_port: 80,
        songbird_socket: "/run/songbird/songbird.sock".into(),
        data_dir: "/var/lib/beardog".into(),
    };
    let toml_str = toml::to_string_pretty(&cfg).unwrap();
    let parsed: TlsGatewayConfig = toml::from_str(&toml_str).unwrap();
    assert_eq!(parsed.bind, "0.0.0.0:443");
    assert_eq!(parsed.domains, vec!["lab.primals.eco"]);
    assert_eq!(parsed.challenge_port, 80);
}

#[test]
fn tls_config_serde_roundtrip() {
    let cfg = TlsGatewayConfig {
        bind: "0.0.0.0:443".into(),
        domains: vec!["lab.primals.eco".into()],
        acme_directory: "https://acme-v02.api.letsencrypt.org/directory".into(),
        acme_contacts: vec!["mailto:ops@primals.eco".into()],
        challenge_port: 80,
        songbird_socket: "/run/songbird/songbird.sock".into(),
        data_dir: "/var/lib/beardog".into(),
    };
    let json = serde_json::to_string(&cfg).unwrap();
    let parsed: TlsGatewayConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.bind, "0.0.0.0:443");
    assert_eq!(parsed.domains, vec!["lab.primals.eco"]);
}

#[test]
fn gateway_health_serde() {
    let health = GatewayHealth {
        tls_listening: true,
        mesh_connected: true,
        active_routes: 3,
        cert_status: vec![CertExpiry {
            domain: "lab.primals.eco".into(),
            days_remaining: 60,
            valid: true,
        }],
        backends_reachable: vec![BackendStatus {
            capability: "jupyter".into(),
            gate: "ironGate".into(),
            reachable: true,
            latency_ms: Some(1),
        }],
    };
    let json = serde_json::to_string(&health).unwrap();
    let parsed: GatewayHealth = serde_json::from_str(&json).unwrap();
    assert!(parsed.tls_listening);
    assert_eq!(parsed.active_routes, 3);
}

#[test]
fn tls_config_validate_valid() {
    let cfg = TlsGatewayConfig {
        bind: "0.0.0.0:443".into(),
        domains: vec!["lab.primals.eco".into()],
        acme_directory: "https://acme-v02.api.letsencrypt.org/directory".into(),
        acme_contacts: vec!["mailto:ops@primals.eco".into()],
        challenge_port: 80,
        songbird_socket: "/run/songbird/songbird.sock".into(),
        data_dir: "/var/lib/beardog".into(),
    };
    assert!(cfg.validate().is_empty());
}

#[test]
fn tls_config_validate_empty_fields() {
    let cfg = TlsGatewayConfig {
        bind: String::new(),
        domains: vec![],
        acme_directory: String::new(),
        acme_contacts: vec![],
        challenge_port: 80,
        songbird_socket: String::new(),
        data_dir: String::new(),
    };
    let errors = cfg.validate();
    assert!(
        errors.len() >= 5,
        "expected at least 5 errors, got: {errors:?}"
    );
    assert!(errors.iter().any(|e| e.contains("bind")));
    assert!(errors.iter().any(|e| e.contains("domains")));
    assert!(errors.iter().any(|e| e.contains("acme_directory")));
    assert!(errors.iter().any(|e| e.contains("acme_contacts")));
    assert!(errors.iter().any(|e| e.contains("songbird_socket")));
    assert!(errors.iter().any(|e| e.contains("data_dir")));
}

#[test]
fn tls_config_validate_partial() {
    let cfg = TlsGatewayConfig {
        bind: "0.0.0.0:443".into(),
        domains: vec!["lab.primals.eco".into()],
        acme_directory: String::new(),
        acme_contacts: vec!["mailto:ops@primals.eco".into()],
        challenge_port: 80,
        songbird_socket: "/run/songbird/songbird.sock".into(),
        data_dir: "/var/lib/beardog".into(),
    };
    let errors = cfg.validate();
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("acme_directory"));
}

// ── Tower Shadow Tests ────────────────────────────────────────────

fn sample_probe(transport: &str, latency_us: u64, throughput_bps: u64) -> TransportProbe {
    TransportProbe {
        transport: transport.into(),
        latency_us,
        throughput_bps,
        jitter_us: latency_us / 10,
        samples: 10,
        error: None,
    }
}

#[test]
fn transport_probe_ok_when_no_error() {
    let p = sample_probe("wireguard", 1000, 100_000);
    assert!(p.is_ok());
}

#[test]
fn transport_probe_not_ok_with_error() {
    let p = TransportProbe {
        error: Some("timeout".into()),
        ..sample_probe("tower", 0, 0)
    };
    assert!(!p.is_ok());
}

#[test]
fn gate_pair_tower_exceeds_when_faster() {
    let pair = GatePairShadow {
        from_gate: "sporeGate".into(),
        to_gate: "flockGate".into(),
        to_ip: "10.13.37.6".into(),
        wireguard: sample_probe("wireguard", 1000, 50_000),
        tower: sample_probe("tower", 993, 99_000),
        latency_ratio: 0.993,
        throughput_ratio: 1.98,
    };
    assert!(pair.tower_exceeds());
}

#[test]
fn gate_pair_tower_regressed_when_slower() {
    let pair = GatePairShadow {
        from_gate: "sporeGate".into(),
        to_gate: "eastGate".into(),
        to_ip: "10.13.37.5".into(),
        wireguard: sample_probe("wireguard", 1000, 100_000),
        tower: sample_probe("tower", 2000, 50_000),
        latency_ratio: 2.0,
        throughput_ratio: 0.5,
    };
    assert!(!pair.tower_exceeds());
}

#[test]
fn tower_shadow_report_all_exceed() {
    let pairs = vec![GatePairShadow {
        from_gate: "sporeGate".into(),
        to_gate: "flockGate".into(),
        to_ip: "10.13.37.6".into(),
        wireguard: sample_probe("wireguard", 1000, 50_000),
        tower: sample_probe("tower", 993, 99_000),
        latency_ratio: 0.993,
        throughput_ratio: 1.98,
    }];
    let report = TowerShadowReport::from_pairs(
        "sporeGate".into(),
        "150w".into(),
        "2026-07-23T10:00:00Z".into(),
        pairs,
    );
    assert_eq!(report.verdict, "EXCEEDS");
    assert_eq!(report.tower_exceeds_count, 1);
    assert_eq!(report.total_pairs, 1);
}

#[test]
fn tower_shadow_report_no_data() {
    let report = TowerShadowReport::from_pairs(
        "sporeGate".into(),
        "150w".into(),
        "2026-07-23T10:00:00Z".into(),
        vec![],
    );
    assert_eq!(report.verdict, "NO_DATA");
}

#[test]
fn tower_shadow_report_serialization() {
    let report = TowerShadowReport::from_pairs(
        "sporeGate".into(),
        "150w".into(),
        "2026-07-23T10:00:00Z".into(),
        vec![],
    );
    let json = serde_json::to_string_pretty(&report).unwrap();
    assert!(json.contains("\"source_gate\": \"sporeGate\""));
    assert!(json.contains("\"verdict\": \"NO_DATA\""));
    let parsed: TowerShadowReport = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.source_gate, "sporeGate");
}
