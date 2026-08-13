// SPDX-License-Identifier: AGPL-3.0-or-later

//! Deprecated `TargetArch` enum — backward-compatible shim for
//! callers that predate the decomposed [`super::Platform`] type.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use super::{CpuArch, LinkModel, Platform, TargetOs};

/// Target architecture for depot binaries (legacy — prefer [`Platform`]).
///
/// Each variant maps to a Rust target triple and determines:
/// - Depot directory path (`primals/{triple}/`)
/// - ELF validation policy (static musl vs dynamic gnu)
/// - Build toolchain selection
#[deprecated(since = "0.1.0", note = "use `Platform` + `Platform::gpu()` instead")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetArch {
    /// `x86_64` static musl — default for all gates.
    X86_64Musl,
    /// `x86_64` glibc — GPU primals (`barracuda`, `coralreef`) needing `dlopen`.
    X86_64Gnu,
    /// `aarch64` static musl — ARM gates (future).
    Aarch64Musl,
}

#[allow(
    deprecated,
    reason = "impl block for deprecated TargetArch migration shim"
)]
impl TargetArch {
    /// Rust target triple string.
    #[must_use]
    pub const fn triple(self) -> &'static str {
        match self {
            Self::X86_64Musl => "x86_64-unknown-linux-musl",
            Self::X86_64Gnu => "x86_64-unknown-linux-gnu",
            Self::Aarch64Musl => "aarch64-unknown-linux-musl",
        }
    }

    /// Whether binaries for this target must be statically linked (no `PT_INTERP`).
    #[must_use]
    pub const fn requires_static_linking(self) -> bool {
        match self {
            Self::X86_64Musl | Self::Aarch64Musl => true,
            Self::X86_64Gnu => false,
        }
    }

    /// Whether this target supports GPU workloads (dlopen for CUDA/Vulkan).
    #[must_use]
    pub const fn supports_gpu(self) -> bool {
        matches!(self, Self::X86_64Gnu)
    }

    /// Detect the host platform's default target arch.
    #[must_use]
    pub const fn detect_host() -> Self {
        if cfg!(target_arch = "aarch64") {
            Self::Aarch64Musl
        } else {
            Self::X86_64Musl
        }
    }

    /// Convert to the new [`Platform`] type.
    #[must_use]
    pub const fn to_platform(self) -> Platform {
        match self {
            Self::X86_64Musl => Platform {
                os: TargetOs::Linux,
                arch: CpuArch::X86_64,
                link: LinkModel::MuslStatic,
            },
            Self::X86_64Gnu => Platform {
                os: TargetOs::Linux,
                arch: CpuArch::X86_64,
                link: LinkModel::GnuDynamic,
            },
            Self::Aarch64Musl => Platform {
                os: TargetOs::Linux,
                arch: CpuArch::Aarch64,
                link: LinkModel::MuslStatic,
            },
        }
    }
}

#[allow(
    deprecated,
    reason = "Display for deprecated TargetArch migration shim"
)]
impl fmt::Display for TargetArch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.triple())
    }
}

#[allow(
    deprecated,
    reason = "From conversion for deprecated TargetArch migration shim"
)]
impl From<TargetArch> for Platform {
    fn from(arch: TargetArch) -> Self {
        arch.to_platform()
    }
}

/// Error returned when parsing an unknown target architecture string.
#[derive(Debug, Clone, thiserror::Error)]
#[error("unknown target arch: {0}")]
pub struct ArchParseError(pub String);

#[allow(
    deprecated,
    reason = "FromStr for deprecated TargetArch migration shim"
)]
impl FromStr for TargetArch {
    type Err = ArchParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "x86_64-unknown-linux-musl" | "x86_64_musl" | "musl" => Ok(Self::X86_64Musl),
            "x86_64-unknown-linux-gnu" | "x86_64_gnu" | "gnu" => Ok(Self::X86_64Gnu),
            "aarch64-unknown-linux-musl" | "aarch64_musl" | "aarch64" => Ok(Self::Aarch64Musl),
            _ => Err(ArchParseError(s.to_string())),
        }
    }
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::*;

    #[test]
    fn triple_roundtrip() {
        for arch in [
            TargetArch::X86_64Musl,
            TargetArch::X86_64Gnu,
            TargetArch::Aarch64Musl,
        ] {
            let parsed: TargetArch = arch.triple().parse().unwrap();
            assert_eq!(parsed, arch);
        }
    }

    #[test]
    fn short_names_parse() {
        assert_eq!(
            "musl".parse::<TargetArch>().unwrap(),
            TargetArch::X86_64Musl
        );
        assert_eq!("gnu".parse::<TargetArch>().unwrap(), TargetArch::X86_64Gnu);
        assert_eq!(
            "aarch64".parse::<TargetArch>().unwrap(),
            TargetArch::Aarch64Musl
        );
    }

    #[test]
    fn static_linking_policy() {
        assert!(TargetArch::X86_64Musl.requires_static_linking());
        assert!(TargetArch::Aarch64Musl.requires_static_linking());
        assert!(!TargetArch::X86_64Gnu.requires_static_linking());
    }

    #[test]
    fn gpu_support() {
        assert!(TargetArch::X86_64Gnu.supports_gpu());
        assert!(!TargetArch::X86_64Musl.supports_gpu());
    }

    #[test]
    fn display_matches_triple() {
        assert_eq!(
            TargetArch::X86_64Gnu.to_string(),
            "x86_64-unknown-linux-gnu"
        );
    }

    #[test]
    fn detect_host_returns_valid() {
        let host = TargetArch::detect_host();
        assert!(!host.triple().is_empty());
    }

    #[test]
    fn serde_roundtrip() {
        let arch = TargetArch::X86_64Gnu;
        let json = serde_json::to_string(&arch).unwrap();
        let back: TargetArch = serde_json::from_str(&json).unwrap();
        assert_eq!(back, arch);
    }

    #[test]
    fn target_arch_to_platform_conversion() {
        let legacy = TargetArch::X86_64Musl;
        let platform: Platform = legacy.into();
        assert_eq!(platform.os, TargetOs::Linux);
        assert_eq!(platform.arch, CpuArch::X86_64);
        assert_eq!(platform.link, LinkModel::MuslStatic);
        assert_eq!(platform.triple(), legacy.triple());
    }
}
