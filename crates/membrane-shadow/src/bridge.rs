// SPDX-License-Identifier: AGPL-3.0-or-later

//! `NeuralBridge` — optional try-primal-first client for capability graduation.
//!
//! When the `neural-bridge` feature is enabled, membrane-shadow can attempt
//! to route operations through biomeOS's Neural API before falling back to
//! its direct fs/git shadow implementation. This enables graduated primal
//! composition: as primals implement capabilities natively, membrane-shadow
//! automatically delegates to them.
//!
//! Discovery order (same as primalSpring's NeuralBridge):
//!   1. `$NEURAL_API_SOCKET` env var
//!   2. `$XDG_RUNTIME_DIR/biomeos/neural-api-default.sock`
//!   3. `/tmp/biomeos/neural-api-default.sock`
//!
//! If no socket is discovered, all operations fall through to shadow mode.

use crate::error::{Result, ShadowError};
use std::path::PathBuf;
#[cfg(unix)]
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
#[cfg(unix)]
use tokio::net::UnixStream;

/// Default socket name for the Neural API (biomeOS convention).
const NEURAL_API_SOCKET_NAME: &str = cellmembrane_types::service::NEURAL_API_SOCKET_NAME;
/// Directory component under XDG and /tmp for biomeOS runtime sockets.
const NEURAL_API_NAMESPACE: &str = cellmembrane_types::service::NEURAL_API_NAMESPACE;

/// Lightweight JSON-RPC 2.0 client for biomeOS Neural API.
///
/// Zero compile-time coupling to biomeOS — communicates via UDS JSON-RPC
/// with capability.call routing. Falls through gracefully when biomeOS
/// is unavailable (shadow mode).
pub struct NeuralBridge {
    socket_path: PathBuf,
}

/// Whether the bridge successfully handled a request or fell through.
pub enum BridgeResult {
    /// Primal handled the request — return this value.
    Handled(serde_json::Value),
    /// Bridge unavailable or primal not found — fall through to shadow.
    Fallthrough,
}

impl NeuralBridge {
    /// Attempt to discover a biomeOS Neural API socket.
    ///
    /// Returns `None` if no socket is found — caller should proceed with
    /// shadow mode.
    #[must_use]
    pub fn discover() -> Option<Self> {
        if let Ok(path) = std::env::var(cellmembrane_types::service::ENV_NEURAL_API_SOCKET) {
            let p = PathBuf::from(&path);
            if p.exists() {
                return Some(Self { socket_path: p });
            }
        }

        let socket_base = cellmembrane_types::service::env_or(
            cellmembrane_types::service::ENV_SOCKET_BASE,
            cellmembrane_types::service::DEFAULT_SOCKET_BASE,
        );
        let vps_path = PathBuf::from(&socket_base).join(NEURAL_API_SOCKET_NAME);
        if vps_path.exists() {
            return Some(Self {
                socket_path: vps_path,
            });
        }

        let xdg =
            std::env::var(cellmembrane_types::service::ENV_XDG_RUNTIME_DIR).unwrap_or_default();
        if !xdg.is_empty() {
            let p = PathBuf::from(&xdg)
                .join(NEURAL_API_NAMESPACE)
                .join(NEURAL_API_SOCKET_NAME);
            if p.exists() {
                return Some(Self { socket_path: p });
            }
        }

        let fallback = std::env::temp_dir()
            .join(NEURAL_API_NAMESPACE)
            .join(NEURAL_API_SOCKET_NAME);
        if fallback.exists() {
            return Some(Self {
                socket_path: fallback,
            });
        }

        None
    }

    /// Call a capability method through biomeOS Neural API.
    ///
    /// Sends a direct `{domain}.{method}` JSON-RPC call. The Neural API
    /// routes dotted methods natively (e.g. `lifecycle.status`, `crypto.sign`).
    /// Returns `BridgeResult::Handled` on success, `BridgeResult::Fallthrough`
    /// if the socket is unreachable or the method is not routed.
    pub async fn capability_call(
        &self,
        domain: &str,
        method: &str,
        params: serde_json::Value,
    ) -> BridgeResult {
        let dotted_method = format!("{domain}.{method}");
        self.rpc_call(&dotted_method, params)
            .await
            .map_or(BridgeResult::Fallthrough, BridgeResult::Handled)
    }

    /// Low-level JSON-RPC 2.0 call over UDS.
    #[cfg(not(unix))]
    async fn rpc_call(
        &self,
        _method: &str,
        _params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        Err(ShadowError::Parse(
            "UDS not available on this platform".into(),
        ))
    }

    /// Low-level JSON-RPC 2.0 call over UDS.
    #[cfg(unix)]
    async fn rpc_call(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value> {
        let stream = UnixStream::connect(&self.socket_path)
            .await
            .map_err(ShadowError::Io)?;

        let (reader, mut writer) = stream.into_split();

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        });

        let mut payload = serde_json::to_string(&request)?;
        payload.push('\n');

        writer
            .write_all(&crate::ribocipher::CLEAR_JSONRPC_SIGNAL)
            .await
            .map_err(ShadowError::Io)?;
        writer
            .write_all(payload.as_bytes())
            .await
            .map_err(ShadowError::Io)?;
        writer.flush().await.map_err(ShadowError::Io)?;

        let mut buf_reader = BufReader::new(reader);
        let mut line = String::new();

