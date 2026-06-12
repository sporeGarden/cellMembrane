// SPDX-License-Identifier: AGPL-3.0-or-later

//! Canary pool — maintain previous-good primal instances as fallback targets.
//!
//! After sandbox validation promotes a new binary, the previous production
//! binary is retired to a canary slot. Canary instances:
//! - Run on isolated sockets (`/run/membrane/canary/{primal}.sock`)
//! - Serve as fallback mesh peers / auth providers if production fails
//! - Are periodically health-checked to confirm they remain viable
//! - Pool size defaults to 1 per primal (configurable via `membrane.toml`)

use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Base directory for canary sockets.
const CANARY_SOCKET_DIR: &str = "/run/membrane/canary";

/// Base directory for canary binaries.
const CANARY_BIN_DIR: &str = "/opt/membrane/canary";

/// Environment variable to override canary socket directory.
const ENV_CANARY_SOCKET_DIR: &str = "MEMBRANE_CANARY_SOCKET_DIR";

/// Environment variable to override canary binary directory.
const ENV_CANARY_BIN_DIR: &str = "MEMBRANE_CANARY_BIN_DIR";

/// A canary primal instance — the previous known-good binary kept alive as fallback.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CanarySlot {
    pub primal: String,
    pub binary_path: PathBuf,
    pub socket_path: PathBuf,
    pub commit: String,
    pub promoted_at: String,
    pub pid: Option<u32>,
}

/// Health status of a canary instance.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CanaryHealth {
    pub primal: String,
    pub commit: String,
    pub alive: bool,
    pub detail: String,
}

/// Canary pool state (persisted as TOML).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct CanaryPool {
    pub slots: Vec<CanarySlot>,
}

fn resolve_canary_socket_dir() -> PathBuf {
    PathBuf::from(std::env::var(ENV_CANARY_SOCKET_DIR).unwrap_or_else(|_| CANARY_SOCKET_DIR.into()))
}

fn resolve_canary_bin_dir() -> PathBuf {
    PathBuf::from(std::env::var(ENV_CANARY_BIN_DIR).unwrap_or_else(|_| CANARY_BIN_DIR.into()))
}

fn pool_state_path() -> PathBuf {
    resolve_canary_bin_dir().join("canary-pool.toml")
}

