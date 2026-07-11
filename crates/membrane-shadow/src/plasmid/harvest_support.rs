// SPDX-License-Identifier: AGPL-3.0-or-later

//! Harvest support — mesh notification and outcome formatting.

use crate::ShadowOutcome;
use tracing::{info, warn};

use super::harvest::{HarvestResult, HarvestStatus};

/// Notify the local songBird mesh that the depot was updated.
///
/// Sends `mesh.publish { topic: "depot.updated" }` via the local songBird
/// UDS socket. Peers receive this as `mesh.subscribe` and auto-fetch.
/// Failures are non-fatal — depot data is still consistent.
pub(super) async fn notify_mesh_depot_updated(built_primals: &[String]) {
    let socket_path = std::path::PathBuf::from(cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_SONGBIRD_SOCKET,
        cellmembrane_types::service::DEFAULT_SONGBIRD_SOCKET,
    ));

    if !socket_path.exists() {
        info!("mesh.publish skipped — songBird socket not found");
        return;
    }

    let gate = crate::gate::resolve_local_gate_identity();

    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "mesh.publish",
        "params": {
            "topic": "depot.updated",
            "payload": {
                "primals_updated": built_primals,
                "builder": gate,
            }
        },
        "id": 1
    });

    let request_str = request.to_string();
    match crate::jsonrpc::call(&socket_path, &request_str).await {
        Ok(response) => {
            info!(
                primals = ?built_primals,
                "mesh.publish depot.updated sent: {response}"
            );
        }
        Err(e) => {
            warn!("mesh.publish depot.updated failed (non-fatal): {e}");
        }
    }
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
