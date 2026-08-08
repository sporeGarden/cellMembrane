// SPDX-License-Identifier: AGPL-3.0-or-later

//! Platform-agnostic synchronous IPC — centralizes all sync socket operations.
//!
//! Replaces per-callsite `#[cfg(unix)]` blocks in `impulse/primal.rs` and
//! `plasmid/signing_crypto.rs` with a single platform gate at the connect
//! point. BTSP handshake and signal negotiation are handled generically
//! over any `Read + Write` stream.
//!
//! On Unix: connects via `UnixStream` (UDS).
//! On other platforms: returns `None` / no-op (IPC not yet available).

use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use tracing::debug;

/// Connect to a local IPC socket at `path`, returning a stream that
/// implements `Read + Write` with timeouts already set.
///
/// Returns `None` if the socket is unreachable or the platform does not
/// support sync IPC.
#[cfg(unix)]
fn connect(path: &Path) -> Option<std::os::unix::net::UnixStream> {
    let stream = std::os::unix::net::UnixStream::connect(path).ok()?;
    let _ = stream.set_write_timeout(Some(Duration::from_secs(
        cellmembrane_types::service::DEFAULT_IPC_WRITE_TIMEOUT_SECS,
    )));
    let _ = stream.set_read_timeout(Some(Duration::from_secs(
        cellmembrane_types::service::DEFAULT_IPC_READ_TIMEOUT_SECS,
    )));
    Some(stream)
}

/// Check if a socket path belongs to the crypto signer primal.
pub(crate) fn is_crypto_signer_socket(path: &Path) -> bool {
    let binary = cellmembrane_types::MembraneService::binary_for(
        cellmembrane_types::ServiceCapability::CryptoSigner,
    );
    path.to_str().is_some_and(|s| s.contains(binary))
}

/// Write the appropriate riboCipher signal and perform BTSP handshake if needed.
///
/// For bearDog sockets: writes BTSP signal + performs handshake. If handshake
/// fails, returns `BtspFailed` (caller should retry with plain fallback).
/// For other sockets: writes the clear JSON-RPC signal.
fn negotiate_signal(stream: &mut (impl Read + Write), path: &Path) -> NegotiateResult {
    if is_crypto_signer_socket(path) {
        if stream
            .write_all(&crate::btsp_client::BTSP_JSONLINE_SIGNAL)
            .is_err()
        {
            return NegotiateResult::Failed;
        }
        if crate::btsp_client::handshake_sync(stream).is_none() {
            return NegotiateResult::BtspFailed;
        }
    } else if stream
        .write_all(&crate::ribocipher::CLEAR_JSONRPC_SIGNAL)
        .is_err()
    {
        return NegotiateResult::Failed;
    }
    NegotiateResult::Ok
}

enum NegotiateResult {
    Ok,
    BtspFailed,
    Failed,
}

/// Fire-and-forget: connect, negotiate signal, write request, close.
///
/// For bearDog sockets, attempts BTSP handshake with plain fallback.
/// Platform-agnostic — returns silently on unsupported platforms.
pub(crate) fn ipc_send(socket_path: &Path, request: &str) {
    #[cfg(unix)]
    {
        let Some(mut stream) = connect(socket_path) else {
            debug!(socket = %socket_path.display(), "ipc_send: not reachable");
            return;
        };

        match negotiate_signal(&mut stream, socket_path) {
            NegotiateResult::Ok => {}
            NegotiateResult::BtspFailed => {
                debug!(socket = %socket_path.display(), "ipc_send: BTSP failed, falling back");
                drop(stream);
                ipc_send_plain(socket_path, request);
                return;
            }
            NegotiateResult::Failed => {
                debug!(socket = %socket_path.display(), "ipc_send: signal write failed");
                return;
            }
        }

        if writeln!(stream, "{request}").is_err() {
            debug!(socket = %socket_path.display(), "ipc_send: request write failed");
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (socket_path, request);
    }
}

/// Request-response: connect, negotiate signal, write request, read response.
///
/// For bearDog sockets, attempts BTSP handshake with plain fallback.
/// Returns `None` on unsupported platforms or any connection/protocol failure.
pub(crate) fn ipc_request(socket_path: &Path, request: &str) -> Option<Vec<u8>> {
    #[cfg(unix)]
    {
        let mut stream = connect(socket_path)?;

        match negotiate_signal(&mut stream, socket_path) {
            NegotiateResult::Ok => {}
            NegotiateResult::BtspFailed => {
                debug!(socket = %socket_path.display(), "ipc_request: BTSP failed, falling back");
                drop(stream);
                return ipc_request_plain(socket_path, request);
            }
            NegotiateResult::Failed => return None,
        }

        writeln!(stream, "{request}").ok()?;
        stream.shutdown(std::net::Shutdown::Write).ok()?;

        let mut buf = Vec::with_capacity(4096);
        stream.read_to_end(&mut buf).ok()?;
        Some(buf)
    }
    #[cfg(not(unix))]
    {
        let _ = (socket_path, request);
        None
    }
}

/// Plain (no BTSP) fire-and-forget — fallback when BTSP handshake fails.
#[cfg(unix)]
fn ipc_send_plain(socket_path: &Path, request: &str) {
    let Some(mut stream) = connect(socket_path) else {
        return;
    };
    let _ = stream.write_all(&crate::ribocipher::CLEAR_JSONRPC_SIGNAL);
    let _ = writeln!(stream, "{request}");
}

/// Plain (no BTSP) request-response — fallback when BTSP handshake fails.
#[cfg(unix)]
fn ipc_request_plain(socket_path: &Path, request: &str) -> Option<Vec<u8>> {
    let mut stream = connect(socket_path)?;
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
    fn is_crypto_signer_socket_detects_signer() {
        let signer = cellmembrane_types::MembraneService::binary_for(
            cellmembrane_types::ServiceCapability::CryptoSigner,
        );
        let path = std::path::PathBuf::from(format!("/run/membrane/{signer}.sock"));
        assert!(is_crypto_signer_socket(&path));
    }

    #[test]
    fn is_crypto_signer_socket_rejects_non_signer() {
        let path = std::path::Path::new("/run/membrane/songbird.sock");
        assert!(!is_crypto_signer_socket(path));
    }

    #[test]
    fn ipc_send_returns_for_missing_socket() {
        ipc_send(Path::new("/tmp/nonexistent-test-membrane.sock"), "{}");
    }

    #[test]
    fn ipc_request_returns_none_for_missing_socket() {
        let result = ipc_request(Path::new("/tmp/nonexistent-test-membrane.sock"), "{}");
        assert!(result.is_none());
    }
}
