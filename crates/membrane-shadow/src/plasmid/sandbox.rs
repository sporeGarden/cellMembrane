// SPDX-License-Identifier: AGPL-3.0-or-later

//! Sandbox validation — spin up isolated primal instances to health-check
//! new binaries before promoting to production.
//!
//! Flow: staged binary → sandbox socket namespace → health probe → teardown.
//! On pass, the caller promotes; on fail, production remains untouched.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Saturating conversion from `Duration` millis to `u64`.
fn millis_u64(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// Default time (seconds) to wait for sandbox instance to become healthy.
const SANDBOX_HEALTH_TIMEOUT_SECS: u64 = 15;

/// How many probe attempts before declaring failure.
const SANDBOX_PROBE_RETRIES: u32 = 5;

/// Delay between probe attempts (milliseconds).
const SANDBOX_PROBE_INTERVAL_MS: u64 = 2000;

use cellmembrane_types::service::{
    DEFAULT_SANDBOX_BIN_DIR, DEFAULT_SANDBOX_SOCKET_DIR, ENV_SANDBOX_BIN_DIR,
    ENV_SANDBOX_SOCKET_DIR,
};

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
        std::env::var(ENV_SANDBOX_SOCKET_DIR).unwrap_or_else(|_| DEFAULT_SANDBOX_SOCKET_DIR.into()),
    )
}

fn resolve_sandbox_bin_dir() -> PathBuf {
    PathBuf::from(
        std::env::var(ENV_SANDBOX_BIN_DIR).unwrap_or_else(|_| DEFAULT_SANDBOX_BIN_DIR.into()),
    )
}

/// Spin up a sandboxed instance, returning the handle.
///
/// The binary is copied to the sandbox staging area and started with
/// `--socket` pointing to an isolated namespace. If `security_socket` is
/// provided (from a dependency chain), the process also gets `--security-socket`.
pub async fn spin_up(args: &SandboxArgs) -> Result<SandboxInstance, String> {
    spin_up_with_deps(args, None).await
}

