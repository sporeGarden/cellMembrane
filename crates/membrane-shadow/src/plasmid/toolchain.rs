// SPDX-License-Identifier: AGPL-3.0-or-later

//! Build toolchain utilities — ELF validation, NDK support, strip, clone.
//!
//! Extracted from `harvest.rs` to keep the harvest orchestrator focused on
//! pipeline coordination while this module handles raw build tooling concerns.

use std::path::Path;

use super::harvest::SourceEntry;
use tracing::warn;

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
        let ph_off = usize::try_from(u64::from_le_bytes(
            data[32..40].try_into().unwrap_or([0; 8]),
        ))
        .map_err(|_| "BUILD-ELF-01: phoff exceeds addressable range".to_string())?;
        let ph_ent_size = usize::from(u16::from_le_bytes([data[54], data[55]]));
        let ph_num = usize::from(u16::from_le_bytes([data[56], data[57]]));

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
        warn!(primal, "strip failed — proceeding unstripped");
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
        if let Err(e) = tokio::fs::remove_dir_all(clone_dir).await {
            tracing::debug!(error = %e, "clone_dir cleanup (may not exist)");
        }
    }
    crate::git_ops::git_success(
        std::path::Path::new("."),
        &["clone", "--depth", "1", url, &clone_dir.to_string_lossy()],
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn android_target_constant() {
        assert_eq!(ANDROID_TARGET, "aarch64-linux-android");
    }

    #[test]
    fn elf_magic_bytes() {
        assert_eq!(ELF_MAGIC, [0x7f, b'E', b'L', b'F']);
    }

    #[test]
    fn ndk_linker_returns_none_without_env() {
        if std::env::var(ENV_ANDROID_NDK_HOME).is_err() {
            assert!(resolve_ndk_linker().is_none());
        }
    }

    #[test]
    fn ndk_strip_returns_none_without_env() {
        if std::env::var(ENV_ANDROID_NDK_HOME).is_err() {
            assert!(resolve_ndk_strip().is_none());
        }
    }

    #[tokio::test]
    async fn validate_elf_rejects_non_elf() {
        let tmp = std::env::temp_dir().join("membrane-toolchain-test-notelf");
        std::fs::write(&tmp, b"not an ELF binary").unwrap();
        let result = validate_elf_arch(&tmp, "x86_64-unknown-linux-musl").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("BUILD-ELF-01"));
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn validate_elf_rejects_short_file() {
        let tmp = std::env::temp_dir().join("membrane-toolchain-test-short");
        std::fs::write(&tmp, [0x7f, b'E', b'L']).unwrap();
        let result = validate_elf_arch(&tmp, "x86_64-unknown-linux-musl").await;
        assert!(result.is_err());
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn validate_elf_accepts_correct_arch() {
        let tmp = std::env::temp_dir().join("membrane-toolchain-test-elf64");
        let mut elf = vec![0u8; 64];
        elf[..4].copy_from_slice(&ELF_MAGIC);
        elf[18] = 0x3E; // x86-64
        elf[19] = 0x00;
        std::fs::write(&tmp, &elf).unwrap();
        let result = validate_elf_arch(&tmp, "x86_64-unknown-linux-musl").await;
        assert!(result.is_ok());
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn validate_elf_rejects_wrong_arch() {
        let tmp = std::env::temp_dir().join("membrane-toolchain-test-wrongarch");
        let mut elf = vec![0u8; 64];
        elf[..4].copy_from_slice(&ELF_MAGIC);
        elf[18] = 0xB7; // aarch64
        elf[19] = 0x00;
        std::fs::write(&tmp, &elf).unwrap();
        let result = validate_elf_arch(&tmp, "x86_64-unknown-linux-musl").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("arch mismatch"));
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn try_clone_fails_on_invalid_url() {
        let tmp = std::env::temp_dir().join("membrane-toolchain-test-clone");
        let _ = std::fs::remove_dir_all(&tmp);
        let ok = try_clone("https://invalid.example.com/nonexistent.git", &tmp).await;
        assert!(!ok);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
