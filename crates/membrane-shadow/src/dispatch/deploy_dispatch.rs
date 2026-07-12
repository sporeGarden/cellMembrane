// SPDX-License-Identifier: AGPL-3.0-or-later

//! Deploy and lifecycle dispatch — Neural API front-end commands.
//!
//! These commands route through the Neural API (`capability.call`) with no
//! shadow fallback. They require biomeOS to be running on the target gate
//! (or reachable via mesh). This is the canonical deployment authority path:
//! `membrane deploy.*` / `membrane lifecycle.*` replace ad-hoc manual patterns.
//!
//! ## Commands
//!
//! - `deploy.composition <gate> <tier> [--dry-run]`
//! - `deploy.graph <gate> <graph_id> [--dry-run] [--param key=value ...]`
//! - `deploy.resurrect <primal> [--gate <gate>]`
//! - `lifecycle.status [--gate <gate>] [--primal <primal>]`

use crate::bridge;
use crate::cli;
use crate::ShadowOutcome;

const NEURAL_API_REQUIRED: &str =
    "Neural API unavailable — deploy commands require biomeOS on the target gate";

pub(super) async fn dispatch_deploy(
    cmd: &str,
    args: &[&str],
) -> crate::Result<ShadowOutcome> {
    match cmd {
        "deploy.composition" => deploy_composition(args).await,
        "deploy.graph" => deploy_graph(args).await,
        "deploy.resurrect" => deploy_resurrect(args).await,
        _ => Ok(ShadowOutcome::fail(format!("unknown deploy command: {cmd}"))),
    }
}

pub(super) async fn dispatch_lifecycle(
    cmd: &str,
    args: &[&str],
) -> crate::Result<ShadowOutcome> {
    match cmd {
        "lifecycle.status" => lifecycle_status(args).await,
        _ => Ok(ShadowOutcome::fail(format!("unknown lifecycle command: {cmd}"))),
    }
}

/// `deploy.composition <gate> <tier> [--dry-run]`
///
/// Deploys a composition tier to a gate via Neural API. The target gate's
/// `biomeOS` `LifecycleManager` handles dependency-aware startup, health
/// aggregation, and rollback on failure.
async fn deploy_composition(args: &[&str]) -> crate::Result<ShadowOutcome> {
    let gate = cli::require_arg(args, 0, "gate name")?;
    let tier = cli::require_arg(args, 1, "composition tier")?;
    let dry_run = args.contains(&"--dry-run");

    let params = serde_json::json!({
        "gate": gate,
        "tier": tier,
        "dry_run": dry_run,
    });

    route_to_gate(gate, "composition", "deploy", params)
        .await
        .map_or_else(
            || Ok(ShadowOutcome::fail(format!("{NEURAL_API_REQUIRED} ({gate})"))),
            |value| Ok(format_deploy_result("deploy.composition", gate, &value)),
        )
}

/// `deploy.graph <gate> <graph_id> [--dry-run] [--param key=value ...]`
///
/// Executes a deployment graph on the target gate. Graphs define multi-step
/// deployment sequences with rollback semantics.
async fn deploy_graph(args: &[&str]) -> crate::Result<ShadowOutcome> {
    let gate = cli::require_arg(args, 0, "gate name")?;
    let graph_id = cli::require_arg(args, 1, "graph ID or path")?;
    let dry_run = args.contains(&"--dry-run");

    let graph_params = extract_key_value_params(args);

    let params = serde_json::json!({
        "gate": gate,
        "graph_id": graph_id,
        "dry_run": dry_run,
        "params": graph_params,
    });

    route_to_gate(gate, "graph", "execute", params)
        .await
        .map_or_else(
            || Ok(ShadowOutcome::fail(format!("{NEURAL_API_REQUIRED} ({gate})"))),
            |value| Ok(format_deploy_result("deploy.graph", gate, &value)),
        )
}

/// `deploy.resurrect <primal> [--gate <gate>]`
///
/// Resurrects a failed primal via `LifecycleManager`. If `--gate` is omitted,
/// targets the local gate.
async fn deploy_resurrect(args: &[&str]) -> crate::Result<ShadowOutcome> {
    let primal = cli::require_arg(args, 0, "primal name")?;
    let default_gate = local_gate_name();
    let gate = cli::extract_flag_value(args, "--gate").unwrap_or(default_gate);

    let params = serde_json::json!({
        "primal": primal,
        "gate": gate,
    });

    route_to_gate(gate, "lifecycle", "resurrect", params)
        .await
        .map_or_else(
            || Ok(ShadowOutcome::fail(format!("{NEURAL_API_REQUIRED} ({gate})"))),
            |value| {
                let status = value
                    .get("status")
                    .and_then(|s| s.as_str())
                    .unwrap_or("unknown");
                Ok(ShadowOutcome::ok_with(
                    format!("{primal} on {gate}: {status}"),
                    value,
                ))
            },
        )
}