/// Spin up with an optional security socket path for Tower dependency injection.
pub async fn spin_up_with_deps(
    args: &SandboxArgs,
    security_socket: Option<&Path>,
) -> Result<SandboxInstance, String> {
    let socket_dir = resolve_sandbox_socket_dir();
    let bin_dir = resolve_sandbox_bin_dir();

    tokio::fs::create_dir_all(&socket_dir)
        .await
        .map_err(|e| format!("create sandbox socket dir: {e}"))?;
    tokio::fs::create_dir_all(&bin_dir)
        .await
        .map_err(|e| format!("create sandbox bin dir: {e}"))?;

    let commit_short = if args.commit.len() >= 8 {
        &args.commit[..8]
    } else {
        &args.commit
    };

    let sandbox_binary = bin_dir.join(format!("{}-{commit_short}", args.primal));
    let socket_path = socket_dir.join(format!("{}-{commit_short}.sock", args.primal));

    let _ = tokio::fs::remove_file(&socket_path).await;

    tokio::fs::copy(&args.binary_path, &sandbox_binary)
        .await
        .map_err(|e| format!("stage sandbox binary: {e}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&sandbox_binary, std::fs::Permissions::from_mode(0o755))
            .await
            .map_err(|e| format!("chmod sandbox binary: {e}"))?;
    }

    let mut cmd = tokio::process::Command::new(&sandbox_binary);
    cmd.arg("server").arg("--socket").arg(&socket_path);

    if let Some(sec_sock) = security_socket {
        cmd.arg("--security-socket").arg(sec_sock);
    }

    let child = cmd
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
                    elapsed_ms: millis_u64(start.elapsed()),
                };
            }
            if response.contains("\"result\"") {
                return SandboxResult {
                    primal: instance.primal.clone(),
                    commit: instance.commit.clone(),
                    health_ok: true,
                    detail: format!("responding (attempt {})", attempt + 1),
                    elapsed_ms: millis_u64(start.elapsed()),
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
        elapsed_ms: millis_u64(start.elapsed()),
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

    let _ = tokio::fs::remove_file(&instance.socket_path).await;
    let _ = tokio::fs::remove_file(&instance.binary_path).await;
}

/// Full validation cycle: spin up → probe → teardown → return result.
///
/// This is the primary entry point for the pipeline integration.
/// Returns `Ok(SandboxResult)` whether the health check passed or failed;
/// returns `Err` only for infrastructure failures (can't spawn, can't create dirs).
pub async fn validate(args: &SandboxArgs) -> Result<SandboxResult, String> {
    let timeout =
        std::time::Duration::from_secs(args.timeout_secs.unwrap_or(SANDBOX_HEALTH_TIMEOUT_SECS));

    let instance = spin_up(args).await?;

    let result = tokio::time::timeout(timeout, probe_health(&instance))
        .await
        .unwrap_or_else(|_| SandboxResult {
            primal: instance.primal.clone(),
            commit: instance.commit.clone(),
            health_ok: false,
            detail: format!("timeout after {}s", timeout.as_secs()),
            elapsed_ms: millis_u64(timeout),
        });

    teardown(&instance).await;

    Ok(result)
}

/// Resolve Tower dependencies required to sandbox-validate a given primal.
///
/// Non-Tower primals need bearDog (crypto signer) running so they can connect
/// to `--security-socket` during health checks. Tower primals themselves
/// (beardog, songbird, skunkbat) have no upstream dependencies.
///
/// Returns `None` if the primal has no dependencies, or `Some(binary_name)` of
/// the required security provider.
#[must_use]
pub fn resolve_security_dependency(primal: &str) -> Option<&'static str> {
    use cellmembrane_types::MembraneComposition;
    use cellmembrane_types::service::ServiceCapability;

    let service = cellmembrane_types::MembraneService::all()
        .iter()
        .find(|s| s.binary == primal)?;

    // Tower-tier primals (beardog, songbird, skunkbat) have no upstream deps
    if service.min_composition <= MembraneComposition::Tower {
        return None;
    }

    // All non-Tower primals depend on the crypto signer for security-socket
    let signer =
        cellmembrane_types::MembraneService::with_capability(ServiceCapability::CryptoSigner)?;
    Some(signer.binary)
}

/// Full validation cycle with automatic dependency provisioning.
///
/// If the target primal requires bearDog (security spine), a sandbox bearDog is
/// started first and its socket path is injected as `--security-socket`. After
/// validation completes, both the target and its dependencies are torn down.
///
/// This is the preferred entry point for pipeline integration — handles the
/// SANDBOX-DEPENDENCY-CHAIN scenario transparently.
pub async fn validate_with_deps(args: &SandboxArgs) -> Result<SandboxResult, String> {
    let timeout =
        std::time::Duration::from_secs(args.timeout_secs.unwrap_or(SANDBOX_HEALTH_TIMEOUT_SECS));

    let dep_instance = match resolve_security_dependency(&args.primal) {
        Some(dep_binary) => {
            let dep_path = resolve_dependency_binary_path(dep_binary)?;
            let dep_args = SandboxArgs {
                primal: dep_binary.to_string(),
                commit: args.commit.clone(),
                binary_path: dep_path,
                timeout_secs: Some(10),
            };
            let instance = spin_up(&dep_args).await?;
            // Wait for dependency to become ready before starting the target
            let dep_ready = tokio::time::timeout(
                std::time::Duration::from_secs(8),
                wait_for_socket(&instance.socket_path),
            )
            .await;
            if dep_ready.is_err() {
                teardown(&instance).await;
                return Ok(SandboxResult {
                    primal: args.primal.clone(),
                    commit: args.commit.clone(),
                    health_ok: false,
                    detail: format!("dependency {dep_binary} socket never appeared"),
                    elapsed_ms: 8000,
                });
            }
            Some(instance)
        }
        None => None,
    };

    let security_socket = dep_instance.as_ref().map(|d| d.socket_path.as_path());
    let instance = spin_up_with_deps(args, security_socket).await?;

    let result = tokio::time::timeout(timeout, probe_health(&instance))
        .await
        .unwrap_or_else(|_| SandboxResult {
            primal: instance.primal.clone(),
            commit: instance.commit.clone(),
            health_ok: false,
            detail: format!("timeout after {}s", timeout.as_secs()),
            elapsed_ms: millis_u64(timeout),
        });

    // Teardown: target first, then dependencies
    teardown(&instance).await;
    if let Some(ref dep) = dep_instance {
        teardown(dep).await;
    }

    Ok(result)
}

