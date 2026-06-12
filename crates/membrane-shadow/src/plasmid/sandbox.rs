// SPDX-License-Identifier: AGPL-3.0-or-later

//! Sandbox validation — spin up isolated primal instances to health-check
//! new binaries before promoting to production.
//!
//! Flow: staged binary → sandbox socket namespace → health probe → teardown.
//! On pass, the caller promotes; on fail, production remains untouched.

use std::path::{Path, PathBuf};
use std::time::Instant;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Default time (seconds) to wait for sandbox instance to become healthy.
const SANDBOX_HEALTH_TIMEOUT_SECS: u64 = 15;

/// How many probe attempts before declaring failure.
const SANDBOX_PROBE_RETRIES: u32 = 5;

/// Delay between probe attempts (milliseconds).
const SANDBOX_PROBE_INTERVAL_MS: u64 = 2000;

/// Base directory for sandbox sockets.
const SANDBOX_SOCKET_DIR: &str = "/run/membrane/sandbox";

/// Base directory for sandbox binaries (VPS-side staging).
const SANDBOX_BIN_DIR: &str = "/opt/membrane/sandbox";

/// Environment variable to override the sandbox socket directory.
const ENV_SANDBOX_SOCKET_DIR: &str = "MEMBRANE_SANDBOX_SOCKET_DIR";

/// Environment variable to override the sandbox binary directory.
const ENV_SANDBOX_BIN_DIR: &str = "MEMBRANE_SANDBOX_BIN_DIR";

/// A sandboxed primal instance under validation.
#[derive(Debug, Clone)]
pub struct SandboxInstance {
    pub primal: String,
    pub commit: String,
    pub binary_path: PathBuf,
    pub socket_path: PathBuf,
    pub pid: Option<u32>,
    pub started_at: Option<Instant>,
}

/// Result of sandbox validation for a single primal.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SandboxResult {
    pub primal: String,
    pub commit: String,
    pub health_ok: bool,
    pub detail: String,
    pub elapsed_ms: u64,
}

/// Arguments for sandbox validation.
#[derive(Debug, Clone)]
pub struct SandboxArgs {
    pub primal: String,
    pub commit: String,
    pub binary_path: PathBuf,
    pub timeout_secs: Option<u64>,
}

fn resolve_sandbox_socket_dir() -> PathBuf {
    PathBuf::from(
        std::env::var(ENV_SANDBOX_SOCKET_DIR).unwrap_or_else(|_| SANDBOX_SOCKET_DIR.into()),
    )
}

fn resolve_sandbox_bin_dir() -> PathBuf {
    PathBuf::from(std::env::var(ENV_SANDBOX_BIN_DIR).unwrap_or_else(|_| SANDBOX_BIN_DIR.into()))
}

