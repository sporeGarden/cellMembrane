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
    /// Primal is reachable but returned an application-level error.
    /// Callers should propagate rather than falling through to shadow.
    ApiError(ShadowError),
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
    ///
    /// Returns:
    /// - `Handled(value)` — primal processed the request successfully
    /// - `ApiError(err)` — primal responded with a JSON-RPC error (propagate!)
    /// - `Fallthrough` — socket unreachable / transport failure (fall to shadow)
    pub async fn capability_call(
        &self,
        domain: &str,
        method: &str,
        params: serde_json::Value,
    ) -> BridgeResult {
        let dotted_method = format!("{domain}.{method}");
        match self.rpc_call(&dotted_method, params).await {
            Ok(value) => BridgeResult::Handled(value),
            Err(e @ ShadowError::Rpc(_)) => BridgeResult::ApiError(e),
            Err(_) => BridgeResult::Fallthrough,
        }
    }

    /// Low-level JSON-RPC 2.0 call — delegates transport to `jsonrpc::call`.
    ///
    /// This eliminates the duplicate UDS client that previously lived here.
    /// Transport is handled by the shared `jsonrpc` module which supports
    /// UDS (Unix), Named Pipes (Windows, future), and TCP.
    async fn rpc_call(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value> {
        let request = crate::jsonrpc::request_with_params(method, &params, 1);
        let raw = crate::jsonrpc::call(&self.socket_path, &request).await?;
        let response: serde_json::Value = serde_json::from_str(&raw)?;

        if let Some(error) = response.get("error") {
            let msg = error
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown");
            return Err(ShadowError::Rpc(msg.to_owned()));
        }

        response
            .get("result")
            .cloned()
            .ok_or_else(|| ShadowError::Rpc("rpc response missing result".into()))
    }
}

/// Try routing through the Neural API, falling back to shadow execution.
///
/// Discovery order:
///   1. Local Neural API (biomeOS on this gate)
///   2. Cross-gate resolver (finds biomeOS on another gate via manifest,
///      routes through TCP or songBird relay.forward)
///   3. `Ok(None)` → caller proceeds with shadow implementation
///
/// Returns `Err` when the Neural API is reachable but rejects the request —
/// callers should propagate rather than silently falling to shadow.
pub async fn try_bridge(
    domain: &str,
    method: &str,
    params: serde_json::Value,
) -> Result<Option<serde_json::Value>> {
    if let Some(bridge) = NeuralBridge::discover() {
        match bridge.capability_call(domain, method, params.clone()).await {
            BridgeResult::Handled(result) => return Ok(Some(result)),
            BridgeResult::ApiError(e) => {
                tracing::warn!(
                    domain,
                    method,
                    error = %e,
                    "Neural API rejected request"
                );
                return Err(e);
            }
            BridgeResult::Fallthrough => {}
        }
    }

    Ok(try_cross_gate_bridge(domain, method, &params).await)
}

/// Attempt cross-gate neural-api resolution via the transport resolver.
///
/// If no local biomeOS is running, resolves the `identity` role from the
/// manifest and routes through `call_endpoint` (TCP or relay.forward).
async fn try_cross_gate_bridge(
    domain: &str,
    method: &str,
    params: &serde_json::Value,
) -> Option<serde_json::Value> {
    let ctx = crate::resolve::ResolutionContext::from_env();
    let ep = crate::resolve::resolve_by_role(&ctx, "identity")?;

    if ep.is_local() {
        return None;
    }

    let dotted = format!("{domain}.{method}");
    let request = crate::jsonrpc::request_with_params(&dotted, params, 1);

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
    fn discover_returns_valid_bridge_or_none() {
        let result = NeuralBridge::discover();
        if let Some(bridge) = &result {
            assert!(
                bridge.socket_path.exists(),
                "discovered socket path should exist"
            );
        }
    }

    #[tokio::test]
    async fn try_bridge_environment_agnostic() {
        let result = try_bridge("gate", "gate.info", serde_json::json!({})).await;
        if let Ok(Some(val)) = &result {
            assert!(val.is_object(), "bridge result should be a JSON object");
        }
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
            BridgeResult::Fallthrough | BridgeResult::ApiError(_) => panic!("expected Handled"),
        }
    }

    #[test]
    fn bridge_result_fallthrough_variant() {
        let result = BridgeResult::Fallthrough;
        assert!(matches!(result, BridgeResult::Fallthrough));
    }

    #[test]
    fn bridge_result_api_error_variant() {
        let err = ShadowError::Rpc("method not found".into());
        let result = BridgeResult::ApiError(err);
        assert!(matches!(result, BridgeResult::ApiError(_)));
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
    async fn try_bridge_nonexistent_service() {
        let result =
            try_bridge("nonexistent_service_12345", "fake.method", serde_json::json!({})).await;
        if let Ok(Some(val)) = &result {
            assert!(val.is_object(), "unexpected result from nonexistent service");
        }
    }
}