/// `lifecycle.status [--gate <gate>] [--primal <primal>]`
///
/// Queries lifecycle status for all primals on a gate, or a specific primal.
/// Returns composition health, individual primal states, and dependency graph.
async fn lifecycle_status(args: &[&str]) -> crate::Result<ShadowOutcome> {
    let default_gate = local_gate_name();
    let gate = cli::extract_flag_value(args, "--gate").unwrap_or(default_gate);
    let primal_filter = cli::extract_flag_value(args, "--primal");

    let mut params = serde_json::json!({ "gate": gate });
    if let Some(primal) = primal_filter {
        params["primal"] = serde_json::Value::String(primal.to_owned());
    }

    route_to_gate(gate, "lifecycle", "status", params)
        .await
        .map_or_else(
            || Ok(ShadowOutcome::fail(format!("{NEURAL_API_REQUIRED} ({gate})"))),
            |value| Ok(format_lifecycle_status(gate, primal_filter, &value)),
        )
}

// ── Routing ─────────────────────────────────────────────────────────

/// Route a capability call to a specific gate's Neural API.
///
/// Resolution order:
///   1. If gate is local → local `NeuralBridge`
///   2. If gate is remote → resolve endpoint via manifest + `call_endpoint`
///   3. Fallback → `try_bridge` (auto-discovers any available Neural API)
async fn route_to_gate(
    target_gate: &str,
    domain: &str,
    method: &str,
    params: serde_json::Value,
) -> Option<serde_json::Value> {
    let ctx = crate::resolve::ResolutionContext::from_env();

    if crate::resolve::is_local_gate(&ctx, target_gate) {
        return bridge::try_bridge(domain, method, params).await;
    }

    let ep = crate::resolve::resolve_endpoint(
        &ctx,
        target_gate,
        cellmembrane_types::ServiceCapability::Identity,
    )?;

    let request = crate::jsonrpc::request_with_params(
        "capability.call",
        &serde_json::json!({
            "capability": domain,
            "method": method,
            "params": params,
        }),
        1,
    );

    let response = crate::jsonrpc::call_endpoint(&ep, &request).await.ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&response).ok()?;

    if parsed.get("error").is_some() {
        tracing::debug!(
            gate = target_gate,
            domain,
            method,
            "remote capability.call returned error"
        );
        return None;
    }

    parsed.get("result").cloned()
}

fn local_gate_name() -> &'static str {
    static GATE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    GATE.get_or_init(|| {
        std::env::var("GATE_NAME").unwrap_or_else(|_| {
            crate::gate::resolve_local_gate_identity()
        })
    })
}

// ── Formatting ──────────────────────────────────────────────────────

fn format_deploy_result(
    cmd: &str,
    gate: &str,
    value: &serde_json::Value,
) -> ShadowOutcome {
    let status = value
        .get("status")
        .and_then(|s| s.as_str())
        .unwrap_or("completed");
    let detail = value
        .get("detail")
        .and_then(|d| d.as_str())
        .unwrap_or("");

    let mut lines = vec![format!("{cmd} → {gate}: {status}")];
    if !detail.is_empty() {
        lines.push(format!("  {detail}"));
    }

    if let Some(steps) = value.get("steps").and_then(|s| s.as_array()) {
        for step in steps {
            let name = step.get("name").and_then(|n| n.as_str()).unwrap_or("?");
            let step_status = step.get("status").and_then(|s| s.as_str()).unwrap_or("?");
            lines.push(format!("  {name}: {step_status}"));
        }
    }

    ShadowOutcome::ok_with(lines.join("\n"), value.clone())
}

fn format_lifecycle_status(
    gate: &str,
    primal_filter: Option<&str>,
    value: &serde_json::Value,
) -> ShadowOutcome {
    let mut lines = Vec::new();

    if let Some(filter) = primal_filter {
        lines.push(format!("=== {filter} on {gate} ==="));
    } else {
        lines.push(format!("=== Lifecycle Status: {gate} ==="));
    }

    if let Some(composition) = value.get("composition").and_then(|c| c.as_str()) {
        lines.push(format!("  composition: {composition}"));
    }

    if let Some(primals) = value.get("primals").and_then(|p| p.as_array()) {
        for p in primals {
            let name = p.get("name").and_then(|n| n.as_str()).unwrap_or("?");
            let state = p.get("state").and_then(|s| s.as_str()).unwrap_or("unknown");
            let pid = p
                .get("pid")
                .and_then(serde_json::Value::as_u64)
                .map_or(String::new(), |p| format!(" (pid {p})"));
            lines.push(format!("  {name}: {state}{pid}"));
        }
    }

    if let Some(health) = value.get("health").and_then(|h| h.as_str()) {
        lines.push(format!("  overall: {health}"));
    }

    ShadowOutcome::ok_with(lines.join("\n"), value.clone())
}

