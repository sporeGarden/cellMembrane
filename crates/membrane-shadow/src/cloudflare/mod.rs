// SPDX-License-Identifier: AGPL-3.0-or-later

//! Cloudflare API v4 client — agentic outer membrane management.
//!
//! Wraps the Cloudflare REST API for DNS, cache, SSL, and zone operations
//! on the outer membrane (`primals.eco`). All operations route through
//! golgiBody-ext where the API token lives in `tower.env`.
//!
//! Design constraints:
//! - Read-only ops work without a write-scoped token
//! - All mutations logged for audit trail
//! - Inner membrane never contacts Cloudflare directly

pub mod dns;

pub use dns::{DnsRecord, DnsRecordParams, dns_create, dns_delete, dns_list, dns_update};

use crate::error::{Result, ShadowError};
use serde::{Deserialize, Serialize};

const CF_API_BASE: &str = cellmembrane_types::service::DEFAULT_CLOUDFLARE_API;
const API_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(cellmembrane_types::service::DEFAULT_CLOUDFLARE_TIMEOUT_SECS);

// ── Configuration ───────────────────────────────────────────────────

/// Cloudflare API credentials resolved from environment.
#[derive(Debug, Clone)]
pub struct CloudflareConfig {
    /// API token (scoped: DNS read/write, Cache purge, Zone read).
    pub api_token: String,
    /// Zone ID for the target domain (resolved from zone name if not set).
    pub zone_id: Option<String>,
}

impl CloudflareConfig {
    /// Resolve from environment variables.
    ///
    /// - `CLOUDFLARE_API_TOKEN` or `CF_API_TOKEN`
    /// - `CLOUDFLARE_ZONE_ID` or `CF_ZONE_ID` (optional — resolved from zone name)
    pub fn from_env() -> Result<Self> {
        let api_token = std::env::var(cellmembrane_types::service::ENV_CLOUDFLARE_TOKEN)
            .or_else(|_| std::env::var(cellmembrane_types::service::ENV_CF_API_TOKEN))
            .map_err(|_| {
                ShadowError::Parse("CLOUDFLARE_API_TOKEN or CF_API_TOKEN required".into())
            })?;

        let zone_id = std::env::var(cellmembrane_types::service::ENV_CLOUDFLARE_ZONE)
            .or_else(|_| std::env::var(cellmembrane_types::service::ENV_CF_ZONE_ID))
            .ok();

        Ok(Self { api_token, zone_id })
    }

    fn client() -> Result<reqwest::Client> {
        reqwest::Client::builder()
            .timeout(API_TIMEOUT)
            .build()
            .map_err(|e| ShadowError::Parse(format!("HTTP client error: {e}")))
    }

    fn auth_header(&self) -> (&str, String) {
        ("Authorization", format!("Bearer {}", self.api_token))
    }
}

// ── API Response Types ──────────────────────────────────────────────

/// Cloudflare API envelope.
#[derive(Debug, Deserialize)]
struct CfResponse<T> {
    success: bool,
    #[serde(default)]
    errors: Vec<CfError>,
    result: Option<T>,
}

impl<T> CfResponse<T> {
    /// Extract the result or return a formatted Cloudflare API error.
    fn into_result(self) -> Result<T> {
        if self.success {
            self.result
                .ok_or_else(|| ShadowError::CloudflareApi("empty result".into()))
        } else {
            Err(ShadowError::CloudflareApi(format_cf_errors(&self.errors)))
        }
    }

    /// Extract the result, defaulting to `T::default()` if result is None but success is true.
    fn into_result_or_default(self) -> Result<T>
    where
        T: Default,
    {
        if self.success {
            Ok(self.result.unwrap_or_default())
        } else {
            Err(ShadowError::CloudflareApi(format_cf_errors(&self.errors)))
        }
    }
}

fn format_cf_errors(errors: &[CfError]) -> String {
    if errors.is_empty() {
        return "unknown error".into();
    }
    errors
        .iter()
        .map(|e| format!("[{}] {}", e.code, e.message))
        .collect::<Vec<_>>()
        .join("; ")
}

#[derive(Debug, Deserialize)]
struct CfError {
    #[serde(default)]
    code: u32,
    #[serde(default)]
    message: String,
}

