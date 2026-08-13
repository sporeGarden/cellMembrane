// SPDX-License-Identifier: AGPL-3.0-or-later

//! Content domain dispatch — sporePrint static site build, integrity verification,
//! and provenance braiding (Phase 3 CAS).

use crate::{ShadowConfig, ShadowOutcome};
use tracing::{info, warn};

pub(super) async fn dispatch_content(
    config: &ShadowConfig,
    cmd: &str,
    args: &[&str],
) -> crate::Result<ShadowOutcome> {
    match cmd {
        "content.rebuild" => dispatch_content_rebuild(args).await,
        "content.verify" => dispatch_content_verify(config).await,
        "content.braid" => dispatch_content_braid(args).await,
        _ => Ok(ShadowOutcome::fail(format!(
            "unknown content command: {cmd}"
        ))),
    }
}

/// `content.rebuild` — run `zola build` in the sporePrint directory.
///
/// Intended to be chained after cascade on gates that serve the static site
/// (currently golgi). Finds sporePrint via workspace root + manifest path,
/// or falls back to `ECOPRIMALS_ROOT/sporePrint`.
async fn dispatch_content_rebuild(args: &[&str]) -> crate::Result<ShadowOutcome> {
    let site_dir = resolve_sporeprint_dir(args);

    if !site_dir.join("config.toml").exists() {
        return Ok(ShadowOutcome::fail(format!(
            "content.rebuild: no config.toml in {} — not a Zola site",
            site_dir.display()
        )));
    }

    info!(path = %site_dir.display(), "content.rebuild: running zola build");

    let result = tokio::process::Command::new("zola")
        .arg("build")
        .current_dir(&site_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await;

    match result {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let line_count = stdout.lines().count();
            info!(pages = line_count, "content.rebuild: zola build succeeded");
            Ok(ShadowOutcome::ok(format!(
                "content.rebuild: OK — zola build in {} ({line_count} output lines)",
                site_dir.display()
            )))
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let first_line = stderr.lines().next().unwrap_or("unknown error");
            warn!(error = %first_line, "content.rebuild: zola build failed");
            Ok(ShadowOutcome::fail(format!(
                "content.rebuild: FAIL — {first_line}"
            )))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ShadowOutcome::fail(
            "content.rebuild: zola binary not found — install with: cargo install zola",
        )),
        Err(e) => Ok(ShadowOutcome::fail(format!(
            "content.rebuild: execution error — {e}"
        ))),
    }
}

/// Resolve the sporePrint directory from args, env, or workspace.
///
/// Priority: `--path <dir>` flag > first positional arg > workspace/manifest > default.
fn resolve_sporeprint_dir(args: &[&str]) -> std::path::PathBuf {
    if let Some(path) = crate::cli::extract_flag_value(args, "--path") {
        return std::path::PathBuf::from(path);
    }

    if let Some(positional) = args.iter().find(|a| !a.starts_with('-')) {
        let p = std::path::PathBuf::from(positional);
        if p.exists() {
            return p;
        }
    }

    if let Ok(root) = crate::temporal::resolve_workspace_root() {
        let manifest_path = root.join(cellmembrane_types::service::SPOREPRINT_CONTENT_DIR);
        if manifest_path.exists() {
            return manifest_path;
        }
    }

    let eco_root = std::env::var(cellmembrane_types::service::ENV_ECOPRIMALS_ROOT)
        .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT.to_string());
    std::path::PathBuf::from(eco_root).join(cellmembrane_types::service::SPOREPRINT_CONTENT_DIR)
}

