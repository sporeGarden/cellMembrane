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
    let path = resolve_path(override_dir, "PLASMIDBIN_DEPOT", || {
        let eco_root = std::env::var("ECOPRIMALS_ROOT")
            .unwrap_or_else(|_| "/home/irongate/Development/ecoPrimals".into());
        PathBuf::from(eco_root).join("infra/plasmidBin")
    });
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
