// SPDX-License-Identifier: AGPL-3.0-or-later

//! Continuous shadow benchmarking — systemd timer lifecycle + on-demand runs.
//!
//! `tower.shadow --enable` installs a timer that periodically benchmarks every
//! mesh peer via `songbird benchmark`, collecting Tower vs WG metrics and
//! exporting them to `benchScale/tower_shadow/` JSON.

use crate::cli;
use crate::error::{Result, ShadowError, ShadowOutcome};
use std::path::{Path, PathBuf};

const SHADOW_TIMER_UNIT: &str = "tower-shadow-benchmark.timer";
const SHADOW_SERVICE_UNIT: &str = "tower-shadow-benchmark.service";
const DEFAULT_INTERVAL_MIN: u32 = 60;
const DEFAULT_PROBES: u32 = 20;
const DEFAULT_DURATION_SEC: u32 = 10;

/// `tower.shadow --enable|--disable`
pub async fn dispatch_shadow_timer(args: &[&str]) -> Result<ShadowOutcome> {
    if args.contains(&"--disable") {
        return disable_shadow().await;
    }

    let interval = cli::extract_flag_value(args, "--interval")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(DEFAULT_INTERVAL_MIN);

    enable_shadow(interval).await
}

/// `tower.shadow.status` — report timer + results state.
pub async fn dispatch_shadow_timer_status() -> Result<ShadowOutcome> {
    let timer_active = systemctl_is_active(SHADOW_TIMER_UNIT).await;
    let output_dir = resolve_shadow_output_dir().ok();

    let result_count = if let Some(ref dir) = output_dir {
        count_json_files(dir).await
    } else {
        0
    };

    let latest = if let Some(ref dir) = output_dir {
        latest_result_file(dir).await
    } else {
        None
    };

    Ok(ShadowOutcome::ok_with(
        format!(
            "tower shadow: {}\nresults: {} files\nlatest: {}",
            if timer_active { "ACTIVE" } else { "INACTIVE" },
            result_count,
            latest.as_deref().unwrap_or("none"),
        ),
        serde_json::json!({
            "active": timer_active,
            "result_count": result_count,
            "latest": latest,
            "output_dir": output_dir.map(|d| d.to_string_lossy().into_owned()),
        }),
    ))
}

/// `tower.status` — Tower Atomic stack health on this gate.
pub async fn dispatch_tower_status() -> Result<ShadowOutcome> {
    let songbird_socket = resolve_songbird_socket();
    let beardog_socket = resolve_beardog_socket();
    let skunkbat_socket = resolve_skunkbat_socket();

    let songbird_ok = probe_socket(&songbird_socket).await;
    let beardog_ok = probe_socket(&beardog_socket).await;
    let skunkbat_ok = probe_socket(&skunkbat_socket).await;

    let mesh_info = if songbird_ok {
        probe_mesh(&songbird_socket).await
    } else {
        None
    };

    let all_ok = songbird_ok && beardog_ok && skunkbat_ok;
    let shadow_active = systemctl_is_active(SHADOW_TIMER_UNIT).await;

    Ok(ShadowOutcome::ok_with(
        format!(
            "Tower Atomic: {}\n  songBird: {} | bearDog: {} | skunkBat: {}\n  mesh: {}\n  shadow: {}",
            if all_ok { "3/3 LIVE" } else { "DEGRADED" },
            if songbird_ok { "LIVE" } else { "DOWN" },
            if beardog_ok { "LIVE" } else { "DOWN" },
            if skunkbat_ok { "LIVE" } else { "DOWN" },
            mesh_info.as_deref().unwrap_or("unavailable"),
            if shadow_active { "ACTIVE" } else { "INACTIVE" },
        ),
        serde_json::json!({
            "songbird": songbird_ok,
            "beardog": beardog_ok,
            "skunkbat": skunkbat_ok,
            "all_live": all_ok,
            "mesh": mesh_info,
            "shadow_active": shadow_active,
        }),
    ))
}

