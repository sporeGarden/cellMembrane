// SPDX-License-Identifier: AGPL-3.0-or-later

//! UDS webhook listener — receives Forgejo/GitHub webhook POSTs via Unix socket.
//!
//! Architecture: `Forgejo → Caddy reverse proxy → membrane UDS → handle_push()`
//!
//! Caddy routes `/webhook` to a `unix//run/membrane/webhook.sock` upstream.
//! This listener accepts HTTP POST requests on that socket, verifies the
//! HMAC-SHA256 signature, and dispatches to the existing webhook pipeline.
//!
//! The listener runs in a background task and is started by `webhook.listen`.

use std::path::Path;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tracing::{debug, info, warn};

use super::{PushEvent, WebhookProvider, verify_provider_signature};
use crate::error::Result;

async fn send_response(writer: &mut (impl AsyncWriteExt + Unpin), response: &str) {
    if let Err(e) = writer.write_all(response.as_bytes()).await {
        debug!(error = %e, "webhook: response write failed (client may have disconnected)");
    }
}

fn default_socket_path() -> String {
    let base = cellmembrane_types::service::resolve_socket_base();
    format!(
        "{base}/{}",
        cellmembrane_types::service::WEBHOOK_SOCKET_NAME
    )
}

/// Start listening for webhook POSTs on a Unix domain socket.
///
/// Returns after the listener is shut down (e.g. by signal). Each accepted
/// connection is handled in a separate tokio task.
#[cfg(unix)]
pub async fn listen(config: &crate::ShadowConfig, socket_path: Option<&str>) -> Result<()> {
    let default = default_socket_path();
    let path = socket_path.unwrap_or(&default);

    if Path::new(path).exists() {
        if let Err(e) = std::fs::remove_file(path) {
            debug!(socket = %path, "remove stale socket: {e}");
        }
    }

    if let Some(parent) = Path::new(path).parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            debug!(dir = %parent.display(), "create socket dir: {e}");
        }
    }

    let listener = tokio::net::UnixListener::bind(path).map_err(|e| {
        crate::error::ShadowError::Io(std::io::Error::other(format!(
            "webhook listener bind {path}: {e}"
        )))
    })?;

    if let Err(e) = cellmembrane_types::PlatformAccess::GroupReadWrite.apply(std::path::Path::new(path)) {
        debug!(socket = %path, "set socket permissions: {e}");
    }

    info!(socket = %path, "webhook listener started");

    let config = std::sync::Arc::new(config.clone());
    loop {
        let (stream, _addr) = match listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                warn!("webhook accept error: {e}");
                continue;
            }
        };

        let config = std::sync::Arc::clone(&config);
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, &config).await {
                warn!("webhook connection error: {e}");
            }
        });
    }
}

/// Webhook listener requires Unix domain sockets.
#[cfg(not(unix))]
pub async fn listen(_config: &crate::ShadowConfig, _socket_path: Option<&str>) -> Result<()> {
    Err(crate::error::ShadowError::config(
        "webhook UDS listener unavailable on this platform",
    ))
}

/// Handle a single HTTP connection on the webhook socket.
///
/// Generic over any async stream — the `#[cfg(unix)]` gate lives only at
/// the `listen()` bind point, not in the HTTP handling logic.
async fn handle_connection<S>(stream: S, config: &crate::ShadowConfig) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (reader, mut writer) = tokio::io::split(stream);
    let mut buf_reader = tokio::io::BufReader::new(reader);

    let mut request_line = String::new();
    buf_reader
        .read_line(&mut request_line)
        .await
        .map_err(io_err)?;

    if !request_line.starts_with("POST ") {
        send_response(&mut writer, &http_response(405, "Method Not Allowed")).await;
        return Ok(());
    }

    let mut headers: Vec<(String, String)> = Vec::new();
    let mut content_length: usize = 0;

    loop {
        let mut line = String::new();
        buf_reader.read_line(&mut line).await.map_err(io_err)?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            let name = name.trim().to_string();
            let value = value.trim().to_string();
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.parse().unwrap_or(0);
            }
            headers.push((name, value));
        }
    }

    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        tokio::io::AsyncReadExt::read_exact(&mut buf_reader, &mut body)
            .await
            .map_err(io_err)?;
    }

    let secret = cellmembrane_types::service::resolve_webhook_secret_env().unwrap_or_default();
    if secret.is_empty() {
        send_response(
            &mut writer,
            &http_response(500, "webhook secret not configured"),
        )
        .await;
        return Ok(());
    }

    let Some((provider, raw_sig)) = WebhookProvider::detect(&headers) else {
        send_response(
            &mut writer,
            &http_response(400, "no webhook signature header"),
        )
        .await;
        return Ok(());
    };

    if let Err(e) = verify_provider_signature(provider, secret.as_bytes(), &body, &raw_sig) {
        warn!(error = %e, "webhook signature verification failed");
        send_response(
            &mut writer,
            &http_response(401, "signature verification failed"),
        )
        .await;
        return Ok(());
    }

    let event: PushEvent = match serde_json::from_slice(&body) {
        Ok(e) => e,
        Err(e) => {
            send_response(
                &mut writer,
                &http_response(400, &format!("invalid push event: {e}")),
            )
            .await;
            return Ok(());
        }
    };

    info!(
        repo = %event.repository.name,
        branch = %event.git_ref,
        provider = ?provider,
        "webhook received"
    );

    let outcome = super::handle_push(&event, config, provider).await;

    let response = match outcome {
        Ok(o) => http_response(200, &o.message),
        Err(e) => http_response(500, &format!("pipeline error: {e}")),
    };
    send_response(&mut writer, &response).await;

    Ok(())
}

fn http_response(status: u16, body: &str) -> String {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        405 => "Method Not Allowed",
        500 => "Internal Server Error",
        _ => "Unknown",
    };
    format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: text/plain\r\n\
         Content-Length: {len}\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        len = body.len(),
    )
}

const fn io_err(e: std::io::Error) -> crate::error::ShadowError {
    crate::error::ShadowError::Io(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_response_has_status_line() {
        let resp = http_response(200, "ok");
        assert!(resp.starts_with("HTTP/1.1 200 OK"));
    }

    #[test]
    fn http_response_has_content_length() {
        let resp = http_response(200, "hello");
        assert!(resp.contains("Content-Length: 5"));
    }

    #[test]
    fn http_response_401_unauthorized() {
        let resp = http_response(401, "bad sig");
        assert!(resp.contains("401 Unauthorized"));
    }
}
