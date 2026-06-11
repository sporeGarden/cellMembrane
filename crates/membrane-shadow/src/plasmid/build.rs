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
use std::path::{Path, PathBuf};

use super::depot::{compute_blake3_file, load_sources, resolve_depot, update_depot_metadata};
use super::detect_target_triple;
use super::harvest::{self, ENV_ANDROID_NDK_HOME, HarvestResult, HarvestStatus, validate_elf_arch};

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
                eprintln!("warn: metadata update failed: {e}");
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

    // Phase 1: Clone (ephemeral — always fresh)
    if clone_dir.exists() {
        let _ = std::fs::remove_dir_all(&clone_dir);
    }
    std::fs::create_dir_all(&build_root).ok();

    if let Err(detail) = clone_source(primal, source, &clone_dir).await {
        let status = if source.private {
            HarvestStatus::Skipped
        } else {
            HarvestStatus::Failed
        };
        return HarvestResult {
            binary: primal.into(),
            status,
            detail,
        };
    }

    let head_commit = get_head(&clone_dir).await.unwrap_or_default();

    // Phase 2: Build
    if let Err(detail) = compile(source, target, &clone_dir).await {
        return HarvestResult {
            binary: primal.into(),
            status: HarvestStatus::Failed,
            detail,
        };
    }

    // Phase 3: Locate binary (HARVEST-NAME-01 — resolve binary_name vs primal name)
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

    // Phase 4: Validate ELF architecture (BUILD-ELF-01)
    if let Err(detail) = validate_elf_arch(&bin_path, target).await {
        return HarvestResult {
            binary: primal.into(),
            status: HarvestStatus::Failed,
            detail,
        };
    }

    // Phase 5: Strip
    strip_binary(&bin_path, primal, target).await;

    // Phase 6: Stage to depot (atomic)
    match stage_to_depot(primal, &bin_path, depot_dir, target) {
        Ok((size, blake3)) => {
            let _ = std::fs::remove_dir_all(&clone_dir);
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
        Err(detail) => HarvestResult {
            binary: primal.into(),
            status: HarvestStatus::Failed,
            detail,
        },
    }
}

async fn clone_source(
    primal: &str,
    source: &harvest::SourceEntry,
    clone_dir: &Path,
) -> std::result::Result<(), String> {
    let forgejo_host = std::env::var(cellmembrane_types::service::ENV_FORGEJO_SSH_HOST)
        .unwrap_or_else(|_| "git.primals.eco:2222".into());
    let forgejo_url = format!("ssh://git@{forgejo_host}/{}.git", source.repo);
    let github_url = format!("https://github.com/{}.git", source.repo);

    if try_clone(&forgejo_url, clone_dir).await {
        return Ok(());
    }
    if try_clone(&github_url, clone_dir).await {
        return Ok(());
    }

    if source.private {
        Err(format!("{primal}: private repo — SSH + HTTPS both failed"))
    } else {
        Err(format!("{primal}: git clone failed (Forgejo + GitHub)"))
    }
}

async fn try_clone(url: &str, clone_dir: &Path) -> bool {
    if clone_dir.exists() {
        let _ = std::fs::remove_dir_all(clone_dir);
    }
    tokio::process::Command::new("git")
        .args(["clone", "--depth", "1", url, &clone_dir.to_string_lossy()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .is_ok_and(|s| s.success())
}

async fn compile(
    source: &harvest::SourceEntry,
    target: &str,
    clone_dir: &Path,
) -> std::result::Result<(), String> {
    let target_dir = clone_dir.join("target");
    let mut cmd = tokio::process::Command::new("cargo");
    cmd.args([
        "build",
        "--release",
        "--target",
        target,
        "--manifest-path",
        &clone_dir.join("Cargo.toml").to_string_lossy(),
        "--target-dir",
        &target_dir.to_string_lossy(),
    ]);

    if let Some(extra) = &source.build_args {
        for arg in extra.split_whitespace() {
            cmd.arg(arg);
        }
    }

    // NDK configuration for Android targets
    if target.contains("android") {
        if let Some(linker) = harvest::resolve_ndk_linker() {
            let target_upper = target.to_uppercase().replace('-', "_");
            cmd.env(format!("CARGO_TARGET_{target_upper}_LINKER"), &linker);

            let cc_env = format!("CC_{}", target.replace('-', "_"));
            let ar_env = format!("AR_{}", target.replace('-', "_"));
            let bin_dir = linker.parent().unwrap_or_else(|| Path::new("."));
            cmd.env(&cc_env, &linker);
            cmd.env(&ar_env, bin_dir.join("llvm-ar"));

            if let Ok(ndk_home) = std::env::var(ENV_ANDROID_NDK_HOME) {
                cmd.env("ANDROID_NDK_HOME", &ndk_home);
            }
        } else {
            return Err(format!(
                "NDK linker not found for {target}. Set {ENV_ANDROID_NDK_HOME}"
            ));
        }
    }

    let output = cmd.output().await;
    match output {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            let tail: String = stderr.lines().rev().take(5).collect::<Vec<_>>().join("\n");
            Err(format!("cargo build failed:\n{tail}"))
        }
        Err(e) => Err(format!("cargo build spawn failed: {e}")),
    }
}

async fn strip_binary(bin_path: &Path, primal: &str, target: &str) {
    let strip_cmd: PathBuf = if target.contains("android") {
        harvest::resolve_ndk_linker()
            .and_then(|l| l.parent().map(|p| p.join("llvm-strip")))
            .unwrap_or_else(|| "llvm-strip".into())
    } else {
        "strip".into()
    };

    let result = tokio::process::Command::new(&strip_cmd)
        .arg(bin_path)
        .output()
        .await;
    if result.is_err() {
        eprintln!("warn: strip failed for {primal} — proceeding unstripped");
    }
}

fn stage_to_depot(
    primal: &str,
    bin_path: &Path,
    depot_dir: &Path,
    target: &str,
) -> std::result::Result<(u64, String), String> {
    let staging_dir = depot_dir.join("primals").join(target);
    std::fs::create_dir_all(&staging_dir).ok();
    let dest = staging_dir.join(primal);
    let tmp = staging_dir.join(format!(".{primal}.new"));

    std::fs::copy(bin_path, &tmp).map_err(|e| format!("depot stage failed: {e}"))?;
    std::fs::rename(&tmp, &dest).map_err(|e| format!("atomic rename failed: {e}"))?;

    let size = std::fs::metadata(&dest).map_or(0, |m| m.len());
    let blake3 = compute_blake3_file(&dest);
    Ok((size, blake3))
}

async fn get_head(repo_dir: &Path) -> Option<String> {
    let output = tokio::process::Command::new("git")
        .args(["rev-parse", "--short=8", "HEAD"])
        .current_dir(repo_dir)
        .output()
        .await
        .ok()?;

    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}