/// Zone info from the Cloudflare API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoneInfo {
    /// Zone ID.
    pub id: String,
    /// Domain name.
    pub name: String,
    /// Zone status.
    pub status: String,
}

/// SSL/TLS settings for a zone.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SslSettings {
    /// TLS mode (off, flexible, full, strict).
    pub value: String,
}

/// Zone setting entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoneSetting {
    /// Setting ID (e.g. `always_use_https`, `min_tls_version`).
    pub id: String,
    /// Current value.
    pub value: serde_json::Value,
    /// Whether modifiable.
    #[serde(default)]
    pub modified_on: Option<String>,
}

// ── Cache Operations ────────────────────────────────────────────────

/// Purge cached files by URL pattern or purge everything.
pub async fn cache_purge(
    cf: &CloudflareConfig,
    zone: &str,
    urls: Option<&[&str]>,
    purge_everything: bool,
) -> Result<String> {
    let zone_id = resolve_zone_id(cf, zone).await?;
    let client = CloudflareConfig::client()?;
    let (header_key, header_val) = cf.auth_header();

    let payload = if purge_everything {
        serde_json::json!({ "purge_everything": true })
    } else if let Some(file_urls) = urls {
        serde_json::json!({ "files": file_urls })
    } else {
        return Err(ShadowError::Parse(
            "cache.purge requires --urls or --all".into(),
        ));
    };

    let resp = client
        .post(format!("{CF_API_BASE}/zones/{zone_id}/purge_cache"))
        .header(header_key, &header_val)
        .json(&payload)
        .send()
        .await
        .map_err(|e| ShadowError::CloudflareApi(format!("request failed: {e}")))?;

    let body: CfResponse<serde_json::Value> = resp
        .json()
        .await
        .map_err(|e| ShadowError::CloudflareApi(format!("parse failed: {e}")))?;

    body.into_result().map(|_: serde_json::Value| ())?;

    if purge_everything {
        Ok("cache purged (all)".into())
    } else {
        Ok(format!(
            "cache purged ({} URLs)",
            urls.map_or(0, <[&str]>::len)
        ))
    }
}

// ── SSL/TLS Operations ──────────────────────────────────────────────

/// Get the current SSL/TLS encryption mode for a zone.
pub async fn ssl_settings(cf: &CloudflareConfig, zone: &str) -> Result<SslSettings> {
    let zone_id = resolve_zone_id(cf, zone).await?;
    let client = CloudflareConfig::client()?;
    let (header_key, header_val) = cf.auth_header();

    let resp = client
        .get(format!("{CF_API_BASE}/zones/{zone_id}/settings/ssl"))
        .header(header_key, &header_val)
        .send()
        .await
        .map_err(|e| ShadowError::CloudflareApi(format!("request failed: {e}")))?;

    let body: CfResponse<SslSettings> = resp
        .json()
        .await
        .map_err(|e| ShadowError::CloudflareApi(format!("parse failed: {e}")))?;

    body.into_result()
}

// ── Zone Settings ───────────────────────────────────────────────────

/// Get all zone settings (security headers, always-HTTPS, HSTS, etc.).
pub async fn zone_settings(cf: &CloudflareConfig, zone: &str) -> Result<Vec<ZoneSetting>> {
    let zone_id = resolve_zone_id(cf, zone).await?;
    let client = CloudflareConfig::client()?;
    let (header_key, header_val) = cf.auth_header();

    let resp = client
        .get(format!("{CF_API_BASE}/zones/{zone_id}/settings"))
        .header(header_key, &header_val)
        .send()
        .await
        .map_err(|e| ShadowError::CloudflareApi(format!("request failed: {e}")))?;

    let body: CfResponse<Vec<ZoneSetting>> = resp
        .json()
        .await
        .map_err(|e| ShadowError::CloudflareApi(format!("parse failed: {e}")))?;

    body.into_result_or_default()
}

// ── Zone ID Resolution ──────────────────────────────────────────────

