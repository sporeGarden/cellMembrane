// SPDX-License-Identifier: AGPL-3.0-or-later

//! Optional primal UDS integration (mesh relay, crypto signing).
//!
//! All functions are best-effort: missing sockets or failed connections
//! fall through silently — git push is the reliable baseline.
//!
//! Providers are discovered via [`ServiceCapability`] rather than hardcoded names.

use std::path::{Path, PathBuf};

use cellmembrane_types::ServiceCapability;
use chrono::Local;

use super::types::{ImpulseFile, ImpulseSignature};

fn relay_socket_name() -> String {
    let binary = cellmembrane_types::MembraneService::binary_for(ServiceCapability::MeshRelay);
    format!("{binary}-default.sock")
}

fn signer_socket_name() -> String {
    let binary = cellmembrane_types::MembraneService::binary_for(ServiceCapability::CryptoSigner);
    format!("{binary}-default.sock")
}

pub(super) fn try_relay_impulse(impulse: &ImpulseFile) {
    #[cfg(not(unix))]
    {
        let _ = impulse;
        return;
    }

    #[cfg(unix)]
    {
        let payload = serde_json::json!({
            "id": impulse.impulse.id,
            "type": impulse.impulse.impulse_type,
            "from": impulse.from.gate,
            "to": impulse.to.gates,
            "subject": impulse.content.subject,
            "priority": impulse.impulse.priority,
        });

        let Ok(payload_str) = serde_json::to_string(&payload) else {
            return;
        };

        try_forward_to_gates(impulse, &payload_str);

        let Some(socket_path) = discover_socket(&relay_socket_name()) else {
            return;
        };
        let notification = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "mesh.publish",
            "params": {
                "topic": format!("impulse/{}", impulse.from.gate),
                "payload": payload,
            }
        });
        let Ok(request_str) = serde_json::to_string(&notification) else {
            return;
        };
        uds_send(&socket_path, &request_str);
    }
}

/// Targeted cross-gate delivery via `relay.forward` for explicitly named gates.
///
/// Uses `resolve_endpoint` + `call_endpoint` for each target gate, routing
/// through UDS (local), TCP (mesh), or songBird relay (NAT-traversal).
/// Failures are silent — git push remains the reliable baseline.
fn try_forward_to_gates(impulse: &ImpulseFile, payload: &str) {
    let local_gate = crate::gate::resolve_local_gate_identity();
    let ctx = crate::resolve::ResolutionContext::from_env();

    let request = crate::jsonrpc::request_with_params(
        "impulse.deliver",
        &serde_json::json!({
            "from": impulse.from.gate,
            "impulse_id": impulse.impulse.id,
            "payload": payload,
        }),
        1,
    );

    for gate in &impulse.to.gates {
        if gate.eq_ignore_ascii_case(&local_gate) {
            continue;
        }
        let Some(ep) = crate::resolve::resolve_endpoint(&ctx, gate, ServiceCapability::MeshRelay)
        else {
            tracing::debug!(target_gate = %gate, "impulse relay: no endpoint resolved — skipping");
            continue;
        };
        let target = gate.clone();
        let req = request.clone();
        let _ = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();
            match rt {
                Ok(rt) => {
                    if rt.block_on(crate::jsonrpc::call_endpoint(&ep, &req)).is_err() {
                        tracing::debug!(target_gate = %target, "impulse relay: delivery failed");
                    }
                }
                Err(e) => {
                    tracing::debug!(
                        target_gate = %target,
                        error = %e,
                        "impulse relay: runtime build failed"
                    );
                }
            }
        });
    }
}

#[must_use]
pub(super) fn try_sign_impulse(_workspace_root: &Path, impulse_id: &str) -> Option<ImpulseSignature> {
    #[cfg(not(unix))]
    {
        let _ = impulse_id;
        return None;
    }

    #[cfg(unix)]
    {
        let socket_path = discover_socket(&signer_socket_name())?;

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "crypto.sign_ed25519",
            "params": { "data": impulse_id }
        });
        let request_str = serde_json::to_string(&request).ok()?;

        let response_bytes = uds_request(&socket_path, &request_str)?;
        let response: serde_json::Value = serde_json::from_slice(&response_bytes).ok()?;
        let result = response.get("result")?;

        Some(ImpulseSignature {
            algorithm: "ed25519".to_string(),
            public_key: result.get("public_key")?.as_str()?.to_string(),
            value: result.get("signature")?.as_str()?.to_string(),
            signed_at: Local::now().format("%Y-%m-%dT%H:%M:%S%:z").to_string(),
        })
    }
}

