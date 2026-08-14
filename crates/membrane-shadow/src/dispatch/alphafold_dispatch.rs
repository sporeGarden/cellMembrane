// SPDX-License-Identifier: AGPL-3.0-or-later

//! AlphaFold Neural API ingestion pipeline.
//!
//! Three-phase ingestion of AlphaFold DB (~23 TB) through biomeOS Neural API,
//! braiding at ingress. Replaces the deprecated `alphafold_full_sync.sh` +
//! `native_braid.py` pipeline with primal-native composition.
//!
//! Phases:
//!   A — Proteome tars (on-disk, ~47 files, 1.5 TB) via `content.ingest`
//!   B — Expanded structures (on-disk, ~11M CIF files, 1.5 TB) via `content.ingest` per bucket
//!   C — Remote EBI download (~235M files, ~20 TB) via `content.fetch` per file
//!
//! Each phase uses the nest signal graph lifecycle:
//!   1. nest.declare_dataset (pre-braid intent)
//!   2. nest.acquire_file / content.ingest (per-file or per-dir acquisition)
//!   3. braid.partial_update (checkpoint braids)
//!   4. nest.complete_dataset (finalize: dehydrate → commit → sign → braid)

use crate::bridge::{BridgeResult, NeuralBridge};
use crate::ShadowOutcome;
use serde_json::json;
use std::time::Duration;
use tracing::{error, info, warn};

const ALPHAFOLD_DATA: &str = "/mnt/nestgate/cold/zfs/data/alphafold";
const ALPHAFOLD_STRUCTURES: &str = "/mnt/nestgate/cold/zfs/data/alphafold_structures";
const COMMITTER_DID: &str = "did:eco:westgate";
const FAMILY_ID: &str = "westgate-tower-155f";
const DEFAULT_BATCH_SIZE: usize = 500;
const DEFAULT_CHECKPOINT_INTERVAL: u64 = 50_000;
const DEFAULT_RATE_LIMIT_MBPS: u64 = 200;
const DEFAULT_CONCURRENCY: usize = 4;
const EBI_BASE_URL: &str = "https://alphafold.ebi.ac.uk/files";
const STATE_FILE: &str = ".alphafold_ingress_state.json";

// ═══════════════════════════════════════════════════════════════════════════════
// DISPATCH ENTRY
// ═══════════════════════════════════════════════════════════════════════════════

pub(super) async fn dispatch_alphafold(
    cmd: &str,
    args: &[&str],
) -> crate::Result<ShadowOutcome> {
    match cmd {
        "alphafold.ingest" => dispatch_ingest(args).await,
        "alphafold.status" => dispatch_status(args).await,
        "alphafold.manifest" => dispatch_manifest(args).await,
        _ => Ok(ShadowOutcome::fail(format!("unknown alphafold command: {cmd}"))),
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// INGEST — main orchestrator
// ═══════════════════════════════════════════════════════════════════════════════

async fn dispatch_ingest(args: &[&str]) -> crate::Result<ShadowOutcome> {
    let bridge = match NeuralBridge::discover() {
        Some(b) => b,
        None => {
            return Ok(ShadowOutcome::fail(
                "alphafold.ingest: biomeOS Neural API not reachable — ingestion requires live primals",
            ))
        }
    };

    let phase = crate::cli::extract_flag_value(args, "--phase").unwrap_or("all");
    let batch_size: usize = crate::cli::extract_flag_value(args, "--batch-size")
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_BATCH_SIZE);
    let checkpoint_interval: u64 = crate::cli::extract_flag_value(args, "--checkpoint-interval")
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_CHECKPOINT_INTERVAL);
    let rate_limit: u64 = crate::cli::extract_flag_value(args, "--rate-limit-mbps")
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_RATE_LIMIT_MBPS);
    let concurrency: usize = crate::cli::extract_flag_value(args, "--concurrency")
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_CONCURRENCY);
    let dry_run = args.iter().any(|a| *a == "--dry-run");
    let skip_braided = args.iter().any(|a| *a == "--skip-braided");
    let resume = args.iter().any(|a| *a == "--resume");

    let config = IngestConfig {
        batch_size,
        checkpoint_interval,
        rate_limit_mbps: rate_limit,
        concurrency,
        dry_run,
        skip_braided,
    };

    let mut state = if resume {
        IngestState::load().unwrap_or_default()
    } else {
        IngestState::default()
    };

    info!(
        "alphafold.ingest: phase={phase} batch_size={batch_size} checkpoint={checkpoint_interval} rate_limit={rate_limit}Mbps concurrency={concurrency}{}{}",
        if dry_run { " dry-run" } else { "" },
        if resume { " resume" } else { "" },
    );

    let mut results = Vec::new();

    if matches!(phase, "a" | "all") {
        info!("alphafold.ingest: === Phase A: Proteome Tars ===");
        match phase_a_proteome_tars(&bridge, &config, &mut state).await {
            Ok(r) => results.push(("phase_a", r)),
            Err(e) => {
                error!("Phase A failed: {e}");
                results.push(("phase_a", json!({"status": "failed", "error": e.to_string()})));
            }
        }
        state.save();
    }

    if matches!(phase, "b" | "all") {
        info!("alphafold.ingest: === Phase B: Expanded Structures ===");
        match phase_b_expanded_structures(&bridge, &config, &mut state).await {
            Ok(r) => results.push(("phase_b", r)),
            Err(e) => {
                error!("Phase B failed: {e}");
                results.push(("phase_b", json!({"status": "failed", "error": e.to_string()})));
            }
        }
        state.save();
    }

    if matches!(phase, "c" | "all") {
        info!("alphafold.ingest: === Phase C: Remote EBI Fetch ===");
        match phase_c_remote_fetch(&bridge, &config, &mut state).await {
            Ok(r) => results.push(("phase_c", r)),
            Err(e) => {
                error!("Phase C failed: {e}");
                results.push(("phase_c", json!({"status": "failed", "error": e.to_string()})));
            }
        }
        state.save();
    }

    let summary_data: serde_json::Map<String, serde_json::Value> = results
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();

    let total_files = state.total_files_ingested;
    let total_bytes = state.total_bytes_ingested;

    Ok(ShadowOutcome::ok_with(
        format!(
            "alphafold.ingest: {total_files} files, {} ingested across completed phases",
            human_bytes(total_bytes),
        ),
        json!({
            "phases": summary_data,
            "total_files": total_files,
            "total_bytes": total_bytes,
            "state": state,
        }),
    ))
}

