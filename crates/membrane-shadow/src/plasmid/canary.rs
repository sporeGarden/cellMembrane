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
use tracing::{debug, warn};

use cellmembrane_types::service::{
    DEFAULT_CANARY_BIN_DIR, DEFAULT_CANARY_SOCKET_DIR, ENV_CANARY_BIN_DIR, ENV_CANARY_SOCKET_DIR,
};

use super::canary_remote::remote_health_check;
pub use super::canary_remote::{
    deregister_remote_canary, list_remote_canaries, load_remote_canaries, register_remote_canary,
};

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
    PathBuf::from(
        std::env::var(ENV_CANARY_SOCKET_DIR).unwrap_or_else(|_| DEFAULT_CANARY_SOCKET_DIR.into()),
    )
}

pub fn resolve_canary_bin_dir() -> PathBuf {
    PathBuf::from(
        std::env::var(ENV_CANARY_BIN_DIR).unwrap_or_else(|_| DEFAULT_CANARY_BIN_DIR.into()),
    )
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

    tokio::fs::create_dir_all(&socket_dir)
        .await
        .map_err(|e| format!("create canary socket dir: {e}"))?;
    tokio::fs::create_dir_all(&bin_dir)
        .await
        .map_err(|e| format!("create canary bin dir: {e}"))?;

    // Kill any existing canary for this primal
    let mut pool = load_pool().await;
    if let Some(existing) = pool.slots.iter().find(|s| s.primal == primal) {
        kill_canary(existing).await;
    }
    pool.slots.retain(|s| s.primal != primal);

    let canary_binary = bin_dir.join(primal);
    let socket_path = socket_dir.join(format!("{primal}.sock"));

    // Remove stale socket
    if socket_path.exists() {
        tokio::fs::remove_file(&socket_path).await.ok();
    }

    // Stage binary to canary directory
    tokio::fs::copy(old_binary, &canary_binary)
        .await
        .map_err(|e| format!("stage canary binary: {e}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&canary_binary, std::fs::Permissions::from_mode(0o755))
            .await
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
    save_pool(&pool).await;

    Ok(slot)
}

/// Health-check all canary instances in the pool.
pub async fn canary_health_watch() -> Vec<CanaryHealth> {
    let pool = load_pool().await;
    let mut results = Vec::with_capacity(pool.slots.len());

    for slot in &pool.slots {
        let health = probe_canary(slot).await;
        results.push(health);
    }

    results
}

/// Maximum age (in hours) before a canary is considered stale and refused for failover.
/// Configurable via `MEMBRANE_CANARY_MAX_AGE_HOURS` (default: 168 = 7 days / ~2 waves).
const DEFAULT_MAX_AGE_HOURS: i64 = cellmembrane_types::service::DEFAULT_CANARY_MAX_AGE_HOURS;

/// Check if a canary slot is stale (`promoted_at` older than max age).
fn is_stale(slot: &CanarySlot) -> bool {
    let max_hours = std::env::var(cellmembrane_types::service::ENV_CANARY_MAX_AGE_HOURS)
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
pub async fn staleness_audit(auto_refresh: bool) -> Vec<CanaryStalenessReport> {
    let pool = load_pool().await;
    let mut reports = Vec::with_capacity(pool.slots.len());
    let mut stale_primals = Vec::new();

    for slot in &pool.slots {
        let stale = is_stale(slot);
        let age_hours = chrono::DateTime::parse_from_rfc3339(&slot.promoted_at).map_or(-1, |t| {
            chrono::Utc::now().signed_duration_since(t).num_hours()
        });

        reports.push(CanaryStalenessReport {
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
        let mut pool = load_pool().await;
        for primal in &stale_primals {
            if let Some(slot) = pool.slots.iter().find(|s| &s.primal == primal) {
                kill_canary(slot).await;
            }
        }
        pool.slots.retain(|s| !stale_primals.contains(&s.primal));
        save_pool(&pool).await;
    }

    reports
}

/// Staleness report for a single canary slot.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CanaryStalenessReport {
    pub primal: String,
    pub commit: String,
    pub promoted_at: String,
    pub age_hours: i64,
    pub stale: bool,
}

/// A failover target — either a local UDS socket or a remote SSH-reachable canary.
#[derive(Debug, Clone, serde::Serialize)]
pub enum FailoverTarget {
    /// Local canary (same host, isolated UDS socket).
    Local { primal: String, socket: PathBuf },
    /// Remote canary (SSH-reachable VPS droplet).
    Remote {
        primal: String,
        ip: String,
        gate: String,
    },
}

/// Return all healthy AND fresh canary instances — usable as fallback targets.
///
/// Combines local canary pool (UDS sockets) with remote canary droplets (SSH health probe).
/// Stale canaries (older than `MEMBRANE_CANARY_MAX_AGE_HOURS`) are refused for failover
/// to prevent rolling back to dangerously outdated binaries.
pub async fn failover_targets() -> Vec<FailoverTarget> {
    let mut targets = Vec::new();

    // Local canary pool
    let pool = load_pool().await;
    for slot in &pool.slots {
        if is_stale(slot) {
            debug!(
                primal = %slot.primal,
                commit = %slot.commit,
                promoted_at = %slot.promoted_at,
                "refusing stale failover target"
            );
            continue;
        }
        if slot.socket_path.exists() {
            let health = probe_canary(slot).await;
            if health.alive {
                targets.push(FailoverTarget::Local {
                    primal: slot.primal.clone(),
                    socket: slot.socket_path.clone(),
                });
            }
        }
    }

    // Remote canary droplets (SSH health probe)
    let remote_canaries = load_remote_canaries().await;
    for remote in &remote_canaries.entries {
        if remote_health_check(&remote.ip).await {
            for primal in &remote.primals {
                targets.push(FailoverTarget::Remote {
                    primal: primal.clone(),
                    ip: remote.ip.clone(),
                    gate: remote.gate_name.clone(),
                });
            }
        } else {
            warn!(
                gate = %remote.gate_name,
                ip = %remote.ip,
                "remote canary unreachable — skipping"
            );
        }
    }

    targets
}

/// Promote a canary back to production (rollback scenario).
///
/// Copies the canary binary to the production path and returns the slot.
pub async fn promote_canary(primal: &str, production_path: &Path) -> Result<CanarySlot, String> {
    let pool = load_pool().await;

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
    tokio::fs::copy(&slot.binary_path, &staging)
        .await
        .map_err(|e| format!("copy canary to production staging: {e}"))?;
    tokio::fs::rename(&staging, production_path)
        .await
        .map_err(|e| format!("atomic canary promote: {e}"))?;

    // Kill the canary instance (it's now production)
    kill_canary(&slot).await;

    // Remove from pool
    let mut pool = load_pool().await;
    pool.slots.retain(|s| s.primal != primal);
    save_pool(&pool).await;

    Ok(slot)
}

/// List current canary pool state.
pub async fn list() -> Vec<CanarySlot> {
    load_pool().await.slots
}

/// Kill all canary instances (shutdown).
pub async fn teardown_all() {
    let pool = load_pool().await;
    for slot in &pool.slots {
        kill_canary(slot).await;
    }
    let empty = CanaryPool::default();
    save_pool(&empty).await;
}

// ── Internal helpers ──────────────────────────────────────────────────────

async fn load_pool() -> CanaryPool {
    let path = pool_state_path();
    tokio::fs::read_to_string(&path).await.map_or_else(
        |_| CanaryPool::default(),
        |s| {
            toml::from_str(&s).unwrap_or_else(|e| {
                tracing::warn!(path = %path.display(), error = %e, "corrupt canary pool TOML — resetting");
                CanaryPool::default()
            })
        },
    )
}

async fn save_pool(pool: &CanaryPool) {
    let path = pool_state_path();
    if let Some(parent) = path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    if let Ok(content) = toml::to_string_pretty(pool) {
        let _ = tokio::fs::write(&path, content).await;
    }
}

async fn kill_canary(slot: &CanarySlot) {
    if let Some(pid) = slot.pid {
        super::graceful_kill(pid, 300).await;
    }

    let _ = tokio::fs::remove_file(&slot.socket_path).await;
}

async fn probe_canary(slot: &CanarySlot) -> CanaryHealth {
    let request = crate::jsonrpc::HEALTH_REQUEST;

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
    crate::jsonrpc::call(socket_path, request).await
}

#[cfg(test)]
mod tests {
    use super::super::canary_remote::{RemoteCanary, RemoteCanaryRegistry};
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

    #[test]
    fn remote_canary_registry_roundtrip() {
        let registry = RemoteCanaryRegistry {
            entries: vec![RemoteCanary {
                gate_name: "canary-fieldmouse".into(),
                ip: "1.2.3.4".into(),
                droplet_id: Some(98765),
                primals: vec!["beardog".into(), "songbird".into(), "toadstool".into()],
                registered_at: "2026-06-12T12:00:00Z".into(),
            }],
        };

        let serialized = toml::to_string_pretty(&registry).expect("serialize");
        let deserialized: RemoteCanaryRegistry = toml::from_str(&serialized).expect("deserialize");
        assert_eq!(deserialized.entries.len(), 1);
        assert_eq!(deserialized.entries[0].gate_name, "canary-fieldmouse");
        assert_eq!(deserialized.entries[0].ip, "1.2.3.4");
        assert_eq!(deserialized.entries[0].droplet_id, Some(98765));
        assert_eq!(deserialized.entries[0].primals.len(), 3);
    }

    #[test]
    fn empty_registry_serializes() {
        let registry = RemoteCanaryRegistry::default();
        let serialized = toml::to_string_pretty(&registry).expect("serialize");
        assert!(serialized.contains("entries"));
    }

    #[test]
    fn staleness_detection() {
        let fresh_slot = CanarySlot {
            primal: "beardog".into(),
            binary_path: PathBuf::from("/opt/membrane/canary/beardog"),
            socket_path: PathBuf::from("/run/membrane/canary/beardog.sock"),
            commit: "fresh123".into(),
            promoted_at: chrono::Utc::now().to_rfc3339(),
            pid: None,
        };
        assert!(!is_stale(&fresh_slot));

        let stale_slot = CanarySlot {
            primal: "beardog".into(),
            binary_path: PathBuf::from("/opt/membrane/canary/beardog"),
            socket_path: PathBuf::from("/run/membrane/canary/beardog.sock"),
            commit: "stale456".into(),
            promoted_at: "2020-01-01T00:00:00Z".into(),
            pid: None,
        };
        assert!(is_stale(&stale_slot));
    }
}
