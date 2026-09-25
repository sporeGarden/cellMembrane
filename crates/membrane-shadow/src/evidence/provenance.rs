// SPDX-License-Identifier: AGPL-3.0-or-later

//! Evidence provenance — nest atomic integration for evidence braiding.
//!
//! Graduates evidence from static SCP hosting into the provenance trio:
//! rhizoCrypt DAG sessions → loamSpine spine commits → sweetGrass PROV-O braids.
//!
//! Uses direct UDS socket calls (riboCipher prefix `[0xEC, 0x01]`) to bypass
//! the NeuralBridge 3s timeout — same proven pattern as `alphafold_dispatch.rs`.
//!
//! ## Progression
//!
//! - **Phase 1** (this): DAG + spine + braid via direct sockets, BLAKE3 as content addresses
//! - **Phase 2**: nestGate CAS `content.put` once BTSP auth is wired
//! - **Phase 3**: rootPulse graph orchestration for automatic braiding on push

use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing::{info, warn};

/// Committer identity for evidence provenance.
const COMMITTER_DID: &str = "did:eco:ecoPrimal";

/// Braided marker filename (same convention as `content.braid`).
const BRAIDED_MARKER: &str = ".braided";

/// Default timeout for primal UDS calls.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

// ── Direct UDS JSON-RPC Client ──────────────────────────────────────

/// Direct UDS JSON-RPC call to a primal socket with custom timeout.
///
/// Sends the riboCipher `[0xEC, 0x01]` prefix that all primals expect
/// on UDS connections. This bypasses the NeuralBridge (3s default timeout)
/// for operations that may take longer.
///
/// Extracted from `alphafold_dispatch.rs::direct_nestgate_call` and
/// generalized to work with any primal socket.
pub(crate) async fn direct_uds_call(
    socket_name: &str,
    method: &str,
    params: serde_json::Value,
    timeout: Duration,
) -> std::result::Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
    let socket_path = resolve_socket(socket_name);

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
        return Err(format!("empty response from {socket_name}").into());
    }

    let response: serde_json::Value = serde_json::from_str(line.trim())?;

    if let Some(error) = response.get("error") {
        let msg = error
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown error");
        return Err(format!("{method}: {msg}").into());
    }

    response
        .get("result")
        .cloned()
        .ok_or_else(|| format!("{method}: no result in response").into())
}

/// Resolve a primal socket path from known locations.
fn resolve_socket(name: &str) -> PathBuf {
    let candidates = [
        format!("/run/membrane/{name}.sock"),
        format!("/run/user/1000/membrane/{name}.sock"),
        format!("/tmp/membrane/{name}.sock"),
    ];

    for candidate in &candidates {
        let p = PathBuf::from(candidate);
        if p.exists() {
            return p;
        }
    }

    PathBuf::from(&candidates[0])
}

/// Check whether the nest atomic provenance stack is available locally.
///
/// Returns true if the rhizoCrypt DAG socket exists — the minimum
/// requirement for evidence braiding. On golgiBody (VPS), the trio
/// isn't running, so braiding is skipped.
pub(crate) fn is_trio_available() -> bool {
    resolve_socket("dag").exists()
}

// ── Evidence Braid Pipeline ─────────────────────────────────────────

/// Parsed manifest entry from `evidence-manifest.toml`.
pub(super) struct ManifestEntry {
    path: String,
    blake3: String,
    size: u64,
    mime: String,
}

/// Result of braiding an evidence collection.
pub(crate) struct BraidResult {
    pub files: u64,
    pub bytes: u64,
    pub merkle_root: Option<String>,
    pub braid_hash: Option<String>,
    pub spine_id: Option<String>,
    pub signature: Option<String>,
    pub session_id: String,
}