/// `tower.benchmark [--peer ADDR] [--probes N] [--duration N]`
pub async fn dispatch_benchmark(args: &[&str]) -> Result<ShadowOutcome> {
    let songbird_bin = resolve_songbird_bin()?;
    let output_dir = resolve_shadow_output_dir()?;
    tokio::fs::create_dir_all(&output_dir)
        .await
        .map_err(ShadowError::Io)?;

    let peers = if let Some(peer_addr) = cli::extract_flag_value(args, "--peer") {
        vec![MeshPeer {
            name: peer_addr.to_string(),
            addr: peer_addr.to_string(),
        }]
    } else {
        discover_mesh_peers().await
    };

    if peers.is_empty() {
        return Ok(ShadowOutcome::fail(
            "no mesh peers — specify --peer ADDR or configure mesh topology",
        ));
    }

    let probes = cli::extract_flag_value(args, "--probes")
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_PROBES);
    let duration = cli::extract_flag_value(args, "--duration")
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_DURATION_SEC);

    let mut results = Vec::new();
    let ts = crate::utc_now_compact();

    for peer in &peers {
        for mode in &["tower-atomic", "wireguard"] {
            let output = run_songbird_benchmark(
                &songbird_bin,
                mode,
                &peer.addr,
                duration,
                probes,
            )
            .await;

            let safe_name = peer.name.replace('.', "_");
            let filename = format!("{mode}_{safe_name}_{ts}.json");
            let filepath = output_dir.join(&filename);

            if let Some(ref json_str) = output {
                let _ = crate::atomic_write_async(&filepath, json_str.as_bytes()).await;
            }

            results.push(serde_json::json!({
                "peer": peer.addr,
                "mode": mode,
                "file": filename,
                "ok": output.is_some(),
            }));
        }
    }

    let ok_count = results
        .iter()
        .filter(|r| r["ok"].as_bool() == Some(true))
        .count();
    Ok(ShadowOutcome::ok_with(
        format!(
            "benchmark: {ok_count}/{} runs complete → {}",
            results.len(),
            output_dir.display()
        ),
        serde_json::json!({
            "results": results,
            "output_dir": output_dir.to_string_lossy(),
        }),
    ))
}

// ── Enable / Disable ─────────────────────────────────────────────────

async fn enable_shadow(interval_min: u32) -> Result<ShadowOutcome> {
    let output_dir = resolve_shadow_output_dir()?;
    tokio::fs::create_dir_all(&output_dir)
        .await
        .map_err(ShadowError::Io)?;

    let songbird_bin = resolve_songbird_bin()?;
    let peers = discover_mesh_peers().await;

    if peers.is_empty() {
        return Ok(ShadowOutcome::fail(
            "no mesh peers discovered — cannot enable shadow benchmarking",
        ));
    }

    let script_content = generate_benchmark_script(&songbird_bin, &peers, &output_dir);
    let script_path = output_dir.join("shadow-benchmark.sh");
    crate::atomic_write_async(&script_path, script_content.as_bytes())
        .await
        .map_err(ShadowError::Io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755));
    }

    let service_content = generate_service_unit(&output_dir);
    let timer_content = generate_timer_unit(interval_min);

    let service_path = format!("/etc/systemd/system/{SHADOW_SERVICE_UNIT}");
    let timer_path = format!("/etc/systemd/system/{SHADOW_TIMER_UNIT}");

    write_unit_file(&service_path, &service_content).await?;
    write_unit_file(&timer_path, &timer_content).await?;

    systemctl(&["daemon-reload"]).await?;
    systemctl(&["enable", "--now", SHADOW_TIMER_UNIT]).await?;

    let peer_list: Vec<String> = peers
        .iter()
        .map(|p| format!("{}:{}", p.name, p.addr))
        .collect();
    Ok(ShadowOutcome::ok_with(
        format!(
            "tower shadow enabled — benchmarking {} peers every {}min\npeers: {}\noutput: {}",
            peers.len(),
            interval_min,
            peer_list.join(", "),
            output_dir.display(),
        ),
        serde_json::json!({
            "enabled": true,
            "interval_min": interval_min,
            "peers": peer_list,
            "output_dir": output_dir.to_string_lossy(),
            "timer_unit": SHADOW_TIMER_UNIT,
        }),
    ))
}

async fn disable_shadow() -> Result<ShadowOutcome> {
    let _ = systemctl(&["disable", "--now", SHADOW_TIMER_UNIT]).await;
    let _ = systemctl(&["stop", SHADOW_SERVICE_UNIT]).await;

    let service_path = format!("/etc/systemd/system/{SHADOW_SERVICE_UNIT}");
    let timer_path = format!("/etc/systemd/system/{SHADOW_TIMER_UNIT}");
    let _ = tokio::fs::remove_file(&service_path).await;
    let _ = tokio::fs::remove_file(&timer_path).await;
    let _ = systemctl(&["daemon-reload"]).await;

    Ok(ShadowOutcome::ok("tower shadow disabled"))
}

// ── Mesh peer discovery ──────────────────────────────────────────────

