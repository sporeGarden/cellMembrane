// SPDX-License-Identifier: AGPL-3.0-or-later

//! Plasmid binary lifecycle — fetch, refresh, harvest, and deploy primal binaries.
//!
//! Manages the binary supply chain for membrane services:
//! - `fetch` — Download binaries from sovereign or external sources (GitHub, VPS, Forgejo)
//! - `harvest` — Build from source, detect changes, checksum, stage to depot
//! - `refresh` — Push local pre-built binaries to VPS with atomic replacement
//! - `pipeline` — End-to-end zero-touch: harvest → refresh → alive
//! - `status` — Report depot freshness and drift against upstream
//!
//! BLAKE3 checksums are verified in-process using the `blake3` crate.

pub mod auto_fetch;
pub mod build;
pub(crate) mod canary;
pub(crate) mod canary_remote;
pub(crate) mod checksum;
mod commands;
pub(crate) mod depot;
mod depot_sync;
mod download;
mod drift;
mod fetch;
mod harvest;
mod harvest_manifest;
mod harvest_support;
pub(crate) mod integrity;
pub(crate) mod lineage;
mod refresh;
pub(crate) mod sandbox;
pub(crate) mod scheduler;
pub(crate) mod signing;
mod signing_crypto;
pub(crate) mod toolchain;

pub use build::BuildArgs;
pub use checksum::fetch_wan_checksums;
pub use fetch::*;
pub use harvest::{HarvestArgs, harvest};
pub use refresh::{RefreshArgs, refresh};

pub use commands::{pipeline, status, trigger};
pub use depot::StalenessReport;
pub use depot_sync::depot_sync;
pub(crate) use depot_sync::depot_sync_push_standalone;
pub(crate) use lineage::{LineageResult, validate_lineage};

/// Gracefully stop a process: SIGTERM → grace period → SIGKILL.
///
/// Uses platform substrate for cross-platform process control.
pub(crate) async fn graceful_kill(pid: u32, grace_ms: u64) {
    if !cellmembrane_types::is_process_alive(pid) {
        return;
    }
    if !cellmembrane_types::kill_process(pid) {
        tracing::debug!(pid, "SIGTERM failed");
    }
    tokio::time::sleep(std::time::Duration::from_millis(grace_ms)).await;
    if cellmembrane_types::is_process_alive(pid) && !cellmembrane_types::force_kill_process(pid) {
        tracing::debug!(pid, "SIGKILL failed");
    }
}

/// Compute BLAKE3 hash of a file, returning hex string.
pub(crate) fn compute_blake3_file(path: &std::path::Path) -> crate::error::Result<String> {
    depot::compute_blake3_file(path)
}

/// Async variant — runs the full-file BLAKE3 read on a blocking thread.
pub(crate) async fn compute_blake3_file_async(
    path: impl AsRef<std::path::Path>,
) -> crate::error::Result<String> {
    let path = path.as_ref().to_path_buf();
    tokio::task::spawn_blocking(move || depot::compute_blake3_file(&path))
        .await
        .map_err(|e| {
            crate::error::ShadowError::Io(std::io::Error::other(format!(
                "BLAKE3 hash task panicked: {e}"
            )))
        })?
}

/// Ensure a directory pair exists (socket dir + binary staging dir).
///
/// Explicitly sets 0755 permissions on unix to avoid umask-dependent modes
/// that would prevent non-root processes from traversing the socket dir.
pub(crate) async fn ensure_staging_dirs(
    socket_dir: &std::path::Path,
    bin_dir: &std::path::Path,
) -> crate::Result<()> {
    for dir in [socket_dir, bin_dir] {
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|e| crate::error::ShadowError::build(format!("create dir: {e}")))?;
        let _ = crate::platform::apply_access_async(
            cellmembrane_types::PlatformAccess::Executable,
            dir,
        )
        .await;
    }
    Ok(())
}