/// Retire a binary to the canary pool, replacing any existing canary for the same primal.
///
/// The binary is moved/copied to the canary directory and started on an isolated
/// socket. If an existing canary for this primal is found, it's killed first.
pub async fn retire_to_canary(
    primal: &str,
    old_binary: &Path,
    commit: &str,
) -> Result<CanarySlot, String> {
    let socket_dir = resolve_canary_socket_dir();
    let bin_dir = resolve_canary_bin_dir();

    std::fs::create_dir_all(&socket_dir).map_err(|e| format!("create canary socket dir: {e}"))?;
    std::fs::create_dir_all(&bin_dir).map_err(|e| format!("create canary bin dir: {e}"))?;

    // Kill any existing canary for this primal
    let mut pool = load_pool();
    if let Some(existing) = pool.slots.iter().find(|s| s.primal == primal) {
        kill_canary(existing).await;
    }
    pool.slots.retain(|s| s.primal != primal);

    let canary_binary = bin_dir.join(primal);
    let socket_path = socket_dir.join(format!("{primal}.sock"));

    // Remove stale socket
    if socket_path.exists() {
        std::fs::remove_file(&socket_path).ok();
    }

    // Stage binary to canary directory
    std::fs::copy(old_binary, &canary_binary).map_err(|e| format!("stage canary binary: {e}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&canary_binary, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("chmod canary binary: {e}"))?;
    }

    // Start canary on isolated socket
    let child = tokio::process::Command::new(&canary_binary)
        .arg("server")
        .arg("--socket")
        .arg(&socket_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn canary {primal}: {e}"))?;

    let slot = CanarySlot {
        primal: primal.to_string(),
        binary_path: canary_binary,
        socket_path,
        commit: commit.to_string(),
        promoted_at: chrono::Utc::now().to_rfc3339(),
        pid: child.id(),
    };

    pool.slots.push(slot.clone());
    save_pool(&pool);

    Ok(slot)
}

/// Health-check all canary instances in the pool.
pub async fn canary_health_watch() -> Vec<CanaryHealth> {
    let pool = load_pool();
    let mut results = Vec::with_capacity(pool.slots.len());

    for slot in &pool.slots {
        let health = probe_canary(slot).await;
        results.push(health);
    }

    results
}

/// Maximum age (in hours) before a canary is considered stale and refused for failover.
/// Configurable via `MEMBRANE_CANARY_MAX_AGE_HOURS` (default: 168 = 7 days / ~2 waves).
const DEFAULT_MAX_AGE_HOURS: i64 = 168;

/// Check if a canary slot is stale (`promoted_at` older than max age).
fn is_stale(slot: &CanarySlot) -> bool {
    let max_hours = std::env::var("MEMBRANE_CANARY_MAX_AGE_HOURS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(DEFAULT_MAX_AGE_HOURS);

    let Ok(promoted) = chrono::DateTime::parse_from_rfc3339(&slot.promoted_at) else {
        return true; // unparseable timestamp = stale
    };

    let age = chrono::Utc::now().signed_duration_since(promoted);
    age.num_hours() > max_hours
}

/// Audit the canary pool for staleness.
///
/// Returns a report of each canary's age and staleness status.
/// If `auto_refresh` is true, stale canaries are killed and removed from the pool.
pub async fn staleness_audit(auto_refresh: bool) -> Vec<StalenessReport> {
    let pool = load_pool();
    let mut reports = Vec::with_capacity(pool.slots.len());
    let mut stale_primals = Vec::new();

    for slot in &pool.slots {
        let stale = is_stale(slot);
        let age_hours = chrono::DateTime::parse_from_rfc3339(&slot.promoted_at)
            .map_or(-1, |t| chrono::Utc::now().signed_duration_since(t).num_hours());

        reports.push(StalenessReport {
            primal: slot.primal.clone(),
            commit: slot.commit.clone(),
            promoted_at: slot.promoted_at.clone(),
            age_hours,
            stale,
        });

        if stale {
            stale_primals.push(slot.primal.clone());
        }
    }

    if auto_refresh && !stale_primals.is_empty() {
        let mut pool = load_pool();
        for primal in &stale_primals {
            if let Some(slot) = pool.slots.iter().find(|s| &s.primal == primal) {
                kill_canary(slot).await;
            }
        }
        pool.slots.retain(|s| !stale_primals.contains(&s.primal));
        save_pool(&pool);
    }

    reports
}

/// Staleness report for a single canary slot.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StalenessReport {
    pub primal: String,
    pub commit: String,
    pub promoted_at: String,
    pub age_hours: i64,
    pub stale: bool,
}

/// Return socket paths of all healthy AND fresh canary instances — usable as fallback targets.
///
/// Stale canaries (older than `MEMBRANE_CANARY_MAX_AGE_HOURS`) are refused for failover
/// to prevent rolling back to dangerously outdated binaries.
pub async fn failover_targets() -> Vec<(String, PathBuf)> {
    let pool = load_pool();
    let mut targets = Vec::new();

    for slot in &pool.slots {
        if is_stale(slot) {
            eprintln!(
                "canary: refusing stale failover target {}/{} (promoted {})",
                slot.primal, slot.commit, slot.promoted_at
            );
            continue;
        }
        if slot.socket_path.exists() {
            let health = probe_canary(slot).await;
            if health.alive {
                targets.push((slot.primal.clone(), slot.socket_path.clone()));
            }
        }
    }

    targets
}

/// Promote a canary back to production (rollback scenario).
///
/// Copies the canary binary to the production path and returns the slot.
pub async fn promote_canary(primal: &str, production_path: &Path) -> Result<CanarySlot, String> {
    let pool = load_pool();

    let slot = pool
        .slots
        .iter()
        .find(|s| s.primal == primal)
        .ok_or_else(|| format!("no canary found for {primal}"))?
        .clone();

    if !slot.binary_path.exists() {
        return Err(format!(
            "canary binary missing: {}",
            slot.binary_path.display()
        ));
    }

    // Atomic promotion: .new + rename
    let staging = production_path.with_extension("new");
    std::fs::copy(&slot.binary_path, &staging)
        .map_err(|e| format!("copy canary to production staging: {e}"))?;
    std::fs::rename(&staging, production_path)
        .map_err(|e| format!("atomic canary promote: {e}"))?;

    // Kill the canary instance (it's now production)
    kill_canary(&slot).await;

    // Remove from pool
    let mut pool = load_pool();
    pool.slots.retain(|s| s.primal != primal);
    save_pool(&pool);

    Ok(slot)
}

/// List current canary pool state.
pub fn list() -> Vec<CanarySlot> {
    load_pool().slots
}

/// Kill all canary instances (shutdown).
pub async fn teardown_all() {
    let pool = load_pool();
    for slot in &pool.slots {
        kill_canary(slot).await;
    }
    let empty = CanaryPool::default();
    save_pool(&empty);
}

// ── Internal helpers ──────────────────────────────────────────────────────

fn load_pool() -> CanaryPool {
    let path = pool_state_path();
    if !path.exists() {
        return CanaryPool::default();
    }
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_pool(pool: &CanaryPool) {
    let path = pool_state_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    if let Ok(content) = toml::to_string_pretty(pool) {
        std::fs::write(&path, content).ok();
    }
}

async fn kill_canary(slot: &CanarySlot) {
    if let Some(pid) = slot.pid {
        let _ = tokio::process::Command::new("kill")
            .arg(pid.to_string())
            .output()
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let _ = tokio::process::Command::new("kill")
            .args(["-9", &pid.to_string()])
            .output()
            .await;
    }

    if slot.socket_path.exists() {
        std::fs::remove_file(&slot.socket_path).ok();
    }
}

async fn probe_canary(slot: &CanarySlot) -> CanaryHealth {
    let request = r#"{"jsonrpc":"2.0","method":"health","params":{},"id":1}"#;

    if !slot.socket_path.exists() {
        return CanaryHealth {
            primal: slot.primal.clone(),
            commit: slot.commit.clone(),
            alive: false,
            detail: "socket not found".into(),
        };
    }

    match uds_probe(&slot.socket_path, request).await {
        Ok(response) if response.contains("\"status\"") || response.contains("\"result\"") => {
            CanaryHealth {
                primal: slot.primal.clone(),
                commit: slot.commit.clone(),
                alive: true,
                detail: "healthy".into(),
            }
        }
        Ok(response) => CanaryHealth {
            primal: slot.primal.clone(),
            commit: slot.commit.clone(),
            alive: false,
            detail: format!(
                "unexpected response: {}",
                response.chars().take(80).collect::<String>()
            ),
        },
        Err(e) => CanaryHealth {
            primal: slot.primal.clone(),
            commit: slot.commit.clone(),
            alive: false,
            detail: e,
        },
    }
}

async fn uds_probe(socket_path: &Path, request: &str) -> Result<String, String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_roundtrip() {
        let pool = CanaryPool {
            slots: vec![CanarySlot {
                primal: "beardog".into(),
                binary_path: PathBuf::from("/opt/membrane/canary/beardog"),
                socket_path: PathBuf::from("/run/membrane/canary/beardog.sock"),
                commit: "abc12345".into(),
                promoted_at: "2026-06-11T23:00:00Z".into(),
                pid: Some(12345),
            }],
        };

        let serialized = toml::to_string_pretty(&pool).expect("serialize");
        let deserialized: CanaryPool = toml::from_str(&serialized).expect("deserialize");
        assert_eq!(deserialized.slots.len(), 1);
        assert_eq!(deserialized.slots[0].primal, "beardog");
        assert_eq!(deserialized.slots[0].commit, "abc12345");
    }

    #[test]
    fn empty_pool_serializes() {
        let pool = CanaryPool::default();
        let serialized = toml::to_string_pretty(&pool).expect("serialize");
        assert!(serialized.contains("slots"));
    }
}