/// Extract `--param key=value` pairs from CLI args.
fn extract_key_value_params(args: &[&str]) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--param" {
            if let Some(kv) = args.get(i + 1) {
                if let Some((k, v)) = kv.split_once('=') {
                    map.insert(k.to_owned(), serde_json::Value::String(v.to_owned()));
                }
                i += 2;
                continue;
            }
        }
        i += 1;
    }
    serde_json::Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_unknown_deploy_returns_fail() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = rt.block_on(dispatch_deploy("deploy.unknown", &[])).unwrap();
        assert!(!result.ok, "unknown deploy command should fail");
    }

    #[test]
    fn dispatch_unknown_lifecycle_returns_fail() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = rt.block_on(dispatch_lifecycle("lifecycle.unknown", &[])).unwrap();
        assert!(!result.ok, "unknown lifecycle command should fail");
    }

    #[test]
    fn deploy_composition_requires_gate_and_tier() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let no_args = rt.block_on(deploy_composition(&[]));
        assert!(no_args.is_err(), "should require gate arg");

        let one_arg = rt.block_on(deploy_composition(&["eastGate"]));
        assert!(one_arg.is_err(), "should require tier arg");
    }

    #[test]
    fn deploy_graph_requires_gate_and_graph() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let no_args = rt.block_on(deploy_graph(&[]));
        assert!(no_args.is_err(), "should require gate arg");

        let one_arg = rt.block_on(deploy_graph(&["eastGate"]));
        assert!(one_arg.is_err(), "should require graph_id arg");
    }

    #[test]
    fn deploy_resurrect_requires_primal() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let no_args = rt.block_on(deploy_resurrect(&[]));
        assert!(no_args.is_err(), "should require primal arg");
    }

    #[test]
    fn lifecycle_status_defaults_to_local() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let result = rt.block_on(lifecycle_status(&[])).unwrap();
        assert!(
            !result.ok,
            "should fail gracefully without Neural API (no bridge)"
        );
        assert!(
            result.message.contains("Neural API"),
            "should mention Neural API requirement"
        );
    }

    #[test]
    fn extract_key_value_params_parses_pairs() {
        let args = ["--param", "SESSION_ID=abc", "--dry-run", "--param", "WAVE=137"];
        let params = extract_key_value_params(&args);
        assert_eq!(params.get("SESSION_ID").and_then(|v| v.as_str()), Some("abc"));
        assert_eq!(params.get("WAVE").and_then(|v| v.as_str()), Some("137"));
    }

    #[test]
    fn extract_key_value_params_empty_when_no_flags() {
        let args: [&str; 2] = ["eastGate", "nucleus"];
        let params = extract_key_value_params(&args);
        assert!(params.as_object().unwrap().is_empty());
    }

    #[test]
    fn format_deploy_result_basic() {
        let value = serde_json::json!({
            "status": "deployed",
            "detail": "4 primals started"
        });
        let outcome = format_deploy_result("deploy.composition", "eastGate", &value);
        assert!(outcome.ok);
        assert!(outcome.message.contains("deployed"));
        assert!(outcome.message.contains("eastGate"));
    }

    #[test]
    fn format_deploy_result_with_steps() {
        let value = serde_json::json!({
            "status": "completed",
            "steps": [
                {"name": "bearDog", "status": "started"},
                {"name": "songBird", "status": "started"}
            ]
        });
        let outcome = format_deploy_result("deploy.graph", "sporeGate", &value);
        assert!(outcome.message.contains("bearDog: started"));
        assert!(outcome.message.contains("songBird: started"));
    }

    #[test]
    fn format_lifecycle_status_all_primals() {
        let value = serde_json::json!({
            "composition": "nucleus",
            "health": "healthy",
            "primals": [
                {"name": "bearDog", "state": "active", "pid": 1234},
                {"name": "songBird", "state": "active", "pid": 5678}
            ]
        });
        let outcome = format_lifecycle_status("eastGate", None, &value);
        assert!(outcome.message.contains("Lifecycle Status: eastGate"));
        assert!(outcome.message.contains("bearDog: active (pid 1234)"));
        assert!(outcome.message.contains("overall: healthy"));
    }

    #[test]
    fn format_lifecycle_status_single_primal() {
        let value = serde_json::json!({
            "primals": [{"name": "bearDog", "state": "restarting"}]
        });
        let outcome = format_lifecycle_status("eastGate", Some("bearDog"), &value);
        assert!(outcome.message.contains("bearDog on eastGate"));
    }
}
