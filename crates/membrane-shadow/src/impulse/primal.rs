// SPDX-License-Identifier: AGPL-3.0-or-later

//! Optional primal UDS integration (songbird relay, bearDog signing).
//!
//! All functions are best-effort: missing sockets or failed connections
//! fall through silently — git push is the reliable baseline.

use std::path::{Path, PathBuf};

use chrono::Local;

use super::types::{ImpulseFile, ImpulseSignature};

pub(crate) fn try_relay_impulse(impulse: &ImpulseFile) {
    let Some(socket_path) = discover_socket("songbird-default.sock") else {
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

pub(crate) fn try_sign_impulse(
    _workspace_root: &Path,
    impulse_id: &str,
) -> Option<ImpulseSignature> {
    let socket_path = discover_socket("beardog-default.sock")?;

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

pub(crate) fn discover_socket(socket_name: &str) -> Option<PathBuf> {
    let xdg = std::env::var("XDG_RUNTIME_DIR").unwrap_or_default();
    let candidates = [
        PathBuf::from(format!("{xdg}/biomeos/{socket_name}")),
        PathBuf::from(format!("/tmp/biomeos/{socket_name}")),
    ];
    candidates.into_iter().find(|p| p.exists())
}

fn uds_send(socket_path: &Path, request: &str) {
    use std::io::Write;
    use std::os::unix::net::UnixStream;

    let Ok(mut stream) = UnixStream::connect(socket_path) else {
        return;
    };
    let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(2)));
    let _ = writeln!(stream, "{request}");
}

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
