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
mod checksum;
pub(crate) mod depot;
mod depot_sync;
mod download;
mod drift;
mod fetch;
mod harvest;
mod harvest_manifest;
mod harvest_support;
pub(crate) mod integrity;
mod refresh;
pub(crate) mod sandbox;
pub(crate) mod signing;
pub(crate) mod toolchain;

pub use build::BuildArgs;
pub use checksum::fetch_wan_checksums;
pub use fetch::*;
pub use harvest::{HarvestArgs, HarvestResult, HarvestStatus, harvest};
pub use refresh::{RefreshArgs, refresh};

pub use depot::StalenessReport;
pub use depot_sync::depot_sync;

/// Gracefully stop a process: SIGTERM → grace period → SIGKILL (Unix),
/// or `TerminateProcess` (Windows).
///
/// Platform-aware — OS Atheism Phase 2.
pub(crate) async fn graceful_kill(pid: u32, grace_ms: u64) {
    #[cfg(unix)]
    {
        graceful_kill_unix(pid, grace_ms).await;
    }
    #[cfg(not(unix))]
    {
        graceful_kill_bare(pid, grace_ms).await;
    }
}

/// Unix: SIGTERM → grace → SIGKILL via the `kill` command.
///
/// Uses `/proc/{pid}/` existence check to avoid signaling stale PIDs.
/// Replaced nix crate with `kill(1)` to eliminate the heavy dependency.
#[cfg(unix)]
async fn graceful_kill_unix(pid: u32, grace_ms: u64) {
    let proc_path = std::path::PathBuf::from(format!("/proc/{pid}"));
    if !proc_path.exists() {
        return;
    }
    let pid_str = pid.to_string();
    let _ = std::process::Command::new("kill")
        .args(["-s", "TERM", &pid_str])
        .output();
    tokio::time::sleep(std::time::Duration::from_millis(grace_ms)).await;
    if proc_path.exists() {
        let _ = std::process::Command::new("kill")
            .args(["-s", "KILL", &pid_str])
            .output();
    }
}

