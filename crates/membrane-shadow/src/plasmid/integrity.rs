// SPDX-License-Identifier: AGPL-3.0-or-later

//! Depot integrity — BLAKE3 checksum generation and verification.
//!
//! Scans arch directories under `primals/`, computes BLAKE3 hashes,
//! and writes/verifies `checksums.toml` for tamper detection.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::Path;

use serde::Deserialize;

use super::depot::compute_blake3_file;
use super::harvest::ChecksumEntry;
use crate::error::{Result, ShadowError};

/// Result of a depot integrity check (generate or verify).
#[derive(Debug, Clone, serde::Serialize)]
pub struct IntegrityReport {
    /// Absolute path to the depot directory.
    pub depot_path: String,
    /// Architecture directories found/checked.
    pub architectures: Vec<String>,
    /// Total binary files processed.
    pub total_binaries: u32,
    /// Count of binaries that passed verification.
    pub verified: u32,
    /// Binaries whose hash did not match the recorded value.
    pub mismatches: Vec<IntegrityMismatch>,
    /// Binaries listed in checksums.toml but not on disk.
    pub missing: Vec<String>,
    /// Whether checksums.toml was (re)generated (vs only verified).
    pub generated: bool,
}

/// A single binary whose hash does not match the recorded checksum.
#[derive(Debug, Clone, serde::Serialize)]
pub struct IntegrityMismatch {
    /// Binary filename.
    pub binary: String,
    /// Architecture directory containing the binary.
    pub arch: String,
    /// Hash recorded in checksums.toml.
    pub expected: String,
    /// Actual computed BLAKE3 hash.
    pub actual: String,
}

/// Scan all arch directories under `primals/`, compute BLAKE3 hashes, and write
/// a fresh `checksums.toml`. Used after harvest to regenerate integrity metadata.
pub(crate) fn generate_checksums(depot_dir: &Path) -> Result<IntegrityReport> {
    let primals_dir = depot_dir.join("primals");
    let mut all_targets: BTreeMap<String, BTreeMap<String, ChecksumEntry>> = BTreeMap::new();
    let mut architectures: Vec<String> = Vec::new();
    let mut total_binaries: u32 = 0;

    if primals_dir.is_dir() {
        let mut entries: Vec<_> = std::fs::read_dir(&primals_dir)
            .map_err(ShadowError::Io)?
            .filter_map(std::result::Result::ok)
            .filter(|e| e.path().is_dir())
            .collect();
        entries.sort_by_key(std::fs::DirEntry::file_name);

        for entry in entries {
            let arch = entry.file_name().to_string_lossy().to_string();
            let arch_dir = entry.path();
            let mut arch_checksums: BTreeMap<String, ChecksumEntry> = BTreeMap::new();

            let mut files: Vec<_> = std::fs::read_dir(&arch_dir)
                .map_err(ShadowError::Io)?
                .filter_map(std::result::Result::ok)
                .filter(|e| e.path().is_file())
                .collect();
            files.sort_by_key(std::fs::DirEntry::file_name);

            for file in files {
                let name = file.file_name().to_string_lossy().to_string();
                let path = file.path();
                let size = std::fs::metadata(&path).map_or(0, |m| m.len());
                let hash = compute_blake3_file(&path);
                arch_checksums.insert(name, ChecksumEntry { blake3: hash, size });
                total_binaries += 1;
            }

            if !arch_checksums.is_empty() {
                architectures.push(arch.clone());
                all_targets.insert(arch, arch_checksums);
            }
        }
    }

    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let mut out = format!("# plasmidBin checksums — BLAKE3\n# Generated: {now}\n\n");
    for (tgt, entries) in &all_targets {
        let _ = writeln!(out, "[{tgt}]");
        for (name, entry) in entries {
            let _ = writeln!(
                out,
                "{name} = {{ blake3 = \"{}\", size = {} }}",
                entry.blake3, entry.size
            );
        }
        out.push('\n');
    }

    let checksums_path = depot_dir.join(cellmembrane_types::service::CHECKSUMS_FILE);
    crate::atomic_write(&checksums_path, out.as_bytes()).map_err(ShadowError::Io)?;

    Ok(IntegrityReport {
        depot_path: depot_dir.display().to_string(),
        architectures,
        total_binaries,
        verified: 0,
        mismatches: Vec::new(),
        missing: Vec::new(),
        generated: true,
    })
}