// ═══════════════════════════════════════════════════════════════════════════════
// PHASE A — Proteome tars (on-disk)
// ═══════════════════════════════════════════════════════════════════════════════

async fn phase_a_proteome_tars(
    bridge: &NeuralBridge,
    config: &IngestConfig,
    state: &mut IngestState,
) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
    let root = std::path::Path::new(ALPHAFOLD_DATA);
    if !root.is_dir() {
        return Err(format!("AlphaFold data directory not found: {ALPHAFOLD_DATA}").into());
    }

    if config.skip_braided && root.join(".braided").exists() {
        info!("Phase A: already braided (--skip-braided), skipping");
        return Ok(json!({"status": "skipped", "reason": "already braided"}));
    }

    let dataset_name = "alphafold_proteome_archive";

    // Declare dataset (pre-braid intent)
    let (session_id, spine_id) = declare_dataset(bridge, dataset_name).await?;
    info!("Phase A: DAG session={session_id} spine={spine_id}");

    if config.dry_run {
        return Ok(json!({
            "status": "dry-run",
            "session_id": session_id,
            "spine_id": spine_id,
        }));
    }

    // Collect directories to ingest: v1..v6 + root-level files
    let mut dirs_to_ingest: Vec<(String, std::path::PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            dirs_to_ingest.push((name, path));
        }
    }
    dirs_to_ingest.sort_by(|a, b| a.0.cmp(&b.0));

    let mut phase_files = 0u64;
    let mut phase_bytes = 0u64;
    let mut dir_results = Vec::new();

    // Ingest each sub-directory via content.ingest
    for (dir_name, dir_path) in &dirs_to_ingest {
        if state.phase_a_completed_dirs.contains(dir_name) {
            info!("Phase A: {dir_name} — already ingested, skipping");
            continue;
        }

        info!("Phase A: ingesting {dir_name}");
        match ingest_directory(bridge, &session_id, dir_path, config.batch_size).await {
            Ok((files, bytes)) => {
                phase_files += files;
                phase_bytes += bytes;
                state.phase_a_completed_dirs.push(dir_name.clone());
                dir_results.push(json!({
                    "dir": dir_name,
                    "files": files,
                    "bytes": bytes,
                    "status": "ok",
                }));
                info!("Phase A: {dir_name} — {files} files, {}", human_bytes(bytes));
            }
            Err(e) => {
                warn!("Phase A: {dir_name} — failed: {e}");
                dir_results.push(json!({
                    "dir": dir_name,
                    "files": 0,
                    "error": e.to_string(),
                    "status": "failed",
                }));
            }
        }
    }

    // Ingest root-level files (tars, sequences.fasta, etc.)
    info!("Phase A: ingesting root-level files");
    match ingest_directory(bridge, &session_id, root, config.batch_size).await {
        Ok((files, bytes)) => {
            phase_files += files;
            phase_bytes += bytes;
            info!("Phase A: root — {files} files, {}", human_bytes(bytes));
        }
        Err(e) => {
            warn!("Phase A: root ingest failed: {e}");
        }
    }

    // Finalize: dehydrate → commit → sign → braid
    let finalize = complete_dataset(bridge, &session_id, &spine_id, dataset_name).await;
    let braid_hash = finalize.as_ref().ok().and_then(|v| {
        v.get("braid_hash").and_then(|h| h.as_str()).map(String::from)
    });

    state.total_files_ingested += phase_files;
    state.total_bytes_ingested += phase_bytes;
    state.phase_a_complete = true;

    // Write .braided marker
    write_braided_marker(root, dataset_name, phase_files, phase_bytes, &spine_id, braid_hash.as_deref());

    Ok(json!({
        "status": "complete",
        "dataset": dataset_name,
        "session_id": session_id,
        "spine_id": spine_id,
        "braid_hash": braid_hash,
        "files": phase_files,
        "bytes": phase_bytes,
        "directories": dir_results,
    }))
}

// ═══════════════════════════════════════════════════════════════════════════════
// PHASE B — Expanded structures (on-disk, streaming walk per prefix bucket)
//
// A0 alone has ~10M files. content.ingest on that is impractical (hours on
// spinners). Instead: walk directory locally, content.put each file, batch
// DAG events, checkpoint regularly.
// ═══════════════════════════════════════════════════════════════════════════════