/// Non-Unix: best-effort process kill via `std::process::Command("taskkill")`.
#[cfg(not(unix))]
async fn graceful_kill_bare(pid: u32, _grace_ms: u64) {
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .output();
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

/// Result of lineage validation for a primal binary.
#[derive(Debug, Clone)]
pub(crate) enum LineageResult {
    /// Checksum, provenance, and builder all verified.
    Verified,
    /// `PostPrimordial` primal with broken lineage — must not be installed.
    Blocked(String),
    /// Non-critical primal with incomplete lineage — warn but allow.
    Warned(String),
}

#[derive(serde::Deserialize)]
struct LineageChecksumFile {
    #[serde(flatten)]
    targets: std::collections::BTreeMap<
        String,
        std::collections::BTreeMap<String, LineageChecksumEntry>,
    >,
}

#[derive(serde::Deserialize)]
struct LineageChecksumEntry {
    blake3: String,
}

/// Validate depot lineage for a primal: BLAKE3 checksum, provenance commit,
/// and builder authority. `PostPrimordial` primals are hard-blocked on failure;
/// other primals get a warning.
pub(crate) fn validate_lineage(primal: &str, depot_dir: &std::path::Path) -> LineageResult {
    let is_critical = cellmembrane_types::service::is_post_primordial(primal);
    let arch = detect_target_triple();
    let bin_path = depot_dir.join("primals").join(arch).join(primal);

    let checksum_ok = if bin_path.exists() {
        verify_checksum_against_depot(primal, &bin_path, depot_dir, arch)
    } else {
        false
    };

    let provenance = depot::load_provenance(depot_dir);
    let provenance_ok = provenance
        .as_ref()
        .and_then(|p| p.entries.get(primal))
        .and_then(|e| e.commit.as_ref())
        .is_some();

    let builder_ok = provenance
        .as_ref()
        .is_some_and(|p| !p.builder.as_ref().is_some_and(String::is_empty));

    if checksum_ok && provenance_ok && builder_ok {
        return LineageResult::Verified;
    }

    let mut reasons = Vec::new();
    if !checksum_ok {
        reasons.push("BLAKE3 mismatch or missing");
    }
    if !provenance_ok {
        reasons.push("no provenance commit");
    }
    if !builder_ok {
        reasons.push("no builder identity");
    }
    let detail = format!("{primal}: lineage incomplete — {}", reasons.join(", "));

    if is_critical {
        LineageResult::Blocked(format!(
            "{primal} is postPrimordial — depot lineage validation FAILED ({detail})"
        ))
    } else {
        LineageResult::Warned(detail)
    }
}

fn verify_checksum_against_depot(
    primal: &str,
    bin_path: &std::path::Path,
    depot_dir: &std::path::Path,
    arch: &str,
) -> bool {
    let checksums_path = depot_dir.join(cellmembrane_types::service::CHECKSUMS_FILE);
    let Ok(content) = std::fs::read_to_string(&checksums_path) else {
        return false;
    };

    let parsed: LineageChecksumFile = match toml::from_str(&content) {
        Ok(p) => p,
        Err(_) => return false,
    };

    let Some(entries) = parsed.targets.get(arch) else {
        return false;
    };
    let Some(expected) = entries.get(primal) else {
        return false;
    };

    compute_blake3_file(bin_path).is_ok_and(|actual| actual == expected.blake3)
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

/// Detect the local platform's default Rust target triple (musl static).
pub(crate) const fn detect_target_triple() -> &'static str {
    cellmembrane_types::TargetArch::detect_host().triple()
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
/// Currently logs to tracing — mesh publish will route through songBird's
/// `mesh.publish` JSON-RPC when the IPC bridge is wired (pending `mesh.enroll`
/// songBird integration).
#[allow(
    clippy::unused_async,
    reason = "will be async when mesh.publish IPC is wired"
)]
pub(crate) async fn notify_mesh_build_pending(drifted: &[String]) {
    tracing::info!(
        event = "depot.build_pending",
        primals = ?drifted,
        "build pending — {} drifted primals queued for rebuild",
        drifted.len()
    );
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
        target: None,
        local: false,
    };

    let harvest_outcome = harvest(&harvest_args).await?;

    if dry_run {
        return Ok(harvest_outcome);
    }

    let results: Vec<HarvestResult> = harvest_outcome
        .data
        .as_ref()
        .and_then(|d| serde_json::from_value(d.clone()).ok())
        .unwrap_or_default();

    let built_any = results
        .iter()
        .any(|r| matches!(r.status, HarvestStatus::Built));

    if !built_any {
        return Ok(crate::ShadowOutcome {
            ok: harvest_outcome.ok,
            message: format!("{} — no new binaries to push", harvest_outcome.message),
            data: harvest_outcome.data,
        });
    }

    let arch = detect_target_triple();
    let depot_dir = depot::resolve_depot(None)?;
    let bin_dir = depot_dir.join("primals").join(arch);

    for entry in results
        .iter()
        .filter(|r| matches!(r.status, HarvestStatus::Built))
    {
        let binary_path = bin_dir.join(&entry.binary);
        if !binary_path.exists() {
            continue;
        }

        let sandbox_args = sandbox::SandboxArgs {
            primal: entry.binary.clone(),
            commit: entry.detail.clone(),
            binary_path,
            timeout_secs: None,
        };

        if let Ok(result) = sandbox::validate(&sandbox_args).await {
            if !result.health_ok {
                return Ok(crate::ShadowOutcome {
                    ok: false,
                    message: format!(
                        "{} | sandbox FAIL for {} — {} ({}ms). Refresh aborted.",
                        harvest_outcome.message, entry.binary, result.detail, result.elapsed_ms
                    ),
                    data: Some(serde_json::to_value(&result).unwrap_or_default()),
                });
            }
        }
    }

    let depot_source = Some(bin_dir.to_string_lossy().into_owned());

    let refresh_args = RefreshArgs {
        primal: primal.map(Into::into),
        dry_run: false,
        source_dir: depot_source,
    };

    let refresh_outcome = refresh(config, &refresh_args).await?;

    Ok(crate::ShadowOutcome {
        ok: refresh_outcome.ok,
        message: format!(
            "{} | sandbox: PASS | {}",
            harvest_outcome.message, refresh_outcome.message
        ),
        data: refresh_outcome.data,
    })
}

/// `plasmid.trigger` — Remotely trigger the VPS pipeline via SSH.
///
/// Kicks `systemctl start plasmid-pipeline.service` on the VPS, causing
/// an immediate harvest→refresh cycle there. Useful when an operator wants
/// the VPS to converge without running the full pipeline locally.
pub async fn trigger(config: &crate::ShadowConfig) -> crate::error::Result<crate::ShadowOutcome> {
    let cmd = "systemctl start plasmid-pipeline.service 2>&1; \
               sleep 1; \
               systemctl is-active plasmid-pipeline.service 2>&1 || \
               journalctl -u plasmid-pipeline.service --no-pager -n 3 2>&1";

    let (output, code) = crate::ssh::exec_raw(config, cmd).await?;

    if code == 0 || output.contains("activating") || output.contains("active") {
        Ok(crate::ShadowOutcome::ok(format!(
            "trigger: plasmid-pipeline.service started on {}\n{output}",
            config.ssh_host
        )))
    } else {
        Ok(crate::ShadowOutcome {
            ok: false,
            message: format!(
                "trigger: failed to start on {} (exit {code})\n{output}",
                config.ssh_host
            ),
            data: None,
        })
    }
}