/// Braid an evidence collection through the provenance trio.
///
/// 7-step pipeline following `content_dispatch.rs::braid_dataset()`:
/// 1. DAG session create (rhizoCrypt)
/// 2. DAG event append (evidence files as DataCreate events)
/// 3. Dehydration trigger → merkle root
/// 4. Spine create + session commit (loamSpine)
/// 5. Sign composite hash (bearDog via NeuralBridge)
/// 6. Braid create (sweetGrass PROV-O attribution)
/// 7. Write `.braided` marker for incremental support
pub(crate) async fn braid_evidence_collection(
    collection_name: &str,
    collection_dir: &Path,
    entries: &[ManifestEntry],
    _source_description: &str,
) -> std::result::Result<BraidResult, Box<dyn std::error::Error + Send + Sync>> {
    let file_count = entries.len() as u64;
    let total_bytes: u64 = entries.iter().map(|e| e.size).sum();

    info!(
        collection = %collection_name,
        files = file_count,
        bytes = total_bytes,
        "evidence braid: starting pipeline"
    );

    // Step 1: Create DAG session via rhizoCrypt
    let session_result = direct_uds_call(
        "dag",
        "dag.session.create",
        json!({
            "name": collection_name,
            "committer": COMMITTER_DID,
        }),
        DEFAULT_TIMEOUT,
    )
    .await?;

    let session_id = if let Some(s) = session_result.as_str() {
        // DAG returns session ID as a bare string result
        s.to_string()
    } else {
        // Fallback: result is an object with session_id or id field
        session_result
            .get("session_id")
            .or_else(|| session_result.get("id"))
            .and_then(|v| v.as_str())
            .ok_or("dag.session.create: no session_id in response")?
            .to_string()
    };

    info!(
        session_id = %session_id,
        "evidence braid: DAG session created"
    );

    // Step 2: Append evidence files as DAG events
    // rhizoCrypt uses Rust-style externally tagged enums for event_type
    let mut append_count = 0usize;
    for entry in entries {
        let result = direct_uds_call(
            "dag",
            "dag.event.append",
            json!({
                "session_id": session_id,
                "event_type": {
                    "DataCreate": {
                        "blake3": entry.blake3,
                        "path": entry.path,
                        "mime": entry.mime,
                        "bytes": entry.size,
                        "dataset": collection_name,
                    }
                },
            }),
            DEFAULT_TIMEOUT,
        )
        .await;

        match result {
            Ok(_) => append_count += 1,
            Err(e) => warn!(file = %entry.path, error = %e, "evidence braid: event append failed"),
        }
    }
    info!(appended = append_count, total = entries.len(), "evidence braid: DAG events appended");

    // Step 3: Trigger dehydration → merkle root
    let merkle_root = match direct_uds_call(
        "dag",
        "dag.dehydrate",
        json!({ "session_id": session_id }),
        DEFAULT_TIMEOUT,
    )
    .await
    {
        Ok(v) => {
            // dag.dehydrate returns the merkle root as a bare string
            let root = v
                .as_str()
                .map(String::from)
                .or_else(|| v.get("merkle_root").and_then(|m| m.as_str()).map(String::from));
            info!(merkle_root = ?root, "evidence braid: dehydration complete");
            root
        }
        Err(e) => {
            warn!(error = %e, "evidence braid: dehydration failed (continuing)");
            None
        }
    };

    // Step 4: Create spine + commit session via loamSpine
    let spine_id = match direct_uds_call(
        "loamspine",
        "spine.create",
        json!({
            "name": format!("evidence-{collection_name}"),
            "owner": COMMITTER_DID,
        }),
        DEFAULT_TIMEOUT,
    )
    .await
    {
        Ok(v) => {
            let sid = v
                .get("spine_id")
                .or_else(|| v.get("id"))
                .and_then(|s| s.as_str())
                .map(String::from);
            info!(spine_id = ?sid, "evidence braid: spine created");
            sid
        }
        Err(e) => {
            warn!(error = %e, "evidence braid: spine creation failed (continuing)");
            None
        }
    };

    let composite = merkle_root.as_deref().unwrap_or(&session_id);

    if let Some(ref sid) = spine_id {
        let session_hash = composite.to_string();
        let commit_result = direct_uds_call(
            "loamspine",
            "session.commit",
            json!({
                "spine_id": sid,
                "session_id": session_id,
                "session_hash": session_hash,
                "vertex_count": append_count,
                "committer": COMMITTER_DID,
            }),
            DEFAULT_TIMEOUT,
        )
        .await;

        match &commit_result {
            Ok(_) => info!("evidence braid: session committed to spine"),
            Err(e) => warn!(error = %e, "evidence braid: session commit failed (continuing)"),
        }
    }

    // Step 5: Sign composite hash via bearDog (through NeuralBridge)
    let signature = if let Some(bridge) = crate::bridge::NeuralBridge::discover() {
        match bridge
            .capability_call("crypto", "sign", json!({ "message": composite }))
            .await
        {
            crate::bridge::BridgeResult::Handled(v) => {
                let sig = v.get("signature").and_then(|s| s.as_str()).map(String::from);
                info!(signed = sig.is_some(), "evidence braid: bearDog signature");
                sig
            }
            _ => {
                warn!("evidence braid: bearDog signing unavailable (continuing)");
                None
            }
        }
    } else {
        warn!("evidence braid: NeuralBridge not discovered, skipping signing");
        None
    };

    // Step 6: Create provenance braid via sweetGrass
    let braid_hash = match direct_uds_call(
        "sweetgrass",
        "braid.create",
        json!({
            "label": format!("evidence/{collection_name}"),
            "data_hash": composite,
            "committer": COMMITTER_DID,
            "mime_type": "application/x-evidence-collection",
            "size": total_bytes,
            "spine_id": spine_id,
            "session_id": session_id,
        }),
        DEFAULT_TIMEOUT,
    )
    .await
    {
        Ok(v) => {
            // sweetGrass returns a full PROV-O braid object; extract the @id or data_hash
            let hash = v
                .get("data_hash")
                .or_else(|| v.get("@id"))
                .and_then(|h| h.as_str())
                .map(String::from);
            info!(braid_hash = ?hash, "evidence braid: sweetGrass braid created");
            hash
        }
        Err(e) => {
            warn!(error = %e, "evidence braid: sweetGrass braid creation failed");
            None
        }
    };

    // Step 7: Write .braided marker for incremental support
    let marker = collection_dir.join(BRAIDED_MARKER);
    let marker_data = json!({
        "collection": collection_name,
        "merkle_root": merkle_root,
        "braid_hash": braid_hash,
        "signature": signature,
        "spine_id": spine_id,
        "session_id": session_id,
        "files": file_count,
        "bytes": total_bytes,
        "committer": COMMITTER_DID,
        "braided_at": crate::utc_now_iso8601(),
        "braider": "membrane evidence.braid v1.0",
    });

    if let Err(e) = std::fs::write(
        &marker,
        serde_json::to_string_pretty(&marker_data).unwrap_or_default(),
    ) {
        warn!(
            error = %e,
            "evidence braid: failed to write marker {}",
            marker.display()
        );
    }

    Ok(BraidResult {
        files: file_count,
        bytes: total_bytes,
        merkle_root,
        braid_hash,
        spine_id,
        signature,
        session_id,
    })
}

