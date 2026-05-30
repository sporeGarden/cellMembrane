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
//! - **Future**: UDS JSON-RPC when primals gain native shadow dispatch
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

pub mod config;
pub mod error;
pub mod forgejo;
pub mod gate;
pub mod identity;
pub mod manifest;
pub mod service;
pub mod ssh;
pub mod temporal;

pub use config::ShadowConfig;
pub use error::{Result, ShadowError, ShadowOutcome};
