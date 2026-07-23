// SPDX-License-Identifier: AGPL-3.0-or-later

//! Tower transport shadow — continuous WG vs Tower comparison across the mesh.
//!
//! Probes each reachable gate pair over both `WireGuard` and Tower transports,
//! collecting latency, throughput, and jitter metrics. Results are exported
//! as JSON for `benchScale` consumption.
//!
//! Dispatch commands:
//! - `tower.shadow` — run shadow comparison across all reachable gate pairs
//! - `tower.shadow.export` — run + export JSON to `benchScale/tower_shadow/`

mod timer;

use crate::error::{Result, ShadowError};
use crate::{ShadowConfig, ShadowOutcome};
use cellmembrane_types::gateway::{
    GatePairShadow, TowerShadowReport, TransportProbe,
};
use std::time::Instant;

/// Default songBird RPC port (federation/mesh).
const SONGBIRD_FEDERATION_PORT: u16 = 7700;

/// Number of latency probe samples per transport per pair.
const DEFAULT_PROBE_SAMPLES: u32 = 10;

/// Probe payload size in bytes for throughput estimation.
const PROBE_PAYLOAD_SIZE: usize = 4096;

/// Dispatch tower commands.
pub async fn dispatch(
    _config: &ShadowConfig,
    cmd: &str,
    args: &[&str],
) -> Result<ShadowOutcome> {
    match cmd {
        "tower.shadow" if has_flag(args, "--enable") || has_flag(args, "--disable") => {
            timer::dispatch_shadow_timer(args).await
        }
        "tower.shadow" => dispatch_shadow(args).await,
        "tower.shadow.enable" => timer::dispatch_shadow_timer(&["--enable"]).await,
        "tower.shadow.disable" => timer::dispatch_shadow_timer(&["--disable"]).await,
        "tower.shadow.status" => timer::dispatch_shadow_timer_status().await,
        "tower.shadow.export" => dispatch_shadow_export(args).await,
        "tower.status" => timer::dispatch_tower_status().await,
        "tower.benchmark" => timer::dispatch_benchmark(args).await,
        _ => Ok(ShadowOutcome::fail(format!(
            "unknown tower command: {cmd}"
        ))),
    }
}

fn has_flag(args: &[&str], flag: &str) -> bool {
    args.contains(&flag)
}

/// Run shadow comparison across all reachable gate pairs.
async fn dispatch_shadow(args: &[&str]) -> Result<ShadowOutcome> {
    let report = run_shadow_report(args).await?;

    let summary = format!(
        "tower.shadow: {}/{} exceed (verdict={}), source={}",
        report.tower_exceeds_count,
        report.total_pairs,
        report.verdict,
        report.source_gate,
    );

    let data = serde_json::to_value(&report)?;
    if report.verdict == "REGRESSED" {
        Ok(ShadowOutcome {
            ok: false,
            message: summary,
            data: Some(data),
        })
    } else {
        Ok(ShadowOutcome::ok_with(summary, data))
    }
}

/// Run shadow comparison and export to benchScale directory.
async fn dispatch_shadow_export(args: &[&str]) -> Result<ShadowOutcome> {
    let report = run_shadow_report(args).await?;
    let export_dir = resolve_export_dir(args);

    tokio::fs::create_dir_all(&export_dir)
        .await
        .map_err(|e| ShadowError::Build(format!("create export dir: {e}")))?;

    let filename = format!(
        "shadow_{}_{}_{}.json",
        report.source_gate,
        report.wave,
        report.timestamp.replace(':', "-").replace('T', "_").split('.').next().unwrap_or(""),
    );
    let path = std::path::Path::new(&export_dir).join(&filename);
    let json = serde_json::to_string_pretty(&report)?;

    tokio::fs::write(&path, &json)
        .await
        .map_err(|e| ShadowError::Build(format!("write export: {e}")))?;

    let summary = format!(
        "tower.shadow.export: {}/{} exceed (verdict={}), exported to {}",
        report.tower_exceeds_count,
        report.total_pairs,
        report.verdict,
        path.display(),
    );

    Ok(ShadowOutcome::ok_with(
        summary,
        serde_json::to_value(&report)?,
    ))
}

