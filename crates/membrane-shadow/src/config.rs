// SPDX-License-Identifier: AGPL-3.0-or-later

//! Configuration for membrane shadow functions.
//!
//! Resolution priority (highest to lowest):
//! 1. Environment variables (`GOLGI_HOST`, `FORGEJO_API`, etc.)
//! 2. `membrane.toml` in the workspace or `/etc/membrane/membrane.toml`
//! 3. Compiled defaults (host aliases as last-resort fallbacks)

use crate::error::{Result, ShadowError};

/// Default `systemctl` service filter regex — matches nucleus services on the VPS.
const DEFAULT_SERVICE_FILTER: &str = "membrane|forgejo|caddy|knot|hbb|fail2ban";

/// Shadow function configuration — all the context needed to reach the VPS.
#[derive(Debug, Clone)]
pub struct ShadowConfig {
    /// SSH host alias (resolved from env → membrane.toml → default "golgi").
    pub ssh_host: String,
    /// SSH host alias for the outer membrane (ext). Defaults to "golgi-ext".
    pub ssh_host_ext: String,
    /// Forgejo API base URL.
    pub forgejo_api: String,
    /// Forgejo API token (resolved lazily).
    pub forgejo_token: Option<String>,
    /// ecoPrimals root on the VPS.
    pub vps_root: String,
    /// SSH connect timeout in seconds.
    pub ssh_timeout: u32,
    /// Forgejo data directory on VPS (e.g. `/opt/forgejo/data`).
    pub forgejo_data_dir: Option<String>,
    /// Forgejo working directory on VPS (e.g. `/opt/forgejo`).
    pub forgejo_work_dir: Option<String>,
    /// Forgejo admin username for token ops.
    pub forgejo_admin_user: Option<String>,
    /// Grep filter for systemd service discovery.
    pub service_filter: String,
}

/// Partial config from `membrane.toml` [membrane.provider] and [membrane.layers].
#[derive(Debug, Default)]
struct TomlOverrides {
    ssh_host: Option<String>,
    forgejo_api: Option<String>,
    vps_root: Option<String>,
    forgejo_admin_user: Option<String>,
}

impl Default for ShadowConfig {
    fn default() -> Self {
        Self {
            ssh_host: std::env::var("MEMBRANE_SSH_HOST").unwrap_or_else(|_| "golgi".into()),
            ssh_host_ext: std::env::var("MEMBRANE_SSH_HOST_EXT")
                .or_else(|_| std::env::var("GOLGI_EXT_HOST"))
                .unwrap_or_else(|_| "golgi-ext".into()),
            forgejo_api: String::new(),
            forgejo_token: None,
            vps_root: "/opt/ecoPrimals".into(),
            ssh_timeout: 10,
            forgejo_data_dir: None,
            forgejo_work_dir: None,
            forgejo_admin_user: None,
            service_filter: DEFAULT_SERVICE_FILTER.into(),
        }
    }
}

