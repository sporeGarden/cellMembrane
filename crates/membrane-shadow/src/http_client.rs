// SPDX-License-Identifier: AGPL-3.0-or-later

//! Minimal pure-Rust HTTP/1.1 client — replaces `reqwest`.
//!
//! Uses `tokio-rustls` for TLS and `serde_json` for JSON bodies. Supports
//! GET, HEAD, POST, PUT, DELETE with custom headers, timeouts, and both
//! `Content-Length` and chunked transfer-encoding responses.
//!
//! No connection pooling, no HTTP/2, no proxy support — intentionally
//! minimal for the ecosystem's sovereign infrastructure needs.

use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use crate::error::{Result, ShadowError};

// ── Types ───────────────────────────────────────────────────────────

/// HTTP method.
#[derive(Debug, Clone, Copy)]
pub enum Method {
    /// GET request.
    Get,
    /// HEAD request (no body returned).
    Head,
    /// POST request.
    Post,
    /// PUT request.
    Put,
    /// DELETE request.
    Delete,
}

impl Method {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Head => "HEAD",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Delete => "DELETE",
        }
    }
}

/// Parsed URL components.
struct ParsedUrl {
    tls: bool,
    host: String,
    port: u16,
    path: String,
}

fn parse_url(url: &str) -> Result<ParsedUrl> {
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| ShadowError::Http(format!("invalid URL (no scheme): {url}")))?;

    let tls = match scheme {
        "https" => true,
        "http" => false,
        _ => return Err(ShadowError::Http(format!("unsupported scheme: {scheme}"))),
    };

    let (authority, path) = rest.find('/').map_or((rest, "/"), |i| (&rest[..i], &rest[i..]));

    let (host, port) = if let Some((h, p)) = authority.rsplit_once(':') {
        let port = p
            .parse::<u16>()
            .map_err(|_| ShadowError::Http(format!("invalid port: {p}")))?;
        (h.to_string(), port)
    } else {
        let default = if tls {
            cellmembrane_types::service::DEFAULT_HTTPS_PORT
        } else {
            cellmembrane_types::service::DEFAULT_HTTP_PORT
        };
        (authority.to_string(), default)
    };

    Ok(ParsedUrl {
        tls,
        host,
        port,
        path: path.to_string(),
    })
}

// ── Client ──────────────────────────────────────────────────────────

/// Reusable HTTP client with shared TLS configuration.
#[derive(Clone)]
pub struct HttpClient {
    tls_config: Arc<rustls::ClientConfig>,
    timeout: std::time::Duration,
}

impl HttpClient {
    /// Build a client with standard TLS verification and Mozilla root CAs.
    pub fn new(timeout: std::time::Duration) -> Result<Self> {
        let root_store: rustls::RootCertStore =
            webpki_roots::TLS_SERVER_ROOTS.iter().cloned().collect();

        let tls_config = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(|e| ShadowError::Http(format!("TLS config: {e}")))?
        .with_root_certificates(root_store)
        .with_no_client_auth();

        Ok(Self {
            tls_config: Arc::new(tls_config),
            timeout,
        })
    }

    /// Build a client that accepts invalid/self-signed TLS certificates.
    ///
    /// Only for local shadow comparison — never for WAN traffic.
    pub fn insecure(timeout: std::time::Duration) -> Result<Self> {
        let tls_config = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(|e| ShadowError::Http(format!("TLS config: {e}")))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(InsecureVerifier))
        .with_no_client_auth();

        Ok(Self {
            tls_config: Arc::new(tls_config),
            timeout,
        })
    }

    /// Start a GET request.
    #[must_use]
    pub fn get(&self, url: &str) -> RequestBuilder {
        RequestBuilder::new(self.clone(), Method::Get, url)
    }

    /// Start a HEAD request.
    #[must_use]
    pub fn head(&self, url: &str) -> RequestBuilder {
        RequestBuilder::new(self.clone(), Method::Head, url)
    }

    /// Start a POST request.
    #[must_use]
    pub fn post(&self, url: &str) -> RequestBuilder {
        RequestBuilder::new(self.clone(), Method::Post, url)
    }

    /// Start a PUT request.
    #[must_use]
    pub fn put(&self, url: &str) -> RequestBuilder {
        RequestBuilder::new(self.clone(), Method::Put, url)
    }

    /// Start a DELETE request.
    #[must_use]
    pub fn delete(&self, url: &str) -> RequestBuilder {
        RequestBuilder::new(self.clone(), Method::Delete, url)
    }
}

