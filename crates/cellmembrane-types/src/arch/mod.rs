// SPDX-License-Identifier: AGPL-3.0-or-later

//! Platform-aware target types for binary depot layout and cross-compilation.
//!
//! Decomposes the deployment target into orthogonal axes — OS, CPU architecture,
//! and link model — enabling isomorphic depot fetch across consumer Linux, cloud
//! VPS, Pixel, Windows, and Mac. "Silicon atheism is preceded by OS atheism."
//!
//! The legacy [`TargetArch`] enum is preserved for backward compatibility with
//! existing callers. New code should prefer [`Platform`].

use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

// ── OS ──────────────────────────────────────────────────────────────────────

/// Target operating system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetOs {
    /// Desktop/server Linux (any distro).
    Linux,
    /// Microsoft Windows (desktop or server).
    Windows,
    /// Apple macOS / Mac OS X.
    MacOs,
    /// Android (bionic libc, ADB transport).
    Android,
    /// Apple iOS (XPC transport, future).
    Ios,
    /// WebAssembly (browser or WASI, future).
    Wasm,
}

impl TargetOs {
    /// Detect the host OS at compile time.
    #[must_use]
    pub const fn detect() -> Self {
        if cfg!(target_os = "android") {
            Self::Android
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(target_os = "ios") {
            Self::Ios
        } else if cfg!(target_family = "wasm") {
            Self::Wasm
        } else {
            Self::Linux
        }
    }

    /// Whether this OS uses Unix-style paths and semantics.
    #[must_use]
    pub const fn is_unix(self) -> bool {
        matches!(self, Self::Linux | Self::MacOs | Self::Android | Self::Ios)
    }

    /// Whether systemd is the expected service manager.
    #[must_use]
    pub const fn has_systemd(self) -> bool {
        matches!(self, Self::Linux)
    }

    /// Default install base directory for primal binaries.
    #[must_use]
    pub const fn default_install_base(self) -> &'static str {
        match self {
            Self::Linux => crate::service::DEFAULT_INSTALL_BASE,
            Self::MacOs => "/usr/local/bin",
            Self::Windows => "C:\\Program Files\\membrane",
            Self::Android => "/data/local/tmp",
            Self::Ios | Self::Wasm => "/tmp/membrane",
        }
    }

    /// Default ecoPrimals workspace root.
    #[must_use]
    pub const fn default_eco_root(self) -> &'static str {
        match self {
            Self::Linux | Self::MacOs | Self::Android | Self::Ios => {
                crate::service::DEFAULT_ECOPRIMALS_ROOT
            }
            Self::Windows => "C:\\ecoPrimals",
            Self::Wasm => "/ecoPrimals",
        }
    }

    /// Binary file extension for this OS (empty string for Unix).
    #[must_use]
    pub const fn exe_extension(self) -> &'static str {
        match self {
            Self::Windows => ".exe",
            Self::Wasm => ".wasm",
            _ => "",
        }
    }
}

impl fmt::Display for TargetOs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Linux => "linux",
            Self::Windows => "windows",
            Self::MacOs => "macos",
            Self::Android => "android",
            Self::Ios => "ios",
            Self::Wasm => "wasm",
        };
        f.write_str(s)
    }
}

// ── CPU Architecture ────────────────────────────────────────────────────────

/// Target CPU architecture (ISA family).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CpuArch {
    /// x86-64 / AMD64.
    X86_64,
    /// ARM 64-bit / `AArch64`.
    Aarch64,
    /// RISC-V 64-bit (future).
    Riscv64,
    /// WebAssembly 32-bit (browser/WASI).
    Wasm32,
}

impl CpuArch {
    /// Detect the host CPU architecture at compile time.
    #[must_use]
    pub const fn detect() -> Self {
        if cfg!(target_arch = "x86_64") {
            Self::X86_64
        } else if cfg!(target_arch = "aarch64") {
            Self::Aarch64
        } else if cfg!(target_arch = "riscv64") {
            Self::Riscv64
        } else if cfg!(target_arch = "wasm32") {
            Self::Wasm32
        } else {
            Self::X86_64
        }
    }
}

