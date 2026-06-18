// SPDX-License-Identifier: AGPL-3.0-or-later

//! Binary integrity expectations — hash verification for membrane service binaries.
//!
//! Maps to MEM-09 (Songbird binary integrity) in `darkforest_membrane.sh`.

use super::{MembraneService, ServicePaths};

/// Binary integrity expectation for a membrane service.
///
/// The BLAKE3 hash is verified against `plasmidBin`'s `checksums.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryIntegrity {
    /// Binary name.
    pub binary: &'static str,
    /// Resolved install path (runtime-configurable).
    pub install_path: String,
    /// Hash algorithm used for verification.
    pub hash_algorithm: HashAlgorithm,
    /// Whether the binary must be a static musl ELF (stripped).
    pub require_static_musl: bool,
}

/// Hash algorithm for binary verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashAlgorithm {
    /// BLAKE3 — used by `plasmidBin` `checksums.toml`.
    Blake3,
    /// SHA-256 — fallback when b3sum is not installed.
    Sha256,
}

/// Returns binary integrity expectations using default paths (backward compat).
#[must_use]
pub fn binary_integrity_for(
    composition: crate::composition::MembraneComposition,
) -> Vec<BinaryIntegrity> {
    binary_integrity_for_paths(composition, &ServicePaths::from_env())
}

/// Returns binary integrity expectations using configurable `ServicePaths`.
///
/// ecoPrimals binaries: static musl ELFs, BLAKE3 checksums.
/// Symbiotic binaries: SHA-256 from upstream releases.
///
/// Install paths are resolved from `ServicePaths` — no hardcoded assumptions.
#[must_use]
pub fn binary_integrity_for_paths(
    composition: crate::composition::MembraneComposition,
    paths: &ServicePaths,
) -> Vec<BinaryIntegrity> {
    let spec = composition.spec();
    let mut entries = Vec::new();

    for primal in &spec.primals {
        if let Some(svc) = MembraneService::for_binary(primal) {
            entries.push(BinaryIntegrity {
                binary: svc.binary,
                install_path: paths.install_path(svc),
                hash_algorithm: HashAlgorithm::Blake3,
                require_static_musl: true,
            });
        }
    }

    for sym in &spec.symbiotic {
        if let Some(svc) = MembraneService::for_binary(sym) {
            entries.push(BinaryIntegrity {
                binary: svc.binary,
                install_path: paths.install_path(svc),
                hash_algorithm: HashAlgorithm::Sha256,
                require_static_musl: false,
            });
        }
    }

    entries
}