// ── Request builder ─────────────────────────────────────────────────

/// Fluent request builder — chain headers, body, then `.send().await`.
pub struct RequestBuilder {
    client: HttpClient,
    method: Method,
    url: String,
    headers: Vec<(String, String)>,
    body: Option<Vec<u8>>,
    timeout_override: Option<std::time::Duration>,
}

impl RequestBuilder {
    fn new(client: HttpClient, method: Method, url: &str) -> Self {
        Self {
            client,
            method,
            url: url.to_string(),
            headers: Vec::new(),
            body: None,
            timeout_override: None,
        }
    }

    /// Add a request header.
    #[must_use]
    pub fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.to_string(), value.into()));
        self
    }

    /// Add `Authorization: Bearer {token}` header.
    #[must_use]
    pub fn bearer_auth(self, token: &str) -> Self {
        self.header("Authorization", format!("Bearer {token}"))
    }

    /// Serialize `body` as JSON and set `Content-Type: application/json`.
    #[must_use]
    pub fn json<T: serde::Serialize>(mut self, body: &T) -> Self {
        match serde_json::to_vec(body) {
            Ok(bytes) => {
                self.headers
                    .push(("Content-Type".into(), "application/json".into()));
                self.body = Some(bytes);
            }
            Err(e) => {
                tracing::error!("JSON serialize failed: {e}");
            }
        }
        self
    }

    /// Override the client-level timeout for this request.
    #[must_use]
    pub const fn timeout(mut self, d: std::time::Duration) -> Self {
        self.timeout_override = Some(d);
        self
    }

    /// Execute the request, returning the fully-buffered response.
    pub async fn send(self) -> Result<HttpResponse> {
        let timeout = self.timeout_override.unwrap_or(self.client.timeout);
        tokio::time::timeout(timeout, self.send_inner())
            .await
            .map_err(|_| ShadowError::Http("request timed out".into()))?
    }

    async fn send_inner(self) -> Result<HttpResponse> {
        let parsed = parse_url(&self.url)?;

        let addr = format!("{}:{}", parsed.host, parsed.port);
        let tcp = tokio::net::TcpStream::connect(&addr)
            .await
            .map_err(|e| ShadowError::Http(format!("connect {addr}: {e}")))?;

        if parsed.tls {
            let server_name = rustls::pki_types::ServerName::try_from(parsed.host.as_str())
                .map_err(|e| ShadowError::Http(format!("invalid SNI: {e}")))?
                .to_owned();

            let connector = tokio_rustls::TlsConnector::from(self.client.tls_config.clone());
            let tls_stream = connector
                .connect(server_name, tcp)
                .await
                .map_err(|e| ShadowError::Http(format!("TLS handshake {addr}: {e}")))?;

            self.exchange(tls_stream, &parsed.host, &parsed.path)
                .await
        } else {
            self.exchange(tcp, &parsed.host, &parsed.path).await
        }
    }

    async fn exchange<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
        &self,
        stream: S,
        host: &str,
        path: &str,
    ) -> Result<HttpResponse> {
        let (reader, mut writer) = tokio::io::split(stream);

        let mut request = format!(
            "{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n",
            method = self.method.as_str(),
        );

        for (k, v) in &self.headers {
            request.push_str(k);
            request.push_str(": ");
            request.push_str(v);
            request.push_str("\r\n");
        }

        if let Some(ref body) = self.body {
            use std::fmt::Write;
            let _ = write!(request, "Content-Length: {}\r\n", body.len());
        }
        request.push_str("\r\n");

        writer.write_all(request.as_bytes()).await.map_err(|e| {
            ShadowError::Http(format!("write request: {e}"))
        })?;
        if let Some(ref body) = self.body {
            writer.write_all(body).await.map_err(|e| {
                ShadowError::Http(format!("write body: {e}"))
            })?;
        }
        writer.flush().await.map_err(|e| {
            ShadowError::Http(format!("flush: {e}"))
        })?;

        parse_response(reader, matches!(self.method, Method::Head)).await
    }
}

