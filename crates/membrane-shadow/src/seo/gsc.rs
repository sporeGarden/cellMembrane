// SPDX-License-Identifier: AGPL-3.0-or-later

//! Pure-Rust Google Search Console API client.
//!
//! Authenticates via Google service account (RS256 JWT → OAuth2 access token).
//! Uses the membrane HTTP client — no `reqwest`, no `google-api-python-client`.
//!
//! ## OAuth2 Flow
//!
//! 1. Load service account JSON (private key + client email)
//! 2. Create JWT assertion (RS256-signed, 1h expiry)
//! 3. Exchange at `https://oauth2.googleapis.com/token` for access token
//! 4. Use `Authorization: Bearer <token>` for all GSC API calls

#[cfg(feature = "http")]
use crate::error::{Result, ShadowError};

const TOKEN_URI: &str = "https://oauth2.googleapis.com/token";
const GSC_API_BASE: &str = "https://searchconsole.googleapis.com";
const API_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

// ── Client ──────────────────────────────────────────────────────────

/// Google Search Console API client with cached access token.
pub struct GscClient {
    #[cfg(feature = "http")]
    access_token: String,
    #[cfg(not(feature = "http"))]
    _phantom: (),
}

impl GscClient {
    /// Create a GSC client from environment/defaults.
    ///
    /// Loads the service account JSON, signs a JWT, and exchanges for
    /// an access token. Returns `Err` if credentials are missing or
    /// the token exchange fails.
    #[cfg(feature = "http")]
    pub async fn from_env() -> Result<Self> {
        let creds_path = std::env::var(super::ENV_GSC_CREDENTIALS)
            .unwrap_or_else(|_| super::DEFAULT_CREDENTIALS_PATH.to_string());

        let creds_json = tokio::fs::read_to_string(&creds_path)
            .await
            .map_err(|e| ShadowError::config(format!("GSC credentials at {creds_path}: {e}")))?;

        let sa: ServiceAccount = serde_json::from_str(&creds_json)
            .map_err(|e| ShadowError::config(format!("GSC credentials parse: {e}")))?;

        let access_token = exchange_jwt_for_token(&sa).await?;
        Ok(Self { access_token })
    }

    #[cfg(not(feature = "http"))]
    pub async fn from_env() -> Result<Self> {
        Err(ShadowError::config(
            "seo module requires 'http' feature".to_string(),
        ))
    }
}

// ── Service Account Types ───────────────────────────────────────────

#[derive(serde::Deserialize)]
struct ServiceAccount {
    client_email: String,
    private_key: String,
    #[allow(dead_code)]
    project_id: String,
}

#[derive(serde::Deserialize)]
struct TokenResponse {
    access_token: String,
    #[allow(dead_code)]
    expires_in: u64,
}

// ── OAuth2 JWT Exchange ─────────────────────────────────────────────

#[cfg(feature = "http")]
async fn exchange_jwt_for_token(sa: &ServiceAccount) -> Result<String> {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // JWT header
    let header = serde_json::json!({"alg": "RS256", "typ": "JWT"});
    let header_b64 = URL_SAFE_NO_PAD.encode(header.to_string().as_bytes());

    // JWT claims
    let claims = serde_json::json!({
        "iss": sa.client_email,
        "scope": "https://www.googleapis.com/auth/webmasters",
        "aud": TOKEN_URI,
        "iat": now,
        "exp": now + 3600,
    });
    let claims_b64 = URL_SAFE_NO_PAD.encode(claims.to_string().as_bytes());

    // Sign with RS256
    let message = format!("{header_b64}.{claims_b64}");
    let signature = rs256_sign(&sa.private_key, message.as_bytes())?;
    let sig_b64 = URL_SAFE_NO_PAD.encode(&signature);

    let jwt = format!("{message}.{sig_b64}");

    // Exchange JWT for access token via form-encoded POST
    let body = format!(
        "grant_type={}&assertion={}",
        urlencod("urn:ietf:params:oauth:grant-type:jwt-bearer"),
        urlencod(&jwt)
    );

    let client = crate::http_client(API_TIMEOUT)?;
    let resp = client
        .post(TOKEN_URI)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .raw_body(body.into_bytes())
        .send()
        .await
        .map_err(|e| ShadowError::config(format!("GSC token exchange: {e}")))?;

    let resp_body = resp.text().map_err(|e| {
        ShadowError::config(format!("GSC token response: {e}"))
    })?;

    let token: TokenResponse = serde_json::from_str(&resp_body).map_err(|e| {
        ShadowError::config(format!(
            "GSC token parse: {e} — body: {}",
            &resp_body[..resp_body.len().min(200)]
        ))
    })?;

    Ok(token.access_token)
}