/// Discover a primal UDS socket by name.
///
/// Resolution chain (production → development):
///   1. `MEMBRANE_SOCKET_{NAME}` env var (e.g. `MEMBRANE_SOCKET_SONGBIRD`)
///   2. `$MEMBRANE_SOCKET_BASE/{socket_name}` (VPS standard, default `/run/membrane/`)
///   3. `$XDG_RUNTIME_DIR/membrane/{socket_name}` (user session)
///   4. `/tmp/membrane/{socket_name}` (last-resort dev fallback)
#[must_use]
pub fn discover_socket(socket_name: &str) -> Option<PathBuf> {
    let env_key = format!(
        "MEMBRANE_SOCKET_{}",
        socket_name
            .split_once('-')
            .map_or(socket_name, |(prefix, _)| prefix)
            .to_ascii_uppercase()
    );
    if let Ok(path) = std::env::var(&env_key) {
        let p = PathBuf::from(&path);
        if p.exists() {
            return Some(p);
        }
    }

    let socket_base = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_SOCKET_BASE,
        cellmembrane_types::service::DEFAULT_SOCKET_BASE,
    );
    let vps_path = PathBuf::from(&socket_base).join(socket_name);
    if vps_path.exists() {
        return Some(vps_path);
    }

    let socket_dir_name = Path::new(cellmembrane_types::service::DEFAULT_SOCKET_BASE)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("membrane");
    let xdg = std::env::var(cellmembrane_types::service::ENV_XDG_RUNTIME_DIR).unwrap_or_default();
    if !xdg.is_empty() {
        let p = PathBuf::from(&xdg).join(socket_dir_name).join(socket_name);
        if p.exists() {
            return Some(p);
        }
    }

    let fallback = std::env::temp_dir().join(socket_dir_name).join(socket_name);
    if fallback.exists() {
        return Some(fallback);
    }

    None
}

#[cfg(unix)]
fn uds_send(socket_path: &Path, request: &str) {
    use std::io::Write;
    use std::os::unix::net::UnixStream;

    let Ok(mut stream) = UnixStream::connect(socket_path) else {
        tracing::debug!(socket = %socket_path.display(), "impulse relay: UDS not reachable");
        return;
    };
    if stream
        .set_write_timeout(Some(std::time::Duration::from_secs(2)))
        .is_err()
    {
        tracing::debug!(socket = %socket_path.display(), "impulse relay: cannot set write timeout");
    }
    if stream
        .write_all(&crate::ribocipher::CLEAR_JSONRPC_SIGNAL)
        .is_err()
    {
        tracing::debug!(socket = %socket_path.display(), "impulse relay: signal write failed");
        return;
    }
    if writeln!(stream, "{request}").is_err() {
        tracing::debug!(socket = %socket_path.display(), "impulse relay: request write failed");
    }
}

#[cfg(unix)]
fn uds_request(socket_path: &Path, request: &str) -> Option<Vec<u8>> {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;

    let mut stream = UnixStream::connect(socket_path).ok()?;
    stream
        .set_write_timeout(Some(std::time::Duration::from_secs(2)))
        .ok()?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .ok()?;
    stream
        .write_all(&crate::ribocipher::CLEAR_JSONRPC_SIGNAL)
        .ok()?;
    writeln!(stream, "{request}").ok()?;
    stream.shutdown(std::net::Shutdown::Write).ok()?;

    let mut buf = Vec::with_capacity(4096);
    stream.read_to_end(&mut buf).ok()?;
    Some(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_socket_returns_none_for_missing() {
        let result = discover_socket("nonexistent-primal-socket.sock");
        assert!(result.is_none());
    }

    #[test]
    fn relay_socket_name_uses_capability() {
        let name = relay_socket_name();
        assert!(
            name.contains("songbird"),
            "relay socket should be songbird, got: {name}"
        );
        assert!(name.ends_with("-default.sock"));
    }

    #[test]
    fn signer_socket_name_uses_capability() {
        let name = signer_socket_name();
        assert!(
            name.contains("beardog"),
            "signer socket should be beardog, got: {name}"
        );
        assert!(name.ends_with("-default.sock"));
    }

    #[test]
    fn discover_socket_env_key_format() {
        let env_key = format!(
            "MEMBRANE_SOCKET_{}",
            "songbird-default.sock"
                .split_once('-')
                .map_or("songbird-default.sock", |(prefix, _)| prefix)
                .to_ascii_uppercase()
        );
        assert_eq!(env_key, "MEMBRANE_SOCKET_SONGBIRD");
    }
}