// ── Manifest Parsing ────────────────────────────────────────────────

/// Parse entries from an `evidence-manifest.toml` file.
pub(super) fn parse_manifest(manifest_path: &Path) -> Vec<ManifestEntry> {
    let Ok(content) = std::fs::read_to_string(manifest_path) else {
        return vec![];
    };

    let Ok(table) = content.parse::<toml::Table>() else {
        return vec![];
    };

    let Some(files) = table.get("files").and_then(|f| f.as_array()) else {
        return vec![];
    };

    files
        .iter()
        .filter_map(|entry| {
            let t = entry.as_table()?;
            Some(ManifestEntry {
                path: t.get("path")?.as_str()?.to_string(),
                blake3: t.get("blake3")?.as_str()?.to_string(),
                size: t.get("size")?.as_integer()? as u64,
                mime: t
                    .get("mime")
                    .and_then(|m| m.as_str())
                    .unwrap_or("application/octet-stream")
                    .to_string(),
            })
        })
        .collect()
}

// ── Braids JSON Output ──────────────────────────────────────────────

/// Write a `braids.json` file alongside the evidence manifest.
///
/// This file is consumable by the detroit site for rendering provenance
/// badges on evidence pages.
pub(super) fn write_braids_json(
    evidence_dir: &Path,
    collection_name: &str,
    result: &BraidResult,
) -> std::io::Result<()> {
    let braids_data = json!({
        "collection": collection_name,
        "committer": COMMITTER_DID,
        "session_id": result.session_id,
        "braid_hash": result.braid_hash,
        "merkle_root": result.merkle_root,
        "spine_id": result.spine_id,
        "signature": result.signature,
        "files": result.files,
        "bytes": result.bytes,
        "braided_at": crate::utc_now_iso8601(),
        "convergence_depth": convergence_depth(result),
    });

    let braids_path = evidence_dir.join("braids.json");
    crate::atomic_write(
        &braids_path,
        serde_json::to_string_pretty(&braids_data)
            .unwrap_or_default()
            .as_bytes(),
    )
}

