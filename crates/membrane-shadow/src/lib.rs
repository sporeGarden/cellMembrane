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
#![allow(
    clippy::missing_errors_doc,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

pub mod bridge;
pub mod caddy;
pub mod cli;
#[cfg(feature = "cloudflare")]
pub mod cloudflare;
pub mod config;
pub mod context;
pub mod dispatch;
pub mod error;
pub mod forgejo;
pub mod freshness;
pub mod gate;
pub mod git_ops;
pub mod identity;
pub mod jsonrpc;
pub mod impulse;
pub mod manifest;
pub mod plasmid;
#[cfg(feature = "http")]
pub mod provision;
pub mod relay;
pub mod ribocipher;
pub mod service;
pub mod ssh;
pub mod temporal;
pub mod webhook;

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

    Err(ShadowError::Parse(
        "cannot resolve ecoPrimals workspace root — set ECOPRIMALS_ROOT".into(),
    ))
}

/// Resolve the XDG data home directory (`$XDG_DATA_HOME` or `$HOME/.local/share`).
#[must_use]
pub fn resolve_xdg_data_home() -> std::path::PathBuf {
    use std::path::PathBuf;
    std::env::var(cellmembrane_types::service::ENV_XDG_DATA_HOME).map_or_else(
        |_| {
            let home = std::env::var(cellmembrane_types::service::ENV_HOME)
                .unwrap_or_else(|_| "/tmp".into());
            PathBuf::from(home).join(".local").join("share")
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
