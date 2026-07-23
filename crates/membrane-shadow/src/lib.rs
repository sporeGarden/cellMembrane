// SPDX-License-Identifier: AGPL-3.0-or-later

//! `membrane-shadow` — Sovereign shadow functions for agentic VPS control.
//!
//! Replaces the bash `membrane.sh` script with typed Rust operations that
//! can be called from biomeOS `capability.call` or any gate-local tool.
//!
//! # Architecture
//!
//! Shadow functions bridge the gap between primal capability domains and
//! the golgiBody VPS infrastructure. Each function maps to a primal's
//! capability method:
//!
//! | Shadow module | Primal   | Capability domain        |
//! |---------------|----------|--------------------------|
//! | `forgejo`     | nestGate | `content.repo.*`         |
//! | `forgejo`     | nestGate | `content.mirror.*`       |
//! | `forgejo`     | bearDog  | `auth.token.*`           |
//! | `gate`        | biomeOS  | `gate.info/pull/check`   |
//! | `service`     | biomeOS  | `gate.service.*`         |
//!
//! # Transport
//!
//! - **Forgejo API**: HTTPS via `reqwest` (feature `http`)
//! - **VPS commands**: SSH via system client (`ssh golgi '...'`)
//! - **Neural API**: UDS JSON-RPC via `bridge` module
//!   — try-primal-first, fall back to shadow when biomeOS unavailable
//!
//! # Usage
//!
//! ```no_run
//! use membrane_shadow::{ShadowConfig, gate, forgejo, service};
//!
//! # async fn example() -> membrane_shadow::Result<()> {
//! let config = ShadowConfig::from_env().await;
//!
//! // biomeOS gate.info
//! let info = gate::info(&config).await?;
//! println!("{}: {} services", info.hostname, info.services.len());
//!
//! // nestGate content.repo.list
//! let repos = forgejo::repo_list(&config, "ecoPrimals").await?;
//! println!("{} repos", repos.len());
//!
//! // biomeOS gate.service.restart
//! let status = service::restart(&config, "beardog-membrane").await?;
//! assert!(status.active);
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::redundant_pub_crate)]

pub(crate) mod bridge;
pub(crate) mod caddy;
pub(crate) mod cli;
#[cfg(feature = "cloudflare")]
pub(crate) mod cloudflare;
pub mod config;
pub(crate) mod context;
pub mod dispatch;
pub mod error;
pub mod forgejo;
pub(crate) mod freshness;
pub mod gate;
pub(crate) mod gateway;
pub(crate) mod git_ops;
pub(crate) mod identity;
pub(crate) mod impulse;
pub(crate) mod jsonrpc;
pub(crate) mod manifest;
pub(crate) mod plasmid;
#[cfg(feature = "http")]
pub(crate) mod provision;
pub(crate) mod relay;
pub(crate) mod resolve;
pub(crate) mod ribocipher;
pub mod service;
pub(crate) mod sovereignty_ledger;
pub(crate) mod ssh;
pub(crate) mod temporal;
pub(crate) mod topology;
pub(crate) mod tower;
pub(crate) mod webhook;

pub use config::ShadowConfig;
pub use error::{Result, ShadowError, ShadowOutcome};

/// Resolve the ecoPrimals workspace root directory.
///
/// Resolution chain:
/// 1. `ECOPRIMALS_ROOT` env var (validated by workspace marker)
/// 2. Walk up from current executable looking for workspace markers
///
/// Recognized markers: `primals/`, `infra/`, `gardens/`, `.ecoprimals`
/// This supports both full development workspaces and sparse VPS deployments.
pub fn resolve_workspace_root() -> Result<std::path::PathBuf> {
    use std::path::{Path, PathBuf};

    fn is_workspace(p: &Path) -> bool {
        p.join("primals").exists()
            || p.join("infra").exists()
            || p.join("gardens").exists()
            || p.join(".ecoprimals").exists()
    }

    if let Ok(root) = std::env::var(cellmembrane_types::service::ENV_ECOPRIMALS_ROOT) {
        let path = PathBuf::from(&root);
        if is_workspace(&path) {
            return Ok(path);
        }
    }

    // Walk up from CWD (handles running membrane from within the workspace)
    if let Ok(cwd) = std::env::current_dir() {
        let mut dir: Option<PathBuf> = Some(cwd);
        while let Some(d) = dir {
            if is_workspace(&d) {
                return Ok(d);
            }
            dir = d.parent().map(Path::to_path_buf);
        }
    }

    // Walk up from executable location (VPS deployments where binary is inside workspace)
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(Path::to_path_buf);
        while let Some(d) = dir {
            if is_workspace(&d) {
                return Ok(d);
            }
            dir = d.parent().map(Path::to_path_buf);
        }
    }

    Err(ShadowError::Config(
        "cannot resolve ecoPrimals workspace root — set ECOPRIMALS_ROOT".into(),
    ))
}

/// Resolve the XDG data home directory (`$XDG_DATA_HOME` or `$HOME/.local/share`).
#[must_use]
pub fn resolve_xdg_data_home() -> std::path::PathBuf {
    use std::path::PathBuf;
    std::env::var(cellmembrane_types::service::ENV_XDG_DATA_HOME).map_or_else(
        |_| {
            PathBuf::from(cellmembrane_types::service::env_or(
                cellmembrane_types::service::ENV_HOME,
                "/tmp",
            ))
            .join(".local")
            .join("share")
        },
        PathBuf::from,
    )
}