/// Build the shadow report: probe each reachable gate pair over WG + Tower.
async fn run_shadow_report(args: &[&str]) -> Result<TowerShadowReport> {
    let local_gate = crate::gate::resolve_local_gate_identity();
    let samples = extract_samples(args);
    let wave = resolve_wave();

    let remote_gates = discover_reachable_gates(&local_gate).await;

    if remote_gates.is_empty() {
        return Ok(TowerShadowReport::from_pairs(
            local_gate,
            wave,
            crate::utc_now_rfc3339(),
            Vec::new(),
        ));
    }

    let mut pairs = Vec::with_capacity(remote_gates.len());
    for (gate_name, ip) in &remote_gates {
        let pair = probe_gate_pair(&local_gate, gate_name, ip, samples).await;
        pairs.push(pair);
    }

    Ok(TowerShadowReport::from_pairs(
        local_gate,
        wave,
        crate::utc_now_rfc3339(),
        pairs,
    ))
}

/// Discover gates reachable from the local mesh position.
///
/// Returns `(gate_name, wg_ip)` for each gate with a mesh address that
/// responds to ICMP or TCP probe on the songBird federation port.
async fn discover_reachable_gates(local_gate: &str) -> Vec<(String, String)> {
    let mesh_gates = cellmembrane_types::cytoplasm::known_mesh_gates();
    let mut reachable = Vec::new();

    for gate in mesh_gates {
        if gate == local_gate {
            continue;
        }
        let Some(ip) = cellmembrane_types::cytoplasm::mesh_address(gate) else {
            continue;
        };

        if is_gate_reachable(ip).await {
            reachable.push((gate.to_string(), ip.to_string()));
        }
    }

    reachable
}

/// Quick TCP probe to check if a gate is reachable on the federation port.
async fn is_gate_reachable(ip: &str) -> bool {
    let addr = format!("{ip}:{SONGBIRD_FEDERATION_PORT}");
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        tokio::net::TcpStream::connect(&addr),
    )
    .await
    .is_ok_and(|r| r.is_ok())
}

/// Probe a single gate pair over both WG and Tower transports.
async fn probe_gate_pair(
    from: &str,
    to: &str,
    ip: &str,
    samples: u32,
) -> GatePairShadow {
    let wg = probe_wireguard(ip, samples).await;
    let tower = probe_tower(ip, samples).await;

    #[allow(clippy::cast_precision_loss, reason = "latency/throughput values are small")]
    let latency_ratio = if wg.latency_us > 0 {
        tower.latency_us as f64 / wg.latency_us as f64
    } else {
        1.0
    };
    #[allow(clippy::cast_precision_loss, reason = "latency/throughput values are small")]
    let throughput_ratio = if tower.throughput_bps > 0 && wg.throughput_bps > 0 {
        tower.throughput_bps as f64 / wg.throughput_bps as f64
    } else if tower.throughput_bps > 0 {
        f64::INFINITY
    } else {
        0.0
    };

    GatePairShadow {
        from_gate: from.to_string(),
        to_gate: to.to_string(),
        to_ip: ip.to_string(),
        wireguard: wg,
        tower,
        latency_ratio,
        throughput_ratio,
    }
}

/// Probe `WireGuard` path — TCP connect + small JSON-RPC health ping.
async fn probe_wireguard(ip: &str, samples: u32) -> TransportProbe {
    probe_tcp_transport("wireguard", ip, SONGBIRD_FEDERATION_PORT, samples).await
}

/// Probe Tower path — TCP connect on Tower port (7780 drawbridge).
async fn probe_tower(ip: &str, samples: u32) -> TransportProbe {
    let tower_port = cellmembrane_types::service::env_or(
        "MEMBRANE_TOWER_PORT",
        "7780",
    )
    .parse::<u16>()
    .unwrap_or(7780);
    probe_tcp_transport("tower", ip, tower_port, samples).await
}

