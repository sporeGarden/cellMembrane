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
    pub fn from_comparisons(comparisons: Vec<ShadowComparison>) -> Self {
        let total = comparisons.len();
        let passed = comparisons.iter().filter(|c| c.passes()).count();
        let pass_rate = if total == 0 {
            0.0
        } else {
            f64::from(u32::try_from(passed).unwrap_or(u32::MAX))
                / f64::from(u32::try_from(total).unwrap_or(u32::MAX))
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
#[path = "gateway_tests.rs"]
mod tests;