async fn dispatch_content_verify(config: &ShadowConfig) -> crate::Result<ShadowOutcome> {
    let (caddy_out, caddy_code) =
        crate::ssh::exec_raw(config, "systemctl is-active caddy-tls").await?;
    let caddy_active = caddy_code == 0;

    let content_binary = cellmembrane_types::MembraneService::binary_for(
        cellmembrane_types::ServiceCapability::ContentServing,
    );
    let content_unit = format!("{content_binary}-membrane");
    let (svc_out, svc_code) =
        crate::ssh::exec_raw(config, &format!("systemctl is-active {content_unit}")).await?;
    let svc_active = svc_code == 0;

    let content_path = std::env::var(cellmembrane_types::service::ENV_NESTGATE_CONTENT_PATH)
        .unwrap_or_else(|_| {
            let install_base = cellmembrane_types::service::env_or(
                cellmembrane_types::service::ENV_INSTALL_BASE,
                cellmembrane_types::service::DEFAULT_INSTALL_BASE,
            );
            format!("{install_base}/{content_binary}/content")
        });
    let (content_count_out, _) = crate::ssh::exec_raw(
        config,
        &format!("find {content_path} -type f 2>/dev/null | wc -l"),
    )
    .await?;
    let content_files: u32 = content_count_out.trim().parse().unwrap_or(0);

    let content_svc = cellmembrane_types::MembraneService::with_capability(
        cellmembrane_types::ServiceCapability::ContentServing,
    );
    let content_port = std::env::var(cellmembrane_types::service::ENV_NESTGATE_PORT)
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or_else(|| {
            content_svc
                .and_then(|s| s.port)
                .unwrap_or(cellmembrane_types::service::DEFAULT_NESTGATE_PORT)
        });
    let bind = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_NUCLEUS_BIND,
        cellmembrane_types::service::BIND_LOOPBACK,
    );
    let (curl_out, curl_code) = crate::ssh::exec_raw(
        config,
        &format!("curl -s -o /dev/null -w '%{{http_code}}' http://{bind}:{content_port}/health 2>/dev/null"),
    )
    .await?;
    let http_status = curl_out.trim().to_string();
    let http_ok = curl_code == 0 && http_status == "200";

    let status = if caddy_active && svc_active && http_ok {
        "READY"
    } else {
        "NOT READY"
    };

    let msg = format!(
        "=== S3 Content Verification ===\n\
         Status:         {status}\n\
         Caddy TLS:      {} ({})\n\
         {content_binary}:       {} ({})\n\
         {content_binary} HTTP:  {} ({bind}:{content_port}/health)\n\
         Content files:  {content_files}",
        if caddy_active { "active" } else { "inactive" },
        caddy_out.trim(),
        if svc_active { "active" } else { "inactive" },
        svc_out.trim(),
        if http_ok { "200 OK" } else { &http_status },
    );

    let ok = caddy_active && svc_active && http_ok;
    Ok(if ok {
        ShadowOutcome::ok_with(
            msg,
            serde_json::json!({
                "status": status,
                "caddy": caddy_active,
                "content_service": content_binary,
                "content_active": svc_active,
                "content_http": http_status,
                "content_files": content_files,
            }),
        )
    } else {
        ShadowOutcome {
            ok: false,
            message: msg,
            data: Some(serde_json::json!({
                "status": status,
                "caddy": caddy_active,
                "content_service": content_binary,
                "content_active": svc_active,
                "content_http": http_status,
                "content_files": content_files,
            })),
        }
    })
}

// ═══════════════════════════════════════════════════════════════════════════════
// content.braid — Rust-native provenance braiding (replaces native_braid.py)
// ═══════════════════════════════════════════════════════════════════════════════

const DATA_ROOT_DEFAULT: &str = "/mnt/nestgate/cold/zfs/data";
const COMMITTER_DID: &str = "did:eco:westgate";
const BATCH_SIZE: usize = 500;
const BRAIDED_MARKER: &str = ".braided";