/// RS256 signing using ring's RSA-PKCS1-SHA256.
#[cfg(feature = "http")]
fn rs256_sign(pem_key: &str, message: &[u8]) -> Result<Vec<u8>> {
    // Strip PEM armor and decode
    let pem_body: String = pem_key
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect();
    let der = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        &pem_body,
    )
    .map_err(|e| ShadowError::config(format!("RSA key decode: {e}")))?;

    // ring requires PKCS#8 DER format (Google service accounts use this)
    let key_pair = ring::signature::RsaKeyPair::from_pkcs8(&der)
        .map_err(|e| ShadowError::config(format!("RSA key parse: {e}")))?;

    let rng = ring::rand::SystemRandom::new();
    let mut sig = vec![0u8; key_pair.public().modulus_len()];
    key_pair
        .sign(&ring::signature::RSA_PKCS1_SHA256, &rng, message, &mut sig)
        .map_err(|e| ShadowError::config(format!("RSA sign: {e}")))?;

    Ok(sig)
}

/// Minimal URL encoding for form bodies.
fn urlencod(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            _ => format!("%{:02X}", c as u32),
        })
        .collect()
}

// ── GSC API Calls ───────────────────────────────────────────────────

#[cfg(feature = "http")]
async fn gsc_get(client: &GscClient, path: &str) -> Result<String> {
    let url = format!("{GSC_API_BASE}{path}");
    let http = crate::http_client(API_TIMEOUT)?;

    let resp = http
        .get(&url)
        .header("Authorization", &format!("Bearer {}", client.access_token))
        .send()
        .await
        .map_err(|e| ShadowError::config(format!("GSC API GET {path}: {e}")))?;

    resp.text()
        .map_err(|e| ShadowError::config(format!("GSC API response: {e}")))
}

#[cfg(feature = "http")]
async fn gsc_post(client: &GscClient, path: &str, body: &serde_json::Value) -> Result<String> {
    let url = format!("{GSC_API_BASE}{path}");
    let http = crate::http_client(API_TIMEOUT)?;

    let resp = http
        .post(&url)
        .header("Authorization", &format!("Bearer {}", client.access_token))
        .header("Content-Type", "application/json")
        .json(body)
        .send()
        .await
        .map_err(|e| ShadowError::config(format!("GSC API POST {path}: {e}")))?;

    resp.text()
        .map_err(|e| ShadowError::config(format!("GSC API response: {e}")))
}

// ── Public API Functions ────────────────────────────────────────────

/// Full status report for `seo.status`.
#[cfg(feature = "http")]
pub async fn status_report(client: &GscClient, days: u32) -> Result<String> {
    let prop = urlencod(super::GSC_PROPERTY);

    // Sitemaps
    let sm_path = format!(
        "/webmasters/v3/sites/{prop}/sitemaps"
    );
    let sm_body = gsc_get(client, &sm_path).await?;
    let sitemaps: serde_json::Value = serde_json::from_str(&sm_body).unwrap_or_default();

    // Analytics
    let end = time::OffsetDateTime::now_utc().date();
    let start = end - time::Duration::days(i64::from(days));
    let analytics_path = format!(
        "/webmasters/v3/sites/{prop}/searchAnalytics/query"
    );
    let analytics_body = gsc_post(
        client,
        &analytics_path,
        &serde_json::json!({
            "startDate": start.to_string(),
            "endDate": end.to_string(),
            "dimensions": ["page"],
            "rowLimit": 500
        }),
    )
    .await?;
    let analytics: serde_json::Value = serde_json::from_str(&analytics_body).unwrap_or_default();

    // Homepage inspection
    let inspect_path = "/v1/urlInspection/index:inspect";
    let inspect_body = gsc_post(
        client,
        inspect_path,
        &serde_json::json!({
            "inspectionUrl": "https://sporeprint.primals.eco/",
            "siteUrl": super::GSC_PROPERTY
        }),
    )
    .await?;
    let inspect: serde_json::Value = serde_json::from_str(&inspect_body).unwrap_or_default();

    // Format report
    let sm_contents = sitemaps
        .pointer("/sitemap/0/contents/0")
        .unwrap_or(&serde_json::Value::Null);
    let submitted = sm_contents
        .get("submitted")
        .and_then(|v| v.as_str())
        .unwrap_or("0");
    let indexed = sm_contents
        .get("indexed")
        .and_then(|v| v.as_str())
        .unwrap_or("0");

    let rows = analytics
        .get("rows")
        .and_then(|v| v.as_array())
        .map_or(0, Vec::len);
    let total_imp: f64 = analytics
        .get("rows")
        .and_then(|v| v.as_array())
        .map_or(0.0, |rows| {
            rows.iter()
                .filter_map(|r| r.get("impressions").and_then(|v| v.as_f64()))
                .sum()
        });
    let total_clicks: f64 = analytics
        .get("rows")
        .and_then(|v| v.as_array())
        .map_or(0.0, |rows| {
            rows.iter()
                .filter_map(|r| r.get("clicks").and_then(|v| v.as_f64()))
                .sum()
        });

    let home_verdict = inspect
        .pointer("/inspectionResult/indexStatusResult/verdict")
        .and_then(|v| v.as_str())
        .unwrap_or("?");
    let home_crawl = inspect
        .pointer("/inspectionResult/indexStatusResult/lastCrawlTime")
        .and_then(|v| v.as_str())
        .map(|s| &s[..s.len().min(10)])
        .unwrap_or("?");

    let report = serde_json::json!({
        "sitemap": {"submitted": submitted, "indexed": indexed},
        "analytics": {
            "pages": rows,
            "impressions": total_imp,
            "clicks": total_clicks,
            "days": days,
        },
        "homepage": {"verdict": home_verdict, "last_crawl": home_crawl},
    });

    Ok(serde_json::to_string_pretty(&report)?)
}