/// Locate the binary for a dependency (looks in production install, then depot).
fn resolve_dependency_binary_path(binary: &str) -> Result<PathBuf, String> {
    // First: check if production binary exists (live system has it installed)
    let production = Path::new(cellmembrane_types::service::DEFAULT_INSTALL_BASE).join(binary);
    if production.exists() {
        return Ok(production);
    }

    // Second: check local depot
    if let Ok(depot) = crate::plasmid::depot::resolve_depot(None) {
        let arch = crate::plasmid::detect_target_triple();
        let depot_bin = depot.join("primals").join(&arch).join(binary);
        if depot_bin.exists() {
            return Ok(depot_bin);
        }
    }

    // Third: user-local depot fallback
    if let Ok(home) = std::env::var("HOME") {
        let home_depot = PathBuf::from(home)
            .join(".local/share/ecoPrimals/plasmidBin/primals")
            .join(binary);
        if home_depot.exists() {
            return Ok(home_depot);
        }
    }

    Err(format!(
        "dependency binary '{binary}' not found in production, depot, or local"
    ))
}

/// Wait for a socket file to appear on disk (polled at 200ms intervals).
async fn wait_for_socket(path: &Path) {
    for _ in 0..40 {
        if path.exists() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
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
    tokio::fs::copy(&args.binary_path, &new_path)
        .await
        .map_err(|e| format!("copy to production staging: {e}"))?;

    // Preserve the old binary path for canary retirement
    let old_binary = if production_path.exists() {
        let canary_dir = resolve_sandbox_bin_dir().join("retired");
        if let Err(e) = tokio::fs::create_dir_all(&canary_dir).await {
            tracing::warn!(error = %e, "failed to create canary retire directory");
        }
        let retired_path = canary_dir.join(format!(
            "{}-prev",
            production_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        ));
        if let Err(e) = tokio::fs::copy(production_path, &retired_path).await {
            tracing::warn!(error = %e, "failed to retire production binary to canary");
        }
        Some(retired_path)
    } else {
        None
    };

    tokio::fs::rename(&new_path, production_path)
        .await
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
        let (primal, commit) = stem.rfind('-').map_or_else(
            || (stem.clone(), "unknown".to_string()),
            |pos| (stem[..pos].to_string(), stem[pos + 1..].to_string()),
        );

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

async fn uds_jsonrpc_probe(socket_path: &Path, request: &str) -> Result<String, String> {
    crate::jsonrpc::call(socket_path, request).await
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

    #[test]
    fn resolve_dependency_tower_has_no_deps() {
        assert!(resolve_security_dependency("beardog").is_none());
        assert!(resolve_security_dependency("songbird").is_none());
        assert!(resolve_security_dependency("skunkbat").is_none());
    }

    #[test]
    fn resolve_dependency_nucleus_needs_beardog() {
        assert_eq!(resolve_security_dependency("nestgate"), Some("beardog"));
        assert_eq!(resolve_security_dependency("biomeos"), Some("beardog"));
        assert_eq!(resolve_security_dependency("squirrel"), Some("beardog"));
        assert_eq!(resolve_security_dependency("toadstool"), Some("beardog"));
        assert_eq!(resolve_security_dependency("sweetgrass"), Some("beardog"));
        assert_eq!(resolve_security_dependency("loamspine"), Some("beardog"));
    }

    #[test]
    fn resolve_dependency_unknown_returns_none() {
        assert!(resolve_security_dependency("nonexistent-primal").is_none());
    }
}
