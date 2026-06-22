// SPDX-License-Identifier: AGPL-3.0-or-later

//! `plasmid.build` — guideStone-grade single-primal build pipeline.
//!
//! Replaces shell build scripts with deterministic Rust orchestration:
//! 1. Clone source (ephemeral staging)
//! 2. Build for target triple
//! 3. Validate ELF architecture matches target (BUILD-ELF-01)
//! 4. Strip symbols (target-aware)
//! 5. Compute BLAKE3
//! 6. Stage to depot with atomic rename
//! 7. Update provenance + checksums
//!
//! guideStone properties:
//! - P1 (Deterministic): same source + target = same binary
//! - P2 (Reference-Traceable): provenance.toml with commit, rustc, timestamp, blake3
//! - P3 (Self-Verifying): BLAKE3 fail-closed; ELF arch validation at build time
//! - P4 (Environment-Agnostic): musl-static, no runtime deps
//! - P5 (Tolerance-Documented): build timeout, expected arch

use crate::ShadowOutcome;
use crate::error::{Result, ShadowError};
use std::path::Path;
use tracing::warn;

use super::depot::{load_sources, resolve_depot, update_depot_metadata};
use super::detect_target_triple;
use super::harvest::{self, HarvestResult, HarvestStatus, stage_to_depot_async, validate_elf_arch};

/// CLI arguments for `plasmid.build`.
pub struct BuildArgs {
    /// Primal to build (required).
    pub primal: String,
    /// Target triple (defaults to host musl).
    pub target: Option<String>,
    /// Override depot path.
    pub depot_dir: Option<String>,
    /// Show plan without executing.
    pub dry_run: bool,
}

/// Build a single primal with full guideStone validation.
pub async fn build(args: &BuildArgs) -> Result<ShadowOutcome> {
    let depot_dir = resolve_depot(args.depot_dir.as_deref())?;
    let sources = load_sources(&depot_dir)?;
    let target = args.target.clone().unwrap_or_else(detect_target_triple);

    let source = sources.get(&args.primal).ok_or_else(|| {
        ShadowError::Config(format!("primal '{}' not in sources.toml", args.primal))
    })?;

    if args.dry_run {
        return Ok(ShadowOutcome::ok(format!(
            "plasmid.build (dry-run): would build {} for {target}\n  repo: {}\n  build_args: {}\n  binary_name: {}",
            args.primal,
            source.repo,
            source.build_args.as_deref().unwrap_or("(none)"),
            source.binary_name.as_deref().unwrap_or(&args.primal),
        )));
    }

    let result = build_one(&args.primal, source, &target, &depot_dir).await;

    match &result.status {
        HarvestStatus::Built => {
            if let Err(e) = update_depot_metadata(&depot_dir, &target, &[&result]).await {
                warn!(error = %e, "metadata update failed");
            }
            Ok(ShadowOutcome {
                ok: true,
                message: format!("plasmid.build: {} → {}", args.primal, result.detail),
                data: serde_json::to_value(&result).ok(),
            })
        }
        HarvestStatus::Failed => Err(ShadowError::Build(format!(
            "plasmid.build failed for {}: {}",
            args.primal, result.detail
        ))),
        _ => Ok(ShadowOutcome {
            ok: false,
            message: format!("plasmid.build: {} — {}", args.primal, result.detail),
            data: serde_json::to_value(&result).ok(),
        }),
    }
}

async fn build_one(
    primal: &str,
    source: &harvest::SourceEntry,
    target: &str,
    depot_dir: &Path,
) -> HarvestResult {
    let build_root = std::env::temp_dir().join("membrane-build");
    let clone_dir = build_root.join(primal);

    if let Err(e) = super::drift::clone_source(primal, source, &build_root, &clone_dir).await {
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

    let head_commit = crate::git_ops::head_short(&clone_dir)
        .await
        .unwrap_or_default();

    if let Err(e) = super::toolchain::build_binary(source, target, &clone_dir).await {
        return HarvestResult {
            binary: primal.into(),
            status: HarvestStatus::Failed,
            detail: e.to_string(),
        };
    }

    let binary_name = source.binary_name.as_deref().unwrap_or(primal);
    let bin_path = clone_dir
        .join("target")
        .join(target)
        .join("release")
        .join(binary_name);

    if !bin_path.exists() {
        return HarvestResult {
            binary: primal.into(),
            status: HarvestStatus::Failed,
            detail: format!(
                "binary '{}' not found at {} (HARVEST-NAME-01: check binary_name in sources.toml)",
                binary_name,
                bin_path.display()
            ),
        };
    }

    if let Err(e) = validate_elf_arch(&bin_path, target).await {
        return HarvestResult {
            binary: primal.into(),
            status: HarvestStatus::Failed,
            detail: e.to_string(),
        };
    }

    super::toolchain::strip_binary(&bin_path, primal, target).await;

    match stage_to_depot_async(primal, &bin_path, depot_dir, target).await {
        Ok((size, blake3)) => {
            let _ = tokio::fs::remove_dir_all(&clone_dir).await;
            HarvestResult {
                binary: primal.into(),
                status: HarvestStatus::Built,
                detail: format!(
                    "{}KB blake3={} commit={} target={target} elf=VERIFIED",
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