async fn phase_b_expanded_structures(
    bridge: &NeuralBridge,
    config: &IngestConfig,
    state: &mut IngestState,
) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
    let root = std::path::Path::new(ALPHAFOLD_STRUCTURES);
    if !root.is_dir() {
        return Err(format!("AlphaFold structures directory not found: {ALPHAFOLD_STRUCTURES}").into());
    }

    let dataset_name = "alphafold_structures";

    // Collect prefix buckets (A0, A1, ..., Z9)
    let mut buckets: Vec<(String, std::path::PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || !path.is_dir() {
            continue;
        }
        buckets.push((name, path));
    }
    buckets.sort_by(|a, b| a.0.cmp(&b.0));

    info!("Phase B: {} prefix buckets to ingest", buckets.len());

    if config.dry_run {
        return Ok(json!({
            "status": "dry-run",
            "buckets": buckets.len(),
            "bucket_names": buckets.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
        }));
    }

    // Declare dataset (or resume existing session)
    let (session_id, spine_id) = if state.phase_b_session_id.is_some() && state.phase_b_spine_id.is_some() {
        (
            state.phase_b_session_id.clone().unwrap(),
            state.phase_b_spine_id.clone().unwrap(),
        )
    } else {
        let (sid, spid) = declare_dataset(bridge, dataset_name).await?;
        state.phase_b_session_id = Some(sid.clone());
        state.phase_b_spine_id = Some(spid.clone());
        state.save();
        (sid, spid)
    };

    info!("Phase B: DAG session={session_id} spine={spine_id}");

    let mut phase_files = 0u64;
    let mut phase_bytes = 0u64;
    let mut files_since_checkpoint = 0u64;

    for (bucket_name, bucket_path) in &buckets {
        if state.phase_b_completed_buckets.contains(bucket_name) {
            continue;
        }

        info!("Phase B: streaming bucket {bucket_name}");
        match stream_ingest_bucket(
            bridge,
            &session_id,
            bucket_path,
            bucket_name,
            config,
            state,
            &mut files_since_checkpoint,
        ).await {
            Ok((files, bytes)) => {
                phase_files += files;
                phase_bytes += bytes;
                state.phase_b_completed_buckets.push(bucket_name.clone());
                info!("Phase B: {bucket_name} — {files} files, {}", human_bytes(bytes));
            }
            Err(e) => {
                warn!("Phase B: {bucket_name} — failed: {e}");
                // Save progress and continue to next bucket
                state.save();
            }
        }

        // Checkpoint every N files
        if files_since_checkpoint >= config.checkpoint_interval {
            info!(
                "Phase B: checkpoint at {} total files",
                state.total_files_ingested + phase_files,
            );
            let _ = checkpoint_braid(bridge, &session_id, &spine_id, dataset_name, phase_files).await;
            files_since_checkpoint = 0;
            state.save();
        }
    }

    // Finalize
    let finalize = complete_dataset(bridge, &session_id, &spine_id, dataset_name).await;
    let braid_hash = finalize.as_ref().ok().and_then(|v| {
        v.get("braid_hash").and_then(|h| h.as_str()).map(String::from)
    });

    state.total_files_ingested += phase_files;
    state.total_bytes_ingested += phase_bytes;
    state.phase_b_complete = true;

    write_braided_marker(root, dataset_name, phase_files, phase_bytes, &spine_id, braid_hash.as_deref());

    Ok(json!({
        "status": "complete",
        "dataset": dataset_name,
        "session_id": session_id,
        "spine_id": spine_id,
        "braid_hash": braid_hash,
        "files": phase_files,
        "bytes": phase_bytes,
        "buckets_processed": state.phase_b_completed_buckets.len(),
    }))
}

