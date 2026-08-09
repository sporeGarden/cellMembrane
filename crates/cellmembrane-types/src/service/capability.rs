// SPDX-License-Identifier: AGPL-3.0-or-later

//! Service capability, health check, and server contract types.
//!
//! These types describe the runtime behavior and classification of membrane
//! services: what they provide (capabilities), how to verify they're healthy
//! (health check methods), and how to start them (server contracts).

use super::constants::DEFAULT_INSTALL_BASE;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Transport protocol for a service port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    /// TCP only.
    Tcp,
    /// UDP only.
    Udp,
    /// Both TCP and UDP on the same port.
    TcpAndUdp,
    /// Unix domain socket (no port).
    Uds,
}

impl fmt::Display for Protocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tcp => write!(f, "tcp"),
            Self::Udp => write!(f, "udp"),
            Self::TcpAndUdp => write!(f, "tcp+udp"),
            Self::Uds => write!(f, "uds"),
        }
    }
}

/// Transport mode for VPS deployment (Wave 56 standard).
///
/// Determines whether a primal uses TCP ports or Unix domain sockets
/// for inter-primal communication. The VPS standard is `UdsOnly` —
/// zero TCP ports for all NUCLEUS primals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportMode {
    /// UDS-only: no TCP ports allocated. VPS deployment standard.
    /// Health checks via socket file existence.
    UdsOnly,
    /// TCP default: service binds to a TCP port (legacy / symbiotic).
    TcpDefault,
    /// TCP opt-in: UDS primary, TCP available via `TRANSPORT_ENDPOINT` injection.
    TcpOptIn,
}

impl fmt::Display for TransportMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UdsOnly => write!(f, "uds_only"),
            Self::TcpDefault => write!(f, "tcp_default"),
            Self::TcpOptIn => write!(f, "tcp_opt_in"),
        }
    }
}

/// Server CLI contract — describes what args a primal's `server` subcommand accepts.
///
/// Each primal has evolved independently, resulting in CLI divergence. This enum
/// captures the actual capabilities so template systemd units can generate correct
/// `ExecStart` lines without trial and error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerContract {
    /// Full guideStone P4 contract: `server --socket <path> --security-socket <path> --pid-dir <path>`
    /// Used by: songbird, skunkbat
    Full,
    /// Socket + audit-dir: `server --socket <path> --audit-dir <path>`
    /// Used by: beardog (crypto spine)
    SocketAuditDir,
    /// Socket-only: `server --socket <path>`
    /// Used by: sweetgrass, coralreef, squirrel, petaltongue, barracuda, toadstool
    SocketOnly,
    /// Server subcommand only: `server` (socket path via env/convention, no `--socket` flag).
    /// Used by: nestgate (CLI evolved in Wave 150x)
    ServerNoSocket,
    /// biomeOS-style: `api --socket <path>` or `neural-api --socket <path>`
    /// Used by: biomeos
    BiomeosApi,
    /// External binary with no `server` subcommand — started by systemd with args in the unit.
    /// Used by: hbbs, hbbr, caddy
    External,
    /// Tarpc-primary server — accepts both `--socket` (JSON-RPC) and
    /// `--tarpc-socket` (binary protocol) under C2 dual-socket pattern.
    /// Under G65, these primals transition to `SocketOnly` as negotiation
    /// replaces the separate tarpc socket.
    /// Used by: loamspine, rhizocrypt (transitional — will be `SocketOnly` post-G65)
    Tarpc,
}

impl ServerContract {
    /// Generate the `ExecStart` args for a primal given socket/security paths.
    ///
    /// Uses `install_base` to allow deployment to non-standard locations.
    #[must_use]
    pub fn exec_args_with_base(
        &self,
        install_base: &str,
        binary: &str,
        socket_path: &str,
        security_socket: &str,
    ) -> String {
        let socket_base = crate::service::resolve_socket_base();
        match self {
            Self::Full => format!(
                "{install_base}/{binary} server --socket {socket_path} --security-socket {security_socket} --pid-dir {socket_base}"
            ),
            Self::SocketAuditDir => format!(
                "{install_base}/{binary} server --socket {socket_path} --audit-dir {socket_base}/{binary}"
            ),
            Self::SocketOnly => {
                format!("{install_base}/{binary} server --socket {socket_path}")
            }
            Self::Tarpc => {
                let tarpc_path = format!(
                    "{socket_base}/{binary}{}",
                    super::constants::TARPC_SOCKET_SUFFIX
                );
                format!(
                    "{install_base}/{binary} server --socket {socket_path} --tarpc-socket {tarpc_path}"
                )
            }
            Self::ServerNoSocket => format!("{install_base}/{binary} server"),
            Self::BiomeosApi => {
                format!("{install_base}/{binary} neural-api --socket {socket_path}")
            }
            Self::External => format!("{install_base}/{binary}"),
        }
    }

