// SPDX-License-Identifier: AGPL-3.0-or-later

//! Depot metadata management — checksums, provenance, resolution.
//!
//! Manages the `plasmidBin` depot's metadata files:
//! - `checksums.toml`: BLAKE3 hashes for staged binaries
//! - `provenance.toml`: build traceability (commit, builder, timestamp)
//! - `sources.toml`: source registry

use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::{detect_target_triple, resolve_path};
use crate::error::{Result, ShadowError};
use crate::plasmid::harvest::{ChecksumEntry, HarvestResult, ProvenanceEntry, ProvenanceFile, SourceEntry};

#[derive(Deserialize)]
pub(super) struct SourcesFile {
    pub sources: BTreeMap<String, SourceEntry>,
}

pub(super) fn compute_blake3_file(path: &Path) -> String {
    let data = std::fs::read(path).unwrap_or_default();
    blake3::hash(&data).to_hex().to_string()
}

pub(super) async fn update_depot_metadata(
    depot_dir: &Path,
    target: &str,
    built: &[&HarvestResult],
) -> Result<()> {
    let staging_dir = depot_dir.join("primals").join(target);
    update_checksums(depot_dir, target, built, &staging_dir)?;
    update_provenance(depot_dir, built).await?;
    Ok(())
}

fn update_checksums(
    depot_dir: &Path,
    target: &str,
    built: &[&HarvestResult],
    staging_dir: &Path,
) -> Result<()> {
    let checksums_path = depot_dir.join("checksums.toml");

    let mut checksums: BTreeMap<String, ChecksumEntry> = BTreeMap::new();
    if checksums_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&checksums_path) {
            #[derive(Deserialize)]
            struct ChecksumFile {
                #[serde(flatten)]
                targets: BTreeMap<String, BTreeMap<String, ChecksumEntry>>,
            }
            if let Ok(parsed) = toml::from_str::<ChecksumFile>(&content) {
                if let Some(existing) = parsed.targets.get(target) {
                    checksums = existing.clone();
                }
            }
        }
    }

    for result in built {
        let bin_path = staging_dir.join(&result.binary);
        if bin_path.exists() {
            let size = std::fs::metadata(&bin_path).map_or(0, |m| m.len());
            let hash = compute_blake3_file(&bin_path);
            checksums.insert(result.binary.clone(), ChecksumEntry { blake3: hash, size });
        }
    }

    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let mut out = format!(
        "# plasmidBin checksums — BLAKE3\n# Generated: {now}\n# Target: {target}\n\n[{target}]\n"
    );
    for (name, entry) in &checksums {
        let _ = writeln!(
            out,
            "{name} = {{ blake3 = \"{}\", size = {} }}",
            entry.blake3, entry.size
        );
    }
    std::fs::write(&checksums_path, &out).map_err(ShadowError::Io)?;
    Ok(())
}

async fn update_provenance(depot_dir: &Path, built: &[&HarvestResult]) -> Result<()> {
    let provenance_path = depot_dir.join("provenance.toml");
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let header_target = detect_target_triple();

    let mut prov_out = format!(
        "# plasmidBin provenance — build traceability\n\
         generated = \"{now}\"\n\
         builder = \"{}\"\n\
         target = \"{header_target}\"\n\
         rustc = \"{}\"\n\n",
        hostname(),
        rustc_version().await,
    );

    let mut existing_prov: BTreeMap<String, ProvenanceEntry> = BTreeMap::new();
    if provenance_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&provenance_path) {
            if let Ok(parsed) = toml::from_str::<ProvenanceFile>(&content) {
                existing_prov = parsed.entries;
            }
        }
    }

    for result in built {
        if let Some(commit) = result.detail.split("commit=").nth(1) {
            existing_prov.insert(
                result.binary.clone(),
                ProvenanceEntry {
                    version: None,
                    commit: Some(commit.trim().to_string()),
                    source: None,
                },
            );
        }
    }

    for (name, entry) in &existing_prov {
        let _ = writeln!(prov_out, "[{name}]");
        if let Some(v) = &entry.version {
            let _ = writeln!(prov_out, "version = \"{v}\"");
        }
        if let Some(c) = &entry.commit {
            let _ = writeln!(prov_out, "commit = \"{c}\"");
        }
        if let Some(s) = &entry.source {
            let _ = writeln!(prov_out, "source = \"{s}\"");
        }
        prov_out.push('\n');
    }

    std::fs::write(&provenance_path, &prov_out).map_err(ShadowError::Io)?;
    Ok(())
}