/// Submit all sitemaps for `seo.submit`.
#[cfg(feature = "http")]
pub async fn submit_sitemap(client: &GscClient) -> Result<String> {
    let prop = urlencod(super::GSC_PROPERTY);
    let http = crate::http_client(API_TIMEOUT)?;
    let mut results = Vec::new();

    for sm_url in super::SITEMAP_URLS {
        let sitemap = urlencod(sm_url);
        let path = format!("/webmasters/v3/sites/{prop}/sitemaps/{sitemap}");
        let url = format!("{GSC_API_BASE}{path}");
        match http
            .put(&url)
            .header("Authorization", &format!("Bearer {}", client.access_token))
            .send()
            .await
        {
            Ok(_) => results.push(format!("submitted: {sm_url}")),
            Err(e) => results.push(format!("FAILED: {sm_url} — {e}")),
        }
    }

    Ok(results.join("; "))
}

/// Inspect a URL for `seo.inspect`.
#[cfg(feature = "http")]
pub async fn inspect_url(client: &GscClient, url: &str) -> Result<String> {
    let body = gsc_post(
        client,
        "/v1/urlInspection/index:inspect",
        &serde_json::json!({
            "inspectionUrl": url,
            "siteUrl": super::GSC_PROPERTY
        }),
    )
    .await?;

    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
    let ir = parsed
        .pointer("/inspectionResult/indexStatusResult")
        .cloned()
        .unwrap_or_default();

    let report = serde_json::json!({
        "url": url,
        "verdict": ir.get("verdict"),
        "state": ir.get("coverageState"),
        "last_crawl": ir.get("lastCrawlTime"),
        "canonical": ir.get("googleCanonical"),
        "robots": ir.get("robotsTxtState"),
    });

    Ok(serde_json::to_string_pretty(&report)?)
}

/// Compact cascade probe for `seo.cascade`.
#[cfg(feature = "http")]
pub async fn cascade_probe(client: &GscClient) -> Result<String> {
    let prop = urlencod(super::GSC_PROPERTY);

    // Sitemaps (lightweight)
    let sm_path = format!("/webmasters/v3/sites/{prop}/sitemaps");
    let sm_body = gsc_get(client, &sm_path).await?;
    let sitemaps: serde_json::Value = serde_json::from_str(&sm_body).unwrap_or_default();

    let sm_contents = sitemaps
        .pointer("/sitemap/0/contents/0")
        .unwrap_or(&serde_json::Value::Null);
    let submitted = sm_contents
        .get("submitted")
        .and_then(|v| v.as_str())
        .unwrap_or("0");
    let indexed = sm_contents
        .get("indexed")
        .and_then(|v| v.as_str())
        .unwrap_or("0");

    Ok(format!(
        "  [seo] sitemap {submitted}/{indexed} indexed (rust-native)"
    ))
}

// ── Google Indexing API (URL Notification) ───────────────────────────

const INDEXING_API_BASE: &str = "https://indexing.googleapis.com";

