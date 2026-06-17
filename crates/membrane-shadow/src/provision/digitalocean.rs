// SPDX-License-Identifier: AGPL-3.0-or-later
//! `DigitalOcean` API v2 client for droplet lifecycle management.
//!
//! Implements the subset of the DO API needed for fieldMouse provisioning:
//! create droplet, poll status, list SSH keys, destroy droplet.

use super::{DropletState, ProvisionRequest};
use crate::error::{Result, ShadowError};
use serde::{Deserialize, Serialize};

const API_BASE: &str = "https://api.digitalocean.com/v2";
const POLL_INTERVAL_SECS: u64 = 5;
const POLL_TIMEOUT_SECS: u64 = 300;

/// Resolve the DO API token from environment.
/// Checks `DIGITALOCEAN_TOKEN` first, then `DO_TOKEN` (doctl compat).
fn resolve_token() -> Result<String> {
    std::env::var(cellmembrane_types::service::ENV_DIGITALOCEAN_TOKEN)
        .or_else(|_| std::env::var("DO_TOKEN"))
        .map_err(|_| {
            ShadowError::Parse(
                "DIGITALOCEAN_TOKEN or DO_TOKEN not set — required for cloud provisioning".into(),
            )
        })
}

fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| ShadowError::Parse(format!("HTTP client build failed: {e}")))
}

#[derive(Debug, Serialize)]
struct CreateDropletBody<'a> {
    name: &'a str,
    region: &'a str,
    size: &'a str,
    image: &'a str,
    ssh_keys: &'a [String],
    tags: &'a [String],
    monitoring: bool,
    ipv6: bool,
}

#[derive(Debug, Deserialize)]
struct CreateDropletResponse {
    droplet: DropletApiObject,
}

#[derive(Debug, Deserialize)]
struct GetDropletResponse {
    droplet: DropletApiObject,
}

#[derive(Debug, Deserialize)]
struct DropletApiObject {
    id: u64,
    name: String,
    status: String,
    region: RegionObj,
    created_at: String,
    networks: Option<NetworksObj>,
}

#[derive(Debug, Deserialize)]
struct RegionObj {
    slug: String,
}

#[derive(Debug, Deserialize)]
struct NetworksObj {
    v4: Option<Vec<NetworkV4>>,
}

#[derive(Debug, Deserialize)]
struct NetworkV4 {
    ip_address: String,
    #[serde(rename = "type")]
    net_type: String,
}

#[derive(Debug, Deserialize)]
struct ListKeysResponse {
    ssh_keys: Vec<SshKeyObj>,
}

#[derive(Debug, Deserialize)]
struct SshKeyObj {
    id: u64,
    fingerprint: String,
    name: String,
}

/// SSH key info from the DO account.
#[derive(Debug, Clone, Serialize)]
pub struct SshKeyInfo {
    /// Key ID on the provider.
    pub id: u64,
    /// Key fingerprint (for use in create requests).
    pub fingerprint: String,
    /// Human-readable label.
    pub name: String,
}

impl DropletApiObject {
    fn into_state(self, profile: &str) -> DropletState {
        let ip = self
            .networks
            .and_then(|n| n.v4)
            .and_then(|v4| v4.into_iter().find(|n| n.net_type == "public"))
            .map(|n| n.ip_address);
        DropletState {
            id: self.id,
            name: self.name,
            status: self.status,
            ip,
            region: self.region.slug,
            profile: profile.to_string(),
            created_at: self.created_at,
        }
    }
}

/// Create a new droplet. Returns immediately with the droplet in "new" state.
pub async fn create_droplet(req: &ProvisionRequest) -> Result<DropletState> {
    let token = resolve_token()?;
    let http = client()?;

    let body = CreateDropletBody {
        name: &req.name,
        region: &req.region,
        size: &req.size,
        image: &req.image,
        ssh_keys: &req.ssh_keys,
        tags: &req.tags,
        monitoring: true,
        ipv6: false,
    };

    let resp = http
        .post(format!("{API_BASE}/droplets"))
        .bearer_auth(&token)
        .json(&body)
        .send()
        .await
        .map_err(|e| ShadowError::Parse(format!("DO API request failed: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();
        return Err(ShadowError::Parse(format!(
            "DO API create failed ({status}): {body_text}"
        )));
    }

    let parsed: CreateDropletResponse = resp
        .json()
        .await
        .map_err(|e| ShadowError::Parse(format!("DO API response parse failed: {e}")))?;

    Ok(parsed.droplet.into_state(&req.profile))
}

/// Poll a droplet until it reaches "active" status and has a public IP.
/// Times out after `POLL_TIMEOUT_SECS`.
pub async fn wait_until_active(droplet_id: u64, profile: &str) -> Result<DropletState> {
    let token = resolve_token()?;
    let http = client()?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(POLL_TIMEOUT_SECS);

    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(ShadowError::Parse(format!(
                "droplet {droplet_id} did not become active within {POLL_TIMEOUT_SECS}s"
            )));
        }

        tokio::time::sleep(std::time::Duration::from_secs(POLL_INTERVAL_SECS)).await;

        let resp = http
            .get(format!("{API_BASE}/droplets/{droplet_id}"))
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| ShadowError::Parse(format!("DO API poll failed: {e}")))?;

        if !resp.status().is_success() {
            continue;
        }

        let parsed: GetDropletResponse = match resp.json().await {
            Ok(p) => p,
            Err(_) => continue,
        };

        let state = parsed.droplet.into_state(profile);
        if state.status == "active" && state.ip.is_some() {
            return Ok(state);
        }
    }
}