fn hostname() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("HOST"))
        .unwrap_or_else(|_| "unknown".into())
}

async fn rustc_version() -> String {
    tokio::process::Command::new("rustc")
        .arg("--version")
        .output()
        .await
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown".into())
}

pub(super) fn resolve_depot(override_dir: Option<&str>) -> Result<PathBuf> {
    let path = resolve_path(
        override_dir,
        cellmembrane_types::service::ENV_PLASMIDBIN_DEPOT,
        || {
            let eco_root =
                std::env::var(cellmembrane_types::service::ENV_ECOPRIMALS_ROOT)
                    .ok()
                    .map(PathBuf::from)
                    .or_else(|| crate::resolve_workspace_root().ok())
                    .unwrap_or_else(|| {
                        PathBuf::from(cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT)
                    });
            eco_root.join("infra/plasmidBin")
        },
    );
    if !path.exists() {
        return Err(ShadowError::Parse(format!(
            "depot not found at {}",
            path.display()
        )));
    }
    Ok(path)
}

pub(super) fn load_sources(depot_dir: &Path) -> Result<BTreeMap<String, SourceEntry>> {
    let path = depot_dir.join("sources.toml");
    let content = std::fs::read_to_string(&path)
        .map_err(|e| ShadowError::Parse(format!("cannot read sources.toml: {e}")))?;

    let parsed: SourcesFile = toml::from_str(&content)?;
    Ok(parsed.sources)
}

pub(super) fn load_provenance(depot_dir: &Path) -> Option<ProvenanceFile> {
    let path = depot_dir.join("provenance.toml");
    let content = std::fs::read_to_string(path).ok()?;
    toml::from_str(&content).ok()
}

/// Staleness report for a single primal in the depot.
#[derive(Debug, Clone)]
pub struct StalenessEntry {
    /// Primal binary name.
    pub name: String,
    /// Whether the binary file exists in the depot.
    pub binary_exists: bool,
    /// Recorded commit from provenance (if any).
    pub provenance_commit: Option<String>,
    /// Whether this primal is considered stale (provenance missing or binary absent).
    pub stale: bool,
}

/// Full staleness report across the depot.
#[derive(Debug, Clone)]
pub struct StalenessReport {
    /// Per-primal staleness entries.
    pub entries: Vec<StalenessEntry>,
    /// Total primals evaluated.
    pub total: usize,
    /// Count of stale primals.
    pub stale_count: usize,
    /// Count of current (non-stale) primals.
    pub current_count: usize,
}

impl std::fmt::Display for StalenessReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "depot staleness: {}/{} current, {} stale",
            self.current_count, self.total, self.stale_count
        )?;
        if self.stale_count > 0 {
            let stale_names: Vec<&str> = self
                .entries
                .iter()
                .filter(|e| e.stale)
                .map(|e| e.name.as_str())
                .collect();
            write!(f, " [{}]", stale_names.join(", "))?;
        }
        Ok(())
    }
}