/// Notify Google Indexing API about updated URLs.
///
/// Uses `urlNotifications:publish` with `URL_UPDATED` type.
/// Requires the service account to have Indexing API enabled and
/// the site verified in Search Console.
///
/// Note: The Indexing API uses a different OAuth scope than the
/// Search Console API. We create a separate token for it.
#[cfg(feature = "http")]
pub async fn url_notify(urls: &[String]) -> Result<String> {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

    let creds_path = std::env::var(super::ENV_GSC_CREDENTIALS)
        .unwrap_or_else(|_| super::DEFAULT_CREDENTIALS_PATH.to_string());

    let creds_json = tokio::fs::read_to_string(&creds_path)
        .await
        .map_err(|e| ShadowError::config(format!("Indexing API credentials at {creds_path}: {e}")))?;

    let sa: ServiceAccount = serde_json::from_str(&creds_json)
        .map_err(|e| ShadowError::config(format!("Indexing API credentials parse: {e}")))?;

    // Indexing API requires its own scope
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let header = serde_json::json!({"alg": "RS256", "typ": "JWT"});
    let header_b64 = URL_SAFE_NO_PAD.encode(header.to_string().as_bytes());
    let claims = serde_json::json!({
        "iss": sa.client_email,
        "scope": "https://www.googleapis.com/auth/indexing",
        "aud": TOKEN_URI,
        "iat": now,
        "exp": now + 3600,
    });
    let claims_b64 = URL_SAFE_NO_PAD.encode(claims.to_string().as_bytes());

    let message = format!("{header_b64}.{claims_b64}");
    let signature = rs256_sign(&sa.private_key, message.as_bytes())?;
    let sig_b64 = URL_SAFE_NO_PAD.encode(&signature);
    let jwt = format!("{message}.{sig_b64}");

    let body = format!(
        "grant_type={}&assertion={}",
        urlencod("urn:ietf:params:oauth:grant-type:jwt-bearer"),
        urlencod(&jwt)
    );

    let http = crate::http_client(API_TIMEOUT)?;
    let resp = http
        .post(TOKEN_URI)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .raw_body(body.into_bytes())
        .send()
        .await
        .map_err(|e| ShadowError::config(format!("Indexing API token exchange: {e}")))?;

    let resp_body = resp.text().map_err(|e| {
        ShadowError::config(format!("Indexing API token response: {e}"))
    })?;

    let token: TokenResponse = serde_json::from_str(&resp_body).map_err(|e| {
        ShadowError::config(format!(
            "Indexing API token parse: {e} — body: {}",
            &resp_body[..resp_body.len().min(200)]
        ))
    })?;

    // Notify each URL
    let mut results = Vec::new();
    let url_path = format!("{INDEXING_API_BASE}/v3/urlNotifications:publish");

    for url in urls {
        let notify_body = serde_json::json!({
            "url": url,
            "type": "URL_UPDATED",
        });

        match http
            .post(&url_path)
            .header("Authorization", &format!("Bearer {}", token.access_token))
            .header("Content-Type", "application/json")
            .json(&notify_body)
            .send()
            .await
        {
            Ok(resp) => {
                let status = resp.status().as_u16();
                if status == 200 {
                    results.push(format!("notified: {url}"));
                } else {
                    let body = resp.text().unwrap_or_default();
                    results.push(format!("FAILED {url} (HTTP {status}): {}", &body[..body.len().min(100)]));
                }
            }
            Err(e) => results.push(format!("FAILED {url}: {e}")),
        }
    }

    Ok(format!("{}/{} URLs notified", results.iter().filter(|r| r.starts_with("notified")).count(), urls.len()))
}

// Stubs for non-http builds
#[cfg(not(feature = "http"))]
pub async fn status_report(_client: &GscClient, _days: u32) -> Result<String> {
    Err(ShadowError::config("http feature required"))
}
#[cfg(not(feature = "http"))]
pub async fn submit_sitemap(_client: &GscClient) -> Result<String> {
    Err(ShadowError::config("http feature required"))
}
#[cfg(not(feature = "http"))]
pub async fn inspect_url(_client: &GscClient, _url: &str) -> Result<String> {
    Err(ShadowError::config("http feature required"))
}
#[cfg(not(feature = "http"))]
pub async fn cascade_probe(_client: &GscClient) -> Result<String> {
    Err(ShadowError::config("http feature required"))
}
#[cfg(not(feature = "http"))]
pub async fn url_notify(_urls: &[String]) -> Result<String> {
    Err(ShadowError::config("http feature required"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urlencod_preserves_alphanumeric() {
        assert_eq!(urlencod("hello"), "hello");
        assert_eq!(urlencod("test123"), "test123");
    }

    #[test]
    fn urlencod_encodes_special_chars() {
        let encoded = urlencod("urn:ietf:params:oauth:grant-type:jwt-bearer");
        assert!(encoded.contains("%3A"));
        assert!(!encoded.contains(':'));
    }

    #[test]
    fn urlencod_roundtrip_colon() {
        let input = "sc-domain:primals.eco";
        let encoded = urlencod(input);
        assert!(encoded.contains("sc-domain"));
        assert!(encoded.contains("primals.eco"));
    }
}
