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
use crate::plasmid::harvest::{
    ChecksumEntry, HarvestResult, ProvenanceEntry, ProvenanceFile, SourceEntry,
};

#[derive(Deserialize)]
pub(super) struct SourcesFile {
    pub sources: BTreeMap<String, SourceEntry>,
}

pub(super) fn compute_blake3_file(path: &Path) -> Result<String> {
    super::checksum::compute_blake3(path).map_err(|e| {
        ShadowError::Io(std::io::Error::other(format!(
            "BLAKE3: cannot read {}: {e}",
            path.display()
        )))
    })
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
    #[derive(Deserialize)]
    struct ChecksumFile {
        #[serde(flatten)]
        targets: BTreeMap<String, BTreeMap<String, ChecksumEntry>>,
    }

    let checksums_path = depot_dir.join(cellmembrane_types::service::CHECKSUMS_FILE);
    let mut all_targets: BTreeMap<String, BTreeMap<String, ChecksumEntry>> = BTreeMap::new();
    let pre_existing_targets: Vec<String> = std::fs::read_to_string(&checksums_path)
        .ok()
        .and_then(|content| toml::from_str::<ChecksumFile>(&content).ok())
        .map(|parsed| {
            let keys = parsed.targets.keys().cloned().collect();
            all_targets = parsed.targets;
            keys
        })
        .unwrap_or_default();

    let target_checksums = all_targets.entry(target.to_string()).or_default();
    for result in built {
        let bin_path = staging_dir.join(&result.binary);
        if bin_path.exists() {
            let size = std::fs::metadata(&bin_path).map_or(0, |m| m.len());
            let hash = compute_blake3_file(&bin_path)?;
            target_checksums.insert(result.binary.clone(), ChecksumEntry { blake3: hash, size });
        }
    }

    let now = chrono::Utc::now().format(cellmembrane_types::service::ISO8601_UTC).to_string();
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

    for existing_target in &pre_existing_targets {
        if !all_targets.contains_key(existing_target) {
            return Err(ShadowError::Config(format!(
                "checksums validation gate: target section [{existing_target}] would be lost"
            )));
        }
    }

    crate::atomic_write(&checksums_path, out.as_bytes()).map_err(ShadowError::Io)?;
    Ok(())
}

