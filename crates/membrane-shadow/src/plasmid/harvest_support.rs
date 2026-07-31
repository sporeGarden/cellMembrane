// SPDX-License-Identifier: AGPL-3.0-or-later

//! Harvest support — mesh notification and outcome formatting.

use crate::ShadowOutcome;

use super::harvest::{HarvestResult, HarvestStatus};

/// Notify the local songBird mesh that the depot was updated.
pub(super) async fn notify_mesh_depot_updated(built_primals: &[String]) {
    super::notify_mesh("depot.updated", "primals_updated", built_primals).await;
}

pub(super) fn format_harvest_outcome(results: &[HarvestResult]) -> ShadowOutcome {
    let built = results
        .iter()
        .filter(|r| matches!(r.status, HarvestStatus::Built))
        .count();
    let current = results
        .iter()
        .filter(|r| matches!(r.status, HarvestStatus::Current))
        .count();
    let failed = results
        .iter()
        .filter(|r| matches!(r.status, HarvestStatus::Failed))
        .count();
    let skipped = results
        .iter()
        .filter(|r| matches!(r.status, HarvestStatus::Skipped))
        .count();

    let msg =
        format!("harvest: {built} built, {current} current, {skipped} skipped, {failed} failed");

    ShadowOutcome {
        ok: failed == 0,
        message: msg,
        data: serde_json::to_value(results).ok(),
    }
}