impl fmt::Display for CpuArch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::X86_64 => "x86_64",
            Self::Aarch64 => "aarch64",
            Self::Riscv64 => "riscv64",
            Self::Wasm32 => "wasm32",
        };
        f.write_str(s)
    }
}

// ── Link Model ──────────────────────────────────────────────────────────────

/// Binary linking model — determines libc dependency and static/dynamic policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkModel {
    /// musl-static — postPrimordial standard. Zero runtime deps.
    MuslStatic,
    /// glibc dynamic — GPU primals needing `dlopen` for CUDA/Vulkan.
    GnuDynamic,
    /// Windows GNU cross-compile (mingw-w64).
    WindowsGnu,
    /// Windows MSVC native (future).
    Msvc,
    /// Apple Darwin (macOS/iOS native).
    AppleDarwin,
    /// Android NDK (bionic libc).
    AndroidNdk,
    /// WASM (no libc).
    WasmUnknown,
}

impl LinkModel {
    /// Whether binaries with this link model must be fully statically linked.
    #[must_use]
    pub const fn requires_static_linking(self) -> bool {
        matches!(self, Self::MuslStatic | Self::WasmUnknown)
    }

    /// Whether this link model supports GPU workloads (dlopen).
    #[must_use]
    pub const fn supports_gpu(self) -> bool {
        matches!(self, Self::GnuDynamic | Self::Msvc | Self::AppleDarwin)
    }
}

// ── Platform ────────────────────────────────────────────────────────────────

/// Fully decomposed deployment platform: OS x CPU x Link model.
///
/// This is the canonical target identifier for depot layout, binary fetch,
/// and platform-specific deployment decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Platform {
    /// Target operating system.
    pub os: TargetOs,
    /// Target CPU architecture.
    pub arch: CpuArch,
    /// Binary linking model.
    pub link: LinkModel,
}

impl Platform {
    /// Detect the host platform at compile time.
    #[must_use]
    pub const fn detect() -> Self {
        let os = TargetOs::detect();
        let arch = CpuArch::detect();
        let link = match os {
            TargetOs::Linux => LinkModel::MuslStatic,
            TargetOs::Windows => LinkModel::WindowsGnu,
            TargetOs::MacOs | TargetOs::Ios => LinkModel::AppleDarwin,
            TargetOs::Android => LinkModel::AndroidNdk,
            TargetOs::Wasm => LinkModel::WasmUnknown,
        };
        Self { os, arch, link }
    }

    /// Construct a platform for GPU workloads (glibc, `x86_64`).
    #[must_use]
    pub const fn gpu() -> Self {
        Self {
            os: TargetOs::Linux,
            arch: CpuArch::X86_64,
            link: LinkModel::GnuDynamic,
        }
    }