async fn update_provenance(depot_dir: &Path, built: &[&HarvestResult]) -> Result<()> {
    let provenance_path = depot_dir.join(cellmembrane_types::service::PROVENANCE_FILE);
    let now = chrono::Utc::now().format(cellmembrane_types::service::ISO8601_UTC).to_string();
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

    let mut existing_prov: BTreeMap<String, ProvenanceEntry> =
        std::fs::read_to_string(&provenance_path)
            .ok()
            .and_then(|content| toml::from_str::<ProvenanceFile>(&content).ok())
            .map_or_else(BTreeMap::new, |parsed| parsed.entries);

    for result in built {
        if let Some(after_commit) = result.detail.split("commit=").nth(1) {
            let commit = after_commit
                .split_whitespace()
                .next()
                .unwrap_or_else(|| after_commit.trim());
            existing_prov.insert(
                result.binary.clone(),
                ProvenanceEntry {
                    version: None,
                    commit: Some(commit.to_string()),
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

    crate::atomic_write_async(&provenance_path, prov_out.as_bytes())
        .await
        .map_err(ShadowError::Io)?;
    Ok(())
}

fn hostname() -> String {
    std::env::var(cellmembrane_types::service::ENV_HOSTNAME)
        .or_else(|_| std::env::var(cellmembrane_types::service::ENV_HOST))
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

pub(crate) fn resolve_depot(override_dir: Option<&str>) -> Result<PathBuf> {
    let path = resolve_path(
        override_dir,
        cellmembrane_types::service::ENV_PLASMIDBIN_DEPOT,
        || {
            let eco_root = std::env::var(cellmembrane_types::service::ENV_ECOPRIMALS_ROOT)
                .ok()
                .map(PathBuf::from)
                .or_else(|| crate::resolve_workspace_root().ok())
                .unwrap_or_else(|| {
                    PathBuf::from(cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT)
                });
            let infra_depot =
                eco_root.join(cellmembrane_types::service::INFRA_PLASMID_BIN);
            if infra_depot.exists() {
                return infra_depot;
            }
            let flat_depot =
                eco_root.join(cellmembrane_types::service::PLASMID_BIN_DIR);
            if flat_depot.exists() {
                return flat_depot;
            }
            infra_depot
        },
    );
    if !path.exists() {
        return Err(ShadowError::Config(format!(
            "depot not found at {}",
            path.display()
        )));
    }
    Ok(path)
}

pub(super) fn load_sources(depot_dir: &Path) -> Result<BTreeMap<String, SourceEntry>> {
    let path = depot_dir.join("sources.toml");

    match std::fs::read_to_string(&path) {
        Ok(content) => {
            let parsed: SourcesFile = toml::from_str(&content)?;
            Ok(parsed.sources)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::info!("sources.toml not found — auto-provisioning from ecosystem manifest");
            provision_sources_from_manifest(depot_dir)
        }
        Err(e) => Err(ShadowError::Io(e)),
    }
}

/// Auto-generate `sources.toml` from the ecosystem manifest.
///
/// Scans manifest `[repos.*]` entries and creates a source entry for each
/// primal-category repo that exists in the service registry. Writes the
/// generated file to the depot dir for persistence.
fn provision_sources_from_manifest(depot_dir: &Path) -> Result<BTreeMap<String, SourceEntry>> {
    let workspace = std::env::var(cellmembrane_types::service::ENV_ECOPRIMALS_ROOT)
        .ok()
        .map(PathBuf::from)
        .or_else(|| crate::resolve_workspace_root().ok())
        .unwrap_or_else(|| {
            PathBuf::from(cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT)
        });

    let manifest = crate::manifest::load_from_workspace(&workspace).map_err(|e| {
        ShadowError::Config(format!(
            "sources.toml missing and manifest unavailable for auto-provision: {e}"
        ))
    })?;

    let registry: Vec<&str> = super::nucleus_primals();
    let mut sources = BTreeMap::new();
    let mut toml_out = String::from(
        "# Auto-provisioned from ecosystem manifest.\n\
         # Edit to override repo URLs, build args, or GPU flags.\n\n",
    );

    for (name, entry) in &manifest.repos {
        let slug = name.to_lowercase();
        if !registry.contains(&slug.as_str()) {
            continue;
        }

        let repo = if !entry.forgejo_repo.is_empty() {
            entry.forgejo_repo.clone()
        } else if !entry.github_repo.is_empty() {
            entry.github_repo.clone()
        } else {
            format!("{}/{name}", entry.org)
        };

        let _ = writeln!(toml_out, "[sources.{slug}]");
        let _ = writeln!(toml_out, "repo = \"{repo}\"");
        if entry.gpu {
            let _ = writeln!(toml_out, "gpu = true");
        }
        toml_out.push('\n');

        sources.insert(
            slug,
            SourceEntry {
                repo,
                private: false,
                build_args: None,
                binary_name: None,
                gpu: entry.gpu,
            },
        );
    }

    if sources.is_empty() {
        return Err(ShadowError::Config(
            "auto-provision: no primal repos found in manifest".into(),
        ));
    }

    let path = depot_dir.join("sources.toml");
    std::fs::write(&path, toml_out.as_bytes()).map_err(ShadowError::Io)?;
    tracing::info!(
        primals = sources.len(),
        path = %path.display(),
        "sources.toml auto-provisioned from ecosystem manifest"
    );

    Ok(sources)
}

/// Load build entries from the ecosystem manifest and enrich `SourceEntry` values.
///
/// When the manifest has `[build.<slug>]` entries, their `package` and `gpu`
/// fields override whatever `sources.toml` had (or didn't have). This is the
/// convergence path: manifest is authoritative for *how* to build, while
/// `sources.toml` remains authoritative for *where* to fetch releases.
pub(super) fn enrich_sources_from_manifest(
    sources: &mut BTreeMap<String, SourceEntry>,
) {
    let workspace = std::env::var(cellmembrane_types::service::ENV_ECOPRIMALS_ROOT)
        .ok()
        .map(PathBuf::from)
        .or_else(|| crate::resolve_workspace_root().ok())
        .unwrap_or_else(|| {
            PathBuf::from(cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT)
        });

    let Ok(manifest) = crate::manifest::load_from_workspace(&workspace) else {
        return;
    };

    for (slug, build) in &manifest.build {
        if let Some(source) = sources.get_mut(slug.as_str()) {
            source.build_args = Some(format!("-p {}", build.package));
            source.gpu = build.gpu;
            if build.binary_name != *slug {
                source.binary_name = Some(build.binary_name.clone());
            }
        }
    }
}

pub(super) fn load_provenance(depot_dir: &Path) -> Option<ProvenanceFile> {
    let path = depot_dir.join(cellmembrane_types::service::PROVENANCE_FILE);
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
pub(crate) fn detect_stale_primals(depot_dir: &Path) -> Result<StalenessReport> {
    let sources = load_sources(depot_dir)?;
    let provenance = load_provenance(depot_dir);
    let target = detect_target_triple();
    let staging_dir = depot_dir.join("primals").join(target);

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
        let staging = tmp.join("primals").join(target);
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
    fn enrich_sources_overlays_manifest_build_args() {
        use super::super::harvest::SourceEntry;

        let mut sources = BTreeMap::new();
        sources.insert(
            "beardog".to_string(),
            SourceEntry {
                repo: "ecoPrimals/bearDog".into(),
                private: true,
                build_args: None,
                binary_name: None,
                gpu: false,
            },
        );
        sources.insert(
            "barracuda".to_string(),
            SourceEntry {
                repo: "ecoPrimals/barraCuda".into(),
                private: false,
                build_args: None,
                binary_name: None,
                gpu: false,
            },
        );

        // enrich_sources_from_manifest reads the live manifest; in test
        // without one, it returns early (no-op). Verify the function
        // is callable and doesn't panic.
        enrich_sources_from_manifest(&mut sources);
        assert_eq!(sources.len(), 2);
        assert!(sources.contains_key("beardog"));
    }

    #[test]
    fn resolve_depot_fallback_path() {
        let result = resolve_depot(Some("/tmp/nonexistent_depot_xyz"));
        assert!(result.is_err());
    }

    #[test]
    fn load_sources_missing_file_triggers_provision() {
        let tmp = std::env::temp_dir().join("sources_auto_prov_test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let result = load_sources(&tmp);
        // Without a manifest available, auto-provision falls back with a
        // clear Config error (not an Io error for missing file).
        match result {
            Err(ShadowError::Config(msg)) => {
                assert!(
                    msg.contains("auto-provision"),
                    "error should mention auto-provision: {msg}"
                );
            }
            Ok(sources) => {
                // If manifest IS available (dev machine), it should have
                // written sources.toml and returned populated entries.
                assert!(!sources.is_empty());
                assert!(tmp.join("sources.toml").exists());
                let content = std::fs::read_to_string(tmp.join("sources.toml")).unwrap();
                assert!(content.contains("[sources."));
                assert!(content.contains("Auto-provisioned"));
            }
            Err(other) => panic!("unexpected error variant: {other}"),
        }

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn load_sources_existing_file_skips_provision() {
        let tmp = std::env::temp_dir().join("sources_no_prov_test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("sources.toml"),
            "[sources.beardog]\nrepo = \"ecoPrimals/bearDog\"\n",
        )
        .unwrap();

        let sources = load_sources(&tmp).unwrap();
        assert_eq!(sources.len(), 1);
        assert!(sources.contains_key("beardog"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn compute_blake3_file_on_empty() {
        let tmp = std::env::temp_dir().join("blake3_empty_test");
        std::fs::write(&tmp, b"").unwrap();
        let hash = compute_blake3_file(&tmp).unwrap();
        assert_eq!(hash.len(), 64);
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn update_checksums_preserves_other_targets() {
        use crate::plasmid::{HarvestResult, HarvestStatus};

        let tmp = std::env::temp_dir().join("checksums_multi_target_test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let target_a = "x86_64-unknown-linux-musl";
        let target_b = "aarch64-unknown-linux-musl";
        let staging_a = tmp.join("primals").join(target_a);
        let staging_b = tmp.join("primals").join(target_b);
        std::fs::create_dir_all(&staging_a).unwrap();
        std::fs::create_dir_all(&staging_b).unwrap();
        std::fs::write(staging_a.join("beardog"), b"x86 binary").unwrap();
        std::fs::write(staging_b.join("beardog"), b"arm binary").unwrap();

        let result_a = HarvestResult {
            binary: "beardog".into(),
            status: HarvestStatus::Built,
            detail: "100KB blake3=aaa commit=abc".into(),
        };
        let result_b = HarvestResult {
            binary: "beardog".into(),
            status: HarvestStatus::Built,
            detail: "90KB blake3=bbb commit=def".into(),
        };

        update_checksums(&tmp, target_a, &[&result_a], &staging_a).unwrap();
        let after_a = std::fs::read_to_string(tmp.join("checksums.toml")).unwrap();
        assert!(after_a.contains("[x86_64-unknown-linux-musl]"));
        assert!(after_a.contains("beardog"));

        update_checksums(&tmp, target_b, &[&result_b], &staging_b).unwrap();
        let after_b = std::fs::read_to_string(tmp.join("checksums.toml")).unwrap();
        assert!(
            after_b.contains("[x86_64-unknown-linux-musl]"),
            "target A section must survive after target B update"
        );
        assert!(after_b.contains("[aarch64-unknown-linux-musl]"));

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
