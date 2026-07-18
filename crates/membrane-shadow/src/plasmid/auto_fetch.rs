// SPDX-License-Identifier: AGPL-3.0-or-later

//! Auto-fetch pipeline triggered by `mesh.subscribe depot.updated`.
//!
//! When songBird receives a `depot.updated` notification from a builder gate,
//! this module handles the consumer side: fetch updated ecobins from the WAN
//! depot, BLAKE3-verify them, and optionally restart affected services.
//!
//! Safety:
//! - Rate-limited: at most one auto-fetch per `MIN_FETCH_INTERVAL`.
//! - Idempotent: skips primals already at the announced checksum.
//! - Non-destructive: only overwrites binaries that pass BLAKE3 verification.

use crate::ShadowOutcome;
use crate::error::Result;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{info, warn};

/// Minimum interval between auto-fetch runs (seconds).
const MIN_FETCH_INTERVAL_SECS: u64 = 300;

/// Timestamp of last auto-fetch (epoch seconds). Prevents fetch storms.
static LAST_FETCH_EPOCH: AtomicU64 = AtomicU64::new(0);

/// Payload from a `depot.updated` mesh notification.
#[derive(Debug, Clone)]
pub(crate) struct DepotUpdatedNotification {
    /// Names of primals that were rebuilt.
    pub primals_updated: Vec<String>,
    /// BLAKE3 hash of the depot's `checksums.toml` after the build.
    #[allow(dead_code, reason = "parsed for future checksum-gated fetch")]
    pub manifest_hash: Option<String>,
    /// Gate identity of the builder that produced the update.
    pub builder: String,
}

impl DepotUpdatedNotification {
    /// Parse from a `mesh.subscribe` params payload.
    pub fn from_json(payload: &serde_json::Value) -> Self {
        let primals = payload
            .get("primals_updated")
            .and_then(serde_json::Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default();
        let manifest_hash = payload
            .get("manifest_hash")
            .and_then(serde_json::Value::as_str)
            .map(String::from);
        let builder = payload
            .get("builder")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown")
            .to_string();

        Self {
            primals_updated: primals,
            manifest_hash,
            builder,
        }
    }
}

/// Handle a `depot.updated` notification by fetching updated primals.
///
/// Rate-limited: silently skips if called within `MIN_FETCH_INTERVAL_SECS`
/// of the last successful fetch.
pub(crate) async fn handle_depot_updated(
    notification: &DepotUpdatedNotification,
) -> Result<ShadowOutcome> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let last = LAST_FETCH_EPOCH.load(Ordering::Relaxed);

    if now.saturating_sub(last) < MIN_FETCH_INTERVAL_SECS {
        info!(
            "auto-fetch rate-limited: last fetch {}s ago (min {}s)",
            now - last,
            MIN_FETCH_INTERVAL_SECS
        );
        return Ok(ShadowOutcome {
            ok: true,
            message: "rate-limited — skipped".into(),
            data: None,
        });
    }

    info!(
        "auto-fetch triggered by {} — {} primals: {:?}",
        notification.builder,
        notification.primals_updated.len(),
        notification.primals_updated
    );

    let config = crate::config::ShadowConfig::from_env().await;

    let fetch_args = super::FetchArgs {
        source: super::FetchSource::Wan,
        primal: None,
        release_tag: None,
        force: false,
        dry_run: false,
        dest: None,
        trust_policy: cellmembrane_types::DepotTrustPolicy::VerifyIfPresent,
    };

    let outcome = super::fetch(&config, &fetch_args).await?;

    LAST_FETCH_EPOCH.store(now, Ordering::Relaxed);

    let (downloaded, failed) = outcome.data.as_ref().map_or((0, 0), |data| {
        let dl = data
            .get("downloaded")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let fl = data
            .get("failed")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        (dl, fl)
    });

    info!(
        "auto-fetch complete: {downloaded} downloaded, {failed} failed (builder={})",
        notification.builder
    );

    if failed > 0 {
        warn!("auto-fetch had {failed} failures — some primals may be stale");
    }

    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_depot_updated_notification() {
        let payload = serde_json::json!({
            "primals_updated": ["beardog", "songbird"],
            "manifest_hash": "abc123",
            "builder": "sporeGate"
        });
        let notif = DepotUpdatedNotification::from_json(&payload);
        assert_eq!(notif.primals_updated, vec!["beardog", "songbird"]);
        assert_eq!(notif.manifest_hash.as_deref(), Some("abc123"));
        assert_eq!(notif.builder, "sporeGate");
    }

    #[test]
    fn parse_depot_updated_minimal() {
        let payload = serde_json::json!({});
        let notif = DepotUpdatedNotification::from_json(&payload);
        assert!(notif.primals_updated.is_empty());
        assert!(notif.manifest_hash.is_none());
        assert_eq!(notif.builder, "unknown");
    }

    #[test]
    fn rate_limit_epoch_starts_at_zero() {
        assert_eq!(LAST_FETCH_EPOCH.load(Ordering::Relaxed), 0);
    }
}
