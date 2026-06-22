// SPDX-License-Identifier: AGPL-3.0-or-later

//! Remote canary registry — SSH-reachable warm standby droplets.
//!
//! Remote canaries are provisioned droplets that hold recent primal binaries
//! as failover targets. The registry tracks which gates are available and
//! provides SSH-based health probing.

use std::path::PathBuf;

/// A remote canary droplet entry (SSH-reachable warm standby).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RemoteCanary {
    /// Gate name (e.g. `canary-fieldmouse`).
    pub gate_name: String,
    /// Public IP of the remote droplet.
    pub ip: String,
    /// Provider droplet ID (for lifecycle management).
    pub droplet_id: Option<u64>,
    /// Primals available on this remote canary.
    pub primals: Vec<String>,
    /// When this remote was registered.
    pub registered_at: String,
}

/// Registry of remote canary droplets.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct RemoteCanaryRegistry {
    pub entries: Vec<RemoteCanary>,
}

fn remote_canaries_path() -> PathBuf {
    super::canary::resolve_canary_bin_dir().join("remote-canaries.toml")
}

/// Load the remote canary registry from disk.
pub async fn load_remote_canaries() -> RemoteCanaryRegistry {
    let path = remote_canaries_path();
    tokio::fs::read_to_string(&path).await.map_or_else(
        |_| RemoteCanaryRegistry::default(),
        |s| {
            toml::from_str(&s).unwrap_or_else(|e| {
                tracing::warn!(path = %path.display(), error = %e, "corrupt remote canary TOML — resetting");
                RemoteCanaryRegistry::default()
            })
        },
    )
}

/// Save the remote canary registry to disk.
pub async fn save_remote_canaries(registry: &RemoteCanaryRegistry) {
    let path = remote_canaries_path();
    if let Some(parent) = path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    if let Ok(content) = toml::to_string_pretty(registry) {
        let _ = tokio::fs::write(&path, content).await;
    }
}

/// Register a newly provisioned remote canary droplet.
pub async fn register_remote_canary(
    gate_name: &str,
    ip: &str,
    droplet_id: Option<u64>,
    primals: Vec<String>,
) {
    let mut registry = load_remote_canaries().await;
    registry.entries.retain(|e| e.gate_name != gate_name);
    registry.entries.push(RemoteCanary {
        gate_name: gate_name.to_string(),
        ip: ip.to_string(),
        droplet_id,
        primals,
        registered_at: chrono::Utc::now().to_rfc3339(),
    });
    save_remote_canaries(&registry).await;
}

/// Remove a remote canary from the registry.
pub async fn deregister_remote_canary(gate_name: &str) {
    let mut registry = load_remote_canaries().await;
    registry.entries.retain(|e| e.gate_name != gate_name);
    save_remote_canaries(&registry).await;
}

/// SSH-based health check for a remote canary droplet.
/// Discovers the crypto spine binary via capability registry for the probe socket.
pub async fn remote_health_check(ip: &str) -> bool {
    let spine_binary = cellmembrane_types::MembraneService::binary_for(
        cellmembrane_types::ServiceCapability::CryptoSigner,
    );

    let socket_base = std::env::var(cellmembrane_types::service::ENV_SOCKET_BASE)
        .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_SOCKET_BASE.into());
    let probe_cmd = format!(
        "echo '{{\"jsonrpc\":\"2.0\",\"method\":\"health\",\"id\":1}}' | socat - UNIX-CONNECT:{socket_base}/{spine_binary}.sock 2>/dev/null"
    );

    let user = std::env::var(cellmembrane_types::service::ENV_PROVISION_SSH_USER)
        .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_PROVISION_SSH_USER.into());

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        crate::ssh::exec_on_host(&user, ip, &probe_cmd, 5),
    )
    .await;

    match result {
        Ok(Ok((stdout, code))) => {
            code == 0 && (stdout.contains("\"status\"") || stdout.contains("\"result\""))
        }
        _ => false,
    }
}

/// List all remote canary entries.
pub async fn list_remote_canaries() -> Vec<RemoteCanary> {
    load_remote_canaries().await.entries
}
