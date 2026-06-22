// SPDX-License-Identifier: AGPL-3.0-or-later

//! Error types for membrane shadow operations.

use std::fmt;

/// Errors from shadow function execution.
#[derive(Debug, thiserror::Error)]
pub enum ShadowError {
    /// Cloudflare API returned an error response.
    #[error("cloudflare: {0}")]
    CloudflareApi(String),

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
    #[error("no forgejo token: set FORGEJO_TOKEN env var (file-based tokens deprecated Wave 121)")]
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

    /// Configuration error (missing key, invalid structure).
    #[error("config: {0}")]
    Config(String),

    /// Build pipeline failure.
    #[error("build: {0}")]
    Build(String),

    /// Git command failure.
    #[error("git: {0}")]
    Git(String),

    /// IO error (file read, process spawn).
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization/deserialization error.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    /// JSON-RPC transport failure (UDS, TCP, or relay).
    #[error("rpc: {0}")]
    Rpc(String),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_ok_sets_fields() {
        let o = ShadowOutcome::ok("success");
        assert!(o.ok);
        assert_eq!(o.message, "success");
        assert!(o.data.is_none());
    }

    #[test]
    fn outcome_ok_with_carries_data() {
        let data = serde_json::json!({"count": 3});
        let o = ShadowOutcome::ok_with("done", data);
        assert!(o.ok);
        assert_eq!(o.data.unwrap()["count"], 3);
    }

    #[test]
    fn outcome_fail_sets_not_ok() {
        let o = ShadowOutcome::fail("broken");
        assert!(!o.ok);
        assert_eq!(o.message, "broken");
        assert!(o.data.is_none());
    }

    #[test]
    fn outcome_serializes_to_json() {
        let o = ShadowOutcome::ok_with("test", serde_json::json!({"x": 1}));
        let json = serde_json::to_string(&o).unwrap();
        assert!(json.contains(r#""ok":true"#));
        assert!(json.contains(r#""message":"test""#));
    }

    #[test]
    fn outcome_deserializes_from_json() {
        let json = r#"{"ok":false,"message":"err"}"#;
        let o: ShadowOutcome = serde_json::from_str(json).unwrap();
        assert!(!o.ok);
        assert_eq!(o.message, "err");
        assert!(o.data.is_none());
    }

    #[test]
    fn outcome_skip_serializing_none_data() {
        let o = ShadowOutcome::ok("test message");
        let json = serde_json::to_string(&o).unwrap();
        assert!(!json.contains(r#""data""#));
    }

    #[test]
    fn shadow_error_display() {
        let e = ShadowError::Ssh("timeout".into());
        assert_eq!(e.to_string(), "ssh: timeout");

        let e = ShadowError::Parse("bad input".into());
        assert_eq!(e.to_string(), "parse: bad input");

        let e = ShadowError::Config("missing key".into());
        assert_eq!(e.to_string(), "config: missing key");

        let e = ShadowError::Rpc("connect timeout: /tmp/foo.sock".into());
        assert_eq!(e.to_string(), "rpc: connect timeout: /tmp/foo.sock");

        let e = ShadowError::ForgejoApi {
            status: 404,
            message: "not found".into(),
        };
        assert_eq!(e.to_string(), "forgejo api 404: not found");
    }

    #[test]
    fn shadow_error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "gone");
        let e: ShadowError = io_err.into();
        assert!(e.to_string().contains("gone"));
    }

    #[test]
    fn shadow_error_from_json() {
        let json_err = serde_json::from_str::<serde_json::Value>("not json").unwrap_err();
        let e: ShadowError = json_err.into();
        assert!(e.to_string().starts_with("json:"));
    }
}
