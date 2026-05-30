// SPDX-License-Identifier: AGPL-3.0-or-later

//! Forgejo API client — repo, mirror, and token management.
//!
//! Shadow domain mapping:
//!   - `content.repo.*`   → nestGate sovereign shadow
//!   - `content.mirror.*` → nestGate sovereign shadow
//!   - `auth.token.*`     → bearDog sovereign shadow

use crate::config::ShadowConfig;
use crate::error::{Result, ShadowError};
use crate::ssh;
use serde::{Deserialize, Serialize};

// ── Types ───────────────────────────────────────────────────────────

/// Forgejo repository info (subset of API response).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoInfo {
    /// Repository name (e.g. `biomeOS`).
    pub name: String,
    /// Full path including org (e.g. `ecoPrimals/biomeOS`).
    pub full_name: String,
    /// Human-readable description.
    pub description: String,
    /// Whether this repo is a pull mirror from GitHub.
    pub mirror: bool,
    /// Default branch name.
    pub default_branch: String,
    /// Mirror sync interval (e.g. `8h0m0s`), empty if not a mirror.
    #[serde(default)]
    pub mirror_interval: String,
    /// ISO 8601 timestamp of last mirror sync.
    #[serde(default)]
    pub mirror_updated: String,
}

/// Forgejo API token metadata (queried from VPS database, not the API).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenInfo {
    /// Database row ID.
    pub id: u64,
    /// Token name as set at creation.
    pub name: String,
    /// Unix timestamp of creation.
    pub created: String,
}

/// Result of triggering a mirror sync on a single repo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MirrorSyncResult {
    /// Full repo path (e.g. `ecoPrimals/biomeOS`).
    pub repo: String,
    /// Whether the sync was accepted by Forgejo.
    pub triggered: bool,
    /// HTTP status code from the mirror-sync endpoint.
    pub http_code: u16,
}

// ── Repo operations (nestGate content.repo.*) ───────────────────────

/// Create a repository on Forgejo.
///
/// Shadow for: `nestGate content.repo.create`
#[cfg(feature = "http")]
pub async fn repo_create(config: &ShadowConfig, org: &str, name: &str) -> Result<RepoInfo> {
    let token = config.require_token()?;
    let url = format!("{}/orgs/{org}/repos", config.forgejo_api);

    let body = serde_json::json!({
        "name": name,
        "auto_init": true,
        "default_branch": "main",
    });

    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .header("Authorization", format!("token {token}"))
        .json(&body)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await?;

    let status = resp.status().as_u16();
    if status == 201 {
        Ok(resp.json().await?)
    } else {
        let msg = resp.text().await.unwrap_or_default();
        Err(ShadowError::ForgejoApi { status, message: msg })
    }
}

/// List repositories for an org.
///
/// Shadow for: `nestGate content.repo.list`
#[cfg(feature = "http")]
pub async fn repo_list(config: &ShadowConfig, org: &str) -> Result<Vec<RepoInfo>> {
    let token = config.require_token()?;
    let client = reqwest::Client::new();
    let mut all_repos = Vec::new();
    let mut page = 1u32;

    loop {
        let url = format!(
            "{}/orgs/{org}/repos?limit=50&page={page}",
            config.forgejo_api
        );
        let resp = client
            .get(&url)
            .header("Authorization", format!("token {token}"))
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await?;

        let status = resp.status().as_u16();
        if status != 200 {
            let msg = resp.text().await.unwrap_or_default();
            return Err(ShadowError::ForgejoApi { status, message: msg });
        }

        let batch: Vec<RepoInfo> = resp.json().await?;
        let count = batch.len();
        all_repos.extend(batch);
        if count < 50 {
            break;
        }
        page += 1;
    }

    Ok(all_repos)
}

/// Delete a repository from Forgejo.
///
/// Shadow for: `nestGate content.repo.delete`
#[cfg(feature = "http")]
pub async fn repo_delete(config: &ShadowConfig, full_name: &str) -> Result<()> {
    let token = config.require_token()?;
    let url = format!("{}/repos/{full_name}", config.forgejo_api);

    let client = reqwest::Client::new();
    let resp = client
        .delete(&url)
        .header("Authorization", format!("token {token}"))
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await?;

    let status = resp.status().as_u16();
    if status == 204 {
        Ok(())
    } else {
        let msg = resp.text().await.unwrap_or_default();
        Err(ShadowError::ForgejoApi { status, message: msg })
    }
}

