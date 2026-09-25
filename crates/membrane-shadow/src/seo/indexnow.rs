// SPDX-License-Identifier: AGPL-3.0-or-later

//! IndexNow client — instant crawler notification for Bing, DuckDuckGo,
//! Yandex, and Seznam.
//!
//! IndexNow is a push protocol: sites notify search engines immediately
//! when content changes, instead of waiting for crawlers to discover updates.
//!
//! ## Protocol
//!
//! POST `https://api.indexnow.org/indexnow` with JSON body:
//! ```json
//! {
//!   "host": "detroit.primals.eco",
//!   "key": "<verification-key>",
//!   "keyLocation": "https://detroit.primals.eco/<key>.txt",
//!   "urlList": ["https://detroit.primals.eco/", ...]
//! }
//! ```
//!
//! The key file must be accessible at `https://<host>/<key>.txt`.
//! HTTP 200/202 = accepted. One submission fans out to all participating engines.

use super::SiteConfig;
use crate::error::{Result, ShadowError};

const INDEXNOW_ENDPOINT: &str = "https://api.indexnow.org/indexnow";
const INDEXNOW_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// Fetch all URLs from a site's sitemap.
///
/// Parses the sitemap XML to extract `<loc>` URLs. Falls back to just
/// the homepage if the sitemap is unreachable or unparseable.
#[cfg(feature = "http")]
async fn fetch_sitemap_urls(site: &SiteConfig) -> Vec<String> {
    let homepage = format!("https://{}/", site.host);
    let client = match crate::http_client(INDEXNOW_TIMEOUT) {
        Ok(c) => c,
        Err(_) => return vec![homepage],
    };

    let resp = match client.get(site.sitemap).send().await {
        Ok(r) => r,
        Err(_) => return vec![homepage],
    };

    let body = match resp.text() {
        Ok(t) => t,
        Err(_) => return vec![homepage],
    };

    // Simple XML parsing — extract <loc>...</loc> without pulling in an XML crate
    let mut urls = Vec::new();
    for segment in body.split("<loc>") {
        if let Some(end) = segment.find("</loc>") {
            let url = segment[..end].trim();
            if url.starts_with("https://") {
                urls.push(url.to_string());
            }
        }
    }

    if urls.is_empty() {
        vec![homepage]
    } else {
        urls
    }
}

/// Submit a site's URLs to IndexNow.
///
/// Fetches the sitemap to discover all URLs, then POSTs the batch to
/// the IndexNow API endpoint. Returns a human-readable result line.
#[cfg(feature = "http")]
pub async fn submit(site: &SiteConfig) -> Result<String> {
    let urls = fetch_sitemap_urls(site).await;
    let url_count = urls.len();

    let payload = serde_json::json!({
        "host": site.host,
        "key": site.indexnow_key,
        "keyLocation": format!("https://{}/{}.txt", site.host, site.indexnow_key),
        "urlList": urls,
    });

    let client = crate::http_client(INDEXNOW_TIMEOUT)?;
    let resp = client
        .post(INDEXNOW_ENDPOINT)
        .header("Content-Type", "application/json; charset=utf-8")
        .json(&payload)
        .send()
        .await
        .map_err(|e| ShadowError::config(format!("IndexNow POST failed: {e}")))?;

    let code = resp.status().as_u16();
    match code {
        200 | 202 => Ok(format!(
            "{}: {} URLs submitted (HTTP {})",
            site.host, url_count, code
        )),
        _ => {
            let body = resp.text().unwrap_or_default();
            Err(ShadowError::config(format!(
                "IndexNow {}: HTTP {} — {}",
                site.host,
                code,
                &body[..body.len().min(200)]
            )))
        }
    }
}

/// Stub for non-http builds.
#[cfg(not(feature = "http"))]
pub async fn submit(_site: &SiteConfig) -> Result<String> {
    Err(ShadowError::config("IndexNow requires 'http' feature"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexnow_endpoint_is_https() {
        assert!(INDEXNOW_ENDPOINT.starts_with("https://"));
    }
}