/// Get current state of a droplet by ID.
pub async fn get_droplet(droplet_id: u64) -> Result<DropletState> {
    let token = resolve_token()?;
    let http = client()?;

    let resp = http
        .get(format!("{API_BASE}/droplets/{droplet_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| ShadowError::Parse(format!("DO API get failed: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();
        return Err(ShadowError::Parse(format!(
            "DO API get droplet failed ({status}): {body_text}"
        )));
    }

    let parsed: GetDropletResponse = resp
        .json()
        .await
        .map_err(|e| ShadowError::Parse(format!("DO API response parse failed: {e}")))?;

    Ok(parsed.droplet.into_state("unknown"))
}

/// Destroy a droplet by ID. Idempotent — returns Ok even if already deleted.
pub async fn destroy_droplet(droplet_id: u64) -> Result<()> {
    let token = resolve_token()?;
    let http = client()?;

    let resp = http
        .delete(format!("{API_BASE}/droplets/{droplet_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| ShadowError::Parse(format!("DO API destroy failed: {e}")))?;

    let status = resp.status();
    if status.is_success() || status.as_u16() == 404 {
        Ok(())
    } else {
        let body_text = resp.text().await.unwrap_or_default();
        Err(ShadowError::Parse(format!(
            "DO API destroy failed ({status}): {body_text}"
        )))
    }
}

/// List SSH keys on the account (needed to populate `ssh_keys` in create request).
pub async fn list_ssh_keys() -> Result<Vec<SshKeyInfo>> {
    let token = resolve_token()?;
    let http = client()?;

    let resp = http
        .get(format!("{API_BASE}/account/keys"))
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| ShadowError::Parse(format!("DO API keys list failed: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();
        return Err(ShadowError::Parse(format!(
            "DO API keys list failed ({status}): {body_text}"
        )));
    }

    let parsed: ListKeysResponse = resp
        .json()
        .await
        .map_err(|e| ShadowError::Parse(format!("DO API keys parse failed: {e}")))?;

    Ok(parsed
        .ssh_keys
        .into_iter()
        .map(|k| SshKeyInfo {
            id: k.id,
            fingerprint: k.fingerprint,
            name: k.name,
        })
        .collect())
}

#[derive(Deserialize)]
struct ListDropletsResponse {
    droplets: Vec<DropletApiObject>,
}

/// List all droplets tagged with "membrane".
pub async fn list_membrane_droplets() -> Result<Vec<DropletState>> {
    let token = resolve_token()?;
    let http = client()?;

    let resp = http
        .get(format!("{API_BASE}/droplets?tag_name=membrane"))
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| ShadowError::Parse(format!("DO API list failed: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();
        return Err(ShadowError::Parse(format!(
            "DO API list failed ({status}): {body_text}"
        )));
    }

    let parsed: ListDropletsResponse = resp
        .json()
        .await
        .map_err(|e| ShadowError::Parse(format!("DO API list parse failed: {e}")))?;

    Ok(parsed
        .droplets
        .into_iter()
        .map(|d| d.into_state("unknown"))
        .collect())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use super::*;

    #[test]
    fn resolve_token_fails_without_env() {
        let result = resolve_token();
        if std::env::var("DIGITALOCEAN_TOKEN").is_err() && std::env::var("DO_TOKEN").is_err() {
            assert!(result.is_err());
        }
    }

    #[test]
    fn droplet_api_object_extracts_public_ip() {
        let obj = DropletApiObject {
            id: 123,
            name: "test".into(),
            status: "active".into(),
            region: RegionObj {
                slug: "nyc1".into(),
            },
            created_at: "2026-06-12T00:00:00Z".into(),
            networks: Some(NetworksObj {
                v4: Some(vec![
                    NetworkV4 {
                        ip_address: "10.0.0.1".into(),
                        net_type: "private".into(),
                    },
                    NetworkV4 {
                        ip_address: "1.2.3.4".into(),
                        net_type: "public".into(),
                    },
                ]),
            }),
        };
        let state = obj.into_state("test");
        assert_eq!(state.ip, Some("1.2.3.4".to_string()));
    }

    #[test]
    fn droplet_api_object_no_networks() {
        let obj = DropletApiObject {
            id: 456,
            name: "pending".into(),
            status: "new".into(),
            region: RegionObj {
                slug: "sfo3".into(),
            },
            created_at: "2026-06-12T00:00:00Z".into(),
            networks: None,
        };
        let state = obj.into_state("test");
        assert_eq!(state.ip, None);
    }

    #[test]
    fn into_state_carries_profile() {
        let obj = DropletApiObject {
            id: 789,
            name: "canary".into(),
            status: "active".into(),
            region: RegionObj {
                slug: "nyc1".into(),
            },
            created_at: "2026-06-12T12:00:00Z".into(),
            networks: Some(NetworksObj {
                v4: Some(vec![NetworkV4 {
                    ip_address: "5.6.7.8".into(),
                    net_type: "public".into(),
                }]),
            }),
        };
        let state = obj.into_state("canary-fieldmouse");
        assert_eq!(state.profile, "canary-fieldmouse");
        assert_eq!(state.ip, Some("5.6.7.8".to_string()));
        assert_eq!(state.id, 789);
    }
}