/// Read existing `checksums.toml` and verify every listed binary matches its
/// recorded BLAKE3 hash. Reports mismatches and missing files.
pub(crate) fn verify_checksums(depot_dir: &Path) -> Result<IntegrityReport> {
    #[derive(Deserialize)]
    struct ChecksumFile {
        #[serde(flatten)]
        targets: BTreeMap<String, BTreeMap<String, ChecksumEntry>>,
    }

    let checksums_path = depot_dir.join(cellmembrane_types::service::CHECKSUMS_FILE);
    let content = std::fs::read_to_string(&checksums_path).map_err(ShadowError::Io)?;
    let parsed: ChecksumFile = toml::from_str(&content)?;

    let primals_dir = depot_dir.join("primals");
    let mut architectures: Vec<String> = Vec::new();
    let mut total_binaries: u32 = 0;
    let mut verified: u32 = 0;
    let mut mismatches: Vec<IntegrityMismatch> = Vec::new();
    let mut missing: Vec<String> = Vec::new();

    for (arch, entries) in &parsed.targets {
        architectures.push(arch.clone());
        let arch_dir = primals_dir.join(arch);

        for (name, entry) in entries {
            total_binaries += 1;
            let bin_path = arch_dir.join(name);
            if !bin_path.exists() {
                missing.push(format!("{arch}/{name}"));
                continue;
            }
            let actual = compute_blake3_file(&bin_path);
            if actual == entry.blake3 {
                verified += 1;
            } else {
                mismatches.push(IntegrityMismatch {
                    binary: name.clone(),
                    arch: arch.clone(),
                    expected: entry.blake3.clone(),
                    actual,
                });
            }
        }
    }

    Ok(IntegrityReport {
        depot_path: depot_dir.display().to_string(),
        architectures,
        total_binaries,
        verified,
        mismatches,
        missing,
        generated: false,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use super::*;

    #[test]
    fn test_generate_checksums_empty_depot() {
        let tmp = std::env::temp_dir().join("integrity_empty_depot");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("primals")).unwrap();

        let report = generate_checksums(&tmp).unwrap();
        assert_eq!(report.total_binaries, 0);
        assert!(report.architectures.is_empty());
        assert!(report.generated);
        assert!(report.mismatches.is_empty());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_generate_checksums_with_binaries() {
        let tmp = std::env::temp_dir().join("integrity_gen_bins");
        let _ = std::fs::remove_dir_all(&tmp);
        let arch_dir = tmp.join("primals").join("x86_64-unknown-linux-musl");
        std::fs::create_dir_all(&arch_dir).unwrap();
        std::fs::write(arch_dir.join("beardog"), b"fake beardog binary").unwrap();
        std::fs::write(arch_dir.join("songbird"), b"fake songbird binary").unwrap();

        let report = generate_checksums(&tmp).unwrap();
        assert_eq!(report.total_binaries, 2);
        assert_eq!(report.architectures.len(), 1);
        assert_eq!(report.architectures[0], "x86_64-unknown-linux-musl");
        assert!(report.generated);

        let checksums = std::fs::read_to_string(tmp.join("checksums.toml")).unwrap();
        assert!(checksums.contains("[x86_64-unknown-linux-musl]"));
        assert!(checksums.contains("beardog"));
        assert!(checksums.contains("songbird"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_verify_checksums_matches() {
        let tmp = std::env::temp_dir().join("integrity_verify_ok");
        let _ = std::fs::remove_dir_all(&tmp);
        let arch_dir = tmp.join("primals").join("x86_64-unknown-linux-musl");
        std::fs::create_dir_all(&arch_dir).unwrap();
        std::fs::write(arch_dir.join("beardog"), b"good binary").unwrap();

        generate_checksums(&tmp).unwrap();
        let report = verify_checksums(&tmp).unwrap();
        assert_eq!(report.verified, 1);
        assert!(report.mismatches.is_empty());
        assert!(report.missing.is_empty());
        assert!(!report.generated);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_verify_checksums_mismatch() {
        let tmp = std::env::temp_dir().join("integrity_verify_mismatch");
        let _ = std::fs::remove_dir_all(&tmp);
        let arch_dir = tmp.join("primals").join("aarch64-unknown-linux-musl");
        std::fs::create_dir_all(&arch_dir).unwrap();
        std::fs::write(arch_dir.join("beardog"), b"original content").unwrap();

        generate_checksums(&tmp).unwrap();

        std::fs::write(arch_dir.join("beardog"), b"tampered content").unwrap();

        let report = verify_checksums(&tmp).unwrap();
        assert_eq!(report.mismatches.len(), 1);
        assert_eq!(report.mismatches[0].binary, "beardog");
        assert_eq!(report.mismatches[0].arch, "aarch64-unknown-linux-musl");
        assert_ne!(report.mismatches[0].expected, report.mismatches[0].actual);
        assert_eq!(report.verified, 0);

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
