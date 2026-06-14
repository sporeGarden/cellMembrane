// SPDX-License-Identifier: AGPL-3.0-or-later

//! Shared JSON-RPC client over Unix Domain Sockets.
//!
//! Provides a single implementation of the UDS transport used across health probes,
//! sandbox validation, canary monitoring, and impulse relay. Eliminates 5 prior
//! copy-paste implementations.
//!
//! All calls prepend the riboCipher clear signal `[0xEC, 0x01]` with graceful
//! fallback to raw JSON for transitional deployments.

use std::path::Path;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(3);

/// Send a JSON-RPC request over UDS with riboCipher signal and fallback.
///
/// Tries with riboCipher clear signal first. If the response is empty (primal
/// hasn't been restarted with riboCipher support), retries with raw JSON.
pub async fn call(socket_path: &Path, request: &str) -> Result<String, String> {
    if let Ok(response) = raw(socket_path, request, true).await {
        if !response.is_empty() {
            return Ok(response);
        }
    }
    raw(socket_path, request, false).await
}

/// Send a JSON-RPC request with explicit signal control.
///
/// When `with_signal` is true, prepends `[0xEC, 0x01]` before the JSON payload.
pub async fn raw(socket_path: &Path, request: &str, with_signal: bool) -> Result<String, String> {
    let stream = tokio::time::timeout(
        DEFAULT_TIMEOUT,
        tokio::net::UnixStream::connect(socket_path),
    )
    .await
    .map_err(|_| format!("connect timeout: {}", socket_path.display()))?
    .map_err(|e| format!("connect {}: {e}", socket_path.display()))?;

    let (mut reader, mut writer) = stream.into_split();

    if with_signal {
        writer
            .write_all(&crate::ribocipher::CLEAR_JSONRPC_SIGNAL)
            .await
            .map_err(|e| format!("signal write: {e}"))?;
    }
    writer
        .write_all(request.as_bytes())
        .await
        .map_err(|e| format!("write: {e}"))?;
    writer
        .shutdown()
        .await
        .map_err(|e| format!("shutdown: {e}"))?;

    let mut buf = Vec::with_capacity(4096);
    tokio::time::timeout(DEFAULT_TIMEOUT, reader.read_to_end(&mut buf))
        .await
        .map_err(|_| format!("read timeout: {}", socket_path.display()))?
        .map_err(|e| format!("read: {e}"))?;

    String::from_utf8(buf).map_err(|e| format!("utf8: {e}"))
}

/// Convenience: build a JSON-RPC request object for a method with no params.
#[must_use]
pub fn request(method: &str, id: u32) -> String {
    format!(r#"{{"jsonrpc":"2.0","method":"{method}","params":{{}},"id":{id}}}"#)
}

/// Convenience: build a JSON-RPC request with params.
#[must_use]
pub fn request_with_params(method: &str, params: &serde_json::Value, id: u32) -> String {
    format!(r#"{{"jsonrpc":"2.0","method":"{method}","params":{params},"id":{id}}}"#)
}