/// Calculate convergence depth from braid result.
///
/// Depth levels (from DISPERSAL_PATTERN.md):
/// 1 = CAS (BLAKE3 exists)
/// 2 = DAG (rhizoCrypt session recorded)
/// 3 = Spine (loamSpine permanent anchor)
/// 4 = Braid (sweetGrass PROV-O attribution)
/// 5 = Signed (bearDog Ed25519 witness)
fn convergence_depth(result: &BraidResult) -> u8 {
    if result.signature.is_some() {
        5
    } else if result.braid_hash.is_some() {
        4
    } else if result.spine_id.is_some() {
        3
    } else if result.merkle_root.is_some() {
        2
    } else {
        1
    }
}

// ── Braid Command Entry Point ───────────────────────────────────────

/// Braid evidence collections for a site.
///
/// Reads the `evidence-manifest.toml`, identifies unbraided collections,
/// and runs the provenance pipeline for each.
pub(crate) async fn braid_site_evidence(
    evidence_dir: &Path,
    collection_filter: Option<&str>,
) -> crate::error::Result<crate::ShadowOutcome> {
    if !is_trio_available() {
        return Ok(crate::ShadowOutcome::ok(
            "evidence braid: provenance trio not available (no dag.sock), skipping".to_string(),
        ));
    }

    let manifest_path = evidence_dir.join("evidence-manifest.toml");
    if !manifest_path.exists() {
        return Ok(crate::ShadowOutcome {
            ok: false,
            message: format!(
                "evidence braid: no manifest at {} — run evidence.manifest first",
                manifest_path.display()
            ),
            data: None,
        });
    }

    let entries = parse_manifest(&manifest_path);
    if entries.is_empty() {
        return Ok(crate::ShadowOutcome::ok(
            "evidence braid: manifest has no entries".to_string(),
        ));
    }

    // Discover evidence collections (subdirectories of evidence dir)
    let collections = discover_collections(evidence_dir, collection_filter);
    if collections.is_empty() {
        return Ok(crate::ShadowOutcome::ok(
            "evidence braid: no unbraided collections found".to_string(),
        ));
    }

    let mut results = Vec::new();
    let mut total_braided = 0usize;

    for (name, dir) in &collections {
        // Filter manifest entries for this collection
        let collection_entries: Vec<&ManifestEntry> = entries
            .iter()
            .filter(|e| e.path.starts_with(name))
            .collect();

        if collection_entries.is_empty() {
            continue;
        }

        // Build owned entries for the pipeline
        let owned_entries: Vec<ManifestEntry> = collection_entries
            .iter()
            .map(|e| ManifestEntry {
                path: e.path.clone(),
                blake3: e.blake3.clone(),
                size: e.size,
                mime: e.mime.clone(),
            })
            .collect();

        let source_desc = format!("Evidence collection: {name}");

        match braid_evidence_collection(name, dir, &owned_entries, &source_desc).await {
            Ok(result) => {
                // Write braids.json
                if let Err(e) = write_braids_json(evidence_dir, name, &result) {
                    warn!(error = %e, "evidence braid: failed to write braids.json");
                }

                let depth = convergence_depth(&result);
                results.push(json!({
                    "collection": name,
                    "status": "braided",
                    "files": result.files,
                    "bytes": result.bytes,
                    "merkle_root": result.merkle_root,
                    "braid_hash": result.braid_hash,
                    "spine_id": result.spine_id,
                    "convergence_depth": depth,
                }));
                total_braided += 1;
            }
            Err(e) => {
                warn!(collection = %name, error = %e, "evidence braid: collection failed");
                results.push(json!({
                    "collection": name,
                    "status": "failed",
                    "error": e.to_string(),
                }));
            }
        }
    }

    Ok(crate::ShadowOutcome {
        ok: total_braided > 0,
        message: format!(
            "evidence braid: {total_braided}/{} collections braided",
            collections.len()
        ),
        data: Some(json!({
            "collections": results,
            "braided": total_braided,
            "total": collections.len(),
        })),
    })
}

