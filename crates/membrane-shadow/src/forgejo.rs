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

const API_TIMEOUT_WRITE: std::time::Duration = std::time::Duration::from_secs(30);
const API_TIMEOUT_READ: std::time::Duration = std::time::Duration::from_secs(15);
const API_TIMEOUT_FAST: std::time::Duration = std::time::Duration::from_secs(5);
const PAGE_SIZE: u32 = 50;

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

/// Push mirror configuration (Forgejo → external remote like GitHub).
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

// ── Helpers ─────────────────────────────────────────────────────────

fn auth_header(token: &str) -> String {
    format!("token {token}")
}

/// Validate input contains no shell metacharacters. Prevents injection
/// when arguments are interpolated into remote shell commands.
fn validate_shell_safe(input: &str, field: &str) -> Result<()> {
    const FORBIDDEN: &[char] = &[
        '\'', '"', '`', '$', '\\', ';', '&', '|', '(', ')', '{', '}', '<', '>', '\n', '\r', '\0',
    ];
    if input.chars().any(|c| FORBIDDEN.contains(&c)) {
        return Err(ShadowError::Parse(format!(
            "{field} contains forbidden characters: {input:?}"
        )));
    }
    if input.is_empty() {
        return Err(ShadowError::Parse(format!("{field} cannot be empty")));
    }
    Ok(())
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
        .header("Authorization", auth_header(token))
        .json(&body)
        .timeout(API_TIMEOUT_WRITE)
        .send()
        .await?;

    let status = resp.status().as_u16();
    if status == 201 {
        Ok(resp.json().await?)
    } else {
        let msg = resp.text().await.unwrap_or_default();
        Err(ShadowError::ForgejoApi {
            status,
            message: msg,
        })
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
            "{}/orgs/{org}/repos?limit={PAGE_SIZE}&page={page}",
            config.forgejo_api
        );
        let resp = client
            .get(&url)
            .header("Authorization", auth_header(token))
            .timeout(API_TIMEOUT_READ)
            .send()
            .await?;

        let status = resp.status().as_u16();
        if status != 200 {
            let msg = resp.text().await.unwrap_or_default();
            return Err(ShadowError::ForgejoApi {
                status,
                message: msg,
            });
        }

        let batch: Vec<RepoInfo> = resp.json().await?;
        let count = batch.len();
        all_repos.extend(batch);
        if count < PAGE_SIZE as usize {
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
        .header("Authorization", auth_header(token))
        .timeout(API_TIMEOUT_READ)
        .send()
        .await?;

    let status = resp.status().as_u16();
    if status == 204 {
        Ok(())
    } else {
        let msg = resp.text().await.unwrap_or_default();
        Err(ShadowError::ForgejoApi {
            status,
            message: msg,
        })
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

// ── Push mirror operations (Forgejo API) ─────────────────────────────
//
// These create/list/sync push mirrors via the Forgejo API on golgiBody-inner.
// The K-Derm diderm relay chain handles actual GitHub propagation:
// inner (covalent) → peptidoglycan (metallic) → golgiBody-ext (ionic) → GitHub (weak).
// GitHub SSH write credentials live on golgiBody-ext (trans/shipping face).
// See relay.rs for the Rust-native K-Derm relay chain (replaces bash scripts).

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
        interval: "8h0m0s".to_string(),
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

// ── Token operations (bearDog auth.token.*) ─────────────────────────

/// Default Forgejo data directory (standard package layout).
const DEFAULT_FORGEJO_DATA_DIR: &str = "/opt/forgejo/data";
/// Default Forgejo working directory.
const DEFAULT_FORGEJO_WORK_DIR: &str = "/opt/forgejo";

/// Forgejo data directory on the VPS. Resolution chain:
/// 1. `ShadowConfig.forgejo_data_dir` (from `FORGEJO_DATA_DIR` env)
/// 2. `{forgejo_work_dir}/data` (derived from work dir)
/// 3. Compiled default
fn forgejo_db_path(config: &ShadowConfig) -> String {
    if let Some(ref data_dir) = config.forgejo_data_dir {
        return data_dir.clone();
    }
    if let Some(ref work_dir) = config.forgejo_work_dir {
        return format!("{work_dir}/data");
    }
    DEFAULT_FORGEJO_DATA_DIR.to_string()
}

/// Forgejo working directory on the VPS. Resolution chain:
/// 1. `ShadowConfig.forgejo_work_dir` (from `FORGEJO_WORK_DIR` env)
/// 2. Parent of `forgejo_data_dir` (if data dir ends with `/data`)
/// 3. Compiled default
fn forgejo_work_path(config: &ShadowConfig) -> String {
    if let Some(ref work_dir) = config.forgejo_work_dir {
        return work_dir.clone();
    }
    if let Some(ref data_dir) = config.forgejo_data_dir {
        if let Some(parent) = data_dir.strip_suffix("/data") {
            return parent.to_string();
        }
        return data_dir
            .rsplit_once('/')
            .map_or_else(|| data_dir.clone(), |(parent, _)| parent.to_string());
    }
    DEFAULT_FORGEJO_WORK_DIR.to_string()
}

/// List all Forgejo API tokens (via VPS database).
///
/// Shadow for: `bearDog auth.token.list`
pub async fn token_list(config: &ShadowConfig) -> Result<Vec<TokenInfo>> {
    let db = format!("{}/forgejo.db", forgejo_db_path(config));
    let cmd = format!(
        "sudo -u git sqlite3 '{db}' \
         'SELECT id, name, created_unix FROM access_token ORDER BY id;'"
    );
    let output = ssh::exec(config, &cmd).await?;

    let mut tokens = Vec::new();
    for line in output.lines() {
        let parts: Vec<&str> = line.split('|').collect();
        if parts.len() >= 3 {
            let id = parts[0]
                .trim()
                .parse()
                .map_err(|_| ShadowError::Parse(format!("bad token id: {:?}", parts[0])))?;
            tokens.push(TokenInfo {
                id,
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
pub async fn token_create(config: &ShadowConfig, name: &str, scopes: &str) -> Result<String> {
    validate_shell_safe(name, "token name")?;
    validate_shell_safe(scopes, "token scopes")?;

    let forgejo_dir = forgejo_work_path(config);
    let admin_user = config.forgejo_admin_user.as_deref().unwrap_or("golgiAdmin");

    let cmd = format!(
        "sudo -u git FORGEJO_WORK_DIR='{forgejo_dir}' HOME='{forgejo_dir}' \
         forgejo admin user generate-access-token \
         --username '{admin_user}' --token-name '{name}' --scopes '{scopes}' \
         --raw --config '{forgejo_dir}/custom/conf/app.ini' 2>/dev/null"
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
    let db = format!("{}/forgejo.db", forgejo_db_path(config));
    let cmd = format!(
        "sudo -u git sqlite3 '{db}' \
         'DELETE FROM access_token WHERE id={token_id};'"
    );
    ssh::exec(config, &cmd).await?;

    let verify_cmd = format!(
        "sudo -u git sqlite3 '{db}' \
         'SELECT count(*) FROM access_token WHERE id={token_id};'"
    );
    let remaining = ssh::exec(config, &verify_cmd).await?;

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
    let resp = client.get(&url).timeout(API_TIMEOUT_FAST).send().await?;

    let body: serde_json::Value = resp.json().await?;
    body["version"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| ShadowError::Parse("missing 'version' field in response".into()))
}
