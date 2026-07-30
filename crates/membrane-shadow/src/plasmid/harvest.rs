// SPDX-License-Identifier: AGPL-3.0-or-later

//! `plasmid.harvest` — Build primal binaries from source, verify, and stage.
//!
//! Implements the zero-touch binary harvest pipeline:
//! 1. Read `sources.toml` to discover repos + build args
//! 2. Compare HEAD commits against `provenance.toml` to detect drift
//! 3. Clone changed repos (shallow)
//! 4. Cross-compile for target triple (musl static)
//! 5. Compute BLAKE3 checksum
//! 6. Stage binary to plasmidBin depot
//! 7. Update `checksums.toml` and `provenance.toml`

use crate::ShadowOutcome;
use crate::error::{Result, ShadowError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use tracing::{info, warn};

use super::harvest_manifest::{
    apply_manifest_overrides, load_manifest_build_configs, resolve_local_source_dir,
};
use super::harvest_support::{format_harvest_outcome, notify_mesh_depot_updated};
use super::{detect_target_triple, nucleus_primals, toolchain};

/// Parsed CLI arguments for `plasmid.harvest`.
#[allow(clippy::struct_excessive_bools)]
pub struct HarvestArgs {
    /// Single primal to harvest (None = all with changes).
    pub primal: Option<String>,
    /// Force rebuild even if commit hasn't changed.
    pub force: bool,
    /// Show what would be built without executing.
    pub dry_run: bool,
    /// Override plasmidBin depot path.
    pub depot_dir: Option<String>,
    /// Override target triple (e.g. `aarch64-unknown-linux-musl` for cross-compile).
    pub target: Option<String>,
    /// Build from local workspace checkout instead of cloning.
    /// Uses `local_path` from the ecosystem manifest to resolve source dirs.
    /// ~10x faster than clone mode on machines with existing checkouts.
    pub local: bool,
    /// Push to remote depot after successful harvest (combines harvest + push).
    pub push: bool,
}

/// Outcome of harvesting a single primal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarvestResult {
    /// Binary name.
    pub binary: String,
    /// Outcome status.
    pub status: HarvestStatus,
    /// Human-readable detail.
    pub detail: String,
}

/// Status of a single primal harvest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HarvestStatus {
    /// Built and staged successfully.
    Built,
    /// No changes detected — skipped.
    Current,
    /// Build failed.
    Failed,
    /// Skipped (private repo without access, etc.).
    Skipped,
}

/// Source entry from `sources.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct SourceEntry {
    /// Repository path (e.g. `ecoPrimals/bearDog`).
    pub repo: String,
    /// Whether this is a private repo (SSH-only access).
    #[serde(default)]
    pub private: bool,
    /// Additional cargo build arguments.
    #[serde(default)]
    pub build_args: Option<String>,
    /// Override binary name (when it differs from primal name).
    #[serde(default)]
    pub binary_name: Option<String>,
    /// Whether this primal needs a glibc build for GPU/dlopen access.
    /// When true, harvest builds both musl and gnu targets.
    #[serde(default)]
    pub gpu: bool,
}

/// Provenance entry from `provenance.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvenanceEntry {
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
}

/// Full provenance file structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvenanceFile {
    #[serde(default)]
    pub generated: Option<String>,
    #[serde(default)]
    pub builder: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub rustc: Option<String>,
    #[serde(flatten)]
    pub entries: BTreeMap<String, ProvenanceEntry>,
}

pub use super::checksum::ChecksumEntry;

/// Compute which target triples to build for a given primal.
///
/// Priority: CLI `--target` > manifest `[build.<primal>].targets` >
/// host triple. GPU primals always include the gnu target for `dlopen`
/// support (CUDA/Vulkan), even when manifest targets are explicit.
fn targets_for_primal(
    cli_target: Option<&str>,
    source: &SourceEntry,
    manifest_targets: &[String],
) -> Vec<String> {
    if let Some(t) = cli_target {
        return vec![t.to_string()];
    }
    let mut targets = if manifest_targets.is_empty() {
        vec![detect_target_triple().to_string()]
    } else {
        manifest_targets.to_vec()
    };
    if source.gpu && cfg!(target_arch = "x86_64") {
        let gnu = cellmembrane_types::TargetArch::X86_64Gnu.triple();
        if !targets.iter().any(|t| t == gnu) {
            targets.push(gnu.to_string());
        }
    }
    targets
}

