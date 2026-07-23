// SPDX-License-Identifier: AGPL-3.0-or-later

//! Tower HTTP gateway types — typed configuration for the sovereign reverse proxy.
//!
//! Defines the route, config, health, and shadow validation types for the
//! Tower HTTP gateway (songBird `http.proxy` + bearDog ACME). This replaces
//! Caddy as the TLS termination + reverse proxy layer.
//!
//! Design:
//! - Routes resolve to mesh capabilities (not static IPs)
//! - bearDog owns :443 with ACME certs, forwards to songBird
//! - songBird resolves `capability.call` to the best compute backend
//! - Shadow validation compares legacy (Caddy) vs Tower responses

use serde::{Deserialize, Serialize};

/// A single gateway route — maps a host+path to a mesh capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayRoute {
    /// Hostname to match (e.g. `"lab.primals.eco"`).
    pub host: String,
    /// Path prefix to match (e.g. `"/hub"`). Empty means all paths.
    pub path_prefix: String,
    /// Mesh capability to route to (e.g. `"jupyter"`, `"compute"`).
    pub capability: String,
    /// Upstream timeout in seconds.
    pub timeout_secs: u32,
}

/// Tower HTTP gateway configuration for a gate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayConfig {
    /// Gate this config applies to.
    pub gate_name: String,
    /// Whether the reverse proxy is enabled.
    pub enabled: bool,
    /// Maximum upstream connections.
    pub max_connections: u32,
    /// Default upstream timeout in seconds.
    pub default_timeout_secs: u32,
    /// Route table.
    pub routes: Vec<GatewayRoute>,
}

impl GatewayConfig {
    /// Validate the config for correctness.
    #[must_use]
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();

        if self.routes.is_empty() {
            errors.push("no routes defined".into());
        }

        for (i, route) in self.routes.iter().enumerate() {
            if route.host.is_empty() {
                errors.push(format!("route[{i}]: host is empty"));
            }
            if route.capability.is_empty() {
                errors.push(format!("route[{i}]: capability is empty"));
            }
            if route.timeout_secs == 0 {
                errors.push(format!("route[{i}]: timeout_secs must be > 0"));
            }
        }

        if self.max_connections == 0 {
            errors.push("max_connections must be > 0".into());
        }

        errors
    }

    /// Find routes matching a given host.
    #[must_use]
    pub fn routes_for_host(&self, host: &str) -> Vec<&GatewayRoute> {
        self.routes.iter().filter(|r| r.host == host).collect()
    }
}

/// TLS configuration for bearDog gateway.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsGatewayConfig {
    /// Bind address (e.g. `"0.0.0.0:443"` or `"0.0.0.0:8443"` for shadow).
    pub bind: String,
    /// Domains to obtain ACME certs for.
    pub domains: Vec<String>,
    /// ACME directory URL.
    pub acme_directory: String,
    /// ACME contact emails.
    pub acme_contacts: Vec<String>,
    /// HTTP-01 challenge listener port.
    pub challenge_port: u16,
    /// songBird socket path for upstream routing.
    pub songbird_socket: String,
    /// Data directory for cert storage.
    pub data_dir: String,
}

impl TlsGatewayConfig {
    /// Validate the TLS gateway config for correctness.
    #[must_use]
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();

        if self.bind.is_empty() {
            errors.push("bind address is empty".into());
        }
        if self.domains.is_empty() {
            errors.push("no domains configured".into());
        }
        if self.acme_directory.is_empty() {
            errors.push("acme_directory is empty".into());
        }
        if self.acme_contacts.is_empty() {
            errors.push("acme_contacts is empty (ACME requires at least one contact)".into());
        }
        if self.songbird_socket.is_empty() {
            errors.push("songbird_socket path is empty".into());
        }
        if self.data_dir.is_empty() {
            errors.push("data_dir is empty".into());
        }

        errors
    }
}

/// Health status of the gateway components.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayHealth {
    /// Whether bearDog TLS is listening.
    pub tls_listening: bool,
    /// Whether songBird mesh is connected.
    pub mesh_connected: bool,
    /// Number of active routes.
    pub active_routes: usize,
    /// ACME certificate domains and their expiry days.
    pub cert_status: Vec<CertExpiry>,
    /// Upstream backends reachable via mesh.
    pub backends_reachable: Vec<BackendStatus>,
}

/// Certificate expiry info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertExpiry {
    /// Domain name.
    pub domain: String,
    /// Days until expiry (negative = expired).
    pub days_remaining: i32,
    /// Whether the cert is valid.
    pub valid: bool,
}

/// Backend reachability status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendStatus {
    /// Capability name (e.g. `"jupyter"`).
    pub capability: String,
    /// Gate serving this capability.
    pub gate: String,
    /// Whether the backend responded to health probe.
    pub reachable: bool,
    /// Latency in milliseconds (if reachable).
    pub latency_ms: Option<u32>,
}

/// Result of a shadow comparison between legacy (Caddy) and Tower gateway paths.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowComparison {
    /// URL probed.
    pub url: String,
    /// Legacy (Caddy) response.
    pub legacy: ProbeResult,
    /// Tower (bearDog + songBird) response.
    pub tower: ProbeResult,
    /// Whether responses match (status + key headers).
    pub match_status: bool,
}

