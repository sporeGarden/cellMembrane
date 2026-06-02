// SPDX-License-Identifier: AGPL-3.0-or-later

//! Error types for membrane shadow operations.

use std::fmt;

/// Errors from shadow function execution.
#[derive(Debug, thiserror::Error)]
pub enum ShadowError {
    /// SSH transport failure (connection, timeout, command rejection).
    #[error("ssh: {0}")]
    Ssh(String),

    /// Forgejo API returned an error response.
    #[error("forgejo api {status}: {message}")]
    ForgejoApi {
        /// HTTP status code.
        status: u16,
        /// Error message from the API.
        message: String,
    },

    /// HTTP transport failure.
    #[cfg(feature = "http")]
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),

    /// No Forgejo token available.
    #[error("no forgejo token: set FORGEJO_TOKEN or create ~/.config/forgejo/token")]
    NoToken,

    /// Failed to parse response from VPS or API.
    #[error("parse: {0}")]
    Parse(String),

    /// TOML deserialization error.
    #[error("toml: {0}")]
    Toml(#[from] toml::de::Error),

    /// TOML serialization error.
    #[error("serialize: {0}")]
    Serialize(#[from] toml::ser::Error),

    /// Git command failure.
    #[error("git: {0}")]
    Git(String),

    /// IO error (file read, process spawn).
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization/deserialization error.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

/// Result type for shadow operations.
pub type Result<T> = std::result::Result<T, ShadowError>;

/// Outcome of a shadow operation — typed for JSON-RPC response.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ShadowOutcome {
    /// Whether the operation succeeded.
    pub ok: bool,
    /// Human-readable summary.
    pub message: String,
    /// Structured data (operation-specific).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl ShadowOutcome {
    /// Successful outcome with message.
    #[must_use]
    pub fn ok(message: impl Into<String>) -> Self {
        Self {
            ok: true,
            message: message.into(),
            data: None,
        }
    }

    /// Successful outcome with message and structured data.
    #[must_use]
    pub fn ok_with(message: impl Into<String>, data: serde_json::Value) -> Self {
        Self {
            ok: true,
            message: message.into(),
            data: Some(data),
        }
    }

    /// Failed outcome.
    #[must_use]
    pub fn fail(message: impl fmt::Display) -> Self {
        Self {
            ok: false,
            message: message.to_string(),
            data: None,
        }
    }
}
