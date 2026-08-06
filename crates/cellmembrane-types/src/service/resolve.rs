// SPDX-License-Identifier: AGPL-3.0-or-later

//! Runtime environment resolution — env var reading, path discovery, and
//! scope-aware defaults.
//!
//! These functions adapt deployment paths to the current init scope
//! (systemd system, systemd user, bare) and honor `MEMBRANE_*` env
//! overrides throughout. Extracted from `constants.rs` to keep that
//! module focused on static declarations.

use super::constants::{
    DEFAULT_SOCKET_BASE, ENV_BIOMEOS_SOCKET_DIR, ENV_EUID, ENV_FAMILY_SEED, ENV_FAMILY_SEED_LEGACY,
    ENV_FAMILY_SEED_LEGACY2, ENV_GATE_NAME, ENV_GATE_NAME_LEGACY, ENV_HOME, ENV_INIT_SCOPE,
    ENV_SOCKET_BASE, ENV_SYSTEMD_UNIT_DIR, ENV_UID, ENV_WEBHOOK_SECRET, ENV_WEBHOOK_SECRET_LEGACY,
    ENV_XDG_RUNTIME_DIR, NEURAL_API_NAMESPACE, SYSTEMD_UNIT_DIR, SYSTEMD_USER_UNIT_DIR,
};

/// Read an environment variable, falling back to a compile-time default.
///
/// Reduces the `std::env::var(X).unwrap_or_else(|_| DEFAULT.into())` pattern
/// that appears 50+ times across the codebase.
#[must_use]
pub fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.into())
}

/// Resolve the systemd unit directory, honoring env overrides and init scope.
///
/// Priority: `MEMBRANE_SYSTEMD_UNIT_DIR` env -> scope-based default.
/// For user-space deploy (`MEMBRANE_INIT_SCOPE=bare`), this returns
/// `$HOME/.config/systemd/user` so unit files land in the user session.
#[must_use]
pub fn resolve_systemd_unit_dir() -> String {
    if let Ok(dir) = std::env::var(ENV_SYSTEMD_UNIT_DIR) {
        return dir;
    }

    if let Ok(scope) = std::env::var(ENV_INIT_SCOPE)
        && (scope == "bare" || scope == "user")
        && let Ok(home) = std::env::var(ENV_HOME)
    {
        return format!("{home}/{SYSTEMD_USER_UNIT_DIR}");
    }

    SYSTEMD_UNIT_DIR.into()
}

/// Resolve the socket base directory, honoring env overrides and init scope.
///
/// Priority: `MEMBRANE_SOCKET_BASE` env -> `BIOMEOS_SOCKET_DIR` env ->
/// scope-adapted default. For user-space deploy (`MEMBRANE_INIT_SCOPE`
/// = `user` or `bare`), defaults to `$XDG_RUNTIME_DIR/biomeos` to match
/// the user systemd unit template (`%t/biomeos/%i.sock`).
#[must_use]
pub fn resolve_socket_base() -> String {
    if let Ok(base) = std::env::var(ENV_SOCKET_BASE) {
        return base;
    }
    if let Ok(dir) = std::env::var(ENV_BIOMEOS_SOCKET_DIR) {
        return dir;
    }
    if let Ok(scope) = std::env::var(ENV_INIT_SCOPE)
        && (scope == "user" || scope == "bare")
    {
        let xdg = resolve_xdg_runtime_dir();
        return format!("{xdg}/{NEURAL_API_NAMESPACE}");
    }
    DEFAULT_SOCKET_BASE.into()
}

/// Best-effort UID resolution for socket path construction.
fn resolve_uid_best_effort() -> String {
    std::env::var(ENV_UID)
        .or_else(|_| std::env::var(ENV_EUID))
        .unwrap_or_else(|_| "1000".into())
}

/// Resolve `XDG_RUNTIME_DIR`, falling back to `/run/user/{uid}`.
///
/// Checks `XDG_RUNTIME_DIR` env first, then constructs the standard
/// path from the best-effort UID. This is the single source of truth
/// for XDG runtime directory resolution across all crates.
#[must_use]
pub fn resolve_xdg_runtime_dir() -> String {
    std::env::var(ENV_XDG_RUNTIME_DIR)
        .unwrap_or_else(|_| format!("/run/user/{}", resolve_uid_best_effort()))
}

/// Resolve the gate name from environment variables.
///
/// Checks `MEMBRANE_GATE_NAME` first (standard prefix), then falls back to
/// the legacy `GATE_NAME` for backward compatibility with existing
/// `/etc/environment` configurations.
///
/// Returns `None` if neither is set or both are empty.
#[must_use]
pub fn resolve_gate_name_env() -> Option<String> {
    resolve_env_with_legacy(ENV_GATE_NAME, ENV_GATE_NAME_LEGACY)
}

/// Resolve the webhook secret from environment variables.
///
/// Checks `MEMBRANE_WEBHOOK_SECRET` first, then falls back to legacy
/// `WEBHOOK_SECRET` for backward compatibility.
///
/// Returns `None` if neither is set or both are empty.
#[must_use]
pub fn resolve_webhook_secret_env() -> Option<String> {
    resolve_env_with_legacy(ENV_WEBHOOK_SECRET, ENV_WEBHOOK_SECRET_LEGACY)
}

/// Resolve the family seed from environment variables.
///
/// Checks `MEMBRANE_FAMILY_SEED` first, then legacy `BEARDOG_FAMILY_SEED`
/// and `FAMILY_SEED`. The value may be an inline seed or a path to a key file.
///
/// Returns `None` if no seed is configured.
#[must_use]
pub fn resolve_family_seed_env() -> Option<String> {
    resolve_env_chain(&[
        ENV_FAMILY_SEED,
        ENV_FAMILY_SEED_LEGACY,
        ENV_FAMILY_SEED_LEGACY2,
    ])
}

/// Resolve an environment variable with a legacy fallback.
fn resolve_env_with_legacy(primary: &str, legacy: &str) -> Option<String> {
    resolve_env_chain(&[primary, legacy])
}

/// Check an ordered list of env var names, returning the first non-empty value.
fn resolve_env_chain(keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Ok(val) = std::env::var(key) {
            let trimmed = val.trim().to_string();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
    }
    None
}
