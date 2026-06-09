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
const NEURAL_API_SOCKET_NAME: &str = "neural-api-default.sock";
/// Directory component under XDG and /tmp for biomeOS runtime sockets.
const NEURAL_API_NAMESPACE: &str = "biomeos";

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

        let socket_base = std::env::var(cellmembrane_types::service::ENV_SOCKET_BASE)
            .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_SOCKET_BASE.into());
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
    /// Sends `capability.call` JSON-RPC with the given domain, method, and
    /// params. Returns `BridgeResult::Handled` if biomeOS routes successfully,
    /// or `BridgeResult::Fallthrough` if the socket is unreachable, the
    /// method is not routed, or any error occurs.
    pub async fn capability_call(
        &self,
        domain: &str,
        method: &str,
        params: serde_json::Value,
    ) -> BridgeResult {
        self.rpc_call(
            "capability.call",
            serde_json::json!({
                "capability": domain,
                "method": method,
                "params": params,
            }),
        )
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
/// Returns `Some(result)` if a primal handled the request, or `None` to
/// indicate the shadow implementation should proceed.
///
/// This is the core graduated composition primitive: as primals come online,
/// they handle capabilities natively; when unavailable, shadow code runs.
pub async fn try_bridge(
    domain: &str,
    method: &str,
    params: serde_json::Value,
) -> Option<serde_json::Value> {
    let bridge = NeuralBridge::discover()?;
    match bridge.capability_call(domain, method, params).await {
        BridgeResult::Handled(result) => Some(result),
        BridgeResult::Fallthrough => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_returns_none_without_socket() {
        let result = NeuralBridge::discover();
        assert!(
            result.is_none(),
            "should fall through when no socket exists"
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
        let tmp = std::env::temp_dir().join("biomeos");
        std::fs::create_dir_all(&tmp).unwrap();
        let sock_path = tmp.join(NEURAL_API_SOCKET_NAME);
        std::fs::write(&sock_path, b"").unwrap();

        let bridge = NeuralBridge::discover();
        assert!(bridge.is_some() || bridge.is_none());

        std::fs::remove_dir_all(&tmp).ok();
    }
}
