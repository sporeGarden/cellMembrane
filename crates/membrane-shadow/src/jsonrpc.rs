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

use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

use crate::error::{Result, ShadowError};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(3);

fn rpc_err(msg: impl std::fmt::Display) -> ShadowError {
    ShadowError::Rpc(msg.to_string())
}

/// Standard JSON-RPC health probe request. Used by canary, sandbox, health, and
/// sovereignty probes to check liveness via UDS.
pub const HEALTH_REQUEST: &str = r#"{"jsonrpc":"2.0","method":"health","params":{},"id":1}"#;

/// Send a JSON-RPC request over UDS with riboCipher signal.
///
/// In Reject mode (Wave 113 default): sends signal, no fallback.
/// In Error/Warn mode: tries with signal first, falls back to raw JSON
/// if the response is empty (legacy primal without riboCipher support).
pub async fn call(socket_path: &Path, request: &str) -> Result<String> {
    let policy = crate::ribocipher::RiboCipherConfig::default();
    call_with_policy(socket_path, request, &policy).await
}

/// Send a JSON-RPC request respecting the given riboCipher policy.
///
/// This allows callers that need explicit policy control (e.g. health probes
/// during transitional deployments) to specify the fallback behavior.
pub async fn call_with_policy(
    socket_path: &Path,
    request: &str,
    policy: &crate::ribocipher::RiboCipherConfig,
) -> Result<String> {
    match raw(socket_path, request, true).await {
        Ok(response) if !response.is_empty() => return Ok(response),
        Ok(_) => {}
        Err(e) => {
            if !policy.allows_fallback() {
                return Err(e);
            }
        }
    }

    if !policy.allows_fallback() {
        return Err(rpc_err(format_args!(
            "riboCipher REJECT: peer at {} did not respond to signal (policy=reject, no fallback)",
            socket_path.display()
        )));
    }

    raw(socket_path, request, false).await
}

/// Send a JSON-RPC request with explicit signal control.
///
/// When `with_signal` is true, prepends `[0xEC, 0x01]` before the JSON payload.
pub async fn raw(socket_path: &Path, request: &str, with_signal: bool) -> Result<String> {
    let stream = tokio::time::timeout(
        DEFAULT_TIMEOUT,
        tokio::net::UnixStream::connect(socket_path),
    )
    .await
    .map_err(|_| rpc_err(format_args!("connect timeout: {}", socket_path.display())))?
    .map_err(|e| rpc_err(format_args!("connect {}: {e}", socket_path.display())))?;

    let (reader, mut writer) = stream.into_split();

    if with_signal {
        writer
            .write_all(&crate::ribocipher::CLEAR_JSONRPC_SIGNAL)
            .await
            .map_err(|e| rpc_err(format_args!("signal write: {e}")))?;
    }
    writer
        .write_all(request.as_bytes())
        .await
        .map_err(|e| rpc_err(format_args!("write: {e}")))?;
    writer
        .write_all(b"\n")
        .await
        .map_err(|e| rpc_err(format_args!("newline: {e}")))?;

    let mut buf_reader = tokio::io::BufReader::new(reader);
    let mut line = String::new();

    let read_result = tokio::time::timeout(DEFAULT_TIMEOUT, buf_reader.read_line(&mut line))
        .await
        .map_err(|_| rpc_err(format_args!("read timeout: {}", socket_path.display())))?
        .map_err(|e| rpc_err(format_args!("read: {e}")))?;

    if let Err(e) = writer.shutdown().await {
        tracing::debug!(error = %e, "writer shutdown (non-fatal)");
    }

    if read_result == 0 && line.is_empty() {
        return Err(rpc_err(format_args!(
            "empty response: {}",
            socket_path.display()
        )));
    }

    Ok(line)
}

/// Send a JSON-RPC request through a mesh relay endpoint.
///
/// Resolves the local songBird relay socket, then sends a `relay.forward` request
/// wrapping the original method call. The relay infrastructure handles peer
/// resolution, BTSP encryption, and multi-hop routing transparently.
///
/// This is the transport graduation for `TransportEndpoint::MeshRelay` — the
/// abstraction boundary where physical topology becomes invisible.
pub async fn call_via_relay(peer_id: &str, capability: &str, request: &str) -> Result<String> {
    let relay_socket = crate::gate::health::resolve_primal_socket_paths(
        cellmembrane_types::MembraneService::binary_for(
            cellmembrane_types::service::ServiceCapability::MeshRelay,
        ),
    )
    .into_iter()
    .find(|p| Path::new(p).exists())
    .ok_or_else(|| {
        rpc_err(format_args!(
            "no songBird relay socket found — cannot route to {peer_id}/{capability}"
        ))
    })?;

    let relay_request = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "relay.forward",
        "params": {
            "peer_id": peer_id,
            "capability": capability,
            "payload": request,
        },
        "id": 1,
    })
    .to_string();

    call(Path::new(&relay_socket), &relay_request).await
}