/// Copy a binary to a staging path and set executable permissions.
pub(crate) async fn stage_binary(
    source: &std::path::Path,
    dest: &std::path::Path,
) -> crate::Result<()> {
    tokio::fs::copy(source, dest)
        .await
        .map_err(|e| crate::error::ShadowError::build(format!("stage binary: {e}")))?;
    crate::platform::apply_access_async(cellmembrane_types::PlatformAccess::Executable, dest)
        .await
        .map_err(|e| crate::error::ShadowError::build(format!("set executable: {e}")))?;
    Ok(())
}

/// Spawn a primal process on an isolated socket.
///
/// Resolves the `ServerContract` from the service registry so that broker
/// primals (e.g. biomeOS with `BiomeosApi`) get the correct subcommand
/// (`neural-api`) instead of the generic `server`.
pub(super) fn spawn_primal_server(
    binary: &std::path::Path,
    socket: &std::path::Path,
    extra_args: &[(&str, &std::path::Path)],
) -> crate::Result<tokio::process::Child> {
    let raw_name = binary.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let bin_name = strip_sandbox_suffix(raw_name);

    let svc = cellmembrane_types::MembraneService::for_binary(bin_name);
    if svc.is_none() {
        tracing::warn!(
            binary = bin_name,
            "binary not in service registry — defaulting to `server --socket`"
        );
    }
    let (subcmd, socket_flag) = match svc.map(|s| s.server_contract) {
        Some(cellmembrane_types::service::ServerContract::BiomeosApi) => ("neural-api", "--socket"),
        Some(cellmembrane_types::service::ServerContract::ServerNoSocket) => ("server", ""),
        Some(cellmembrane_types::service::ServerContract::External) => ("", ""),
        _ => ("server", "--socket"),
    };

    let mut cmd = tokio::process::Command::new(binary);
    if !subcmd.is_empty() {
        cmd.arg(subcmd);
    }
    if !socket_flag.is_empty() {
        cmd.arg(socket_flag).arg(socket);
    }
    for (flag, path) in extra_args {
        cmd.arg(flag).arg(path);
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| crate::error::ShadowError::build(format!("spawn {bin_name}: {e}")))
}

/// Strip sandbox commit suffix from a binary name.
///
/// Sandbox stages binaries as `{primal}-{commit_short}` (e.g. `biomeos-abc12345`).
/// This strips the suffix so registry lookup finds the correct `ServerContract`.
/// If the name has no recognizable suffix (not hex, <6 chars), returns as-is.
fn strip_sandbox_suffix(name: &str) -> &str {
    if let Some((base, suffix)) = name.rsplit_once('-') {
        let is_hex_suffix = suffix.len() >= 6 && suffix.bytes().all(|b| b.is_ascii_hexdigit());
        if is_hex_suffix {
            return base;
        }
    }
    name
}

/// Detect primals where source HEAD has advanced past depot provenance.
///
/// Compares each primal's upstream HEAD (Forgejo/GitHub `ls-remote`) against the
/// recorded commit in `provenance.toml`. Returns names of drifted primals.
/// Used by `post_sync` commit drift detection (Phase 2 sovereign pipeline).
pub(crate) async fn detect_commit_drift() -> Vec<String> {
    let Ok(depot_dir) = depot::resolve_depot(None) else {
        return Vec::new();
    };
    let provenance = depot::load_provenance(&depot_dir);
    let Ok(sources) = depot::load_sources(&depot_dir) else {
        return Vec::new();
    };

    let mut drifted = Vec::new();
    for name in nucleus_primals() {
        if let Some(source) = sources.get(name) {
            if drift::has_upstream_changes(name, source, provenance.as_ref(), &depot_dir).await {
                drifted.push(name.to_string());
            }
        }
    }
    drifted
}

/// Detect stale primals in the depot. Resolves depot path from env/defaults.
pub fn detect_depot_staleness() -> crate::error::Result<StalenessReport> {
    let depot_dir = depot::resolve_depot(None)?;
    depot::detect_stale_primals(&depot_dir)
}

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