/// Stream-ingest a single bucket directory: walk entries, content.put each file,
/// batch DAG events. Designed for directories with millions of files where
/// content.ingest would time out.
async fn stream_ingest_bucket(
    bridge: &NeuralBridge,
    session_id: &str,
    bucket_path: &std::path::Path,
    bucket_name: &str,
    config: &IngestConfig,
    state: &mut IngestState,
    files_since_checkpoint: &mut u64,
) -> Result<(u64, u64), Box<dyn std::error::Error + Send + Sync>> {
    let mut bucket_files = 0u64;
    let mut bucket_bytes = 0u64;
    let mut dag_events: Vec<serde_json::Value> = Vec::new();

    let entries = std::fs::read_dir(bucket_path)?;

    for entry_result in entries {
        let entry = match entry_result {
            Ok(e) => e,
            Err(e) => {
                warn!("Phase B: {bucket_name}: readdir error: {e}");
                continue;
            }
        };

        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let filename = entry.file_name().to_string_lossy().to_string();

        // Read file and PUT to CAS
        let data = match std::fs::read(&path) {
            Ok(d) => d,
            Err(e) => {
                warn!("Phase B: read failed {}: {e}", path.display());
                continue;
            }
        };

        let file_size = data.len() as u64;
        let encoded = base64_encode(&data);

        let put_result = direct_nestgate_call(
            "content.put",
            json!({
                "content_base64": encoded,
                "source": "alphafold_structures",
                "stored_by": COMMITTER_DID,
                "family_id": FAMILY_ID,
            }),
            Duration::from_secs(30),
        ).await;

        match put_result {
            Ok(v) => {
                let hash = v.get("hash").and_then(|h| h.as_str()).unwrap_or("");
                bucket_files += 1;
                bucket_bytes += file_size;
                *files_since_checkpoint += 1;

                dag_events.push(json!({
                    "type": "DataCreate",
                    "filename": filename,
                    "blake3": hash,
                    "dataset": "alphafold_structures",
                    "bucket": bucket_name,
                    "size": file_size,
                }));
            }
            Err(e) => {
                warn!("Phase B: content.put failed for {filename}: {e}");
            }
        }

        // Flush DAG events in batches
        if dag_events.len() >= config.batch_size {
            let _ = append_dag_batch(bridge, session_id, &dag_events).await;
            dag_events.clear();
        }

        // Progress logging every 10,000 files
        if bucket_files % 10_000 == 0 && bucket_files > 0 {
            info!(
                "Phase B: {bucket_name} progress — {} files, {}",
                bucket_files, human_bytes(bucket_bytes),
            );
        }

        // Checkpoint within a bucket
        if *files_since_checkpoint >= config.checkpoint_interval {
            info!("Phase B: mid-bucket checkpoint at {bucket_name} ({bucket_files} files)");
            let _ = checkpoint_braid(
                bridge, session_id,
                state.phase_b_spine_id.as_deref().unwrap_or("none"),
                "alphafold_structures",
                state.total_files_ingested + bucket_files,
            ).await;
            *files_since_checkpoint = 0;
            state.save();
        }
    }

    // Flush remaining DAG events
    if !dag_events.is_empty() {
        let _ = append_dag_batch(bridge, session_id, &dag_events).await;
    }

    Ok((bucket_files, bucket_bytes))
}

fn base64_encode(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

// ═══════════════════════════════════════════════════════════════════════════════
// PHASE C — Remote EBI fetch
// ═══════════════════════════════════════════════════════════════════════════════

async fn phase_c_remote_fetch(
    bridge: &NeuralBridge,
    config: &IngestConfig,
    state: &mut IngestState,
) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
    let accession_csv = std::path::Path::new(ALPHAFOLD_DATA).join("accession_ids.csv");
    if !accession_csv.exists() {
        return Err("accession_ids.csv not found — cannot build download manifest".into());
    }

    let dataset_name = "alphafold_structures_ebi";

    // Build the work queue from accession_ids.csv minus already-fetched
    let progress_file = std::path::Path::new(ALPHAFOLD_STRUCTURES).join(".progress");
    let already_fetched = load_progress_set(&progress_file);

    info!(
        "Phase C: loading accession manifest (already fetched: {})",
        already_fetched.len(),
    );

    // Parse accession IDs (one per line, skip header if present)
    let csv_content = std::fs::read_to_string(&accession_csv)?;
    let accessions: Vec<&str> = csv_content
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty()
                && !trimmed.starts_with('#')
                && !trimmed.to_lowercase().starts_with("accession")
        })
        .map(|line| {
            // Take only the first column if CSV has multiple columns
            line.split(',').next().unwrap_or(line).trim()
        })
        .filter(|acc| !already_fetched.contains(*acc))
        .collect();

    let total_remaining = accessions.len();
    info!("Phase C: {total_remaining} accessions remaining to fetch");

    if config.dry_run {
        return Ok(json!({
            "status": "dry-run",
            "total_remaining": total_remaining,
            "already_fetched": already_fetched.len(),
            "sample": accessions.iter().take(10).collect::<Vec<_>>(),
        }));
    }

    if total_remaining == 0 {
        return Ok(json!({"status": "complete", "reason": "all accessions already fetched"}));
    }

    // Declare dataset (or resume existing session)
    let (session_id, spine_id) = if state.phase_c_session_id.is_some() && state.phase_c_spine_id.is_some() {
        (
            state.phase_c_session_id.clone().unwrap(),
            state.phase_c_spine_id.clone().unwrap(),
        )
    } else {
        let (sid, spid) = declare_dataset(bridge, dataset_name).await?;
        state.phase_c_session_id = Some(sid.clone());
        state.phase_c_spine_id = Some(spid.clone());
        state.save();
        (sid, spid)
    };

    info!("Phase C: DAG session={session_id} spine={spine_id}");

    let mut phase_files = 0u64;
    let mut phase_bytes = 0u64;
    let mut files_since_checkpoint = 0u64;
    let mut errors = 0u64;
    let mut dag_events: Vec<serde_json::Value> = Vec::new();

    // Process accessions in batches for concurrent fetching
    for (i, chunk) in accessions.chunks(config.concurrency).enumerate() {
        // Fetch each file in the chunk via content.fetch
        for accession in chunk {
            let url = format!(
                "{EBI_BASE_URL}/AF-{accession}-F1-model_v6.cif"
            );

            // content.fetch can take minutes for large files — call nestGate directly
            let fetch_result = direct_nestgate_call(
                "content.fetch",
                json!({
                    "url": url,
                    "rate_limit_mbps": config.rate_limit_mbps,
                    "timeout_secs": 300,
                }),
                Duration::from_secs(600),
            ).await;

            match fetch_result {
                Ok(v) => {
                    let hash = v.get("hash").and_then(|h| h.as_str()).unwrap_or("");
                    let size = v.get("size").and_then(|s| s.as_u64()).unwrap_or(0);

                    phase_files += 1;
                    phase_bytes += size;
                    files_since_checkpoint += 1;

                    dag_events.push(json!({
                        "type": "DataCreate",
                        "filename": format!("AF-{accession}-F1-model_v6.cif"),
                        "blake3": hash,
                        "dataset": dataset_name,
                        "source": "ebi",
                        "accession": accession,
                    }));

                    append_progress(&progress_file, accession);
                }
                Err(e) => {
                    warn!("Phase C: fetch failed for {accession}: {e}");
                    errors += 1;
                }
            }

            // Flush DAG events in batches
            if dag_events.len() >= config.batch_size {
                let _ = append_dag_batch(bridge, &session_id, &dag_events).await;
                dag_events.clear();
            }

            // Checkpoint
            if files_since_checkpoint >= config.checkpoint_interval {
                info!(
                    "Phase C: checkpoint at {} files ({} total, {} errors)",
                    phase_files, i * config.concurrency, errors,
                );
                // Flush remaining events
                if !dag_events.is_empty() {
                    let _ = append_dag_batch(bridge, &session_id, &dag_events).await;
                    dag_events.clear();
                }
                let _ = checkpoint_braid(bridge, &session_id, &spine_id, dataset_name, phase_files).await;
                files_since_checkpoint = 0;
                state.phase_c_files_fetched += phase_files;
                state.save();
                phase_files = 0;
            }
        }

        // Progress logging every 1000 chunks
        if i % 1000 == 0 && i > 0 {
            info!(
                "Phase C: progress — {} files fetched, {} errors, {} remaining",
                state.phase_c_files_fetched + phase_files,
                errors,
                total_remaining as u64 - (state.phase_c_files_fetched + phase_files),
            );
        }
    }

    // Flush remaining DAG events
    if !dag_events.is_empty() {
        let _ = append_dag_batch(bridge, &session_id, &dag_events).await;
    }

    // Finalize
    let finalize = complete_dataset(bridge, &session_id, &spine_id, dataset_name).await;
    let braid_hash = finalize.as_ref().ok().and_then(|v| {
        v.get("braid_hash").and_then(|h| h.as_str()).map(String::from)
    });

    state.total_files_ingested += phase_files;
    state.total_bytes_ingested += phase_bytes;
    state.phase_c_files_fetched += phase_files;
    state.phase_c_complete = true;

    Ok(json!({
        "status": "complete",
        "dataset": dataset_name,
        "session_id": session_id,
        "spine_id": spine_id,
        "braid_hash": braid_hash,
        "files_fetched": state.phase_c_files_fetched,
        "bytes": phase_bytes,
        "errors": errors,
    }))
}