// ── Mirror operations (nestGate content.mirror.*) ───────────────────

/// Trigger mirror sync for a single repo.
///
/// Shadow for: `nestGate content.mirror.sync`
#[cfg(feature = "http")]
pub async fn mirror_sync(config: &ShadowConfig, full_name: &str) -> Result<MirrorSyncResult> {
    let token = config.require_token()?;
    let url = format!("{}/repos/{full_name}/mirror-sync", config.forgejo_api);

    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .header("Authorization", format!("token {token}"))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await?;

    let status = resp.status().as_u16();
    Ok(MirrorSyncResult {
        repo: full_name.to_string(),
        triggered: status == 200,
        http_code: status,
    })
}

/// Get mirror status for a repo.
///
/// Shadow for: `nestGate content.mirror.status`
#[cfg(feature = "http")]
pub async fn mirror_status(config: &ShadowConfig, full_name: &str) -> Result<RepoInfo> {
    let token = config.require_token()?;
    let url = format!("{}/repos/{full_name}", config.forgejo_api);

    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .header("Authorization", format!("token {token}"))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await?;

    let status = resp.status().as_u16();
    if status == 200 {
        Ok(resp.json().await?)
    } else {
        let msg = resp.text().await.unwrap_or_default();
        Err(ShadowError::ForgejoApi { status, message: msg })
    }
}

// ── Token operations (bearDog auth.token.*) ─────────────────────────

/// List all Forgejo API tokens (via VPS database).
///
/// Shadow for: `bearDog auth.token.list`
pub async fn token_list(config: &ShadowConfig) -> Result<Vec<TokenInfo>> {
    let output = ssh::exec(
        config,
        "sudo -u git sqlite3 /opt/forgejo/data/forgejo.db \
         \"SELECT id, name, created_unix FROM access_token ORDER BY id;\"",
    )
    .await?;

    let mut tokens = Vec::new();
    for line in output.lines() {
        let parts: Vec<&str> = line.split('|').collect();
        if parts.len() >= 3 {
            tokens.push(TokenInfo {
                id: parts[0].trim().parse().unwrap_or(0),
                name: parts[1].trim().to_string(),
                created: parts[2].trim().to_string(),
            });
        }
    }
    Ok(tokens)
}

/// Create a Forgejo API token (via Forgejo admin CLI on VPS).
///
/// Shadow for: `bearDog auth.token.create`
pub async fn token_create(
    config: &ShadowConfig,
    name: &str,
    scopes: &str,
) -> Result<String> {
    let cmd = format!(
        "sudo -u git FORGEJO_WORK_DIR=/opt/forgejo HOME=/opt/forgejo \
         forgejo admin user generate-access-token \
         --username golgiAdmin --token-name '{name}' --scopes '{scopes}' \
         --raw --config /opt/forgejo/custom/conf/app.ini 2>/dev/null"
    );

    let output = ssh::exec(config, &cmd).await?;
    let token = output.trim().to_string();
    if token.is_empty() {
        Err(ShadowError::Parse("empty token returned".into()))
    } else {
        Ok(token)
    }
}

/// Revoke a Forgejo API token by database ID.
///
/// Shadow for: `bearDog auth.token.revoke`
pub async fn token_revoke(config: &ShadowConfig, token_id: u64) -> Result<()> {
    ssh::exec(
        config,
        &format!(
            "sudo -u git sqlite3 /opt/forgejo/data/forgejo.db \
             \"DELETE FROM access_token WHERE id={token_id};\""
        ),
    )
    .await?;

    let remaining = ssh::exec(
        config,
        &format!(
            "sudo -u git sqlite3 /opt/forgejo/data/forgejo.db \
             \"SELECT count(*) FROM access_token WHERE id={token_id};\""
        ),
    )
    .await?;

    if remaining.trim() == "0" {
        Ok(())
    } else {
        Err(ShadowError::Parse(format!(
            "token {token_id} still exists after delete"
        )))
    }
}

/// Get Forgejo server version.
#[cfg(feature = "http")]
pub async fn version(config: &ShadowConfig) -> Result<String> {
    let url = format!("{}/version", config.forgejo_api);
    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await?;

    let body: serde_json::Value = resp.json().await?;
    Ok(body["version"]
        .as_str()
        .unwrap_or("unknown")
        .to_string())
}
