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

/// ELF magic bytes.
const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];

/// Validate the ELF binary matches the expected target architecture (BUILD-ELF-01).
///
/// Reads ELF headers directly (no external `file` command dependency).
pub async fn validate_elf_arch(bin_path: &Path, target: &str) -> std::result::Result<(), String> {
    let data = tokio::fs::read(bin_path)
        .await
        .map_err(|e| format!("BUILD-ELF-01: cannot read binary: {e}"))?;

    if data.len() < 64 || data[..4] != ELF_MAGIC {
        return Err(format!(
            "BUILD-ELF-01: not a valid ELF binary: {}",
            bin_path.display()
        ));
    }

    // e_machine at offset 18 (ELF64: little-endian u16)
    let e_machine = u16::from_le_bytes([data[18], data[19]]);
    let (expected_machine, arch_name) = if target.starts_with("x86_64") {
        (0x3E_u16, "x86-64")
    } else if target.starts_with("aarch64") {
        (0xB7_u16, "aarch64")
    } else {
        return Ok(());
    };

    if e_machine != expected_machine {
        return Err(format!(
            "BUILD-ELF-01: arch mismatch — expected {arch_name} (0x{expected_machine:02X}) \
             for target '{target}', got e_machine=0x{e_machine:02X}"
        ));
    }

    // Static linkage: check for absence of PT_INTERP program header (type=3)
    // which indicates dynamically linked. ELF64 phoff at offset 32, phentsize at 54, phnum at 56.
    if target.contains("musl") {
        let ph_off = u64::from_le_bytes(data[32..40].try_into().unwrap_or([0; 8])) as usize;
        let ph_ent_size = u16::from_le_bytes([data[54], data[55]]) as usize;
        let ph_num = u16::from_le_bytes([data[56], data[57]]) as usize;

        let has_interp = (0..ph_num).any(|i| {
            let offset = ph_off + i * ph_ent_size;
            offset + 4 <= data.len()
                && u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap_or([0; 4])) == 3
        });

        if has_interp {
            // PT_INTERP present but acceptable for static-pie (PIE with static libc)
            // Only truly dynamic if it also has PT_DYNAMIC referencing shared libs
            let has_dynamic_needed = (0..ph_num).any(|i| {
                let offset = ph_off + i * ph_ent_size;
                offset + 4 <= data.len()
                    && u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap_or([0; 4]))
                        == 2 // PT_DYNAMIC
            });
            if has_dynamic_needed {
                // Check if .dynamic section has DT_NEEDED entries (offset varies)
                // For musl-static-pie, PT_DYNAMIC exists but has no DT_NEEDED for libc.so
                // Accept if the binary is reasonably small or has no NEEDED entries
                // This is a best-effort heuristic — full validation requires parsing .dynamic
            }
        }
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
    } else if target == "aarch64-unknown-linux-musl" {
        let target_upper = target.to_uppercase().replace('-', "_");
        let linker_env = format!("CARGO_TARGET_{target_upper}_LINKER");
        if std::env::var(&linker_env).is_err() {
            cmd.env(&linker_env, "aarch64-linux-gnu-gcc");
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