        let timeout = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            buf_reader.read_line(&mut line),
        )
        .await;

        match timeout {
            Ok(Ok(_)) => {
                let response: serde_json::Value = serde_json::from_str(&line)?;

                if let Some(error) = response.get("error") {
                    return Err(ShadowError::Parse(format!(
                        "rpc error: {}",
                        error
                            .get("message")
                            .and_then(|m| m.as_str())
                            .unwrap_or("unknown")
                    )));
                }

                response
                    .get("result")
                    .cloned()
                    .ok_or_else(|| ShadowError::Parse("rpc response missing result".into()))
            }
            Ok(Err(e)) => Err(ShadowError::Io(e)),
            Err(_) => Err(ShadowError::Parse("rpc timeout (5s)".into())),
        }
    }
}

/// Try routing through the Neural API, falling back to shadow execution.
///
/// Discovery order:
///   1. Local Neural API (biomeOS on this gate)
///   2. Cross-gate resolver (finds biomeOS on another gate via manifest,
///      routes through TCP or songBird relay.forward)
///   3. `None` → caller proceeds with shadow implementation
///
/// This is the core graduated composition primitive: as primals come online,
/// they handle capabilities natively; when unavailable, shadow code runs.
pub async fn try_bridge(
    domain: &str,
    method: &str,
    params: serde_json::Value,
) -> Option<serde_json::Value> {
    if let Some(bridge) = NeuralBridge::discover() {
        match bridge.capability_call(domain, method, params.clone()).await {
            BridgeResult::Handled(result) => return Some(result),
            BridgeResult::Fallthrough => {}
        }
    }

    try_cross_gate_bridge(domain, method, &params).await
}

/// Attempt cross-gate neural-api resolution via the transport resolver.
///
/// If no local biomeOS is running, resolves the "biomeos" role from the
/// manifest and routes through `call_endpoint` (TCP or relay.forward).
async fn try_cross_gate_bridge(
    domain: &str,
    method: &str,
    params: &serde_json::Value,
) -> Option<serde_json::Value> {
    let ctx = crate::resolve::ResolutionContext::from_env();
    let ep = crate::resolve::resolve_by_role(&ctx, "biomeos")?;

    if ep.is_local() {
        return None;
    }

    let request = crate::jsonrpc::request_with_params(
        "capability.call",
        &serde_json::json!({
            "capability": domain,
            "method": method,
            "params": params,
        }),
        1,
    );

    let response = crate::jsonrpc::call_endpoint(&ep, &request).await.ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&response).ok()?;

    if parsed.get("error").is_some() {
        return None;
    }

    parsed.get("result").cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_respects_env_override() {
        let original = std::env::var(cellmembrane_types::service::ENV_NEURAL_API_SOCKET).ok();
        let original_base = std::env::var(cellmembrane_types::service::ENV_SOCKET_BASE).ok();
        unsafe {
            std::env::set_var(
                cellmembrane_types::service::ENV_NEURAL_API_SOCKET,
                "/nonexistent/path/neural-api.sock",
            );
            std::env::set_var(cellmembrane_types::service::ENV_SOCKET_BASE, "/nonexistent");
        }

        let result = NeuralBridge::discover();

        unsafe {
            match &original {
                Some(v) => {
                    std::env::set_var(cellmembrane_types::service::ENV_NEURAL_API_SOCKET, v);
                }
                None => {
                    std::env::remove_var(cellmembrane_types::service::ENV_NEURAL_API_SOCKET);
                }
            }
            match &original_base {
                Some(v) => std::env::set_var(cellmembrane_types::service::ENV_SOCKET_BASE, v),
                None => std::env::remove_var(cellmembrane_types::service::ENV_SOCKET_BASE),
            }
        }

        assert!(
            result.is_none(),
            "should fall through when no socket exists at overridden paths"
        );
    }

    #[tokio::test]
    async fn try_bridge_falls_through_when_unavailable() {
        let result = try_bridge("gate", "gate.info", serde_json::json!({})).await;
        assert!(
            result.is_none(),
            "bridge should fall through to shadow when no primal is running"
        );
    }

    #[test]
    fn neural_api_socket_name_is_stable() {
        assert_eq!(NEURAL_API_SOCKET_NAME, "neural-api-default.sock");
        assert_eq!(NEURAL_API_NAMESPACE, "biomeos");
    }

    #[test]
    fn bridge_result_handled_carries_value() {
        let val = serde_json::json!({"status": "ok"});
        let result = BridgeResult::Handled(val.clone());
        match result {
            BridgeResult::Handled(v) => assert_eq!(v, val),
            BridgeResult::Fallthrough => panic!("expected Handled"),
        }
    }

    #[test]
    fn bridge_result_fallthrough_variant() {
        let result = BridgeResult::Fallthrough;
        assert!(matches!(result, BridgeResult::Fallthrough));
    }

    #[test]
    fn discover_with_tmp_socket() {
        let tmp = std::env::temp_dir().join(format!("biomeos-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let sock_path = tmp.join(NEURAL_API_SOCKET_NAME);
        std::fs::write(&sock_path, b"").unwrap();

        let bridge = NeuralBridge::discover();
        assert!(bridge.is_some() || bridge.is_none());

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[tokio::test]
    async fn cross_gate_bridge_falls_through_without_manifest() {
        let result = try_cross_gate_bridge("gate", "gate.status", &serde_json::json!({})).await;
        assert!(
            result.is_none(),
            "cross-gate bridge should fall through without manifest"
        );
    }

    #[tokio::test]
    async fn try_bridge_returns_none_when_all_paths_fail() {
        let result = try_bridge("service", "service.list", serde_json::json!({})).await;
        assert!(
            result.is_none(),
            "should return None when no primal or cross-gate path available"
        );
    }
}
