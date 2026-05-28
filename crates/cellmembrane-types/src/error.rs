// SPDX-License-Identifier: AGPL-3.0-or-later

//! Typed error types for membrane configuration operations.
//!
//! Replaces string-based errors with structured variants per
//! ecosystem standard (`thiserror` for typed errors).

use std::path::PathBuf;

/// Errors that can occur when loading or parsing membrane configuration.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The configuration file could not be read from disk.
    #[error("failed to read {path}: {source}")]
    Read {
        /// Path that was attempted.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },

    /// The TOML content could not be parsed into valid configuration.
    #[error("failed to parse {path}: {source}")]
    Parse {
        /// Path that was attempted.
        path: PathBuf,
        /// Underlying TOML deserialization error.
        source: toml::de::Error,
    },
}
