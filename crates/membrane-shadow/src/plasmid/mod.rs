// SPDX-License-Identifier: AGPL-3.0-or-later

//! Plasmid binary lifecycle — fetch, refresh, and deploy primal binaries.
//!
//! Manages the binary supply chain for membrane services:
//! - `fetch` — Download binaries from sovereign or external sources (GitHub, VPS, Forgejo)
//! - `refresh` — Push local pre-built binaries to VPS with atomic replacement
//!
//! BLAKE3 checksums are verified in-process using the `blake3` crate.

mod fetch;
mod harvest;
mod refresh;

pub use fetch::*;
pub use harvest::{HarvestArgs, HarvestResult, HarvestStatus, harvest};
pub use refresh::{RefreshArgs, RefreshResult, RefreshStatus, refresh};

use std::path::PathBuf;

/// Primal binary names derived from the service registry at compile time.
///
/// Previously a hand-maintained `const` list — now sourced directly from
/// `cellmembrane-types::MembraneService::all()` so additions/removals to the
/// registry propagate automatically with zero manual sync.
pub(crate) fn nucleus_primals() -> Vec<&'static str> {
    cellmembrane_types::MembraneService::all()
        .iter()
        .filter(|s| s.is_primal)
        .map(|s| s.binary)
        .collect()
}

/// Detect the local platform's Rust target triple.
pub(crate) fn detect_target_triple() -> String {
    let arch = if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "unknown"
    };
    format!("{arch}-unknown-linux-musl")
}

/// Resolve a path with priority: explicit override → env var → computed default.
pub(crate) fn resolve_path(
    explicit: Option<&str>,
    env_var: &str,
    default_fn: impl FnOnce() -> PathBuf,
) -> PathBuf {
    if let Some(dir) = explicit {
        return PathBuf::from(dir);
    }
    if let Ok(val) = std::env::var(env_var) {
        return PathBuf::from(val);
    }
    default_fn()
}

/// `plasmid.pipeline` — Full zero-touch harvest → refresh cycle.
///
/// Detects upstream changes, rebuilds, checksums, pushes to VPS,
/// and reports aggregated outcome. This is the end-to-end command
/// that replaces manual harvest+refresh cycles.
pub async fn pipeline(
    config: &crate::ShadowConfig,
    primal: Option<&str>,
    dry_run: bool,
) -> crate::error::Result<crate::ShadowOutcome> {
    let harvest_args = HarvestArgs {
        primal: primal.map(Into::into),
        force: false,
        dry_run,
        depot_dir: None,
    };

    let harvest_outcome = harvest(&harvest_args).await?;

    if dry_run || !harvest_outcome.ok {
        return Ok(harvest_outcome);
    }

    let built_any = harvest_outcome
        .data
        .as_ref()
        .and_then(|d| d.as_array())
        .is_some_and(|arr| {
            arr.iter().any(|r| {
                r.get("status")
                    .and_then(|s| s.as_str())
                    .is_some_and(|s| s == "Built")
            })
        });

    if !built_any {
        return Ok(crate::ShadowOutcome {
            ok: true,
            message: format!("{} — no new binaries to push", harvest_outcome.message),
            data: harvest_outcome.data,
        });
    }

    let refresh_args = RefreshArgs {
        primal: primal.map(Into::into),
        dry_run: false,
        source_dir: None,
    };

    let refresh_outcome = refresh(config, &refresh_args).await?;

    Ok(crate::ShadowOutcome {
        ok: harvest_outcome.ok && refresh_outcome.ok,
        message: format!("{} | {}", harvest_outcome.message, refresh_outcome.message),
        data: refresh_outcome.data,
    })
}