struct MeshPeer {
    name: String,
    addr: String,
}

async fn discover_mesh_peers() -> Vec<MeshPeer> {
    let mesh_gates = cellmembrane_types::cytoplasm::known_mesh_gates();
    let local_gate = crate::gate::resolve_local_gate_identity();
    let mut peers = Vec::new();

    for gate in mesh_gates {
        if gate == local_gate {
            continue;
        }
        let Some(ip) = cellmembrane_types::cytoplasm::mesh_address(gate) else {
            continue;
        };
        peers.push(MeshPeer {
            name: gate.to_string(),
            addr: format!("{ip}:{}", super::SONGBIRD_FEDERATION_PORT),
        });
    }

    peers
}

// ── songBird subprocess ──────────────────────────────────────────────

async fn run_songbird_benchmark(
    bin: &Path,
    mode: &str,
    peer: &str,
    duration_sec: u32,
    probes: u32,
) -> Option<String> {
    let output = tokio::process::Command::new(bin)
        .args([
            "benchmark",
            "--mode",
            mode,
            "--peer",
            peer,
            "--duration",
            &format!("{duration_sec}s"),
            "--probes",
            &probes.to_string(),
            "--output",
            "json",
        ])
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        tracing::warn!(
            mode,
            peer,
            "songbird benchmark failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json_start = stdout.find('{')?;
    Some(stdout[json_start..].to_string())
}

// ── Socket probing ───────────────────────────────────────────────────

fn resolve_songbird_socket() -> PathBuf {
    PathBuf::from(cellmembrane_types::service::env_or(
        "MEMBRANE_SOCKET_SONGBIRD",
        cellmembrane_types::service::DEFAULT_SONGBIRD_SOCKET,
    ))
}

fn resolve_beardog_socket() -> PathBuf {
    PathBuf::from(cellmembrane_types::service::env_or(
        "MEMBRANE_SOCKET_BEARDOG",
        "/run/membrane/beardog.sock",
    ))
}

fn resolve_skunkbat_socket() -> PathBuf {
    PathBuf::from(cellmembrane_types::service::env_or(
        "MEMBRANE_SOCKET_SKUNKBAT",
        "/run/membrane/skunkbat.sock",
    ))
}

async fn probe_socket(path: &Path) -> bool {
    crate::jsonrpc::call(path, crate::jsonrpc::HEALTH_REQUEST)
        .await
        .is_ok()
}

async fn probe_mesh(songbird_socket: &Path) -> Option<String> {
    let request = r#"{"jsonrpc":"2.0","method":"mesh.status","params":{},"id":1}"#;
    let response = crate::jsonrpc::call(songbird_socket, request)
        .await
        .ok()?;
    let json: serde_json::Value = serde_json::from_str(&response).ok()?;

    let peers = json
        .pointer("/result/reachable_peers")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let relay = json
        .pointer("/result/relay_enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    Some(format!(
        "{peers} peers reachable, relay={}",
        if relay { "on" } else { "off" }
    ))
}

// ── Binary resolution ────────────────────────────────────────────────

fn resolve_songbird_bin() -> Result<PathBuf> {
    let plasmid_base = crate::resolve_xdg_data_home()
        .join("ecoPrimals/plasmidBin/primals/x86_64-unknown-linux-musl");
    let bin = plasmid_base.join("songbird");
    if bin.exists() {
        return Ok(bin);
    }

    let system_bin = PathBuf::from(cellmembrane_types::service::DEFAULT_INSTALL_BASE)
        .join("primals/x86_64-unknown-linux-musl/songbird");
    if system_bin.exists() {
        return Ok(system_bin);
    }

    Err(ShadowError::Config(
        "songbird binary not found in depot or install base".into(),
    ))
}

// ── systemd unit generation ──────────────────────────────────────────

fn generate_service_unit(output_dir: &Path) -> String {
    let script_path = output_dir.join("shadow-benchmark.sh");

    format!(
        "\
[Unit]
Description=Tower Atomic shadow benchmark — continuous parity metrics
After=songbird-gateway.service

[Service]
Type=oneshot
ExecStartPre=/bin/mkdir -p {output_dir}
ExecStart=/bin/bash {script_path}
Environment=PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
",
        output_dir = output_dir.display(),
        script_path = script_path.display(),
    )
}

fn generate_timer_unit(interval_min: u32) -> String {
    format!(
        "\
[Unit]
Description=Tower Atomic shadow benchmark timer

[Timer]
OnBootSec=5min
OnUnitActiveSec={interval_min}min
RandomizedDelaySec=30

[Install]
WantedBy=timers.target
"
    )
}