    /// Rust target triple string (e.g. `x86_64-unknown-linux-musl`).
    #[must_use]
    pub const fn triple(&self) -> &'static str {
        match (self.arch, self.os, self.link) {
            (CpuArch::X86_64, TargetOs::Linux, LinkModel::GnuDynamic) => "x86_64-unknown-linux-gnu",
            (CpuArch::Aarch64, TargetOs::Linux, LinkModel::MuslStatic) => {
                "aarch64-unknown-linux-musl"
            }
            (CpuArch::X86_64, TargetOs::Windows, LinkModel::WindowsGnu) => "x86_64-pc-windows-gnu",
            (CpuArch::X86_64, TargetOs::Windows, LinkModel::Msvc) => "x86_64-pc-windows-msvc",
            (CpuArch::X86_64, TargetOs::MacOs, LinkModel::AppleDarwin) => "x86_64-apple-darwin",
            (CpuArch::Aarch64, TargetOs::MacOs, LinkModel::AppleDarwin) => "aarch64-apple-darwin",
            (CpuArch::Aarch64, TargetOs::Android, LinkModel::AndroidNdk) => "aarch64-linux-android",
            (CpuArch::Wasm32, _, LinkModel::WasmUnknown) => "wasm32-unknown-unknown",
            _ => "x86_64-unknown-linux-musl",
        }
    }

    /// Depot subdirectory for this platform (matches existing depot layout).
    #[must_use]
    pub fn depot_path(&self) -> PathBuf {
        PathBuf::from("primals").join(self.triple())
    }

    /// Default install base for primal binaries on this platform.
    #[must_use]
    pub const fn install_base(&self) -> &'static str {
        self.os.default_install_base()
    }

    /// Binary filename for a primal on this platform.
    #[must_use]
    pub fn binary_name(&self, primal: &str) -> String {
        format!("{primal}{}", self.os.exe_extension())
    }

    /// Whether binaries for this platform must be statically linked.
    #[must_use]
    pub const fn requires_static_linking(&self) -> bool {
        self.link.requires_static_linking()
    }

    /// Whether this platform supports GPU workloads.
    #[must_use]
    pub const fn supports_gpu(&self) -> bool {
        self.link.supports_gpu()
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.triple())
    }
}

/// Error returned when parsing an unknown platform string.
#[derive(Debug, Clone, thiserror::Error)]
#[error("unknown platform: {0}")]
pub struct PlatformParseError(pub String);

impl FromStr for Platform {
    type Err = PlatformParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "x86_64-unknown-linux-musl" | "x86_64_musl" | "musl" => Ok(Self {
                os: TargetOs::Linux,
                arch: CpuArch::X86_64,
                link: LinkModel::MuslStatic,
            }),
            "x86_64-unknown-linux-gnu" | "x86_64_gnu" | "gnu" => Ok(Self {
                os: TargetOs::Linux,
                arch: CpuArch::X86_64,
                link: LinkModel::GnuDynamic,
            }),
            "aarch64-unknown-linux-musl" | "aarch64_musl" | "aarch64" => Ok(Self {
                os: TargetOs::Linux,
                arch: CpuArch::Aarch64,
                link: LinkModel::MuslStatic,
            }),
            "x86_64-pc-windows-gnu" | "windows" | "win64" => Ok(Self {
                os: TargetOs::Windows,
                arch: CpuArch::X86_64,
                link: LinkModel::WindowsGnu,
            }),
            "x86_64-pc-windows-msvc" => Ok(Self {
                os: TargetOs::Windows,
                arch: CpuArch::X86_64,
                link: LinkModel::Msvc,
            }),
            "x86_64-apple-darwin" | "macos" | "darwin" => Ok(Self {
                os: TargetOs::MacOs,
                arch: CpuArch::X86_64,
                link: LinkModel::AppleDarwin,
            }),
            "aarch64-apple-darwin" | "macos-arm" => Ok(Self {
                os: TargetOs::MacOs,
                arch: CpuArch::Aarch64,
                link: LinkModel::AppleDarwin,
            }),
            "aarch64-linux-android" | "android" => Ok(Self {
                os: TargetOs::Android,
                arch: CpuArch::Aarch64,
                link: LinkModel::AndroidNdk,
            }),
            "wasm32-unknown-unknown" | "wasm" => Ok(Self {
                os: TargetOs::Wasm,
                arch: CpuArch::Wasm32,
                link: LinkModel::WasmUnknown,
            }),
            _ => Err(PlatformParseError(s.to_string())),
        }
    }
}

mod legacy;
pub use legacy::*;