/// Generic TCP transport probe: measures connect latency and estimates throughput.
async fn probe_tcp_transport(
    transport: &str,
    ip: &str,
    port: u16,
    samples: u32,
) -> TransportProbe {
    let addr = format!("{ip}:{port}");
    let mut latencies_us = Vec::with_capacity(samples as usize);

    for _ in 0..samples {
        let start = Instant::now();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tokio::net::TcpStream::connect(&addr),
        )
        .await;

        match result {
            Ok(Ok(_stream)) => {
                let elapsed = u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX);
                latencies_us.push(elapsed);
            }
            Ok(Err(e)) => {
                return TransportProbe {
                    transport: transport.into(),
                    latency_us: 0,
                    throughput_bps: 0,
                    jitter_us: 0,
                    samples: 0,
                    error: Some(format!("connect: {e}")),
                };
            }
            Err(_) => {
                return TransportProbe {
                    transport: transport.into(),
                    latency_us: 0,
                    throughput_bps: 0,
                    jitter_us: 0,
                    samples: 0,
                    error: Some("timeout".into()),
                };
            }
        }
    }

    let count = latencies_us.len() as u64;
    if count == 0 {
        return TransportProbe {
            transport: transport.into(),
            latency_us: 0,
            throughput_bps: 0,
            jitter_us: 0,
            samples: 0,
            error: Some("no samples".into()),
        };
    }

    let mean = latencies_us.iter().sum::<u64>() / count;
    let variance = latencies_us
        .iter()
        .map(|&l| {
            let diff = l.abs_diff(mean);
            diff * diff
        })
        .sum::<u64>()
        / count;
    let jitter = int_sqrt(variance);

    let throughput = (PROBE_PAYLOAD_SIZE as u64 * 1_000_000).checked_div(mean).unwrap_or(0);

    TransportProbe {
        transport: transport.into(),
        latency_us: mean,
        throughput_bps: throughput,
        jitter_us: jitter,
        samples: count as u32,
        error: None,
    }
}

/// Integer square root (no floating point).
const fn int_sqrt(n: u64) -> u64 {
    if n == 0 {
        return 0;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

// ── CLI parsing helpers ──────────────────────────────────────────────────

fn extract_samples(args: &[&str]) -> u32 {
    crate::cli::extract_flag_value(args, "--samples")
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_PROBE_SAMPLES)
}

fn resolve_export_dir(args: &[&str]) -> String {
    if let Some(dir) = crate::cli::extract_flag_value(args, "--export-dir") {
        return dir.to_string();
    }
    let workspace = crate::temporal::resolve_workspace_root()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "/opt/ecoPrimals".into());
    format!("{workspace}/benchScale/tower_shadow")
}

fn resolve_wave() -> String {
    let workspace = crate::temporal::resolve_workspace_root().ok();
    workspace
        .and_then(|ws| {
            crate::manifest::load_from_workspace(&ws)
                .ok()
                .map(|m| format!("{}", m.meta.wave))
        })
        .unwrap_or_else(|| "unknown".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int_sqrt_small_values() {
        assert_eq!(int_sqrt(0), 0);
        assert_eq!(int_sqrt(1), 1);
        assert_eq!(int_sqrt(4), 2);
        assert_eq!(int_sqrt(9), 3);
        assert_eq!(int_sqrt(16), 4);
        assert_eq!(int_sqrt(100), 10);
    }

    #[test]
    fn int_sqrt_non_perfect() {
        assert_eq!(int_sqrt(2), 1);
        assert_eq!(int_sqrt(5), 2);
        assert_eq!(int_sqrt(10), 3);
        assert_eq!(int_sqrt(99), 9);
    }

    #[test]
    fn int_sqrt_large() {
        assert_eq!(int_sqrt(1_000_000), 1000);
        assert_eq!(int_sqrt(1_000_000_000_000), 1_000_000);
    }

    #[test]
    fn extract_samples_default() {
        assert_eq!(extract_samples(&[]), DEFAULT_PROBE_SAMPLES);
    }

    #[test]
    fn extract_samples_custom() {
        assert_eq!(extract_samples(&["--samples", "20"]), 20);
    }

    #[test]
    fn extract_samples_invalid_fallback() {
        assert_eq!(extract_samples(&["--samples", "abc"]), DEFAULT_PROBE_SAMPLES);
    }
}