/// Resolve the primal set for a gate from the manifest composition.
///
/// Resolution chain:
///   1. If `gate` has a `composition` field in the manifest, and that
///      composition is defined in `[compositions]`, use its `primals` list.
///   2. Otherwise fall back to the full registry (`nucleus_primals()`).
///
/// This enables composition-aware operations: a thin-relay gate fetches
/// only songBird + nestGate, while a full NUCLEUS gate gets all 13.
///
/// Uses a process-level cache keyed by gate name. Each gate resolves
/// once and is cached for all subsequent calls within the process.
pub(crate) fn resolve_gate_primals(gate: &str) -> Vec<String> {
    use std::collections::BTreeMap;
    use std::sync::Mutex;
    use std::sync::OnceLock;

    static CACHE: OnceLock<Mutex<BTreeMap<String, Vec<String>>>> = OnceLock::new();

    let cache = CACHE.get_or_init(|| Mutex::new(BTreeMap::new()));
    let guard = cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    if let Some(primals) = guard.get(gate) {
        return primals.clone();
    }
    drop(guard);

    let primals = resolve_primals_from_manifest(gate);

    let mut guard = cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard.entry(gate.to_string()).or_insert(primals).clone()
}

fn resolve_primals_from_manifest(gate: &str) -> Vec<String> {
    let workspace = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_ECOPRIMALS_ROOT,
        cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT,
    );
    crate::manifest::load_from_workspace(std::path::Path::new(&workspace))
        .ok()
        .and_then(|manifest| {
            let profile = manifest.gate_composition(gate)?;
            if profile.primals.is_empty() {
                None
            } else {
                Some(profile.primals.clone())
            }
        })
        .unwrap_or_else(|| nucleus_primals().into_iter().map(String::from).collect())
}

/// Detect the local platform's default Rust target triple.
///
/// Uses compile-time `Platform::detect()` which resolves OS, arch, and link
/// model from `cfg!` directives — correct for both native and cross-compiled
/// binaries (e.g. `x86_64-pc-windows-gnu` when `--target` is set).
pub(crate) const fn detect_target_triple() -> &'static str {
    cellmembrane_types::Platform::detect().triple()
}

/// Check NDK toolchain availability for Android cross-compilation.
///
/// Reports whether `ANDROID_NDK_HOME` is set, the linker exists, and
/// the `aarch64-linux-android` Rust target is installed.
#[must_use]
pub fn ndk_check() -> crate::ShadowOutcome {
    let ndk_home = std::env::var(harvest::ENV_ANDROID_NDK_HOME).ok();
    let linker = harvest::resolve_ndk_linker();

    let target_installed = std::process::Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains(harvest::ANDROID_TARGET));

    let all_ok = ndk_home.is_some() && linker.is_some() && target_installed;

    let linker_str = linker
        .as_ref()
        .map_or_else(|| "NOT FOUND".to_string(), |p| p.display().to_string());

    let msg = format!(
        "NDK check: {}\n  ANDROID_NDK_HOME: {}\n  linker: {linker_str}\n  rustup target: {}",
        if all_ok { "READY" } else { "NOT READY" },
        ndk_home.as_deref().unwrap_or("NOT SET"),
        if target_installed {
            "installed"
        } else {
            "MISSING (run: rustup target add aarch64-linux-android)"
        },
    );

    crate::ShadowOutcome {
        ok: all_ok,
        message: msg,
        data: Some(serde_json::json!({
            "ndk_home": ndk_home,
            "linker": linker.map(|p| p.display().to_string()),
            "target_installed": target_installed,
            "target": harvest::ANDROID_TARGET,
        })),
    }
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

