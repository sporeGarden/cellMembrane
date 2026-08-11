// SPDX-License-Identifier: AGPL-3.0-or-later

//! `builder.serve` — long-lived JSON-RPC build service for mesh-native dispatch.
//!
//! Replaces SSH-based sub-builder dispatch with a JSON-RPC listener that
//! accepts `plasmid.harvest`, `plasmid.staleness`, and other build commands
//! over the songBird mesh relay (Tower Atomic transport).
//!
//! On startup, registers the `build` capability with the local songBird mesh
//! so remote gates can discover this builder via `capability.resolve` or
//! `relay.forward`.

use crate::ShadowOutcome;
use crate::cli;
use crate::error::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tracing::{info, warn};

const DEFAULT_BUILDER_PORT: u16 = cellmembrane_types::service::DEFAULT_BUILDER_PORT;
const DEFAULT_BUILDER_BIND: &str = cellmembrane_types::service::BIND_ALL;

/// Start the builder service: bind TCP, register with mesh, accept JSON-RPC.
pub(super) async fn serve(args: &[&str]) -> Result<ShadowOutcome> {
    let port: u16 = cli::extract_flag_value(args, "--port")
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_BUILDER_PORT);
    let bind = cli::extract_flag_value(args, "--bind").unwrap_or(DEFAULT_BUILDER_BIND);
    let addr = format!("{bind}:{port}");

    let listener = TcpListener::bind(&addr).await.map_err(|e| {
        crate::error::ShadowError::config(format!("builder.serve: cannot bind {addr} — {e}"))
    })?;

    let gate = crate::gate::resolve_local_gate_identity();
    info!(gate, addr, "builder service starting");

    register_build_capability(&gate).await;

    info!(addr, "builder service listening for JSON-RPC");

    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                warn!(error = %e, "accept failed");
                continue;
            }
        };

        let gate = gate.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, &gate, peer).await {
                warn!(peer = %peer, error = %e, "connection handler error");
            }
        });
    }
}

/// Handle a single JSON-RPC connection (one request per line, newline-delimited).
async fn handle_connection(
    stream: tokio::net::TcpStream,
    gate: &str,
    peer: std::net::SocketAddr,
) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();

    while let Some(line) = lines.next_line().await? {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        let response = dispatch_jsonrpc(&line, gate).await;

        writer.write_all(response.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;
    }

    info!(peer = %peer, "builder connection closed");
    Ok(())
}

/// Parse a JSON-RPC request and route to the appropriate plasmid command.
async fn dispatch_jsonrpc(raw: &str, gate: &str) -> String {
    let parsed: serde_json::Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(e) => {
            return serde_json::json!({
                "jsonrpc": "2.0",
                "error": { "code": -32700, "message": format!("parse error: {e}") },
                "id": null,
            })
            .to_string();
        }
    };

    let id = parsed.get("id").cloned().unwrap_or(serde_json::Value::Null);
    let method = parsed
        .get("method")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let params = parsed.get("params").cloned();

    info!(method, gate, "builder dispatch");

    let outcome = match method {
        "plasmid.harvest" => dispatch_harvest(params.as_ref()).await,
        "plasmid.staleness" => {
            let config = crate::ShadowConfig::from_env().await;
            super::plasmid_dispatch::dispatch_plasmid(&config, "plasmid.staleness", &[]).await
        }
        "plasmid.build" => {
            let config = crate::ShadowConfig::from_env().await;
            let primal = params
                .as_ref()
                .and_then(|p| p.get("primal"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            super::plasmid_dispatch::dispatch_plasmid(
                &config,
                "plasmid.build",
                &["--primal", primal],
            )
            .await
        }
        "health" | "health.liveness" => Ok(ShadowOutcome::ok(format!("builder OK ({gate})"))),
        _ => Ok(ShadowOutcome::fail(format!(
            "unknown builder method: {method}"
        ))),
    };

    match outcome {
        Ok(o) => serde_json::json!({
            "jsonrpc": "2.0",
            "result": {
                "ok": o.ok,
                "message": o.message,
                "data": o.data,
            },
            "id": id,
        })
        .to_string(),
        Err(e) => serde_json::json!({
            "jsonrpc": "2.0",
            "error": { "code": -32603, "message": e.to_string() },
            "id": id,
        })
        .to_string(),
    }
}

/// Extract harvest args from JSON-RPC params and dispatch.
async fn dispatch_harvest(params: Option<&serde_json::Value>) -> Result<ShadowOutcome> {
    let mut args: Vec<&str> = Vec::new();
    let primal_str;
    let target_str;

    if let Some(p) = params {
        if let Some(primal) = p.get("primal").and_then(serde_json::Value::as_str) {
            primal_str = primal.to_string();
            args.extend_from_slice(&["--primal", &primal_str]);
        } else {
            args.push("--all");
        }
        if p.get("force")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            args.push("--force");
        }
        if p.get("push")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            args.push("--push");
        }
        if p.get("local")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true)
        {
            args.push("--local");
        }
        if let Some(target) = p.get("target").and_then(serde_json::Value::as_str) {
            target_str = target.to_string();
            args.extend_from_slice(&["--target", &target_str]);
        }
    } else {
        args.push("--all");
        args.push("--local");
    }

    let config = crate::ShadowConfig::from_env().await;
    super::plasmid_dispatch::dispatch_plasmid(&config, "plasmid.harvest", &args).await
}

/// Register the `build` capability with the local songBird mesh.
async fn register_build_capability(gate: &str) {
    let sockets = crate::gate::sockets::resolve_primal_socket_paths(
        cellmembrane_types::MembraneService::binary_for(
            cellmembrane_types::service::ServiceCapability::MeshRelay,
        ),
    );

    let relay_socket = sockets
        .into_iter()
        .find(|p| std::path::Path::new(p).exists());

    let Some(socket_path) = relay_socket else {
        warn!("no relay socket found — builder will not be discoverable via mesh");
        return;
    };

    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "ipc.register",
        "params": {
            "primal_id": "membrane",
            "socket_path": format!("tcp://{}:{DEFAULT_BUILDER_PORT}", cellmembrane_types::service::BIND_ALL),
            "capabilities": ["build", "plasmid_harvest", "plasmid_build"],
            "gate": gate,
        },
        "id": 1,
    })
    .to_string();

    match crate::jsonrpc::call(std::path::Path::new(&socket_path), &request).await {
        Ok(resp) => info!(response = %resp, "registered build capability with relay mesh"),
        Err(e) => warn!(error = %e, "failed to register build capability with relay primal"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn dispatch_jsonrpc_health() {
        let response =
            dispatch_jsonrpc(r#"{"jsonrpc":"2.0","method":"health","id":1}"#, "testGate").await;
        let parsed: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert!(parsed["result"]["ok"].as_bool().unwrap());
        assert_eq!(parsed["id"], 1);
    }

    #[tokio::test]
    async fn dispatch_jsonrpc_unknown_method() {
        let response =
            dispatch_jsonrpc(r#"{"jsonrpc":"2.0","method":"foo.bar","id":2}"#, "testGate").await;
        let parsed: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert!(!parsed["result"]["ok"].as_bool().unwrap());
        assert!(
            parsed["result"]["message"]
                .as_str()
                .unwrap()
                .contains("unknown")
        );
    }

    #[tokio::test]
    async fn dispatch_jsonrpc_parse_error() {
        let response = dispatch_jsonrpc("not json", "testGate").await;
        let parsed: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert!(parsed["error"].is_object());
        assert_eq!(parsed["error"]["code"], -32700);
    }
}