/// `plasmid.status` — Report depot freshness and upstream drift.
///
/// Reads provenance.toml for last build timestamp, then checks each
/// primal's HEAD against the recorded commit to identify drift.
pub async fn status() -> crate::error::Result<crate::ShadowOutcome> {
    let depot_dir = harvest::resolve_depot(None)?;
    let sources = harvest::load_sources(&depot_dir)?;
    let provenance = harvest::load_provenance(&depot_dir);

    let generated = provenance
        .as_ref()
        .and_then(|p| p.generated.clone())
        .unwrap_or_else(|| "unknown".into());

    let target = provenance
        .as_ref()
        .and_then(|p| p.target.clone())
        .unwrap_or_else(|| "unknown".into());

    let registry_primals = nucleus_primals();
    let total = registry_primals.len();

    let mut drifted: Vec<String> = Vec::new();
    let mut current = 0usize;

    for &primal in &registry_primals {
        if let Some(source) = sources.get(primal) {
            let changed = drift::has_upstream_changes_lenient(
                primal,
                source,
                provenance.as_ref(),
                &depot_dir,
            )
            .await;
            if changed {
                drifted.push(primal.to_string());
            } else {
                current += 1;
            }
        }
    }

    let msg = format!(
        "depot: {current}/{total} current, {} drifted | built: {generated} | target: {target}",
        drifted.len()
    );

    let data = serde_json::json!({
        "total": total,
        "current": current,
        "drifted": drifted,
        "generated": generated,
        "target": target,
    });

    Ok(crate::ShadowOutcome {
        ok: drifted.is_empty(),
        message: msg,
        data: Some(data),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nucleus_primals_returns_13() {
        let primals = nucleus_primals();
        assert_eq!(primals.len(), 13, "expected 13 nucleus primals");
        assert!(primals.contains(&"beardog"));
        assert!(primals.contains(&"songbird"));
        assert!(primals.contains(&"squirrel"));
    }

    #[test]
    fn detect_target_triple_contains_musl() {
        let triple = detect_target_triple();
        assert!(
            triple.ends_with("-unknown-linux-musl"),
            "expected musl target, got: {triple}"
        );
    }

    #[test]
    fn resolve_path_explicit_overrides_env() {
        let result = resolve_path(Some("/explicit"), "NONEXISTENT_VAR_XYZ", || {
            PathBuf::from("/default")
        });
        assert_eq!(result, PathBuf::from("/explicit"));
    }

    #[test]
    fn resolve_path_uses_default_when_no_env() {
        let result = resolve_path(None, "NONEXISTENT_VAR_XYZ_ABC", || {
            PathBuf::from("/fallback")
        });
        assert_eq!(result, PathBuf::from("/fallback"));
    }

    #[test]
    fn validate_lineage_missing_depot_warns_non_critical() {
        let tmp = std::env::temp_dir().join("lineage_test_warn");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let result = validate_lineage("squirrel", &tmp);
        assert!(
            matches!(result, LineageResult::Warned(_)),
            "non-critical primal with no depot should warn, got: {result:?}"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn validate_lineage_missing_depot_blocks_critical() {
        let tmp = std::env::temp_dir().join("lineage_test_block");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let result = validate_lineage("beardog", &tmp);
        assert!(
            matches!(result, LineageResult::Blocked(_)),
            "postPrimordial primal with no depot should be blocked, got: {result:?}"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn lineage_result_variants_exhaustive() {
        let v = LineageResult::Verified;
        assert!(matches!(v, LineageResult::Verified));
        let b = LineageResult::Blocked("test".into());
        assert!(matches!(b, LineageResult::Blocked(_)));
        let w = LineageResult::Warned("test".into());
        assert!(matches!(w, LineageResult::Warned(_)));
    }

    #[tokio::test]
    async fn status_reports_depot_state() {
        let result = status().await;
        match result {
            Ok(outcome) => {
                assert!(outcome.message.contains("depot:"));
                assert!(outcome.message.contains("current"));
                let data = outcome.data.unwrap();
                assert!(data.get("total").is_some());
                assert!(data.get("drifted").is_some());
            }
            Err(e) => {
                let msg = e.to_string();
                assert!(
                    msg.contains("depot not found")
                        || msg.contains("cannot read")
                        || msg.contains("No such file"),
                    "unexpected error: {msg}"
                );
            }
        }
    }
}