// ── Response ────────────────────────────────────────────────────────

/// HTTP response with fully-buffered body.
pub struct HttpResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl HttpResponse {
    /// HTTP status code.
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        StatusCode(self.status)
    }

    /// Response body as UTF-8 text.
    pub fn text(self) -> std::result::Result<String, ShadowError> {
        String::from_utf8(self.body)
            .map_err(|e| ShadowError::Http(format!("response not UTF-8: {e}")))
    }

    /// Deserialize response body as JSON.
    pub fn json<T: serde::de::DeserializeOwned>(self) -> std::result::Result<T, ShadowError> {
        serde_json::from_slice(&self.body).map_err(ShadowError::from)
    }

    /// Raw response body bytes.
    #[must_use]
    pub fn bytes(self) -> Vec<u8> {
        self.body
    }

    /// Get a response header value (case-insensitive).
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        let lower = name.to_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| k.to_lowercase() == lower)
            .map(|(_, v)| v.as_str())
    }
}

/// HTTP status code wrapper.
#[derive(Debug, Clone, Copy)]
pub struct StatusCode(u16);

impl StatusCode {
    /// Numeric status code.
    #[must_use]
    pub const fn as_u16(self) -> u16 {
        self.0
    }

    /// Whether status is in the 2xx range.
    #[must_use]
    pub const fn is_success(self) -> bool {
        self.0 >= 200 && self.0 < 300
    }
}

impl std::fmt::Display for StatusCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ── Response parsing ────────────────────────────────────────────────

async fn parse_response<R: tokio::io::AsyncRead + Unpin>(
    reader: R,
    head_only: bool,
) -> Result<HttpResponse> {
    let mut buf_reader = BufReader::new(reader);

    let mut status_line = String::new();
    buf_reader
        .read_line(&mut status_line)
        .await
        .map_err(|e| ShadowError::Http(format!("read status: {e}")))?;

    let status = parse_status_line(&status_line)?;

    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        buf_reader
            .read_line(&mut line)
            .await
            .map_err(|e| ShadowError::Http(format!("read header: {e}")))?;

        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }

        if let Some((k, v)) = trimmed.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }

    if head_only || status == 204 || status == 304 {
        return Ok(HttpResponse {
            status,
            headers,
            body: Vec::new(),
        });
    }

    let body = read_body(&mut buf_reader, &headers).await?;

    Ok(HttpResponse {
        status,
        headers,
        body,
    })
}

fn parse_status_line(line: &str) -> Result<u16> {
    let parts: Vec<&str> = line.splitn(3, ' ').collect();
    if parts.len() < 2 {
        return Err(ShadowError::Http(format!(
            "malformed status line: {line}"
        )));
    }
    parts[1]
        .parse::<u16>()
        .map_err(|_| ShadowError::Http(format!("invalid status code: {}", parts[1])))
}

async fn read_body<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
    headers: &[(String, String)],
) -> Result<Vec<u8>> {
    let is_chunked = headers.iter().any(|(k, v)| {
        k.eq_ignore_ascii_case("transfer-encoding") && v.to_lowercase().contains("chunked")
    });

    if is_chunked {
        return read_chunked(reader).await;
    }

    let content_length: Option<usize> = headers.iter().find_map(|(k, v)| {
        if k.eq_ignore_ascii_case("content-length") {
            v.parse().ok()
        } else {
            None
        }
    });

    if let Some(len) = content_length {
        let mut body = vec![0u8; len];
        reader
            .read_exact(&mut body)
            .await
            .map_err(|e| ShadowError::Http(format!("read body ({len} bytes): {e}")))?;
        Ok(body)
    } else {
        let mut body = Vec::new();
        reader
            .read_to_end(&mut body)
            .await
            .map_err(|e| ShadowError::Http(format!("read body: {e}")))?;
        Ok(body)
    }
}

