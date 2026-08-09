// SPDX-License-Identifier: AGPL-3.0-or-later

//! Transport abstraction (G66) — silicon-agnostic byte pipes.
//!
//! All platform conditionals (`#[cfg(unix)]`) are confined to this module.
//! Business logic receives a [`TransportStream`] and reads/writes bytes
//! without knowing whether the connection is UDS, TCP, or Named Pipe.
//!
//! ## Pattern
//!
//! ```text
//! TransportEndpoint  →  connect_transport()  →  TransportStream
//!   (what to reach)       (platform bridge)       (byte pipe)
//! ```
//!
//! `TransportEndpoint` lives in `cellmembrane-types` (pure data).
//! `TransportStream` lives here (async runtime, `#[cfg]` guards).

use cellmembrane_types::TransportEndpoint;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// A connected byte pipe — platform-aware, protocol-agnostic.
///
/// All `#[cfg(unix)]` for IPC connections is confined to this enum.
/// Protocol negotiation (G65), BTSP framing, JSON-RPC, and tarpc all
/// operate on `TransportStream` without knowing the underlying transport.
pub enum TransportStream {
    /// Unix Domain Socket (Linux, macOS, BSDs).
    #[cfg(unix)]
    Unix(tokio::net::UnixStream),

    /// TCP stream (cross-platform, cross-host).
    Tcp(tokio::net::TcpStream),
}

impl std::fmt::Debug for TransportStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            #[cfg(unix)]
            Self::Unix(_) => f.debug_tuple("Unix").field(&"<UnixStream>").finish(),
            Self::Tcp(_) => f.debug_tuple("Tcp").field(&"<TcpStream>").finish(),
        }
    }
}

impl TransportStream {
    /// Whether the underlying transport is local IPC (UDS or Named Pipe).
    ///
    /// Available for callers evolving toward G66 transport-aware health
    /// checks and BTSP local-trust (G63).
    #[allow(dead_code, reason = "G66 transport-aware health checks")]
    #[must_use]
    pub const fn is_local(&self) -> bool {
        match self {
            #[cfg(unix)]
            Self::Unix(_) => true,
            Self::Tcp(_) => false,
        }
    }

    /// Transport name for diagnostics.
    #[allow(dead_code, reason = "G66 transport diagnostics")]
    #[must_use]
    pub const fn transport_name(&self) -> &'static str {
        match self {
            #[cfg(unix)]
            Self::Unix(_) => "uds",
            Self::Tcp(_) => "tcp",
        }
    }
}

impl AsyncRead for TransportStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            #[cfg(unix)]
            Self::Unix(s) => Pin::new(s).poll_read(cx, buf),
            Self::Tcp(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for TransportStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            #[cfg(unix)]
            Self::Unix(s) => Pin::new(s).poll_write(cx, buf),
            Self::Tcp(s) => Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            #[cfg(unix)]
            Self::Unix(s) => Pin::new(s).poll_flush(cx),
            Self::Tcp(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            #[cfg(unix)]
            Self::Unix(s) => Pin::new(s).poll_shutdown(cx),
            Self::Tcp(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

/// Connect to a [`TransportEndpoint`], producing a platform-appropriate
/// byte stream.
///
/// All `#[cfg(unix)]` IPC connection logic is confined here. Callers
/// operate on the returned [`TransportStream`] without platform awareness.
///
/// # Errors
///
/// Returns `io::Error` for connection failures. `MeshRelay` endpoints
/// are not directly connectable — they require songBird routing.
pub async fn connect_transport(endpoint: &TransportEndpoint) -> io::Result<TransportStream> {
    match endpoint {
        #[cfg(unix)]
        TransportEndpoint::Uds { path } => {
            let stream = tokio::net::UnixStream::connect(path).await?;
            Ok(TransportStream::Unix(stream))
        }
        #[cfg(not(unix))]
        TransportEndpoint::Uds { path } => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("UDS not available on this platform for {path}"),
        )),
        TransportEndpoint::NamedPipe { pipe_name } => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("Named Pipe transport not yet implemented: {pipe_name}"),
        )),
        TransportEndpoint::Tcp { host, port } => {
            let stream = tokio::net::TcpStream::connect(format!("{host}:{port}")).await?;
            Ok(TransportStream::Tcp(stream))
        }
        TransportEndpoint::MeshRelay {
            peer_id,
            capability,
        } => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("mesh relay requires songBird routing: {peer_id}/{capability}"),
        )),
    }
}