/// Send a JSON-RPC request over TCP with riboCipher signal.
///
/// Uses the same framing as UDS calls (riboCipher signal + NDJSON) but over
/// a `WireGuard` mesh TCP connection. The `host:port` pair typically comes from
/// manifest-resolved mesh IPs (e.g. `10.13.37.1:7700`).
pub async fn call_tcp(host: &str, port: u16, request: &str) -> Result<String> {
    let addr = format!("{host}:{port}");
    let stream = tokio::time::timeout(DEFAULT_TIMEOUT, tokio::net::TcpStream::connect(&addr))
        .await
        .map_err(|_| rpc_err(format_args!("tcp connect timeout: {addr}")))?
        .map_err(|e| rpc_err(format_args!("tcp connect {addr}: {e}")))?;

    let (reader, mut writer) = stream.into_split();

    writer
        .write_all(&crate::ribocipher::CLEAR_JSONRPC_SIGNAL)
        .await
        .map_err(|e| rpc_err(format_args!("tcp signal write: {e}")))?;
    writer
        .write_all(request.as_bytes())
        .await
        .map_err(|e| rpc_err(format_args!("tcp write: {e}")))?;
    writer
        .write_all(b"\n")
        .await
        .map_err(|e| rpc_err(format_args!("tcp newline: {e}")))?;

    let mut buf_reader = tokio::io::BufReader::new(reader);
    let mut line = String::new();

    let read_result = tokio::time::timeout(DEFAULT_TIMEOUT, buf_reader.read_line(&mut line))
        .await
        .map_err(|_| rpc_err(format_args!("tcp read timeout: {addr}")))?
        .map_err(|e| rpc_err(format_args!("tcp read: {e}")))?;

    if let Err(e) = writer.shutdown().await {
        tracing::debug!(error = %e, addr = %addr, "tcp writer shutdown (non-fatal)");
    }

    if read_result == 0 && line.is_empty() {
        return Err(rpc_err(format_args!("tcp empty response: {addr}")));
    }

    Ok(line)
}

/// Route a JSON-RPC request through a [`cellmembrane_types::TransportEndpoint`].
///
/// Dispatches to the appropriate transport based on the endpoint variant:
/// - `Uds` → direct UDS call (local primal, no network)
/// - `Tcp` → `WireGuard` mesh TCP (cross-gate, riboCipher framed)
/// - `MeshRelay` → songBird relay (multi-hop, encrypted)
///
/// This is the primary entry point for transport-agnostic capability calls.
pub async fn call_endpoint(
    endpoint: &cellmembrane_types::TransportEndpoint,
    request: &str,
) -> Result<String> {
    match endpoint {
        cellmembrane_types::TransportEndpoint::Uds { path } => call(Path::new(path), request).await,
        cellmembrane_types::TransportEndpoint::Tcp { host, port } => {
            call_tcp(host, *port, request).await
        }
        cellmembrane_types::TransportEndpoint::MeshRelay {
            peer_id,
            capability,
        } => call_via_relay(peer_id, capability, request).await,
    }
}

/// Convenience: build a JSON-RPC request object for a method with no params.
#[must_use]
#[cfg(test)]
pub(crate) fn request(method: &str, id: u32) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": {},
        "id": id,
    })
    .to_string()
}

/// Convenience: build a JSON-RPC request with params.
#[must_use]
pub(crate) fn request_with_params(method: &str, params: &serde_json::Value, id: u32) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": id,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_formats_valid_jsonrpc() {
        let req = request("health", 1);
        let parsed: serde_json::Value = serde_json::from_str(&req).expect("valid JSON");
        assert_eq!(parsed["jsonrpc"], "2.0");
        assert_eq!(parsed["method"], "health");
        assert_eq!(parsed["id"], 1);
        assert!(parsed["params"].is_object());
    }

    #[test]
    fn request_with_params_embeds_params() {
        let params = serde_json::json!({"key": "value", "n": 42});
        let req = request_with_params("test.method", &params, 7);
        let parsed: serde_json::Value = serde_json::from_str(&req).expect("valid JSON");
        assert_eq!(parsed["method"], "test.method");
        assert_eq!(parsed["id"], 7);
        assert_eq!(parsed["params"]["key"], "value");
        assert_eq!(parsed["params"]["n"], 42);
    }

    #[test]
    fn request_escapes_method_name() {
        let req = request("gate.bootstrap", 99);
        assert!(req.contains("gate.bootstrap"));
        let parsed: serde_json::Value = serde_json::from_str(&req).expect("valid JSON");
        assert_eq!(parsed["method"], "gate.bootstrap");
    }

    #[test]
    fn default_timeout_is_reasonable() {
        assert!(DEFAULT_TIMEOUT.as_secs() >= 1);
        assert!(DEFAULT_TIMEOUT.as_secs() <= 30);
    }

    #[tokio::test]
    async fn call_tcp_refuses_unreachable_host() {
        let result = call_tcp("192.0.2.1", 1, "{}").await;
        assert!(result.is_err(), "unreachable host should error");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("timeout") || err.contains("connect"),
            "error should mention connection failure, got: {err}"
        );
    }

    #[tokio::test]
    async fn call_endpoint_dispatches_uds() {
        let ep = cellmembrane_types::TransportEndpoint::Uds {
            path: "/tmp/nonexistent-test-socket.sock".into(),
        };
        let result = call_endpoint(&ep, r#"{"jsonrpc":"2.0","method":"health","id":1}"#).await;
        assert!(result.is_err(), "nonexistent socket should error");
    }

    #[tokio::test]
    async fn call_endpoint_dispatches_tcp() {
        let ep = cellmembrane_types::TransportEndpoint::Tcp {
            host: "192.0.2.1".into(),
            port: 1,
        };
        let result = call_endpoint(&ep, "{}").await;
        assert!(result.is_err(), "unreachable TCP should error");
    }
}