/// Harvest primals: detect changes, build, checksum, stage.
///
/// For GPU primals (`source.gpu = true`), builds both musl and gnu targets
/// so gates with GPU hardware can run CUDA/Vulkan workloads via `dlopen`.
pub async fn harvest(args: &HarvestArgs) -> Result<ShadowOutcome> {
    let depot_dir = resolve_depot(args.depot_dir.as_deref())?;
    let mut sources = load_sources(&depot_dir)?;
    super::depot::enrich_sources_from_manifest(&mut sources);
    let provenance = load_provenance(&depot_dir);

    let manifest_configs = load_manifest_build_configs();

    let primals_to_harvest = determine_primals(args, &sources)?;

    let mut results: Vec<HarvestResult> = Vec::new();
    let mut targets_built: Vec<String> = Vec::new();

    for primal in &primals_to_harvest {
        let Some(source) = sources.get(primal.as_str()) else {
            results.push(HarvestResult {
                binary: primal.clone(),
                status: HarvestStatus::Skipped,
                detail: "not in sources.toml".into(),
            });
            continue;
        };

        let mut source = source.clone();
        if let Some(mcfg) = manifest_configs.get(primal.as_str()) {
            apply_manifest_overrides(&mut source, mcfg);
        }

        let needs_rebuild = args.force
            || drift::has_upstream_changes(primal, &source, provenance.as_ref(), &depot_dir).await;

        if !needs_rebuild {
            results.push(HarvestResult {
                binary: primal.clone(),
                status: HarvestStatus::Current,
                detail: "commit unchanged".into(),
            });
            continue;
        }

        let manifest_linker = manifest_configs
            .get(primal.as_str())
            .and_then(|c| c.linker.as_deref());

        let manifest_targets = manifest_configs
            .get(primal.as_str())
            .map_or(&[][..], |c| &c.targets);
        let targets = targets_for_primal(args.target.as_deref(), &source, manifest_targets);
        for target in &targets {
            if args.dry_run {
                let mode = if args.local {
                    "build from local"
                } else {
                    "clone"
                };
                results.push(HarvestResult {
                    binary: primal.clone(),
                    status: HarvestStatus::Built,
                    detail: format!(
                        "dry-run: would {mode} {} and build for {target}",
                        source.repo
                    ),
                });
                continue;
            }

            let result = harvest_one(
                primal,
                &source,
                target,
                &depot_dir,
                manifest_linker,
                args.local,
            )
            .await;
            if matches!(result.status, HarvestStatus::Built) && !targets_built.contains(target) {
                targets_built.push(target.clone());
            }
            results.push(result);
        }
    }

    if !args.dry_run {
        finalize_depot(&results, &targets_built, &depot_dir).await;
    }

    let mut outcome = format_harvest_outcome(&results);

    if args.push && !args.dry_run {
        append_push_outcome(&mut outcome, &results, &depot_dir).await;
    }

    Ok(outcome)
}

/// Post-build: regenerate checksums from disk, update provenance, sign, publish.
///
/// Checksums are fully regenerated from all on-disk binaries rather than
/// partially merged, so stale entries are dropped and new binaries that
/// weren't in the build list are captured.
async fn finalize_depot(results: &[HarvestResult], _targets_built: &[String], depot_dir: &Path) {
    let built: Vec<&HarvestResult> = results
        .iter()
        .filter(|r| matches!(r.status, HarvestStatus::Built))
        .collect();
    if built.is_empty() {
        return;
    }
    let built_names: Vec<String> = built.iter().map(|r| r.binary.clone()).collect();

    match super::integrity::generate_checksums(depot_dir) {
        Ok(report) => info!(
            binaries = report.total_binaries,
            architectures = ?report.architectures,
            "checksums.toml regenerated from depot"
        ),
        Err(e) => warn!(error = %e, "failed to regenerate checksums.toml"),
    }

    if let Err(e) = update_provenance(depot_dir, &built).await {
        warn!(error = %e, "failed to update provenance");
    }

    if super::signing::sign_and_persist(depot_dir) {
        info!("depot signed (BLAKE3 + ed25519)");
    }

    drift::publish_depot_checksums(depot_dir).await;
    notify_mesh_depot_updated(&built_names).await;
}

/// If any primals were built, push depot to VPS and append the result.
async fn append_push_outcome(
    outcome: &mut ShadowOutcome,
    results: &[HarvestResult],
    depot_dir: &Path,
) {
    let any_built = results
        .iter()
        .any(|r| matches!(r.status, HarvestStatus::Built));
    if !any_built {
        return;
    }
    match super::depot_sync_push_standalone(depot_dir).await {
        Ok(push_result) => {
            outcome.message = format!("{}\npush: {}", outcome.message, push_result.message);
            if !push_result.ok {
                outcome.ok = false;
            }
        }
        Err(e) => {
            outcome.message = format!("{}\npush: failed — {e}", outcome.message);
            outcome.ok = false;
        }
    }
}

fn determine_primals(
    args: &HarvestArgs,
    sources: &BTreeMap<String, SourceEntry>,
) -> Result<Vec<String>> {
    if let Some(name) = args.primal.as_deref() {
        if !sources.contains_key(name) {
            return Err(ShadowError::Config(format!(
                "'{name}' not found in sources.toml"
            )));
        }
        Ok(vec![name.to_string()])
    } else {
        let registry_primals = nucleus_primals();
        Ok(registry_primals
            .into_iter()
            .filter(|p| sources.contains_key(*p))
            .map(ToString::to_string)
            .collect())
    }
}

