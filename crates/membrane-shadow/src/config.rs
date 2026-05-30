// SPDX-License-Identifier: AGPL-3.0-or-later

//! Configuration for membrane shadow functions.
//!
//! Resolves credentials and endpoints from environment, config files,
//! and defaults — same priority chain as the bash `membrane.sh` script.

use crate::error::{Result, ShadowError};

/// Shadow function configuration — all the context needed to reach golgiBody.
#[derive(Debug, Clone)]
pub struct ShadowConfig {
    /// SSH host alias (default: "golgi" from ~/.ssh/config).
    pub ssh_host: String,
    /// Forgejo API base URL.
    pub forgejo_api: String,
    /// Forgejo API token (resolved lazily).
    pub forgejo_token: Option<String>,
    /// ecoPrimals root on the VPS.
    pub vps_root: String,
    /// SSH connect timeout in seconds.
    pub ssh_timeout: u32,
}

impl Default for ShadowConfig {
    fn default() -> Self {
        Self {
            ssh_host: "golgi".into(),
            forgejo_api: "https://git.primals.eco/api/v1".into(),
            forgejo_token: None,
            vps_root: "/opt/ecoPrimals".into(),
            ssh_timeout: 10,
        }
    }
}

impl ShadowConfig {
    /// Build config from environment and config files.
    ///
    /// Token resolution priority:
    /// 1. `FORGEJO_TOKEN` env var
    /// 2. `~/.config/forgejo/token` file
    pub async fn from_env() -> Self {
        let mut cfg = Self {
            ssh_host: std::env::var("GOLGI_HOST").unwrap_or_else(|_| "golgi".into()),
            forgejo_api: std::env::var("FORGEJO_API")
                .unwrap_or_else(|_| "https://git.primals.eco/api/v1".into()),
            vps_root: std::env::var("VPS_ECOPRIMALS_ROOT")
                .unwrap_or_else(|_| "/opt/ecoPrimals".into()),
            ssh_timeout: std::env::var("SSH_TIMEOUT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10),
            forgejo_token: None,
        };

        cfg.forgejo_token = resolve_token().await;
        cfg
    }

    /// Returns the token or an error.
    pub fn require_token(&self) -> Result<&str> {
        self.forgejo_token
            .as_deref()
            .ok_or(ShadowError::NoToken)
    }
}

/// Resolve Forgejo token from env or file.
async fn resolve_token() -> Option<String> {
    if let Ok(token) = std::env::var("FORGEJO_TOKEN") {
        if !token.is_empty() {
            return Some(token);
        }
    }

    let home = std::env::var("HOME").ok()?;
    let path = format!("{home}/.config/forgejo/token");
    let token = tokio::fs::read_to_string(&path).await.ok()?;
    let token = token.trim().to_string();
    if token.is_empty() {
        None
    } else {
        Some(token)
    }
}