/// `content.braid <path> [--only ds1,ds2] [--skip ds3] [--dry-run] [--incremental]`
///
/// Pipeline per dataset chunk:
///   1. content.ingest(directory) → manifest {filename: blake3}
///   2. dag.session.create → session_id
///   3. dag.event.append_batch (BATCH_SIZE events per call)
///   4. dag.dehydration.trigger → merkle_root
///   5. session.commit (spine_id, session_id, merkle_root)
///   6. braid.create (dataset-level provenance braid)
///   7. sign (bearDog composite signature on final hash)
async fn dispatch_content_braid(args: &[&str]) -> crate::Result<ShadowOutcome> {
    let bridge = match crate::bridge::NeuralBridge::discover() {
        Some(b) => b,
        None => {
            return Ok(ShadowOutcome::fail(
                "content.braid: biomeOS Neural API not reachable — braiding requires live primals",
            ))
        }
    };

    let data_root = crate::cli::extract_flag_value(args, "--path")
        .or_else(|| args.iter().find(|a| !a.starts_with('-')).copied())
        .unwrap_or(DATA_ROOT_DEFAULT);

    let only: Vec<&str> = crate::cli::extract_flag_value(args, "--only")
        .map(|v| v.split(',').collect())
        .unwrap_or_default();

    let skip: Vec<&str> = crate::cli::extract_flag_value(args, "--skip")
        .map(|v| v.split(',').collect())
        .unwrap_or_default();

    let dry_run = args.iter().any(|a| *a == "--dry-run");
    let incremental = args.iter().any(|a| *a == "--incremental");

    let root = std::path::Path::new(data_root);
    if !root.is_dir() {
        return Ok(ShadowOutcome::fail(format!(
            "content.braid: data root not found: {data_root}"
        )));
    }

    let datasets = list_datasets(root, &only, &skip, incremental);
    if datasets.is_empty() {
        return Ok(ShadowOutcome::ok(format!(
            "content.braid: no datasets to process in {data_root}"
        )));
    }

    info!(
        "content.braid: {} datasets queued from {}{}",
        datasets.len(),
        data_root,
        if dry_run { " (dry-run)" } else { "" }
    );

    if dry_run {
        let names: Vec<String> = datasets.iter().map(|d| d.name.clone()).collect();
        return Ok(ShadowOutcome::ok_with(
            format!("content.braid: dry-run — {} datasets", names.len()),
            serde_json::json!({ "datasets": names, "dry_run": true }),
        ));
    }

    let mut results = Vec::new();
    let mut total_files = 0u64;
    let mut total_bytes = 0u64;

    for dataset in &datasets {
        info!("content.braid: processing {}", dataset.name);

        match braid_dataset(&bridge, dataset).await {
            Ok(result) => {
                total_files += result.files;
                total_bytes += result.bytes;
                info!(
                    "content.braid: {} — {} files, {} bytes, merkle: {}",
                    dataset.name,
                    result.files,
                    result.bytes,
                    result.merkle_root.as_deref().unwrap_or("none"),
                );
                results.push(serde_json::json!({
                    "dataset": dataset.name,
                    "status": "complete",
                    "files": result.files,
                    "bytes": result.bytes,
                    "merkle_root": result.merkle_root,
                    "braid_hash": result.braid_hash,
                }));
            }
            Err(e) => {
                warn!("content.braid: {} failed: {}", dataset.name, e);
                results.push(serde_json::json!({
                    "dataset": dataset.name,
                    "status": "failed",
                    "error": e.to_string(),
                }));
            }
        }
    }

    let summary = format!(
        "content.braid: {} datasets processed — {} files, {} bytes total",
        datasets.len(),
        total_files,
        total_bytes,
    );

    Ok(ShadowOutcome::ok_with(
        summary,
        serde_json::json!({
            "datasets": results,
            "total_files": total_files,
            "total_bytes": total_bytes,
        }),
    ))
}

struct DatasetEntry {
    name: String,
    path: std::path::PathBuf,
}

struct BraidResult {
    files: u64,
    bytes: u64,
    merkle_root: Option<String>,
    braid_hash: Option<String>,
}

fn list_datasets(
    root: &std::path::Path,
    only: &[&str],
    skip: &[&str],
    incremental: bool,
) -> Vec<DatasetEntry> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return vec![];
    };

    let mut datasets = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }

        if !only.is_empty() && !only.contains(&name.as_str()) {
            continue;
        }
        if skip.contains(&name.as_str()) {
            continue;
        }

        let braided_marker = path.join(BRAIDED_MARKER);
        if braided_marker.exists() && !incremental {
            continue;
        }

        datasets.push(DatasetEntry { name, path });
    }

    datasets.sort_by(|a, b| a.name.cmp(&b.name));
    datasets
}

