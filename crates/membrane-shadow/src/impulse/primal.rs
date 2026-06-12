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
    let svc = cellmembrane_types::MembraneService::with_capability(ServiceCapability::MeshRelay);
    let binary = svc.map_or(cellmembrane_types::service::FALLBACK_MESH_RELAY, |s| {
        s.binary
    });
    format!("{binary}-default.sock")
}

fn signer_socket_name() -> String {
    let svc = cellmembrane_types::MembraneService::with_capability(ServiceCapability::CryptoSigner);
    let binary = svc.map_or(cellmembrane_types::service::FALLBACK_CRYPTO_SIGNER, |s| {
        s.binary
    });
    format!("{binary}-default.sock")
}

pub fn try_relay_impulse(impulse: &ImpulseFile) {
    #[cfg(not(unix))]
    {
        let _ = impulse;
        return;
    }

    #[cfg(unix)]
    {
        let Some(socket_path) = discover_socket(&relay_socket_name()) else {
            return;
        };

        let notification = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "mesh.publish",
            "params": {
                "topic": format!("impulse/{}", impulse.from.gate),
                "payload": {
                    "id": impulse.impulse.id,
                    "type": impulse.impulse.impulse_type,
                    "from": impulse.from.gate,
                    "to": impulse.to.gates,
                    "subject": impulse.content.subject,
                    "priority": impulse.impulse.priority,
                }
            }
        });

        let Ok(request_str) = serde_json::to_string(&notification) else {
            return;
        };

        uds_send(&socket_path, &request_str);
    }
}

#[must_use]
pub fn try_sign_impulse(_workspace_root: &Path, impulse_id: &str) -> Option<ImpulseSignature> {
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

    let socket_base = std::env::var(cellmembrane_types::service::ENV_SOCKET_BASE)
        .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_SOCKET_BASE.into());
    let vps_path = PathBuf::from(&socket_base).join(socket_name);
    if vps_path.exists() {
        return Some(vps_path);
    }

    let xdg = std::env::var(cellmembrane_types::service::ENV_XDG_RUNTIME_DIR).unwrap_or_default();
    if !xdg.is_empty() {
        let p = PathBuf::from(format!("{xdg}/membrane/{socket_name}"));
        if p.exists() {
            return Some(p);
        }
    }

    let fallback = std::env::temp_dir().join("membrane").join(socket_name);
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
        return;
    };
    let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(2)));
    let _ = writeln!(stream, "{request}");
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
}