fn generate_benchmark_script(
    songbird_bin: &Path,
    peers: &[MeshPeer],
    output_dir: &Path,
) -> String {
    let cmds: String = peers
        .iter()
        .flat_map(|p| {
            ["tower-atomic", "wireguard"].iter().map(move |mode| {
                let safe_name = p.name.replace('.', "_");
                format!(
                    "{bin} benchmark --mode {mode} --peer {addr} \
                     --duration {dur}s --probes {probes} --output json \
                     > \"{dir}/{mode}_{safe_name}_$TS.json\" 2>/dev/null || true\n",
                    bin = songbird_bin.display(),
                    addr = p.addr,
                    dur = DEFAULT_DURATION_SEC,
                    probes = DEFAULT_PROBES,
                    dir = output_dir.display(),
                )
            })
        })
        .collect();

    format!(
        "#!/bin/bash\n\
         set -euo pipefail\n\
         TS=$(date +%Y%m%d_%H%M%S)\n\
         {cmds}"
    )
}

// ── systemd interaction ──────────────────────────────────────────────

async fn write_unit_file(path: &str, content: &str) -> Result<()> {
    tokio::fs::write(path, content)
        .await
        .map_err(ShadowError::Io)
}

async fn systemctl(args: &[&str]) -> Result<()> {
    let output = tokio::process::Command::new("sudo")
        .arg("systemctl")
        .args(args)
        .output()
        .await
        .map_err(ShadowError::Io)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::warn!(args = ?args, "systemctl: {}", stderr.trim());
    }
    Ok(())
}

async fn systemctl_is_active(unit: &str) -> bool {
    tokio::process::Command::new("systemctl")
        .args(["is-active", "--quiet", unit])
        .status()
        .await
        .is_ok_and(|s| s.success())
}

// ── Output directory helpers ─────────────────────────────────────────

fn resolve_shadow_output_dir() -> Result<PathBuf> {
    let root = crate::resolve_workspace_root()?;
    Ok(root.join("springs/primalSpring/benchScale/tower_shadow"))
}

async fn count_json_files(dir: &Path) -> usize {
    let mut count = 0;
    if let Ok(mut entries) = tokio::fs::read_dir(dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            if entry
                .path()
                .extension()
                .is_some_and(|ext| ext == "json")
            {
                count += 1;
            }
        }
    }
    count
}

async fn latest_result_file(dir: &Path) -> Option<String> {
    let mut latest: Option<(std::time::SystemTime, String)> = None;
    if let Ok(mut entries) = tokio::fs::read_dir(dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                if let Ok(meta) = entry.metadata().await {
                    let modified = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
                    if latest.as_ref().is_none_or(|(t, _)| modified > *t) {
                        latest = Some((
                            modified,
                            path.file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default(),
                        ));
                    }
                }
            }
        }
    }
    latest.map(|(_, name)| name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_timer_unit_format() {
        let timer = generate_timer_unit(30);
        assert!(timer.contains("OnUnitActiveSec=30min"));
        assert!(timer.contains("[Timer]"));
        assert!(timer.contains("timers.target"));
    }

    #[test]
    fn generate_timer_unit_default_interval() {
        let timer = generate_timer_unit(DEFAULT_INTERVAL_MIN);
        assert!(timer.contains("OnUnitActiveSec=60min"));
    }

    #[test]
    fn generate_service_unit_references_script() {
        let dir = PathBuf::from("/tmp/benchScale/tower_shadow");
        let unit = generate_service_unit(&dir);
        assert!(unit.contains("shadow-benchmark.sh"));
        assert!(unit.contains("songbird-gateway.service"));
        assert!(unit.contains("Type=oneshot"));
    }

    #[test]
    fn resolve_songbird_socket_returns_path() {
        let path = resolve_songbird_socket();
        assert!(path.to_string_lossy().contains("songbird"));
    }

    #[test]
    fn resolve_beardog_socket_returns_path() {
        let path = resolve_beardog_socket();
        assert!(path.to_string_lossy().contains("beardog"));
    }

    #[test]
    fn resolve_skunkbat_socket_returns_path() {
        let path = resolve_skunkbat_socket();
        assert!(path.to_string_lossy().contains("skunkbat"));
    }

    #[tokio::test]
    async fn shadow_timer_status_returns_ok() {
        let result = dispatch_shadow_timer_status().await.unwrap();
        assert!(result.ok);
    }
}