/// Resolve zone ID from the configured value or by querying the API.
async fn resolve_zone_id(cf: &CloudflareConfig, zone_name: &str) -> Result<String> {
    if let Some(ref id) = cf.zone_id {
        return Ok(id.clone());
    }

    let client = CloudflareConfig::client()?;
    let (header_key, header_val) = cf.auth_header();

    let resp = client
        .get(format!("{CF_API_BASE}/zones?name={zone_name}"))
        .header(header_key, &header_val)
        .send()
        .await
        .map_err(|e| ShadowError::CloudflareApi(format!("zone lookup failed: {e}")))?;

    let body: CfResponse<Vec<ZoneInfo>> = resp
        .json()
        .await
        .map_err(|e| ShadowError::CloudflareApi(format!("parse failed: {e}")))?;

    let zones = body.into_result_or_default()?;
    zones
        .into_iter()
        .find(|z| z.name == zone_name)
        .map(|z| z.id)
        .ok_or_else(|| {
            ShadowError::Parse(format!(
                "Zone '{zone_name}' not found in Cloudflare account"
            ))
        })
}

// ── Dispatch ────────────────────────────────────────────────────────

/// Dispatch `cloudflare.*` CLI commands.
pub async fn dispatch(cmd: &str, args: &[&str]) -> Result<crate::ShadowOutcome> {
    let cf = CloudflareConfig::from_env()?;

    match cmd {
        "cloudflare.dns.list" => {
            let zone = extract_zone_arg(args)?;
            let record_type = extract_flag(args, "--type");
            let name = extract_flag(args, "--name");
            let records = dns_list(&cf, zone, record_type, name).await?;
            let json = serde_json::to_string_pretty(&records)?;
            Ok(crate::ShadowOutcome::ok(json))
        }
        "cloudflare.dns.create" => {
            let zone = extract_zone_arg(args)?;
            let rtype = require_flag(args, "--type")?;
            let name = require_flag(args, "--name")?;
            let content = require_flag(args, "--content")?;
            let ttl: u32 = extract_flag(args, "--ttl")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1);
            let proxied = extract_flag(args, "--proxied").is_some_and(|v| v == "true" || v == "1");
            let record = dns_create(&cf, zone, rtype, name, content, ttl, proxied).await?;
            let json = serde_json::to_string_pretty(&record)?;
            Ok(crate::ShadowOutcome::ok(json))
        }
        "cloudflare.dns.update" => {
            let zone = extract_zone_arg(args)?;
            let record_id = require_flag(args, "--id")?;
            let params = DnsRecordParams {
                zone,
                record_type: require_flag(args, "--type")?,
                name: require_flag(args, "--name")?,
                content: require_flag(args, "--content")?,
                ttl: extract_flag(args, "--ttl")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1),
                proxied: extract_flag(args, "--proxied").is_some_and(|v| v == "true" || v == "1"),
            };
            let record = dns_update(&cf, record_id, &params).await?;
            let json = serde_json::to_string_pretty(&record)?;
            Ok(crate::ShadowOutcome::ok(json))
        }
        "cloudflare.dns.delete" => {
            let zone = extract_zone_arg(args)?;
            let record_id = require_flag(args, "--id")?;
            dns_delete(&cf, zone, record_id).await?;
            Ok(crate::ShadowOutcome::ok("record deleted"))
        }
        "cloudflare.cache.purge" => {
            let zone = extract_zone_arg(args)?;
            let purge_all = args.contains(&"--all");
            let urls: Vec<&str> = extract_multi_flag(args, "--url");
            let url_refs = if urls.is_empty() {
                None
            } else {
                Some(urls.as_slice())
            };
            let msg = cache_purge(&cf, zone, url_refs, purge_all).await?;
            Ok(crate::ShadowOutcome::ok(msg))
        }
        "cloudflare.ssl.settings" => {
            let zone = extract_zone_arg(args)?;
            let settings = ssl_settings(&cf, zone).await?;
            let json = serde_json::to_string_pretty(&settings)?;
            Ok(crate::ShadowOutcome::ok(json))
        }
        "cloudflare.zone.settings" => {
            let zone = extract_zone_arg(args)?;
            let settings = zone_settings(&cf, zone).await?;
            let json = serde_json::to_string_pretty(&settings)?;
            Ok(crate::ShadowOutcome::ok(json))
        }
        _ => Ok(crate::ShadowOutcome::fail(format!(
            "unknown cloudflare command: {cmd}"
        ))),
    }
}

