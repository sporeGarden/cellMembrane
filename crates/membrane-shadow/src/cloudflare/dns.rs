// SPDX-License-Identifier: AGPL-3.0-or-later

//! Cloudflare DNS record operations — list, create, update, delete.

use super::{
    CF_API_BASE, CfResponse, CloudflareConfig, cf_parse_err, cf_request_err, resolve_zone_id,
};
use crate::error::Result;
use serde::{Deserialize, Serialize};

/// A single DNS record from the Cloudflare API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsRecord {
    /// Cloudflare record ID.
    pub id: String,
    /// Record type (A, AAAA, CNAME, TXT, MX, etc.).
    #[serde(rename = "type")]
    pub record_type: String,
    /// Fully qualified hostname.
    pub name: String,
    /// Record content (IP address, CNAME target, etc.).
    pub content: String,
    /// TTL in seconds (1 = automatic when proxied).
    pub ttl: u32,
    /// Whether traffic is proxied through Cloudflare.
    pub proxied: bool,
}

/// Parameters for DNS record mutation (create or update).
pub struct DnsRecordParams<'a> {
    /// Zone name (e.g. `primals.eco`).
    pub zone: &'a str,
    /// Record type (A, AAAA, CNAME, TXT, etc.).
    pub record_type: &'a str,
    /// Hostname.
    pub name: &'a str,
    /// Record content/value.
    pub content: &'a str,
    /// TTL in seconds (1 = automatic).
    pub ttl: u32,
    /// Whether proxied through Cloudflare.
    pub proxied: bool,
}

/// List DNS records for a zone, optionally filtered by type or name.
pub async fn dns_list(
    cf: &CloudflareConfig,
    zone: &str,
    record_type: Option<&str>,
    name_filter: Option<&str>,
) -> Result<Vec<DnsRecord>> {
    let zone_id = resolve_zone_id(cf, zone).await?;
    let client = CloudflareConfig::client()?;
    let (header_key, header_val) = cf.auth_header();

    let mut url = format!("{CF_API_BASE}/zones/{zone_id}/dns_records?per_page=100");
    if let Some(rt) = record_type {
        url.push_str("&type=");
        url.push_str(rt);
    }
    if let Some(name) = name_filter {
        url.push_str("&name=");
        url.push_str(name);
    }

    let body: CfResponse<Vec<DnsRecord>> = client
        .get(&url)
        .header(header_key, &header_val)
        .send()
        .await
        .map_err(|e| cf_request_err("dns_list", e))?
        .json()
        .await
        .map_err(|e| cf_parse_err("dns_list", e))?;

    body.into_result_or_default()
}

/// Create a DNS record.
pub async fn dns_create(
    cf: &CloudflareConfig,
    zone: &str,
    record_type: &str,
    name: &str,
    content: &str,
    ttl: u32,
    proxied: bool,
) -> Result<DnsRecord> {
    let zone_id = resolve_zone_id(cf, zone).await?;
    let client = CloudflareConfig::client()?;
    let (header_key, header_val) = cf.auth_header();

    let payload = serde_json::json!({
        "type": record_type,
        "name": name,
        "content": content,
        "ttl": ttl,
        "proxied": proxied,
    });

    let resp = client
        .post(format!("{CF_API_BASE}/zones/{zone_id}/dns_records"))
        .header(header_key, &header_val)
        .json(&payload)
        .send()
        .await
        .map_err(|e| cf_request_err("dns_create", e))?;

    let body: CfResponse<DnsRecord> = resp
        .json()
        .await
        .map_err(|e| cf_parse_err("dns_create", e))?;

    body.into_result()
}

/// Update an existing DNS record by ID.
pub async fn dns_update(
    cf: &CloudflareConfig,
    record_id: &str,
    params: &DnsRecordParams<'_>,
) -> Result<DnsRecord> {
    let zone_id = resolve_zone_id(cf, params.zone).await?;
    let client = CloudflareConfig::client()?;
    let (header_key, header_val) = cf.auth_header();

    let payload = serde_json::json!({
        "type": params.record_type,
        "name": params.name,
        "content": params.content,
        "ttl": params.ttl,
        "proxied": params.proxied,
    });

    let resp = client
        .put(format!(
            "{CF_API_BASE}/zones/{zone_id}/dns_records/{record_id}"
        ))
        .header(header_key, &header_val)
        .json(&payload)
        .send()
        .await
        .map_err(|e| cf_request_err("dns_update", e))?;

    let body: CfResponse<DnsRecord> = resp
        .json()
        .await
        .map_err(|e| cf_parse_err("dns_update", e))?;

    body.into_result()
}

/// Delete a DNS record by ID.
pub async fn dns_delete(cf: &CloudflareConfig, zone: &str, record_id: &str) -> Result<()> {
    let zone_id = resolve_zone_id(cf, zone).await?;
    let client = CloudflareConfig::client()?;
    let (header_key, header_val) = cf.auth_header();

    let resp = client
        .delete(format!(
            "{CF_API_BASE}/zones/{zone_id}/dns_records/{record_id}"
        ))
        .header(header_key, &header_val)
        .send()
        .await
        .map_err(|e| cf_request_err("dns_delete", e))?;

    let body: CfResponse<serde_json::Value> = resp
        .json()
        .await
        .map_err(|e| cf_parse_err("dns_delete", e))?;

    body.into_result().map(|_: serde_json::Value| ())
}
