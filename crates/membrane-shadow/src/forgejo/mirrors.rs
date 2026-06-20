// SPDX-License-Identifier: AGPL-3.0-or-later

//! Forgejo push mirror operations — K-Derm relay chain to GitHub.
//!
//! These create/list/sync push mirrors via the Forgejo API on golgiBody-inner.
//! The K-Derm diderm relay chain handles actual GitHub propagation:
//! inner (covalent) -> peptidoglycan (metallic) -> golgiBody-ext (ionic) -> GitHub (weak).
//! GitHub SSH write credentials live on golgiBody-ext (trans/shipping face).
//! See `relay.rs` for the Rust-native K-Derm relay chain (replaces bash scripts).

use super::{API_TIMEOUT_READ, API_TIMEOUT_WRITE, auth_header};
use crate::config::ShadowConfig;
use crate::error::{Result, ShadowError};
use serde::{Deserialize, Serialize};

/// Result of triggering a mirror sync.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MirrorSyncResult {
    /// Full repo path (e.g. `ecoPrimals/biomeOS`).
    pub repo: String,
    /// Whether the sync was accepted by Forgejo.
    pub triggered: bool,
    /// HTTP status code from the mirror-sync endpoint.
    pub http_code: u16,
}

/// Push mirror configuration (Forgejo -> external remote like GitHub).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushMirror {
    /// Forgejo-assigned mirror name.
    #[serde(default)]
    pub remote_name: String,
    /// Target remote URL.
    #[serde(default)]
    pub remote_address: String,
    /// Sync interval (e.g. `8h0m0s`).
    #[serde(default)]
    pub interval: String,
    /// Whether the mirror syncs on each commit.
    #[serde(default)]
    pub sync_on_commit: bool,
    /// Creation timestamp.
    #[serde(default)]
    pub created: String,
    /// Last sync timestamp.
    #[serde(default)]
    pub last_update: String,
    /// Last error, if any.
    #[serde(default)]
    pub last_error: String,
}

/// Request payload for creating a push mirror.
#[derive(Debug, Clone, Serialize)]
struct PushMirrorCreateRequest {
    remote_address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    remote_username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    remote_password: Option<String>,
    interval: String,
    sync_on_commit: bool,
    use_ssh: bool,
}

/// Create a push mirror from Forgejo to an external remote (typically GitHub).
/// Uses SSH authentication — Forgejo generates and manages the keypair.
///
/// Shadow for: `nestGate content.mirror.push_create`
#[cfg(feature = "http")]
pub async fn push_mirror_create(
    config: &ShadowConfig,
    full_name: &str,
    remote_url: &str,
) -> Result<PushMirror> {
    let token = config.require_token()?;
    let url = format!("{}/repos/{full_name}/push_mirrors", config.forgejo_api);

    let body = PushMirrorCreateRequest {
        remote_address: remote_url.to_string(),
        remote_username: None,
        remote_password: None,
        interval: cellmembrane_types::service::DEFAULT_PUSH_MIRROR_INTERVAL.to_string(),
        sync_on_commit: true,
        use_ssh: true,
    };

    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .header("Authorization", auth_header(token))
        .json(&body)
        .timeout(API_TIMEOUT_WRITE)
        .send()
        .await?;

    let status = resp.status().as_u16();
    if status == 200 || status == 201 {
        Ok(resp.json().await?)
    } else {
        let msg = resp.text().await.unwrap_or_default();
        Err(ShadowError::ForgejoApi {
            status,
            message: msg,
        })
    }
}

/// List push mirrors for a repo.
///
/// Shadow for: `nestGate content.mirror.push_list`
#[cfg(feature = "http")]
pub async fn push_mirror_list(config: &ShadowConfig, full_name: &str) -> Result<Vec<PushMirror>> {
    let token = config.require_token()?;
    let url = format!("{}/repos/{full_name}/push_mirrors", config.forgejo_api);

    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .header("Authorization", auth_header(token))
        .timeout(API_TIMEOUT_READ)
        .send()
        .await?;

    let status = resp.status().as_u16();
    if status == 200 {
        Ok(resp.json().await?)
    } else {
        let msg = resp.text().await.unwrap_or_default();
        Err(ShadowError::ForgejoApi {
            status,
            message: msg,
        })
    }
}

/// Trigger a sync on all push mirrors for a repo.
///
/// Shadow for: `nestGate content.mirror.push_sync`
#[cfg(feature = "http")]
pub async fn push_mirror_sync(config: &ShadowConfig, full_name: &str) -> Result<MirrorSyncResult> {
    let token = config.require_token()?;
    let url = format!("{}/repos/{full_name}/mirror-sync", config.forgejo_api);

    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .header("Authorization", auth_header(token))
        .timeout(API_TIMEOUT_READ)
        .send()
        .await?;

    let status = resp.status().as_u16();
    Ok(MirrorSyncResult {
        repo: full_name.to_string(),
        triggered: status == 200,
        http_code: status,
    })
}