async fn braid_dataset(
    bridge: &crate::bridge::NeuralBridge,
    dataset: &DatasetEntry,
) -> std::result::Result<BraidResult, Box<dyn std::error::Error + Send + Sync>> {
    // 1. Ingest dataset into CAS via nestGate
    let ingest_result = bridge
        .capability_call(
            "content",
            "ingest",
            serde_json::json!({
                "path": dataset.path.to_string_lossy(),
                "recursive": true,
            }),
        )
        .await;

    let manifest = match ingest_result {
        crate::bridge::BridgeResult::Handled(v) => v,
        crate::bridge::BridgeResult::ApiError(e) => {
            return Err(format!("content.ingest failed: {e}").into())
        }
        crate::bridge::BridgeResult::Fallthrough => {
            return Err("content.ingest: Neural API unreachable".into())
        }
    };

    let file_count = manifest
        .get("file_count")
        .or_else(|| manifest.get("files"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let byte_count = manifest
        .get("total_bytes")
        .or_else(|| manifest.get("bytes"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    // 2. Create DAG session via rhizoCrypt
    let session_result = bridge
        .capability_call(
            "dag",
            "session.create",
            serde_json::json!({
                "name": dataset.name,
                "committer": COMMITTER_DID,
            }),
        )
        .await;

    let session = match session_result {
        crate::bridge::BridgeResult::Handled(v) => v,
        crate::bridge::BridgeResult::ApiError(e) => {
            return Err(format!("dag.session.create failed: {e}").into())
        }
        crate::bridge::BridgeResult::Fallthrough => {
            return Err("dag.session.create: Neural API unreachable".into())
        }
    };

    let session_id = session
        .get("session_id")
        .or_else(|| session.get("id"))
        .and_then(|v| v.as_str())
        .ok_or("dag.session.create: no session_id in response")?
        .to_string();

    // 3. Build DAG events from manifest and append in batches
    let entries = manifest
        .get("manifest")
        .or_else(|| manifest.get("entries"))
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    let events: Vec<serde_json::Value> = entries
        .iter()
        .map(|(filename, hash)| {
            serde_json::json!({
                "type": "DataCreate",
                "filename": filename,
                "blake3": hash,
                "dataset": dataset.name,
            })
        })
        .collect();

    for chunk in events.chunks(BATCH_SIZE) {
        let append_result = bridge
            .capability_call(
                "dag",
                "event.append_batch",
                serde_json::json!({
                    "session_id": session_id,
                    "events": chunk,
                }),
            )
            .await;

        match append_result {
            crate::bridge::BridgeResult::Handled(_) => {}
            crate::bridge::BridgeResult::ApiError(e) => {
                warn!("dag.event.append_batch: {e} — trying individual append");
                for event in chunk {
                    let _ = bridge
                        .capability_call(
                            "dag",
                            "event.append",
                            serde_json::json!({
                                "session_id": session_id,
                                "event": event,
                            }),
                        )
                        .await;
                }
            }
            crate::bridge::BridgeResult::Fallthrough => {
                return Err("dag.event.append_batch: Neural API unreachable".into())
            }
        }
    }

    // 4. Trigger dehydration → merkle root
    let dehydrate_result = bridge
        .capability_call(
            "dag",
            "dehydration.trigger",
            serde_json::json!({ "session_id": session_id }),
        )
        .await;

    let merkle_root = match dehydrate_result {
        crate::bridge::BridgeResult::Handled(v) => v
            .get("merkle_root")
            .and_then(|m| m.as_str())
            .map(String::from),
        _ => None,
    };

    // 5. Create spine + commit session via loamSpine
    let spine_result = bridge
        .capability_call(
            "spine",
            "create",
            serde_json::json!({
                "name": dataset.name,
                "committer": COMMITTER_DID,
            }),
        )
        .await;

    let spine_id = match spine_result {
        crate::bridge::BridgeResult::Handled(v) => v
            .get("spine_id")
            .or_else(|| v.get("id"))
            .and_then(|s| s.as_str())
            .map(String::from),
        _ => None,
    };

    if let Some(ref sid) = spine_id {
        let _ = bridge
            .capability_call(
                "session",
                "commit",
                serde_json::json!({
                    "spine_id": sid,
                    "session_id": session_id,
                    "merkle_root": merkle_root,
                    "vertex_count": events.len(),
                    "committer": COMMITTER_DID,
                }),
            )
            .await;
    }

    // 6. Sign composite hash via bearDog
    let composite = merkle_root.as_deref().unwrap_or(&session_id);
    let sign_result = bridge
        .capability_call(
            "crypto",
            "sign",
            serde_json::json!({ "message": composite }),
        )
        .await;

    let signature = match sign_result {
        crate::bridge::BridgeResult::Handled(v) => {
            v.get("signature").and_then(|s| s.as_str()).map(String::from)
        }
        _ => None,
    };

    // 7. Create provenance braid via sweetGrass
    let braid_result = bridge
        .capability_call(
            "braid",
            "create",
            serde_json::json!({
                "data_hash": composite,
                "strand_id": dataset.name,
                "metadata": {
                    "dataset": dataset.name,
                    "files": file_count,
                    "bytes": byte_count,
                    "committer": COMMITTER_DID,
                    "signature": signature,
                    "spine_id": spine_id,
                },
            }),
        )
        .await;

    let braid_hash = match braid_result {
        crate::bridge::BridgeResult::Handled(v) => v
            .get("braid_hash")
            .or_else(|| v.get("hash"))
            .and_then(|h| h.as_str())
            .map(String::from),
        _ => None,
    };

    // 8. Write .braided marker
    let marker = dataset.path.join(BRAIDED_MARKER);
    let marker_data = serde_json::json!({
        "dataset": dataset.name,
        "merkle_root": merkle_root,
        "braid_hash": braid_hash,
        "signature": signature,
        "spine_id": spine_id,
        "files": file_count,
        "bytes": byte_count,
        "committer": COMMITTER_DID,
        "braided_at": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        "braider": "membrane content.braid v1.0",
    });

    if let Err(e) = std::fs::write(&marker, serde_json::to_string_pretty(&marker_data).unwrap_or_default()) {
        warn!("content.braid: failed to write marker {}: {e}", marker.display());
    }

    Ok(BraidResult {
        files: file_count,
        bytes: byte_count,
        merkle_root,
        braid_hash,
    })
}