/// Spin up a sandboxed instance, returning the handle.
///
/// The binary is copied to the sandbox staging area and started with
/// `--socket` pointing to an isolated namespace. The process is detached
/// but tracked by PID for cleanup.
pub fn spin_up(args: &SandboxArgs) -> Result<SandboxInstance, String> {
    let socket_dir = resolve_sandbox_socket_dir();
    let bin_dir = resolve_sandbox_bin_dir();

    std::fs::create_dir_all(&socket_dir).map_err(|e| format!("create sandbox socket dir: {e}"))?;
    std::fs::create_dir_all(&bin_dir).map_err(|e| format!("create sandbox bin dir: {e}"))?;

    let commit_short = if args.commit.len() >= 8 {
        &args.commit[..8]
    } else {
        &args.commit
    };

    let sandbox_binary = bin_dir.join(format!("{}-{commit_short}", args.primal));
    let socket_path = socket_dir.join(format!("{}-{commit_short}.sock", args.primal));

    // Remove stale socket if present
    if socket_path.exists() {
        std::fs::remove_file(&socket_path).ok();
    }

    // Stage binary to sandbox directory
    std::fs::copy(&args.binary_path, &sandbox_binary)
        .map_err(|e| format!("stage sandbox binary: {e}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&sandbox_binary, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("chmod sandbox binary: {e}"))?;
    }

    let child = tokio::process::Command::new(&sandbox_binary)
        .arg("server")
        .arg("--socket")
        .arg(&socket_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn sandbox {}: {e}", args.primal))?;

    let pid = child.id();

    Ok(SandboxInstance {
        primal: args.primal.clone(),
        commit: args.commit.clone(),
        binary_path: sandbox_binary,
        socket_path,
        pid,
        started_at: Some(Instant::now()),
    })
}

/// Probe the sandboxed instance for health via JSON-RPC on its isolated socket.
///
/// Retries up to `SANDBOX_PROBE_RETRIES` times with `SANDBOX_PROBE_INTERVAL_MS`
/// delay between attempts (allows process startup time).
pub async fn probe_health(instance: &SandboxInstance) -> SandboxResult {
    let start = instance.started_at.unwrap_or_else(Instant::now);
    let request = r#"{"jsonrpc":"2.0","method":"health","params":{},"id":1}"#;

    for attempt in 0..SANDBOX_PROBE_RETRIES {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(SANDBOX_PROBE_INTERVAL_MS)).await;
        }

        if !instance.socket_path.exists() {
            continue;
        }

        if let Ok(response) = uds_jsonrpc_probe(&instance.socket_path, request).await {
            if response.contains("\"status\"") && response.contains("healthy") {
                return SandboxResult {
                    primal: instance.primal.clone(),
                    commit: instance.commit.clone(),
                    health_ok: true,
                    detail: extract_health_detail(&response),
                    elapsed_ms: start.elapsed().as_millis() as u64,
                };
            }
            if response.contains("\"result\"") {
                return SandboxResult {
                    primal: instance.primal.clone(),
                    commit: instance.commit.clone(),
                    health_ok: true,
                    detail: format!("responding (attempt {})", attempt + 1),
                    elapsed_ms: start.elapsed().as_millis() as u64,
                };
            }
        }
    }

    SandboxResult {
        primal: instance.primal.clone(),
        commit: instance.commit.clone(),
        health_ok: false,
        detail: format!(
            "no health response after {} attempts ({}ms)",
            SANDBOX_PROBE_RETRIES,
            start.elapsed().as_millis()
        ),
        elapsed_ms: start.elapsed().as_millis() as u64,
    }
}

/// Kill the sandbox process and clean up socket/binary.
pub async fn teardown(instance: &SandboxInstance) {
    if let Some(pid) = instance.pid {
        let _ = tokio::process::Command::new("kill")
            .arg(pid.to_string())
            .output()
            .await;
        // Grace period then force-kill
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let _ = tokio::process::Command::new("kill")
            .args(["-9", &pid.to_string()])
            .output()
            .await;
    }

    // Clean up socket
    if instance.socket_path.exists() {
        std::fs::remove_file(&instance.socket_path).ok();
    }

    // Clean up staged binary
    if instance.binary_path.exists() {
        std::fs::remove_file(&instance.binary_path).ok();
    }
}

/// Full validation cycle: spin up → probe → teardown → return result.
///
/// This is the primary entry point for the pipeline integration.
/// Returns `Ok(SandboxResult)` whether the health check passed or failed;
/// returns `Err` only for infrastructure failures (can't spawn, can't create dirs).
pub async fn validate(args: &SandboxArgs) -> Result<SandboxResult, String> {
    let timeout =
        std::time::Duration::from_secs(args.timeout_secs.unwrap_or(SANDBOX_HEALTH_TIMEOUT_SECS));

    let instance = spin_up(args)?;

    let result = tokio::time::timeout(timeout, probe_health(&instance))
        .await
        .unwrap_or_else(|_| SandboxResult {
            primal: instance.primal.clone(),
            commit: instance.commit.clone(),
            health_ok: false,
            detail: format!("timeout after {}s", timeout.as_secs()),
            elapsed_ms: timeout.as_millis() as u64,
        });

    teardown(&instance).await;

    Ok(result)
}