async fn read_chunked<R: tokio::io::AsyncBufRead + Unpin>(reader: &mut R) -> Result<Vec<u8>> {
    let mut body = Vec::new();

    loop {
        let mut size_line = String::new();
        reader
            .read_line(&mut size_line)
            .await
            .map_err(|e| ShadowError::Http(format!("read chunk size: {e}")))?;

        let size_str = size_line.trim();
        let chunk_size = usize::from_str_radix(size_str, 16)
            .map_err(|_| ShadowError::Http(format!("invalid chunk size: {size_str}")))?;

        if chunk_size == 0 {
            let mut trailer = String::new();
            let _ = reader.read_line(&mut trailer).await;
            break;
        }

        let mut chunk = vec![0u8; chunk_size];
        reader
            .read_exact(&mut chunk)
            .await
            .map_err(|e| ShadowError::Http(format!("read chunk ({chunk_size} bytes): {e}")))?;
        body.extend_from_slice(&chunk);

        let mut crlf = [0u8; 2];
        let _ = reader.read_exact(&mut crlf).await;
    }

    Ok(body)
}

// ── Insecure TLS verifier ───────────────────────────────────────────

#[derive(Debug)]
struct InsecureVerifier;

impl rustls::client::danger::ServerCertVerifier for InsecureVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> std::result::Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_https_url() {
        let p = parse_url("https://api.example.com/v1/things").unwrap();
        assert!(p.tls);
        assert_eq!(p.host, "api.example.com");
        assert_eq!(
            p.port,
            cellmembrane_types::service::DEFAULT_HTTPS_PORT
        );
        assert_eq!(p.path, "/v1/things");
    }

    #[test]
    fn parse_http_url_with_port() {
        let p = parse_url("http://localhost:3000/api/v1").unwrap();
        assert!(!p.tls);
        assert_eq!(p.host, "localhost");
        assert_eq!(p.port, 3000);
        assert_eq!(p.path, "/api/v1");
    }

    #[test]
    fn parse_url_no_path() {
        let p = parse_url("https://depot.primals.eco").unwrap();
        assert!(p.tls);
        assert_eq!(p.host, "depot.primals.eco");
        assert_eq!(
            p.port,
            cellmembrane_types::service::DEFAULT_HTTPS_PORT
        );
        assert_eq!(p.path, "/");
    }

    #[test]
    fn parse_url_with_query() {
        let p = parse_url("https://api.example.com/v1?limit=50&page=2").unwrap();
        assert_eq!(p.path, "/v1?limit=50&page=2");
    }

    #[test]
    fn status_code_success_range() {
        assert!(StatusCode(200).is_success());
        assert!(StatusCode(201).is_success());
        assert!(StatusCode(204).is_success());
        assert!(!StatusCode(301).is_success());
        assert!(!StatusCode(404).is_success());
        assert!(!StatusCode(500).is_success());
    }

    #[test]
    fn parse_status_line_ok() {
        assert_eq!(parse_status_line("HTTP/1.1 200 OK\r\n").unwrap(), 200);
        assert_eq!(parse_status_line("HTTP/1.1 404 Not Found\r\n").unwrap(), 404);
        assert_eq!(parse_status_line("HTTP/1.0 301 Moved\r\n").unwrap(), 301);
    }

    #[test]
    fn client_builds_ok() {
        let c = HttpClient::new(std::time::Duration::from_secs(5));
        assert!(c.is_ok());
    }

    #[test]
    fn insecure_client_builds_ok() {
        let c = HttpClient::insecure(std::time::Duration::from_secs(5));
        assert!(c.is_ok());
    }

    #[test]
    fn method_as_str() {
        assert_eq!(Method::Get.as_str(), "GET");
        assert_eq!(Method::Head.as_str(), "HEAD");
        assert_eq!(Method::Post.as_str(), "POST");
        assert_eq!(Method::Put.as_str(), "PUT");
        assert_eq!(Method::Delete.as_str(), "DELETE");
    }
}