/// Discover evidence collections (subdirectories) that haven't been braided yet.
fn discover_collections(
    evidence_dir: &Path,
    filter: Option<&str>,
) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(evidence_dir) else {
        return vec![];
    };

    let mut collections = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }

        // Apply filter if specified
        if let Some(f) = filter {
            if name != f {
                continue;
            }
        }

        // Skip already-braided collections (unless forced via filter)
        if filter.is_none() && path.join(BRAIDED_MARKER).exists() {
            continue;
        }

        collections.push((name, path));
    }

    collections.sort_by(|a, b| a.0.cmp(&b.0));
    collections
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_socket_returns_path() {
        let path = resolve_socket("dag");
        assert!(path.to_string_lossy().contains("dag.sock"));
    }

    #[test]
    fn convergence_depth_levels() {
        let base = BraidResult {
            files: 1,
            bytes: 100,
            merkle_root: None,
            braid_hash: None,
            spine_id: None,
            signature: None,
            session_id: "test".into(),
        };
        assert_eq!(convergence_depth(&base), 1);

        let with_merkle = BraidResult {
            merkle_root: Some("abc".into()),
            ..BraidResult {
                files: 1,
                bytes: 100,
                merkle_root: None,
                braid_hash: None,
                spine_id: None,
                signature: None,
                session_id: "test".into(),
            }
        };
        assert_eq!(convergence_depth(&with_merkle), 2);

        let with_spine = BraidResult {
            merkle_root: Some("abc".into()),
            spine_id: Some("spine-1".into()),
            ..BraidResult {
                files: 1,
                bytes: 100,
                merkle_root: None,
                braid_hash: None,
                spine_id: None,
                signature: None,
                session_id: "test".into(),
            }
        };
        assert_eq!(convergence_depth(&with_spine), 3);

        let with_braid = BraidResult {
            braid_hash: Some("braid-1".into()),
            ..BraidResult {
                files: 1,
                bytes: 100,
                merkle_root: None,
                braid_hash: None,
                spine_id: None,
                signature: None,
                session_id: "test".into(),
            }
        };
        assert_eq!(convergence_depth(&with_braid), 4);

        let fully_signed = BraidResult {
            merkle_root: Some("abc".into()),
            spine_id: Some("spine-1".into()),
            braid_hash: Some("braid-1".into()),
            signature: Some("sig".into()),
            ..BraidResult {
                files: 1,
                bytes: 100,
                merkle_root: None,
                braid_hash: None,
                spine_id: None,
                signature: None,
                session_id: "test".into(),
            }
        };
        assert_eq!(convergence_depth(&fully_signed), 5);
    }

    #[test]
    fn parse_manifest_reads_entries() {
        let dir = std::env::temp_dir().join("membrane-provenance-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let manifest = r#"
file_count = 2
total_bytes = 300
generated = "2026-09-25T18:00:00Z"

[[files]]
path = "test.pdf"
blake3 = "abc123"
size = 100
mime = "application/pdf"

[[files]]
path = "sub/image.png"
blake3 = "def456"
size = 200
mime = "image/png"
"#;
        std::fs::write(dir.join("evidence-manifest.toml"), manifest).unwrap();

        let entries = parse_manifest(&dir.join("evidence-manifest.toml"));
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, "test.pdf");
        assert_eq!(entries[0].blake3, "abc123");
        assert_eq!(entries[0].size, 100);
        assert_eq!(entries[0].mime, "application/pdf");
        assert_eq!(entries[1].path, "sub/image.png");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn discover_collections_skips_hidden() {
        let dir = std::env::temp_dir().join("membrane-provenance-discover");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("mde-foia-sep25")).unwrap();
        std::fs::create_dir_all(dir.join(".hidden")).unwrap();
        std::fs::write(dir.join("file.txt"), b"not a dir").unwrap();

        let collections = discover_collections(&dir, None);
        assert_eq!(collections.len(), 1);
        assert_eq!(collections[0].0, "mde-foia-sep25");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn discover_collections_respects_filter() {
        let dir = std::env::temp_dir().join("membrane-provenance-filter");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("collection-a")).unwrap();
        std::fs::create_dir_all(dir.join("collection-b")).unwrap();

        let filtered = discover_collections(&dir, Some("collection-a"));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].0, "collection-a");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn discover_collections_skips_braided() {
        let dir = std::env::temp_dir().join("membrane-provenance-braided");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("braided-collection")).unwrap();
        std::fs::write(
            dir.join("braided-collection/.braided"),
            b"{}",
        )
        .unwrap();
        std::fs::create_dir_all(dir.join("fresh-collection")).unwrap();

        let collections = discover_collections(&dir, None);
        assert_eq!(collections.len(), 1);
        assert_eq!(collections[0].0, "fresh-collection");

        // With explicit filter, braided marker is ignored
        let forced = discover_collections(&dir, Some("braided-collection"));
        assert_eq!(forced.len(), 1);

        std::fs::remove_dir_all(&dir).ok();
    }
}
