// SPDX-License-Identifier: AGPL-3.0-or-later

//! Build toolchain utilities — ELF validation, NDK support, strip, clone.
//!
//! Extracted from `harvest.rs` to keep the harvest orchestrator focused on
//! pipeline coordination while this module handles raw build tooling concerns.

use std::path::Path;

use super::harvest::SourceEntry;

/// Android NDK target triple for native grapheneGate binaries.
pub const ANDROID_TARGET: &str = "aarch64-linux-android";

/// Environment variable pointing to the Android NDK root.
pub const ENV_ANDROID_NDK_HOME: &str = "ANDROID_NDK_HOME";

/// Validate the ELF binary matches the expected target architecture (BUILD-ELF-01).
pub async fn validate_elf_arch(bin_path: &Path, target: &str) -> std::result::Result<(), String> {
    let output = tokio::process::Command::new("file")
        .arg(bin_path)
        .output()
        .await
        .map_err(|e| format!("BUILD-ELF-01: `file` command failed: {e}"))?;

    let file_output = String::from_utf8_lossy(&output.stdout);

    let expected_arch = if target.starts_with("x86_64") {
        "x86-64"
    } else if target.starts_with("aarch64") {
        "ARM aarch64"
    } else {
        return Ok(());
    };

    if !file_output.contains(expected_arch) {
        return Err(format!(
            "BUILD-ELF-01: arch mismatch — expected '{expected_arch}' for target '{target}', \
             got: {file_output}"
        ));
    }

    if target.contains("musl") && !file_output.contains("statically linked") {
        return Err(format!(
            "BUILD-ELF-01: expected static binary for musl target '{target}', got: {file_output}"
        ));
    }

    Ok(())
}

/// Resolve the NDK linker path for `aarch64-linux-android`.
///
/// Searches for the linker at `$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/`.
/// Returns `None` if NDK is not installed or the linker is not found.
pub fn resolve_ndk_linker() -> Option<std::path::PathBuf> {
    let ndk_home = std::env::var(ENV_ANDROID_NDK_HOME).ok()?;
    let ndk_path = std::path::Path::new(&ndk_home);

    let prebuilt = ndk_path.join("toolchains/llvm/prebuilt/linux-x86_64/bin");

    for api in [35, 34, 33, 31, 30] {
        let linker = prebuilt.join(format!("aarch64-linux-android{api}-clang"));
        if linker.exists() {
            return Some(linker);
        }
    }

    let unversioned = prebuilt.join("aarch64-linux-android-clang");
    if unversioned.exists() {
        return Some(unversioned);
    }

    None
}

/// Resolve the NDK `llvm-strip` path for Android targets.
pub fn resolve_ndk_strip() -> Option<String> {
    let ndk_home = std::env::var(ENV_ANDROID_NDK_HOME).ok()?;
    let strip = std::path::Path::new(&ndk_home)
        .join("toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-strip");
    if strip.exists() {
        Some(strip.to_string_lossy().into_owned())
    } else {
        None
    }
}

/// Strip debug symbols from a binary (uses NDK strip for Android targets).
pub async fn strip_binary(bin_path: &Path, primal: &str, target: &str) {
    let strip_cmd = if target.contains("android") {
        resolve_ndk_strip().unwrap_or_else(|| "llvm-strip".into())
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

/// Build a primal binary from source using `cargo build`.
///
/// Handles both native (musl static) and Android (NDK cross-compile) targets.
pub async fn build_binary(
    source: &SourceEntry,
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

    if target.contains("android") {
        if let Some(linker) = resolve_ndk_linker() {
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
                "NDK linker not found for {target}. Set {ENV_ANDROID_NDK_HOME} \
                 to the NDK root (e.g. /opt/android-ndk-r26d)"
            ));
        }
    }

    let output = cmd.output().await;
    if output.as_ref().is_ok_and(|o| o.status.success()) {
        Ok(())
    } else {
        Err("cargo build failed".into())
    }
}

/// Shallow-clone a git repository. Returns true on success.
pub async fn try_clone(url: &str, clone_dir: &Path) -> bool {
    if clone_dir.exists() {
        let _ = std::fs::remove_dir_all(clone_dir);
    }
    let result = tokio::process::Command::new("git")
        .args(["clone", "--depth", "1", url, &clone_dir.to_string_lossy()])
        .output()
        .await;
    result.is_ok_and(|o| o.status.success())
}