    /// Generate the `ExecStart` args using the default install base.
    #[must_use]
    pub fn exec_args(&self, binary: &str, socket_path: &str, security_socket: &str) -> String {
        self.exec_args_with_base(DEFAULT_INSTALL_BASE, binary, socket_path, security_socket)
    }
}

/// Capability tag for runtime discovery.
///
/// Instead of hardcoding binary names ("songbird", "beardog") in production
/// code, services declare capabilities and consumers discover providers
/// through the registry at compile time or runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceCapability {
    /// Mesh relay — provides peer-to-peer connectivity and message routing.
    MeshRelay,
    /// TURN server — NAT traversal for real-time connections.
    TurnServer,
    /// Cryptographic signing — ed25519 signatures, key management.
    CryptoSigner,
    /// Security enforcement — authentication, authorization, secrets.
    Security,
    /// Observability — metrics collection, health aggregation.
    Observability,
    /// Content serving — static file / API serving.
    ContentServing,
    /// Storage — persistent data management.
    Storage,
    /// Compute orchestration — job scheduling, pipeline execution.
    ComputeOrchestration,
    /// Identity — gate identity, certificate management.
    Identity,
    /// DNS authority — authoritative DNS serving.
    DnsAuthority,
    /// Reverse proxy — TLS termination, HTTP routing.
    ReverseProxy,
    /// Visualization — scene rendering, data visualization.
    Visualization,
    /// Content-addressed storage — CAS blob serving.
    ContentAddressedStorage,
    /// Build — binary harvest, cross-compilation, depot staging.
    Build,
}

impl ServiceCapability {
    /// Stable wire-format name matching serde `snake_case` rename.
    ///
    /// Used in mesh relay routing (`TransportEndpoint::MeshRelay.capability`)
    /// and JSON-RPC `relay.forward` envelopes.
    #[must_use]
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::MeshRelay => "mesh_relay",
            Self::TurnServer => "turn_server",
            Self::CryptoSigner => "crypto_signer",
            Self::Security => "security",
            Self::Observability => "observability",
            Self::ContentServing => "content_serving",
            Self::Storage => "storage",
            Self::ComputeOrchestration => "compute_orchestration",
            Self::Identity => "identity",
            Self::DnsAuthority => "dns_authority",
            Self::ReverseProxy => "reverse_proxy",
            Self::Visualization => "visualization",
            Self::ContentAddressedStorage => "content_addressed_storage",
            Self::Build => "build",
        }
    }

    /// Parse a wire-format name back into a `ServiceCapability`.
    #[must_use]
    pub fn from_wire(s: &str) -> Option<Self> {
        match s {
            "mesh_relay" => Some(Self::MeshRelay),
            "turn_server" => Some(Self::TurnServer),
            "crypto_signer" => Some(Self::CryptoSigner),
            "security" => Some(Self::Security),
            "observability" => Some(Self::Observability),
            "content_serving" => Some(Self::ContentServing),
            "storage" => Some(Self::Storage),
            "compute_orchestration" => Some(Self::ComputeOrchestration),
            "identity" => Some(Self::Identity),
            "dns_authority" => Some(Self::DnsAuthority),
            "reverse_proxy" => Some(Self::ReverseProxy),
            "visualization" => Some(Self::Visualization),
            "content_addressed_storage" => Some(Self::ContentAddressedStorage),
            "build" => Some(Self::Build),
            _ => None,
        }
    }
}

impl fmt::Display for ServiceCapability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.wire_name())
    }
}

/// Health check strategy for a service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthCheckMethod {
    /// JSON-RPC `health.liveness` probe.
    Liveness,
    /// Raw TCP connection probe.
    TcpConnect,
    /// HTTPS GET probe (200 OK).
    HttpsProbe,
    /// DNS query probe.
    DnsProbe,
    /// UDS socket file existence check (VPS standard).
    SocketExists,
}

impl fmt::Display for HealthCheckMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Liveness => write!(f, "health.liveness"),
            Self::TcpConnect => write!(f, "tcp_connect"),
            Self::HttpsProbe => write!(f, "https_probe"),
            Self::DnsProbe => write!(f, "dns_probe"),
            Self::SocketExists => write!(f, "socket_exists"),
        }
    }
}