/// Result of probing a single endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeResult {
    /// HTTP status code (0 if connection failed).
    pub status: u16,
    /// Response time in milliseconds.
    pub latency_ms: u32,
    /// Content-Length or body size.
    pub body_size: u64,
    /// Error message (if probe failed).
    pub error: Option<String>,
}

impl ProbeResult {
    /// Create a successful probe result.
    #[must_use]
    pub const fn ok(status: u16, latency_ms: u32, body_size: u64) -> Self {
        Self {
            status,
            latency_ms,
            body_size,
            error: None,
        }
    }

    /// Create a failed probe result.
    #[must_use]
    pub fn err(message: impl Into<String>) -> Self {
        Self {
            status: 0,
            latency_ms: 0,
            body_size: 0,
            error: Some(message.into()),
        }
    }

    /// Whether this probe succeeded (non-zero status, no error).
    #[must_use]
    pub const fn is_ok(&self) -> bool {
        self.status > 0 && self.error.is_none()
    }
}

impl ShadowComparison {
    /// Determine if the shadow comparison passes (both responded with matching status).
    #[must_use]
    pub const fn passes(&self) -> bool {
        self.legacy.is_ok() && self.tower.is_ok() && self.match_status
    }
}

/// Aggregate shadow validation report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowReport {
    /// Individual comparisons.
    pub comparisons: Vec<ShadowComparison>,
    /// Overall pass rate (0.0–1.0).
    pub pass_rate: f64,
    /// Whether all comparisons passed.
    pub all_pass: bool,
}

impl ShadowReport {
    /// Build a report from a set of comparisons.
    #[must_use]
    #[allow(clippy::cast_precision_loss, reason = "route counts are small")]
    pub fn from_comparisons(comparisons: Vec<ShadowComparison>) -> Self {
        let total = comparisons.len();
        let passed = comparisons.iter().filter(|c| c.passes()).count();
        let pass_rate = if total == 0 {
            0.0
        } else {
            passed as f64 / total as f64
        };
        Self {
            comparisons,
            pass_rate,
            all_pass: passed == total && total > 0,
        }
    }
}

// ── Tower Shadow — WG vs Tower transport comparison ──────────────────────

/// Metrics from a single transport probe (one direction, one transport).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportProbe {
    /// Transport type: `"wireguard"` or `"tower"`.
    pub transport: String,
    /// Round-trip latency in microseconds.
    pub latency_us: u64,
    /// Throughput in bytes/sec (0 if not measured).
    pub throughput_bps: u64,
    /// Jitter in microseconds (std-dev of latency samples).
    pub jitter_us: u64,
    /// Number of probe samples.
    pub samples: u32,
    /// Error message (if probe failed).
    pub error: Option<String>,
}

impl TransportProbe {
    /// Whether the probe completed without error.
    #[must_use]
    pub const fn is_ok(&self) -> bool {
        self.error.is_none()
    }
}

/// Comparison of WG vs Tower for a single gate pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatePairShadow {
    /// Source gate name.
    pub from_gate: String,
    /// Destination gate name.
    pub to_gate: String,
    /// Destination `WireGuard` mesh IP.
    pub to_ip: String,
    /// `WireGuard` probe results.
    pub wireguard: TransportProbe,
    /// Tower probe results.
    pub tower: TransportProbe,
    /// Latency ratio (Tower / WG). <1.0 means Tower is faster.
    pub latency_ratio: f64,
    /// Throughput ratio (Tower / WG). >1.0 means Tower is faster.
    pub throughput_ratio: f64,
}

impl GatePairShadow {
    /// Whether Tower meets or exceeds `WireGuard` on this pair.
    #[must_use]
    pub fn tower_exceeds(&self) -> bool {
        self.latency_ratio <= 1.05 && self.throughput_ratio >= 0.95
    }
}

/// Full tower shadow report — all gate pairs, summary stats.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TowerShadowReport {
    /// ISO-8601 timestamp of report generation.
    pub timestamp: String,
    /// Gate that ran the shadow command.
    pub source_gate: String,
    /// Wave identifier.
    pub wave: String,
    /// Individual gate-pair comparisons.
    pub pairs: Vec<GatePairShadow>,
    /// Count of pairs where Tower meets/exceeds WG.
    pub tower_exceeds_count: u32,
    /// Total pairs measured.
    pub total_pairs: u32,
    /// Overall verdict: `"EXCEEDS"`, `"PARITY"`, or `"REGRESSED"`.
    pub verdict: String,
}

impl TowerShadowReport {
    /// Build a report from gate-pair measurements.
    #[must_use]
    pub fn from_pairs(
        source_gate: String,
        wave: String,
        timestamp: String,
        pairs: Vec<GatePairShadow>,
    ) -> Self {
        let total = u32::try_from(pairs.len()).unwrap_or(u32::MAX);
        let exceeds =
            u32::try_from(pairs.iter().filter(|p| p.tower_exceeds()).count()).unwrap_or(u32::MAX);
        let verdict = if total == 0 {
            "NO_DATA".to_string()
        } else if exceeds == total {
            "EXCEEDS".to_string()
        } else if exceeds * 2 >= total {
            "PARITY".to_string()
        } else {
            "REGRESSED".to_string()
        };
        Self {
            timestamp,
            source_gate,
            wave,
            pairs,
            tower_exceeds_count: exceeds,
            total_pairs: total,
            verdict,
        }
    }
}

#[cfg(test)]
mod tests {
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
}