impl ShadowConfig {
    /// Build config from environment, `membrane.toml`, and defaults.
    ///
    /// Token resolution priority:
    /// 1. `FORGEJO_TOKEN` env var
    /// 2. `~/.config/forgejo/token` file
    pub async fn from_env() -> Self {
        let toml_overrides = load_toml_overrides().await;

        let mut cfg = Self {
            ssh_host: std::env::var("GOLGI_HOST")
                .ok()
                .or(toml_overrides.ssh_host)
                .unwrap_or_else(|| "golgi".into()),
            ssh_host_ext: std::env::var("GOLGI_EXT_HOST").unwrap_or_else(|_| "golgi-ext".into()),
            forgejo_api: std::env::var("FORGEJO_API")
                .ok()
                .or(toml_overrides.forgejo_api)
                .unwrap_or_else(discover_forgejo_api),
            vps_root: std::env::var("VPS_ECOPRIMALS_ROOT")
                .ok()
                .or(toml_overrides.vps_root)
                .unwrap_or_else(|| "/opt/ecoPrimals".into()),
            ssh_timeout: std::env::var("SSH_TIMEOUT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10),
            forgejo_token: None,
            forgejo_data_dir: std::env::var("FORGEJO_DATA_DIR").ok(),
            forgejo_work_dir: std::env::var("FORGEJO_WORK_DIR").ok(),
            forgejo_admin_user: std::env::var("FORGEJO_ADMIN_USER")
                .ok()
                .or(toml_overrides.forgejo_admin_user),
            service_filter: std::env::var("MEMBRANE_SERVICE_FILTER")
                .unwrap_or_else(|_| DEFAULT_SERVICE_FILTER.into()),
        };

        cfg.forgejo_token = resolve_token().await;
        cfg
    }

    /// Returns the token or an error.
    pub fn require_token(&self) -> Result<&str> {
        self.forgejo_token.as_deref().ok_or(ShadowError::NoToken)
    }
}

/// Discover Forgejo API URL from the ecosystem manifest sync config.
fn discover_forgejo_api() -> String {
    if let Ok(root) = crate::temporal::resolve_workspace_root() {
        if let Ok(m) = crate::manifest::load_from_workspace(&root) {
            let base = m.sync.forgejo_base_url.trim_end_matches('/');
            if !base.is_empty() {
                return format!("{base}/api/v1");
            }
        }
    }
    String::new()
}

/// Load overrides from `membrane.toml` if present.
async fn load_toml_overrides() -> TomlOverrides {
    let candidates = toml_search_paths();
    for path in &candidates {
        if let Ok(content) = tokio::fs::read_to_string(path).await {
            if let Ok(parsed) = content.parse::<toml::Table>() {
                return extract_overrides(&parsed);
            }
        }
    }
    TomlOverrides::default()
}

fn toml_search_paths() -> Vec<String> {
    let mut paths = Vec::with_capacity(4);

    if let Ok(root) = std::env::var("ECOPRIMALS_ROOT") {
        paths.push(format!("{root}/gardens/cellMembrane/membrane.toml"));
    }

    if let Ok(root) = crate::temporal::resolve_workspace_root() {
        let root_str = root.to_string_lossy();
        let p = format!("{root_str}/gardens/cellMembrane/membrane.toml");
        if !paths.contains(&p) {
            paths.push(p);
        }
    }

    paths.push("/etc/membrane/membrane.toml".into());
    paths
}

fn extract_overrides(parsed: &toml::Table) -> TomlOverrides {
    let Some(membrane) = parsed.get("membrane").and_then(|v| v.as_table()) else {
        return TomlOverrides::default();
    };

    let ssh_host = membrane
        .get("layers")
        .and_then(|l| l.get("inner"))
        .and_then(|i| i.get("ssh_alias").or_else(|| i.get("host")))
        .and_then(|h| h.as_str())
        .map(String::from);

    let forgejo_admin_user = membrane
        .get("provider")
        .and_then(|p| p.as_table())
        .and_then(|p| p.get("admin_user"))
        .and_then(|u| u.as_str())
        .map(String::from);

    let sync = parsed.get("sync").and_then(|s| s.as_table());
    let forgejo_api = sync
        .and_then(|s| s.get("forgejo_base_url"))
        .and_then(|u| u.as_str())
        .map(|base| format!("{}/api/v1", base.trim_end_matches('/')));

    TomlOverrides {
        ssh_host,
        forgejo_api,
        vps_root: None,
        forgejo_admin_user,
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
    if token.is_empty() { None } else { Some(token) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_overrides_prefers_ssh_alias() {
        let toml_str = r#"
[membrane.layers.inner]
host = "golgiBody"
ssh_alias = "golgi"
ip = "157.230.3.183"
"#;
        let parsed: toml::Table = toml_str.parse().unwrap();
        let overrides = extract_overrides(&parsed);
        assert_eq!(overrides.ssh_host.as_deref(), Some("golgi"));
    }

    #[test]
    fn extract_overrides_falls_back_to_host() {
        let toml_str = r#"
[membrane.layers.inner]
host = "golgiBody"
ip = "157.230.3.183"
"#;
        let parsed: toml::Table = toml_str.parse().unwrap();
        let overrides = extract_overrides(&parsed);
        assert_eq!(overrides.ssh_host.as_deref(), Some("golgiBody"));
    }

    #[test]
    fn extract_overrides_handles_missing_layers() {
        let toml_str = r#"
[membrane]
name = "test"
"#;
        let parsed: toml::Table = toml_str.parse().unwrap();
        let overrides = extract_overrides(&parsed);
        assert_eq!(overrides.ssh_host, None);
    }

    #[test]
    fn extract_overrides_reads_forgejo_api() {
        let toml_str = r#"
[membrane.layers.inner]
host = "test"

[sync]
forgejo_base_url = "https://git.primals.eco"
"#;
        let parsed: toml::Table = toml_str.parse().unwrap();
        let overrides = extract_overrides(&parsed);
        assert_eq!(
            overrides.forgejo_api.as_deref(),
            Some("https://git.primals.eco/api/v1")
        );
    }

    #[test]
    fn default_config_uses_golgi() {
        let cfg = ShadowConfig::default();
        assert_eq!(cfg.ssh_host, "golgi");
        assert_eq!(cfg.ssh_timeout, 10);
    }

    #[test]
    fn default_service_filter_includes_expected() {
        assert!(DEFAULT_SERVICE_FILTER.contains("membrane"));
        assert!(DEFAULT_SERVICE_FILTER.contains("forgejo"));
        assert!(DEFAULT_SERVICE_FILTER.contains("caddy"));
    }
}