/// Build a [`TransportEndpoint`] from the environment or platform default.
///
/// Resolution chain:
/// 1. `TRANSPORT_ENDPOINT` env var (JSON format)
/// 2. `platform_default()` — UDS on Unix, TCP localhost on Windows
///
/// This is the primary entry point for transport injection. Systemd units,
/// biomeOS, and songBird inject the endpoint via the env var; local dev
/// falls back to the platform default.
#[allow(
    dead_code,
    reason = "G66 transport injection entry point — wired per-primal incrementally"
)]
pub fn endpoint_from_env_or_default(binary: &str, port: Option<u16>) -> TransportEndpoint {
    if let Ok(val) = std::env::var(cellmembrane_types::transport::ENV_TRANSPORT_ENDPOINT) {
        if let Ok(ep) = TransportEndpoint::from_env_value(&val) {
            return ep;
        }
        tracing::warn!(
            value = %val,
            "invalid TRANSPORT_ENDPOINT — falling back to platform default"
        );
    }
    platform_default(binary, port)
}

/// Platform-appropriate default endpoint for a primal.
///
/// - Unix: `Uds { path: "{socket_base}/{binary}.sock" }`
/// - Non-Unix: `Tcp { host: "127.0.0.1", port }` (falls back to port 0 if unknown)
fn platform_default(binary: &str, port: Option<u16>) -> TransportEndpoint {
    let socket_base = cellmembrane_types::service::resolve_socket_base();
    if cfg!(unix) {
        TransportEndpoint::Uds {
            path: format!("{socket_base}/{binary}.sock"),
        }
    } else {
        TransportEndpoint::Tcp {
            host: cellmembrane_types::service::BIND_LOOPBACK.into(),
            port: port.unwrap_or(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_default_returns_uds_on_unix() {
        let ep = platform_default("beardog", Some(8443));
        if cfg!(unix) {
            assert!(
                matches!(ep, TransportEndpoint::Uds { .. }),
                "Unix should get UDS: {ep:?}"
            );
        } else {
            assert!(
                matches!(ep, TransportEndpoint::Tcp { .. }),
                "non-Unix should get TCP: {ep:?}"
            );
        }
    }

    #[test]
    fn endpoint_falls_back_to_platform_default() {
        let ep = platform_default("beardog", Some(8443));
        assert!(!ep.display_uri().is_empty());
        if cfg!(unix) {
            assert!(matches!(ep, TransportEndpoint::Uds { .. }));
        } else {
            assert!(matches!(ep, TransportEndpoint::Tcp { .. }));
        }
    }

    #[test]
    fn transport_stream_trait_impls_compile() {
        fn assert_read<T: AsyncRead>() {}
        fn assert_write<T: AsyncWrite>() {}
        assert_read::<TransportStream>();
        assert_write::<TransportStream>();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn connect_uds_nonexistent_errors() {
        let ep = TransportEndpoint::Uds {
            path: "/tmp/nonexistent-g66-test.sock".into(),
        };
        let result = connect_transport(&ep).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn connect_tcp_unreachable_errors() {
        let ep = TransportEndpoint::Tcp {
            host: "192.0.2.1".into(),
            port: 1,
        };
        let result =
            tokio::time::timeout(std::time::Duration::from_secs(2), connect_transport(&ep)).await;
        assert!(
            result.is_err() || result.unwrap().is_err(),
            "unreachable TCP should error or timeout"
        );
    }

    #[tokio::test]
    async fn connect_mesh_relay_errors() {
        let ep = TransportEndpoint::MeshRelay {
            peer_id: "test".into(),
            capability: "crypto".into(),
        };
        let result = connect_transport(&ep).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Unsupported);
    }

    #[tokio::test]
    async fn connect_named_pipe_errors() {
        let ep = TransportEndpoint::NamedPipe {
            pipe_name: r"\\.\pipe\membrane-test".into(),
        };
        let result = connect_transport(&ep).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Unsupported);
    }
}