// ═══════════════════════════════════════════════════════════════════════════════
// NEURAL API HELPERS
// ═══════════════════════════════════════════════════════════════════════════════

/// Declare a dataset: create DAG session + spine + intent braid.
/// Returns (session_id, spine_id).
async fn declare_dataset(
    bridge: &NeuralBridge,
    dataset_name: &str,
) -> Result<(String, String), Box<dyn std::error::Error + Send + Sync>> {
    // Create DAG session
    let session_result = bridge.capability_call(
        "dag",
        "session.create",
        json!({
            "name": dataset_name,
            "committer": COMMITTER_DID,
        }),
    ).await;

    let session_id = match session_result {
        BridgeResult::Handled(v) => {
            v.get("session_id")
                .or_else(|| v.get("id"))
                .and_then(|x| x.as_str())
                .or_else(|| v.as_str())
                .ok_or("dag.session.create: no session_id")?
                .to_string()
        }
        BridgeResult::ApiError(e) => return Err(format!("dag.session.create: {e}").into()),
        BridgeResult::Fallthrough => return Err("dag.session.create: Neural API unreachable".into()),
    };

    // Create spine
    let spine_result = bridge.capability_call(
        "spine",
        "create",
        json!({
            "name": dataset_name,
            "owner": COMMITTER_DID,
            "committer": COMMITTER_DID,
        }),
    ).await;

    let spine_id = match spine_result {
        BridgeResult::Handled(v) => {
            v.get("spine_id")
                .or_else(|| v.get("id"))
                .and_then(|x| x.as_str())
                .or_else(|| v.as_str())
                .unwrap_or("unknown")
                .to_string()
        }
        BridgeResult::ApiError(e) => {
            warn!("spine.create: {e} — continuing without spine");
            "none".to_string()
        }
        BridgeResult::Fallthrough => {
            warn!("spine.create: Neural API unreachable — continuing without spine");
            "none".to_string()
        }
    };

    // Create intent braid
    let _ = bridge.capability_call(
        "braid",
        "create",
        json!({
            "data_hash": &session_id,
            "strand_id": dataset_name,
            "metadata": {
                "dataset": dataset_name,
                "committer": COMMITTER_DID,
                "family_id": FAMILY_ID,
                "spine_id": &spine_id,
                "status": "intent",
            },
        }),
    ).await;

    Ok((session_id, spine_id))
}