/// Detect stale primals by comparing depot binary presence and provenance records
/// against the sources registry. A primal is stale if:
/// - It has no provenance entry (never built)
/// - It has no binary in the depot staging directory
/// - Its provenance has no commit recorded
///
/// This is a local-only check — no network calls.
/// If `depot_dir` is `None`, resolves depot from env/defaults.
pub fn detect_stale_primals(depot_dir: &Path) -> Result<StalenessReport> {
    let sources = load_sources(depot_dir)?;
    let provenance = load_provenance(depot_dir);
    let target = detect_target_triple();
    let staging_dir = depot_dir.join("primals").join(&target);

    let registry = super::nucleus_primals();
    let mut entries = Vec::with_capacity(registry.len());
    let mut stale_count = 0usize;
    let mut current_count = 0usize;

    for &primal in &registry {
        if !sources.contains_key(primal) {
            continue;
        }

        let binary_exists = staging_dir.join(primal).exists();
        let provenance_commit = provenance
            .as_ref()
            .and_then(|p| p.entries.get(primal))
            .and_then(|e| e.commit.clone());

        let stale = !binary_exists || provenance_commit.is_none();

        if stale {
            stale_count += 1;
        } else {
            current_count += 1;
        }

        entries.push(StalenessEntry {
            name: primal.to_string(),
            binary_exists,
            provenance_commit,
            stale,
        });
    }

    Ok(StalenessReport {
        total: entries.len(),
        entries,
        stale_count,
        current_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staleness_report_display_all_current() {
        let report = StalenessReport {
            entries: vec![StalenessEntry {
                name: "beardog".into(),
                binary_exists: true,
                provenance_commit: Some("abc123".into()),
                stale: false,
            }],
            total: 1,
            stale_count: 0,
            current_count: 1,
        };
        let s = report.to_string();
        assert!(s.contains("1/1 current"));
        assert!(s.contains("0 stale"));
        assert!(!s.contains('['));
    }

    #[test]
    fn staleness_report_display_with_stale() {
        let report = StalenessReport {
            entries: vec![
                StalenessEntry {
                    name: "beardog".into(),
                    binary_exists: true,
                    provenance_commit: Some("abc".into()),
                    stale: false,
                },
                StalenessEntry {
                    name: "songbird".into(),
                    binary_exists: false,
                    provenance_commit: None,
                    stale: true,
                },
            ],
            total: 2,
            stale_count: 1,
            current_count: 1,
        };
        let s = report.to_string();
        assert!(s.contains("1/2 current"));
        assert!(s.contains("1 stale"));
        assert!(s.contains("[songbird]"));
    }

    #[test]
    fn detect_stale_primals_with_tempdir() {
        let tmp = std::env::temp_dir().join("depot_staleness_test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("sources.toml"),
            "[sources.beardog]\nrepo = \"x\"\n[sources.songbird]\nrepo = \"y\"\n",
        )
        .unwrap();
        std::fs::write(
            tmp.join("provenance.toml"),
            "generated = \"2026-01-01\"\n\n[beardog]\ncommit = \"aaa\"\n",
        )
        .unwrap();

        let target = detect_target_triple();
        let staging = tmp.join("primals").join(&target);
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("beardog"), b"binary").unwrap();

        let report = detect_stale_primals(&tmp).unwrap();
        assert_eq!(report.current_count, 1);
        assert_eq!(report.stale_count, 1);
        assert_eq!(report.entries[1].name, "songbird");
        assert!(report.entries[1].stale);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn detect_stale_missing_provenance_marks_all_stale() {
        let tmp = std::env::temp_dir().join("depot_staleness_no_prov");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("sources.toml"),
            "[sources.beardog]\nrepo = \"x\"\n",
        )
        .unwrap();

        let report = detect_stale_primals(&tmp).unwrap();
        assert_eq!(report.stale_count, 1);
        assert!(report.entries[0].stale);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn resolve_depot_fallback_path() {
        let result = resolve_depot(Some("/tmp/nonexistent_depot_xyz"));
        assert!(result.is_err());
    }

    #[test]
    fn compute_blake3_file_on_empty() {
        let tmp = std::env::temp_dir().join("blake3_empty_test");
        std::fs::write(&tmp, b"").unwrap();
        let hash = compute_blake3_file(&tmp);
        assert_eq!(hash.len(), 64);
        let _ = std::fs::remove_file(&tmp);
    }
}