/// Check whether a primal needs a glibc build for GPU access.
///
/// Checks the service registry `gpu_required` flag. For manifest-driven GPU
/// detection, prefer `EcosystemManifest::gpu_primals()` at runtime.
#[must_use]
pub fn is_gpu_primal(name: &str) -> bool {
    crate::MembraneService::for_binary(name).is_some_and(|s| s.gpu_required)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_primal_detection() {
        assert!(is_gpu_primal("barracuda"));
        assert!(is_gpu_primal("coralreef"));
        assert!(!is_gpu_primal("beardog"));
        assert!(!is_gpu_primal("songbird"));
    }

    #[test]
    fn platform_detect_returns_valid_triple() {
        let p = Platform::detect();
        assert!(!p.triple().is_empty());
        assert!(!p.to_string().is_empty());
    }

    #[test]
    fn platform_parse_all_triples() {
        let triples = [
            "x86_64-unknown-linux-musl",
            "x86_64-unknown-linux-gnu",
            "aarch64-unknown-linux-musl",
            "x86_64-pc-windows-gnu",
            "x86_64-pc-windows-msvc",
            "x86_64-apple-darwin",
            "aarch64-apple-darwin",
            "aarch64-linux-android",
            "wasm32-unknown-unknown",
        ];
        for t in triples {
            let p: Platform = t.parse().unwrap();
            assert_eq!(p.triple(), t, "roundtrip failed for {t}");
        }
    }

    #[test]
    fn platform_short_names() {
        assert_eq!("windows".parse::<Platform>().unwrap().os, TargetOs::Windows);
        assert_eq!("macos".parse::<Platform>().unwrap().os, TargetOs::MacOs);
        assert_eq!("android".parse::<Platform>().unwrap().os, TargetOs::Android);
        assert_eq!("wasm".parse::<Platform>().unwrap().arch, CpuArch::Wasm32);
    }

    #[test]
    fn platform_depot_path() {
        let p: Platform = "x86_64-unknown-linux-musl".parse().unwrap();
        assert_eq!(
            p.depot_path(),
            PathBuf::from("primals/x86_64-unknown-linux-musl")
        );
    }

    #[test]
    fn platform_binary_name() {
        let linux: Platform = "x86_64-unknown-linux-musl".parse().unwrap();
        assert_eq!(linux.binary_name("songbird"), "songbird");

        let win: Platform = "x86_64-pc-windows-gnu".parse().unwrap();
        assert_eq!(win.binary_name("songbird"), "songbird.exe");

        let wasm: Platform = "wasm32-unknown-unknown".parse().unwrap();
        assert_eq!(wasm.binary_name("songbird"), "songbird.wasm");
    }

    #[test]
    fn platform_install_base() {
        let linux: Platform = "x86_64-unknown-linux-musl".parse().unwrap();
        assert_eq!(linux.install_base(), "/opt/membrane");

        let win: Platform = "windows".parse().unwrap();
        assert_eq!(win.install_base(), "C:\\Program Files\\membrane");

        let mac: Platform = "macos".parse().unwrap();
        assert_eq!(mac.install_base(), "/usr/local/bin");
    }

    #[test]
    fn platform_gpu() {
        let gpu = Platform::gpu();
        assert!(gpu.supports_gpu());
        assert!(!gpu.requires_static_linking());
        assert_eq!(gpu.triple(), "x86_64-unknown-linux-gnu");
    }

    #[test]
    fn os_properties() {
        assert!(TargetOs::Linux.is_unix());
        assert!(TargetOs::MacOs.is_unix());
        assert!(TargetOs::Android.is_unix());
        assert!(!TargetOs::Windows.is_unix());
        assert!(!TargetOs::Wasm.is_unix());

        assert!(TargetOs::Linux.has_systemd());
        assert!(!TargetOs::MacOs.has_systemd());
        assert!(!TargetOs::Windows.has_systemd());
    }

    #[test]
    fn platform_serde_roundtrip() {
        let p = Platform::detect();
        let json = serde_json::to_string(&p).unwrap();
        let back: Platform = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }
}