/// Atomically write contents to a file via temp + rename.
///
/// Prevents partial/corrupt reads by writing to a sibling `.tmp` file and
/// renaming only on success. On failure, the `.tmp` file is cleaned up.
pub fn atomic_write(path: &std::path::Path, contents: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

// ── Timestamp helpers ────────────────────────────────────────────────

/// Current UTC time formatted as ISO 8601 (e.g. `2026-07-17T13:45:00Z`).
#[must_use]
pub fn utc_now_iso8601() -> String {
    chrono::Utc::now()
        .format(cellmembrane_types::service::ISO8601_UTC)
        .to_string()
}

/// Current UTC date as `YYYY-MM-DD`.
#[must_use]
pub fn utc_today() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

/// Current UTC time as RFC 3339 (e.g. `2026-07-17T13:45:00.123+00:00`).
#[must_use]
pub fn utc_now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Compact UTC timestamp for session IDs (e.g. `20260717T134500`).
#[must_use]
pub fn utc_now_compact() -> String {
    chrono::Utc::now().format("%Y%m%dT%H%M%S").to_string()
}

// ── HTTP client ──────────────────────────────────────────────────────

/// Build a `reqwest::Client` with a timeout.
///
/// All HTTP-using code should route through this to ensure consistent
/// TLS backend (rustls) and timeout policy.
#[cfg(feature = "http")]
pub fn http_client(timeout: std::time::Duration) -> error::Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| error::ShadowError::Config(format!("HTTP client: {e}")))
}

/// Build a `reqwest::Client` that accepts invalid TLS certs.
///
/// Only for loopback/localhost testing — never WAN traffic.
#[cfg(feature = "http")]
pub fn http_client_insecure(timeout: std::time::Duration) -> error::Result<reqwest::Client> {
    reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(timeout)
        .build()
        .map_err(|e| error::ShadowError::Config(format!("HTTP client (insecure): {e}")))
}

// ── Atomic I/O ───────────────────────────────────────────────────────

/// Async variant of [`atomic_write`] using `tokio::fs` for non-blocking I/O.
///
/// Preferred in all `async fn` contexts to avoid stalling the executor.
pub async fn atomic_write_async(path: &std::path::Path, contents: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    tokio::fs::write(&tmp, contents).await?;
    match tokio::fs::rename(&tmp, path).await {
        Ok(()) => Ok(()),
        Err(e) => {
            if let Err(cleanup_err) = tokio::fs::remove_file(&tmp).await {
                tracing::debug!(error = %cleanup_err, "tmp cleanup after rename failure");
            }
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_xdg_data_home_returns_non_empty() {
        let path = resolve_xdg_data_home();
        assert!(!path.as_os_str().is_empty());
    }

    #[test]
    fn atomic_write_creates_file() {
        let dir = std::env::temp_dir().join("membrane-lib-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test_atomic.bin");
        atomic_write(&path, b"hello membrane").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"hello membrane");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn atomic_write_no_partial_on_rename() {
        let dir = std::env::temp_dir().join("membrane-lib-test-atomic");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("atomic_test.txt");
        atomic_write(&path, b"first").unwrap();
        atomic_write(&path, b"second").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
        let tmp = path.with_extension("tmp");
        assert!(!tmp.exists(), ".tmp should be cleaned up");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn atomic_write_async_creates_file() {
        let dir = std::env::temp_dir().join("membrane-lib-test-async");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("async_atomic.bin");
        atomic_write_async(&path, b"async hello").await.unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"async hello");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resolve_workspace_succeeds_in_workspace() {
        let result = resolve_workspace_root();
        assert!(
            result.is_ok(),
            "should find workspace from CWD within ecoPrimals"
        );
    }

    #[test]
    fn utc_now_iso8601_format() {
        let ts = utc_now_iso8601();
        assert!(ts.ends_with('Z'), "should end with Z: {ts}");
        assert!(ts.contains('T'), "should contain T separator: {ts}");
        assert_eq!(ts.len(), 20, "YYYY-MM-DDTHH:MM:SSZ = 20 chars: {ts}");
    }

    #[test]
    fn utc_today_format() {
        let d = utc_today();
        assert_eq!(d.len(), 10, "YYYY-MM-DD = 10 chars: {d}");
        assert_eq!(&d[4..5], "-");
        assert_eq!(&d[7..8], "-");
    }

    #[test]
    fn utc_now_rfc3339_format() {
        let ts = utc_now_rfc3339();
        assert!(ts.contains('T'), "should contain T separator: {ts}");
        assert!(
            ts.contains('+') || ts.contains('Z'),
            "should have offset: {ts}"
        );
    }

    #[test]
    fn utc_now_compact_format() {
        let ts = utc_now_compact();
        assert_eq!(ts.len(), 15, "YYYYMMDDTHHmmss = 15 chars: {ts}");
        assert_eq!(&ts[8..9], "T");
    }

    #[cfg(feature = "http")]
    #[test]
    fn http_client_builds_successfully() {
        let client = http_client(std::time::Duration::from_secs(5));
        assert!(client.is_ok());
    }

    #[cfg(feature = "http")]
    #[test]
    fn http_client_insecure_builds_successfully() {
        let client = http_client_insecure(std::time::Duration::from_secs(5));
        assert!(client.is_ok());
    }
}