// ── Argument Helpers ────────────────────────────────────────────────

fn extract_zone_arg<'a>(args: &[&'a str]) -> Result<&'a str> {
    extract_flag(args, "--zone").ok_or_else(|| {
        ShadowError::Parse("--zone <domain> required (e.g. --zone primals.eco)".into())
    })
}

fn extract_flag<'a>(args: &[&'a str], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|&a| a == flag)
        .and_then(|i| args.get(i + 1).copied())
}

fn require_flag<'a>(args: &[&'a str], flag: &str) -> Result<&'a str> {
    extract_flag(args, flag).ok_or_else(|| ShadowError::Parse(format!("{flag} is required")))
}

fn extract_multi_flag<'a>(args: &[&'a str], flag: &str) -> Vec<&'a str> {
    let mut values = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == flag {
            if let Some(&val) = args.get(i + 1) {
                values.push(val);
                i += 2;
                continue;
            }
        }
        i += 1;
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_flag_finds_value() {
        let args = ["--zone", "primals.eco", "--type", "A"];
        assert_eq!(extract_flag(&args, "--zone"), Some("primals.eco"));
        assert_eq!(extract_flag(&args, "--type"), Some("A"));
        assert_eq!(extract_flag(&args, "--name"), None);
    }

    #[test]
    fn require_flag_errors_when_missing() {
        let args = ["--zone", "primals.eco"];
        assert!(require_flag(&args, "--type").is_err());
        assert!(require_flag(&args, "--zone").is_ok());
    }

    #[test]
    fn extract_multi_flag_collects_all() {
        let args = ["--url", "/a", "--url", "/b", "--zone", "x"];
        let urls = extract_multi_flag(&args, "--url");
        assert_eq!(urls, vec!["/a", "/b"]);
    }

    #[test]
    fn extract_zone_arg_requires_zone() {
        let args = ["--type", "A"];
        assert!(extract_zone_arg(&args).is_err());

        let args = ["--zone", "primals.eco"];
        assert_eq!(extract_zone_arg(&args).unwrap(), "primals.eco");
    }

    #[test]
    fn config_requires_token_env() {
        // Without env vars set, from_env should fail.
        // We can't safely mutate env in tests, so verify the error path
        // by testing the resolution logic directly.
        let result = std::env::var(cellmembrane_types::service::ENV_CLOUDFLARE_TOKEN)
            .or_else(|_| std::env::var(cellmembrane_types::service::ENV_CF_API_TOKEN));
        if result.is_err() {
            assert!(CloudflareConfig::from_env().is_err());
        }
    }

    #[test]
    fn dns_record_deserializes() {
        let json = r#"{
            "id": "abc123",
            "type": "A",
            "name": "www.primals.eco",
            "content": "1.2.3.4",
            "ttl": 300,
            "proxied": true
        }"#;
        let record: DnsRecord = serde_json::from_str(json).unwrap();
        assert_eq!(record.id, "abc123");
        assert_eq!(record.record_type, "A");
        assert_eq!(record.name, "www.primals.eco");
        assert_eq!(record.content, "1.2.3.4");
        assert_eq!(record.ttl, 300);
        assert!(record.proxied);
    }

    #[test]
    fn cf_response_success_parses() {
        let json = r#"{
            "success": true,
            "errors": [],
            "result": [
                {"id": "r1", "type": "A", "name": "test.primals.eco", "content": "5.6.7.8", "ttl": 1, "proxied": false}
            ]
        }"#;
        let resp: CfResponse<Vec<DnsRecord>> = serde_json::from_str(json).unwrap();
        assert!(resp.success);
        assert!(resp.errors.is_empty());
        assert_eq!(resp.result.unwrap().len(), 1);
    }

    #[test]
    fn cf_response_error_parses() {
        let json = r#"{
            "success": false,
            "errors": [{"code": 9103, "message": "Unknown X-Auth-Key"}],
            "result": null
        }"#;
        let resp: CfResponse<Vec<DnsRecord>> = serde_json::from_str(json).unwrap();
        assert!(!resp.success);
        assert_eq!(resp.errors[0].code, 9103);
        assert!(resp.result.is_none());
    }
}