/// Validate and promote: run sandbox, on pass atomically replace production binary.
///
/// `production_path` is the target location (e.g. `/opt/membrane/songbird`).
/// On success, the old production binary is returned as `Some(old_path)` for
/// canary retirement.
pub async fn validate_and_promote(
    args: &SandboxArgs,
    production_path: &Path,
) -> Result<(SandboxResult, Option<PathBuf>), String> {
    let result = validate(args).await?;

    if !result.health_ok {
        return Ok((result, None));
    }

    // Atomic promotion: copy new binary to production via .new + rename
    let new_path = production_path.with_extension("new");
    std::fs::copy(&args.binary_path, &new_path)
        .map_err(|e| format!("copy to production staging: {e}"))?;

    // Preserve the old binary path for canary retirement
    let old_binary = if production_path.exists() {
        let canary_dir = resolve_sandbox_bin_dir().join("retired");
        std::fs::create_dir_all(&canary_dir).ok();
        let retired_path = canary_dir.join(format!(
            "{}-prev",
            production_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        ));
        std::fs::copy(production_path, &retired_path).ok();
        Some(retired_path)
    } else {
        None
    };

    std::fs::rename(&new_path, production_path)
        .map_err(|e| format!("atomic promote rename: {e}"))?;

    Ok((result, old_binary))
}

/// List active sandbox instances by scanning the sandbox socket directory.
pub fn list_active() -> Vec<SandboxInstance> {
    let socket_dir = resolve_sandbox_socket_dir();
    let bin_dir = resolve_sandbox_bin_dir();
    let mut instances = Vec::new();

    let Ok(entries) = std::fs::read_dir(&socket_dir) else {
        return instances;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("sock") {
            continue;
        }

        let stem = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        // Parse {primal}-{commit_short} from filename
        let (primal, commit) = match stem.rfind('-') {
            Some(pos) => (stem[..pos].to_string(), stem[pos + 1..].to_string()),
            None => (stem.clone(), "unknown".to_string()),
        };

        instances.push(SandboxInstance {
            primal,
            commit,
            binary_path: bin_dir.join(&stem),
            socket_path: path,
            pid: None,
            started_at: None,
        });
    }

    instances
}

// ── Internal helpers ──────────────────────────────────────────────────────

/// JSON-RPC call over UDS with timeout.
async fn uds_jsonrpc_probe(socket_path: &Path, request: &str) -> Result<String, String> {
    let stream = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        tokio::net::UnixStream::connect(socket_path),
    )
    .await
    .map_err(|_| "connect timeout".to_string())?
    .map_err(|e| format!("connect: {e}"))?;

    let (mut reader, mut writer) = stream.into_split();

    writer
        .write_all(request.as_bytes())
        .await
        .map_err(|e| format!("write: {e}"))?;
    writer
        .shutdown()
        .await
        .map_err(|e| format!("shutdown: {e}"))?;

    let mut buf = Vec::with_capacity(4096);
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        reader.read_to_end(&mut buf),
    )
    .await
    .map_err(|_| "read timeout".to_string())?
    .map_err(|e| format!("read: {e}"))?;

    String::from_utf8(buf).map_err(|e| format!("utf8: {e}"))
}

/// Extract human-readable detail from a health JSON-RPC response.
fn extract_health_detail(response: &str) -> String {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(response.trim()) else {
        return "healthy (unparsed)".into();
    };

    let result = json.get("result");
    let version = result
        .and_then(|r| r.get("version"))
        .and_then(|v| v.as_str())
        .unwrap_or("?");
    let primal = result
        .and_then(|r| r.get("primal"))
        .and_then(|v| v.as_str())
        .unwrap_or("?");
    let uptime = result
        .and_then(|r| r.get("uptime_s"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);

    format!("{primal} v{version} (uptime {uptime}s)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_health_detail_parses_json() {
        let response = r#"{"jsonrpc":"2.0","result":{"primal":"beardog","version":"1.2.3","uptime_s":42,"status":"healthy"},"id":1}"#;
        let detail = extract_health_detail(response);
        assert!(detail.contains("beardog"));
        assert!(detail.contains("1.2.3"));
        assert!(detail.contains("42"));
    }

    #[test]
    fn extract_health_detail_handles_malformed() {
        let detail = extract_health_detail("not json at all");
        assert_eq!(detail, "healthy (unparsed)");
    }

    #[test]
    fn sandbox_args_defaults() {
        let args = SandboxArgs {
            primal: "beardog".into(),
            commit: "abc12345678".into(),
            binary_path: PathBuf::from("/tmp/test"),
            timeout_secs: None,
        };
        assert_eq!(args.primal, "beardog");
        assert!(args.timeout_secs.is_none());
    }
}