/// Publish `depot.build_pending` to the mesh so consumer gates know binaries
/// are stale and a rebuild is in progress. Consumers should delay fetch until
/// `depot.updated` arrives (or a timeout expires).
///
/// Publish a depot event to the local songBird mesh.
///
/// Sends `mesh.publish { topic, payload }` via the local songBird UDS socket.
/// Failures are non-fatal — tracing log ensures observability even without mesh.
pub(crate) async fn notify_mesh(topic: &str, primals_key: &str, primals: &[String]) {
    tracing::info!(
        event = topic,
        primals = ?primals,
        "{topic} — {} primals",
        primals.len()
    );

    let socket_path = std::path::PathBuf::from(crate::gate::sockets::resolve_mesh_relay_socket());

    if !socket_path.exists() {
        tracing::debug!("mesh.publish {topic} skipped — mesh relay socket not found");
        return;
    }

    let gate = crate::gate::resolve_local_gate_identity();

    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "mesh.publish",
        "params": {
            "topic": topic,
            "payload": {
                primals_key: primals,
                "builder": gate,
            }
        },
        "id": 1
    });

    let request_str = request.to_string();
    match crate::jsonrpc::send_notify(&socket_path, &request_str).await {
        Ok(()) => {
            tracing::info!("mesh.publish {topic} sent (fire-and-forget)");
        }
        Err(e) => {
            tracing::warn!("mesh.publish {topic} failed (non-fatal): {e}");
        }
    }
}

/// Notify mesh that primals are queued for rebuild.
pub(crate) async fn notify_mesh_build_pending(drifted: &[String]) {
    notify_mesh("depot.build_pending", "primals_pending", drifted).await;
}

/// Register all capabilities of running primals with the mesh (songBird).
///
/// Iterates the service registry, collects capability wire names for each
/// primal that has a socket (i.e. is expected to be running), and publishes
/// a `capability.register` message per primal. This is the cellMembrane-side
/// implementation of the westGate self-registration pattern.
pub(crate) async fn register_capabilities_with_mesh() -> crate::ShadowOutcome {
    let gate = crate::gate::resolve_local_gate_identity();
    let socket_base = std::path::PathBuf::from(cellmembrane_types::service::resolve_socket_base());
    let relay_socket = std::path::PathBuf::from(crate::gate::sockets::resolve_mesh_relay_socket());

    if !relay_socket.exists() {
        return crate::ShadowOutcome::fail("mesh relay socket not found — songBird not running");
    }

    let mut registered = 0u32;
    let mut skipped = 0u32;

    for svc in cellmembrane_types::MembraneService::all() {
        if svc.capabilities.is_empty() || !svc.has_socket {
            continue;
        }

        let socket_path = socket_base.join(
            cellmembrane_types::service::constants::socket_filename(svc.binary),
        );
        let api_socket = svc.api_socket.map(|name| {
            socket_base.join(format!(
                "{name}-default{}",
                cellmembrane_types::service::constants::SOCKET_SUFFIX
            ))
        });

        let is_running = socket_path.exists() || api_socket.as_ref().is_some_and(|p| p.exists());

        if !is_running {
            skipped += 1;
            continue;
        }

        let caps: Vec<&str> = svc.capabilities.iter().map(|c| c.wire_name()).collect();

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "mesh.publish",
            "params": {
                "topic": "capability.register",
                "payload": {
                    "gate": gate,
                    "primal": svc.binary,
                    "capabilities": caps,
                }
            },
            "id": 1
        });

        let request_str = request.to_string();
        match crate::jsonrpc::send_notify(&relay_socket, &request_str).await {
            Ok(()) => {
                tracing::info!(
                    primal = svc.binary,
                    capabilities = ?caps,
                    "capability.register sent"
                );
                registered += 1;
            }
            Err(e) => {
                tracing::warn!(
                    primal = svc.binary,
                    error = %e,
                    "capability.register failed"
                );
            }
        }
    }

    crate::ShadowOutcome::ok(format!(
        "capability.register: {registered} primals registered, {skipped} skipped (not running) on gate {gate}"
    ))
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