/// Ingest a directory via content.ingest (direct to nestGate socket for long timeout),
/// then append DAG events from manifest. Returns (file_count, byte_count).
async fn ingest_directory(
    bridge: &NeuralBridge,
    session_id: &str,
    dir_path: &std::path::Path,
    batch_size: usize,
) -> Result<(u64, u64), Box<dyn std::error::Error + Send + Sync>> {
    // content.ingest can take minutes for large directories — call nestGate directly
    // with an extended timeout instead of going through the 3s Neural API default.
    let manifest = direct_nestgate_call(
        "content.ingest",
        json!({ "directory": dir_path.to_string_lossy() }),
        Duration::from_secs(600),
    ).await?;

    let file_count = manifest
        .get("count")
        .or_else(|| manifest.get("file_count"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let byte_count = manifest
        .get("bytes_total")
        .or_else(|| manifest.get("total_bytes"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    // Build DAG events from manifest
    let entries = manifest
        .get("manifest")
        .or_else(|| manifest.get("entries"))
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    let events: Vec<serde_json::Value> = entries
        .iter()
        .map(|(filename, hash)| {
            json!({
                "type": "DataCreate",
                "filename": filename,
                "blake3": hash,
                "dataset": "alphafold",
            })
        })
        .collect();

    // Append in batches
    for chunk in events.chunks(batch_size) {
        let _ = append_dag_batch(bridge, session_id, chunk).await;
    }

    Ok((file_count, byte_count))
}

/// Append a batch of DAG events.
async fn append_dag_batch(
    bridge: &NeuralBridge,
    session_id: &str,
    events: &[serde_json::Value],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // rhizoCrypt expects "requests" array, each with session_id + event_type (struct variant)
    // + metadata. EventType::DataCreate is serialized as {"DataCreate": {"schema": null}}.
    let requests: Vec<serde_json::Value> = events
        .iter()
        .map(|event| {
            let metadata: Vec<serde_json::Value> = event
                .as_object()
                .map(|obj| {
                    obj.iter()
                        .filter(|(k, _)| *k != "type")
                        .map(|(k, v)| json!([k, v]))
                        .collect()
                })
                .unwrap_or_default();

            json!({
                "session_id": session_id,
                "event_type": { "DataCreate": { "schema": null } },
                "metadata": metadata,
            })
        })
        .collect();

    let result = bridge.capability_call(
        "dag",
        "event.append_batch",
        json!({ "requests": requests }),
    ).await;

    match result {
        BridgeResult::Handled(_) => Ok(()),
        BridgeResult::ApiError(e) => {
            warn!("dag.event.append_batch: {e} — trying individual appends");
            for event in events {
                let _ = bridge.capability_call(
                    "dag",
                    "event.append",
                    json!({
                        "session_id": session_id,
                        "event": event,
                    }),
                ).await;
            }
            Ok(())
        }
        BridgeResult::Fallthrough => Err("dag.event.append_batch: Neural API unreachable".into()),
    }
}

/// Checkpoint braid (partial update).
async fn checkpoint_braid(
    bridge: &NeuralBridge,
    session_id: &str,
    spine_id: &str,
    dataset_name: &str,
    files_so_far: u64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Commit current state to spine
    let _ = bridge.capability_call(
        "session",
        "commit",
        json!({
            "spine_id": spine_id,
            "session_id": session_id,
            "vertex_count": files_so_far,
            "committer": COMMITTER_DID,
        }),
    ).await;

    // Create checkpoint braid
    let _ = bridge.capability_call(
        "braid",
        "create",
        json!({
            "data_hash": session_id,
            "strand_id": dataset_name,
            "metadata": {
                "dataset": dataset_name,
                "committer": COMMITTER_DID,
                "family_id": FAMILY_ID,
                "spine_id": spine_id,
                "status": "checkpoint",
                "files_so_far": files_so_far,
            },
        }),
    ).await;

    info!("checkpoint: braid created at {files_so_far} files");
    Ok(())
}

/// Finalize dataset: dehydrate → commit → sign → braid.
async fn complete_dataset(
    bridge: &NeuralBridge,
    session_id: &str,
    spine_id: &str,
    dataset_name: &str,
) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
    // Dehydrate → merkle root
    let dehydrate = bridge.capability_call(
        "dag",
        "dehydration.trigger",
        json!({"session_id": session_id}),
    ).await;

    let merkle_root = match dehydrate {
        BridgeResult::Handled(v) => {
            v.get("merkle_root").and_then(|m| m.as_str()).map(String::from)
        }
        _ => {
            warn!("dehydration.trigger: not available — using session_id as root");
            None
        }
    };

    let root_hash = merkle_root.as_deref().unwrap_or(session_id);

    // Commit to spine
    let _ = bridge.capability_call(
        "session",
        "commit",
        json!({
            "spine_id": spine_id,
            "session_id": session_id,
            "merkle_root": root_hash,
            "committer": COMMITTER_DID,
        }),
    ).await;

    // Sign
    let sign_result = bridge.capability_call(
        "crypto",
        "sign",
        json!({"message": root_hash}),
    ).await;

    let signature = match sign_result {
        BridgeResult::Handled(v) => {
            v.get("signature").and_then(|s| s.as_str()).map(String::from)
        }
        _ => None,
    };

    // Final braid
    let braid_result = bridge.capability_call(
        "braid",
        "create",
        json!({
            "data_hash": root_hash,
            "strand_id": dataset_name,
            "metadata": {
                "dataset": dataset_name,
                "committer": COMMITTER_DID,
                "family_id": FAMILY_ID,
                "spine_id": spine_id,
                "signature": signature,
                "status": "complete",
            },
        }),
    ).await;

    let braid_hash = match braid_result {
        BridgeResult::Handled(v) => {
            v.get("braid_hash").or_else(|| v.get("hash"))
                .and_then(|h| h.as_str()).map(String::from)
        }
        _ => None,
    };

    Ok(json!({
        "merkle_root": merkle_root,
        "signature": signature,
        "braid_hash": braid_hash,
    }))
}

// ═══════════════════════════════════════════════════════════════════════════════
// STATUS + MANIFEST
// ═══════════════════════════════════════════════════════════════════════════════

async fn dispatch_status(_args: &[&str]) -> crate::Result<ShadowOutcome> {
    let state = IngestState::load().unwrap_or_default();

    // Check disk state
    let proteome_braided = std::path::Path::new(ALPHAFOLD_DATA).join(".braided").exists();
    let structures_braided = std::path::Path::new(ALPHAFOLD_STRUCTURES).join(".braided").exists();
    let progress_file = std::path::Path::new(ALPHAFOLD_STRUCTURES).join(".progress");
    let fetched_count = if progress_file.exists() {
        std::fs::read_to_string(&progress_file)
            .map(|s| s.lines().count())
            .unwrap_or(0)
    } else {
        0
    };

    // Count on-disk structures
    let structures_root = std::path::Path::new(ALPHAFOLD_STRUCTURES);
    let bucket_count = if structures_root.is_dir() {
        std::fs::read_dir(structures_root)
            .map(|entries| {
                entries.filter_map(|e| e.ok())
                    .filter(|e| e.path().is_dir() && !e.file_name().to_string_lossy().starts_with('.'))
                    .count()
            })
            .unwrap_or(0)
    } else {
        0
    };

    let accession_csv = std::path::Path::new(ALPHAFOLD_DATA).join("accession_ids.csv");
    let total_accessions = if accession_csv.exists() {
        std::fs::read_to_string(&accession_csv)
            .map(|s| s.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')).count().saturating_sub(1))
            .unwrap_or(0)
    } else {
        0
    };

    Ok(ShadowOutcome::ok_with(
        format!(
            "alphafold.status: Phase A {} | Phase B {} ({}/{} buckets) | Phase C {} ({}/{} accessions)",
            if state.phase_a_complete || proteome_braided { "complete" } else { "pending" },
            if state.phase_b_complete || structures_braided { "complete" } else { "in-progress" },
            state.phase_b_completed_buckets.len(),
            bucket_count,
            if state.phase_c_complete { "complete" } else { "in-progress" },
            fetched_count,
            total_accessions,
        ),
        json!({
            "phase_a": {
                "complete": state.phase_a_complete || proteome_braided,
                "dirs_ingested": state.phase_a_completed_dirs.len(),
            },
            "phase_b": {
                "complete": state.phase_b_complete || structures_braided,
                "buckets_total": bucket_count,
                "buckets_ingested": state.phase_b_completed_buckets.len(),
            },
            "phase_c": {
                "complete": state.phase_c_complete,
                "total_accessions": total_accessions,
                "fetched": fetched_count,
                "remaining": total_accessions.saturating_sub(fetched_count),
            },
            "totals": {
                "files_ingested": state.total_files_ingested,
                "bytes_ingested": state.total_bytes_ingested,
                "human_bytes": human_bytes(state.total_bytes_ingested),
            },
        }),
    ))
}

async fn dispatch_manifest(_args: &[&str]) -> crate::Result<ShadowOutcome> {
    let accession_csv = std::path::Path::new(ALPHAFOLD_DATA).join("accession_ids.csv");
    if !accession_csv.exists() {
        return Ok(ShadowOutcome::fail(
            "accession_ids.csv not found in AlphaFold data directory",
        ));
    }

    let csv_content = std::fs::read_to_string(&accession_csv)
        .map_err(|e| crate::ShadowError::Io(e))?;

    let total_lines = csv_content.lines().count();
    let sample: Vec<&str> = csv_content.lines().take(10).collect();

    // Count on-disk proteome tars
    let proteome_root = std::path::Path::new(ALPHAFOLD_DATA);
    let tar_count = if proteome_root.is_dir() {
        std::fs::read_dir(proteome_root)
            .map(|entries| {
                entries.filter_map(|e| e.ok())
                    .filter(|e| e.file_name().to_string_lossy().ends_with(".tar"))
                    .count()
            })
            .unwrap_or(0)
    } else {
        0
    };

    let structures_root = std::path::Path::new(ALPHAFOLD_STRUCTURES);
    let bucket_count = if structures_root.is_dir() {
        std::fs::read_dir(structures_root)
            .map(|entries| {
                entries.filter_map(|e| e.ok())
                    .filter(|e| e.path().is_dir() && !e.file_name().to_string_lossy().starts_with('.'))
                    .count()
            })
            .unwrap_or(0)
    } else {
        0
    };

    Ok(ShadowOutcome::ok_with(
        format!(
            "alphafold.manifest: {} accession lines, {} proteome tars, {} structure buckets",
            total_lines, tar_count, bucket_count,
        ),
        json!({
            "accession_csv": accession_csv.to_string_lossy(),
            "total_lines": total_lines,
            "proteome_tars": tar_count,
            "structure_buckets": bucket_count,
            "sample": sample,
        }),
    ))
}

// ═══════════════════════════════════════════════════════════════════════════════
// STATE MANAGEMENT
// ═══════════════════════════════════════════════════════════════════════════════

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct IngestState {
    phase_a_complete: bool,
    phase_a_completed_dirs: Vec<String>,
    phase_b_complete: bool,
    phase_b_session_id: Option<String>,
    phase_b_spine_id: Option<String>,
    phase_b_completed_buckets: Vec<String>,
    phase_c_complete: bool,
    phase_c_session_id: Option<String>,
    phase_c_spine_id: Option<String>,
    phase_c_files_fetched: u64,
    total_files_ingested: u64,
    total_bytes_ingested: u64,
}

impl IngestState {
    fn load() -> Option<Self> {
        let path = std::path::Path::new(ALPHAFOLD_DATA).join(STATE_FILE);
        let content = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&content).ok()
    }

    fn save(&self) {
        let path = std::path::Path::new(ALPHAFOLD_DATA).join(STATE_FILE);
        if let Ok(json) = serde_json::to_string_pretty(self) {
            if let Err(e) = std::fs::write(&path, json) {
                warn!("failed to save ingress state: {e}");
            }
        }
    }
}

struct IngestConfig {
    batch_size: usize,
    checkpoint_interval: u64,
    rate_limit_mbps: u64,
    concurrency: usize,
    dry_run: bool,
    skip_braided: bool,
}

// ═══════════════════════════════════════════════════════════════════════════════
// UTILITIES
// ═══════════════════════════════════════════════════════════════════════════════

fn human_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const GIB: u64 = 1024 * MIB;
    const TIB: u64 = 1024 * GIB;

    if bytes >= TIB {
        format!("{:.2} TiB", bytes as f64 / TIB as f64)
    } else if bytes >= GIB {
        format!("{:.2} GiB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.2} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.2} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} B")
    }
}

fn load_progress_set(path: &std::path::Path) -> std::collections::HashSet<String> {
    if !path.exists() {
        return std::collections::HashSet::new();
    }
    std::fs::read_to_string(path)
        .map(|s| s.lines().map(|l| l.trim().to_string()).collect())
        .unwrap_or_default()
}

fn append_progress(path: &std::path::Path, accession: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{accession}");
    }
}

fn write_braided_marker(
    dir: &std::path::Path,
    dataset: &str,
    files: u64,
    bytes: u64,
    spine_id: &str,
    braid_hash: Option<&str>,
) {
    let marker = dir.join(".braided");
    let data = json!({
        "dataset": dataset,
        "files": files,
        "bytes": bytes,
        "spine_id": spine_id,
        "braid_hash": braid_hash,
        "committer": COMMITTER_DID,
        "braider": "membrane alphafold.ingest v1.0",
        "braided_at": epoch_secs(),
    });
    if let Err(e) = std::fs::write(&marker, serde_json::to_string_pretty(&data).unwrap_or_default()) {
        warn!("failed to write .braided marker: {e}");
    }
}

fn epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Direct UDS JSON-RPC call to nestGate with a custom timeout.
///
/// Bypasses the NeuralBridge (3s default timeout) for operations that need
/// minutes to complete (e.g. content.ingest on a large directory, content.fetch
/// for multi-GB files). Sends the riboCipher `[0xEC, 0x01]` prefix that all
/// primals expect on UDS connections.
async fn direct_nestgate_call(
    method: &str,
    params: serde_json::Value,
    timeout: Duration,
) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
    let socket_path = resolve_nestgate_socket();

    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });
    let request_str = serde_json::to_string(&request)?;

    let stream = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::UnixStream::connect(&socket_path),
    )
    .await
    .map_err(|_| format!("connect timeout: {}", socket_path.display()))?
    .map_err(|e| format!("connect failed: {}: {e}", socket_path.display()))?;

    let (reader, mut writer) = tokio::io::split(stream);

    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

    // riboCipher ecosystem signal prefix — required on all UDS connections
    writer.write_all(&[0xEC, 0x01]).await?;
    writer.write_all(request_str.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;

    let mut buf_reader = tokio::io::BufReader::new(reader);
    let mut line = String::new();

    let read_result = tokio::time::timeout(timeout, buf_reader.read_line(&mut line))
        .await
        .map_err(|_| format!("read timeout ({timeout:?}): {}", socket_path.display()))?
        .map_err(|e| format!("read error: {e}"))?;

    if read_result == 0 {
        return Err("empty response from nestGate".into());
    }

    let response: serde_json::Value = serde_json::from_str(line.trim())?;

    if let Some(error) = response.get("error") {
        let msg = error.get("message").and_then(|m| m.as_str()).unwrap_or("unknown error");
        return Err(format!("{method}: {msg}").into());
    }

    response.get("result")
        .cloned()
        .ok_or_else(|| format!("{method}: no result in response").into())
}

/// Resolve the nestGate socket path from known locations.
fn resolve_nestgate_socket() -> std::path::PathBuf {
    let candidates = [
        "/run/user/1000/membrane/nestgate-westgate-tower-155f.sock",
        "/run/user/1000/membrane/nestgate.sock",
        "/tmp/membrane/nestgate.sock",
    ];

    for candidate in &candidates {
        let p = std::path::PathBuf::from(candidate);
        if p.exists() {
            return p;
        }
    }

    std::path::PathBuf::from(candidates[0])
}