use super::drift;

async fn harvest_one(
    primal: &str,
    source: &SourceEntry,
    target: &str,
    depot_dir: &Path,
    manifest_linker: Option<&str>,
    local: bool,
) -> HarvestResult {
    let (source_dir, cleanup) = if local {
        match resolve_local_source_dir(primal) {
            Ok(dir) => {
                info!(primal, path = %dir.display(), "local harvest: using workspace checkout");
                (dir, false)
            }
            Err(e) => {
                return HarvestResult {
                    binary: primal.into(),
                    status: HarvestStatus::Failed,
                    detail: e.to_string(),
                };
            }
        }
    } else {
        let build_root = std::env::temp_dir().join("membrane-harvest");
        let clone_dir = build_root.join(primal);

        if let Err(e) = drift::clone_source(primal, source, &build_root, &clone_dir).await {
            let status = if source.private {
                HarvestStatus::Skipped
            } else {
                HarvestStatus::Failed
            };
            return HarvestResult {
                binary: primal.into(),
                status,
                detail: e.to_string(),
            };
        }
        (clone_dir, true)
    };

    let head_commit = crate::git_ops::head_short(&source_dir)
        .await
        .unwrap_or_default();

    if !local {
        if let Some(warning) =
            drift::check_clone_freshness(primal, source, &source_dir, &head_commit).await
        {
            warn!(primal, warning, "freshness warning");
        }
    }

    if let Err(e) = toolchain::build_binary(source, target, &source_dir, manifest_linker).await {
        return HarvestResult {
            binary: primal.into(),
            status: HarvestStatus::Failed,
            detail: e.to_string(),
        };
    }

    let binary_name = source.binary_name.as_deref().unwrap_or(primal);
    let bin_path = source_dir
        .join("target")
        .join(target)
        .join("release")
        .join(binary_name);

    if !bin_path.exists() {
        return HarvestResult {
            binary: primal.into(),
            status: HarvestStatus::Failed,
            detail: format!("binary not found at {}", bin_path.display()),
        };
    }

    if let Err(e) = validate_elf_arch(&bin_path, target).await {
        return HarvestResult {
            binary: primal.into(),
            status: HarvestStatus::Failed,
            detail: e.to_string(),
        };
    }

    toolchain::strip_binary(&bin_path, primal, target).await;

    match stage_to_depot_async(primal, &bin_path, depot_dir, target).await {
        Ok((size, blake3)) => {
            if cleanup {
                let _ = tokio::fs::remove_dir_all(&source_dir).await;
            }
            let mode = if local { "local" } else { "clone" };
            HarvestResult {
                binary: primal.into(),
                status: HarvestStatus::Built,
                detail: format!(
                    "{}KB blake3={} commit={} ({mode})",
                    size / 1024,
                    &blake3[..16],
                    &head_commit[..std::cmp::min(8, head_commit.len())]
                ),
            }
        }
        Err(e) => HarvestResult {
            binary: primal.into(),
            status: HarvestStatus::Failed,
            detail: e.to_string(),
        },
    }
}

pub(super) use toolchain::{
    ANDROID_TARGET, ENV_ANDROID_NDK_HOME, resolve_ndk_linker, validate_elf_arch,
};

/// Async depot staging: copy binary → atomic rename → BLAKE3 checksum.
/// Shared by both `plasmid.build` and `plasmid.harvest`.
pub(super) async fn stage_to_depot_async(
    primal: &str,
    bin_path: &Path,
    depot_dir: &Path,
    target: &str,
) -> crate::Result<(u64, String)> {
    let staging_dir = depot_dir.join("primals").join(target);
    tokio::fs::create_dir_all(&staging_dir).await.map_err(|e| {
        crate::error::ShadowError::Build(format!("depot staging dir create failed: {e}"))
    })?;
    let dest = staging_dir.join(primal);
    let tmp = staging_dir.join(format!(".{primal}.new"));

    tokio::fs::copy(bin_path, &tmp)
        .await
        .map_err(|e| crate::error::ShadowError::Build(format!("copy to depot failed: {e}")))?;
    tokio::fs::rename(&tmp, &dest)
        .await
        .map_err(|e| crate::error::ShadowError::Build(format!("atomic rename failed: {e}")))?;

    let size = tokio::fs::metadata(&dest).await.map_or(0, |m| m.len());
    let blake3 = super::compute_blake3_file_async(dest).await?;
    Ok((size, blake3))
}

pub(super) use super::depot::{load_provenance, load_sources, resolve_depot, update_provenance};

#[cfg(test)]
#[path = "harvest_tests.rs"]
mod tests;
