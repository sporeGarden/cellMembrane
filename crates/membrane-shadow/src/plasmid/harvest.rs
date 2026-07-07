// SPDX-License-Identifier: AGPL-3.0-or-later

//! `plasmid.harvest` — Build primal binaries from source, verify, and stage.
//!
//! Implements the zero-touch binary harvest pipeline:
//! 1. Read `sources.toml` to discover repos + build args
//! 2. Compare HEAD commits against `provenance.toml` to detect drift
//! 3. Clone changed repos (shallow)
//! 4. Cross-compile for target triple (musl static)
//! 5. Compute BLAKE3 checksum
//! 6. Stage binary to plasmidBin depot
//! 7. Update `checksums.toml` and `provenance.toml`

use crate::ShadowOutcome;
use crate::error::{Result, ShadowError};
use crate::manifest::ManifestBuildConfig;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use tracing::{info, warn};

use super::{detect_target_triple, nucleus_primals, toolchain};

/// Parsed CLI arguments for `plasmid.harvest`.
pub struct HarvestArgs {
    /// Single primal to harvest (None = all with changes).
    pub primal: Option<String>,
    /// Force rebuild even if commit hasn't changed.
    pub force: bool,
    /// Show what would be built without executing.
    pub dry_run: bool,
    /// Override plasmidBin depot path.
    pub depot_dir: Option<String>,
    /// Override target triple (e.g. `aarch64-unknown-linux-musl` for cross-compile).
    pub target: Option<String>,
}

/// Outcome of harvesting a single primal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarvestResult {
    /// Binary name.
    pub binary: String,
    /// Outcome status.
    pub status: HarvestStatus,
    /// Human-readable detail.
    pub detail: String,
}

/// Status of a single primal harvest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HarvestStatus {
    /// Built and staged successfully.
    Built,
    /// No changes detected — skipped.
    Current,
    /// Build failed.
    Failed,
    /// Skipped (private repo without access, etc.).
    Skipped,
}

/// Source entry from `sources.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct SourceEntry {
    /// Repository path (e.g. `ecoPrimals/bearDog`).
    pub repo: String,
    /// Whether this is a private repo (SSH-only access).
    #[serde(default)]
    pub private: bool,
    /// Additional cargo build arguments.
    #[serde(default)]
    pub build_args: Option<String>,
    /// Override binary name (when it differs from primal name).
    #[serde(default)]
    pub binary_name: Option<String>,
    /// Whether this primal needs a glibc build for GPU/dlopen access.
    /// When true, harvest builds both musl and gnu targets.
    #[serde(default)]
    pub gpu: bool,
}

/// Provenance entry from `provenance.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvenanceEntry {
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
}

/// Full provenance file structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvenanceFile {
    #[serde(default)]
    pub generated: Option<String>,
    #[serde(default)]
    pub builder: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub rustc: Option<String>,
    #[serde(flatten)]
    pub entries: BTreeMap<String, ProvenanceEntry>,
}

/// Wrapper for `sources.toml` deserialization.
#[cfg(test)]
#[derive(Deserialize)]
struct SourcesFile {
    sources: BTreeMap<String, SourceEntry>,
}

/// Checksum entry from `checksums.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChecksumEntry {
    pub blake3: String,
    pub size: u64,
}

/// Compute which target triples to build for a given primal.
/// If CLI overrides target, use that. Otherwise: default host triple,
/// plus gnu triple if the source is marked `gpu = true`.
fn targets_for_primal(cli_target: Option<&str>, source: &SourceEntry) -> Vec<String> {
    if let Some(t) = cli_target {
        return vec![t.to_string()];
    }
    let mut targets = vec![detect_target_triple()];
    if source.gpu && cfg!(target_arch = "x86_64") {
        let gnu = cellmembrane_types::TargetArch::X86_64Gnu
            .triple()
            .to_string();
        if !targets.contains(&gnu) {
            targets.push(gnu);
        }
    }
    targets
}

/// Harvest primals: detect changes, build, checksum, stage.
///
/// For GPU primals (`source.gpu = true`), builds both musl and gnu targets
/// so gates with GPU hardware can run CUDA/Vulkan workloads via `dlopen`.
pub async fn harvest(args: &HarvestArgs) -> Result<ShadowOutcome> {
    let depot_dir = resolve_depot(args.depot_dir.as_deref())?;
    let sources = load_sources(&depot_dir)?;
    let provenance = load_provenance(&depot_dir);

    let manifest_configs = load_manifest_build_configs();

    let primals_to_harvest = determine_primals(args, &sources)?;

    let mut results: Vec<HarvestResult> = Vec::new();
    let mut targets_built: Vec<String> = Vec::new();

    for primal in &primals_to_harvest {
        let Some(source) = sources.get(primal.as_str()) else {
            results.push(HarvestResult {
                binary: primal.clone(),
                status: HarvestStatus::Skipped,
                detail: "not in sources.toml".into(),
            });
            continue;
        };

        let mut source = source.clone();
        if let Some(mcfg) = manifest_configs.get(primal.as_str()) {
            apply_manifest_overrides(&mut source, mcfg);
        }

        let needs_rebuild = args.force
            || drift::has_upstream_changes(primal, &source, provenance.as_ref(), &depot_dir).await;

        if !needs_rebuild {
            results.push(HarvestResult {
                binary: primal.clone(),
                status: HarvestStatus::Current,
                detail: "commit unchanged".into(),
            });
            continue;
        }

        let manifest_linker = manifest_configs
            .get(primal.as_str())
            .and_then(|c| c.linker.as_deref());

        let targets = targets_for_primal(args.target.as_deref(), &source);
        for target in &targets {
            if args.dry_run {
                results.push(HarvestResult {
                    binary: primal.clone(),
                    status: HarvestStatus::Built,
                    detail: format!(
                        "dry-run: would clone {} and build for {target}",
                        source.repo
                    ),
                });
                continue;
            }

            let result =
                harvest_one(primal, &source, target, &depot_dir, manifest_linker).await;
            if matches!(result.status, HarvestStatus::Built) && !targets_built.contains(target) {
                targets_built.push(target.clone());
            }
            results.push(result);
        }
    }

    if !args.dry_run {
        let built: Vec<&HarvestResult> = results
            .iter()
            .filter(|r| matches!(r.status, HarvestStatus::Built))
            .collect();
        if !built.is_empty() {
            for target in &targets_built {
                let arch_results: Vec<&HarvestResult> = built
                    .iter()
                    .copied()
                    .filter(|r| r.detail.contains(target))
                    .collect();
                if !arch_results.is_empty() {
                    if let Err(e) = update_depot_metadata(&depot_dir, target, &arch_results).await {
                        warn!(target, error = %e, "failed to update depot metadata");
                    }
                }
            }
            drift::publish_depot_checksums(&depot_dir).await;
        }
    }

    Ok(format_harvest_outcome(&results))
}

fn determine_primals(
    args: &HarvestArgs,
    sources: &BTreeMap<String, SourceEntry>,
) -> Result<Vec<String>> {
    if let Some(name) = args.primal.as_deref() {
        if !sources.contains_key(name) {
            return Err(ShadowError::Config(format!(
                "'{name}' not found in sources.toml"
            )));
        }
        Ok(vec![name.to_string()])
    } else {
        let registry_primals = nucleus_primals();
        Ok(registry_primals
            .into_iter()
            .filter(|p| sources.contains_key(*p))
            .map(ToString::to_string)
            .collect())
    }
}

use super::drift;

async fn harvest_one(
    primal: &str,
    source: &SourceEntry,
    target: &str,
    depot_dir: &Path,
    manifest_linker: Option<&str>,
) -> HarvestResult {
    let build_root = std::env::temp_dir().join("membrane-harvest");
    let clone_dir = build_root.join(primal);

    if let Err(e) = drift::clone_source(primal, source, &build_root, &clone_dir).await {
        let status = if source.private {
            HarvestStatus::Skipped
        } else {
            HarvestStatus::Failed
        };
        return HarvestResult {
            binary: primal.into(),
            status,
            detail: e.to_string(),
        };
    }

    let head_commit = crate::git_ops::head_short(&clone_dir)
        .await
        .unwrap_or_default();

    if let Some(warning) =
        drift::check_clone_freshness(primal, source, &clone_dir, &head_commit).await
    {
        warn!(primal, warning, "freshness warning");
    }

    if let Err(e) = toolchain::build_binary(source, target, &clone_dir, manifest_linker).await {
        return HarvestResult {
            binary: primal.into(),
            status: HarvestStatus::Failed,
            detail: e.to_string(),
        };
    }

    let binary_name = source.binary_name.as_deref().unwrap_or(primal);
    let bin_path = clone_dir
        .join("target")
        .join(target)
        .join("release")
        .join(binary_name);

    if !bin_path.exists() {
        return HarvestResult {
            binary: primal.into(),
            status: HarvestStatus::Failed,
            detail: format!("binary not found at {}", bin_path.display()),
        };
    }

    // BUILD-ELF-01: validate architecture before staging
    if let Err(e) = validate_elf_arch(&bin_path, target).await {
        return HarvestResult {
            binary: primal.into(),
            status: HarvestStatus::Failed,
            detail: e.to_string(),
        };
    }

    toolchain::strip_binary(&bin_path, primal, target).await;

    match stage_to_depot_async(primal, &bin_path, depot_dir, target).await {
        Ok((size, blake3)) => {
            let _ = tokio::fs::remove_dir_all(&clone_dir).await;
            HarvestResult {
                binary: primal.into(),
                status: HarvestStatus::Built,
                detail: format!(
                    "{}KB blake3={} commit={}",
                    size / 1024,
                    &blake3[..16],
                    &head_commit[..std::cmp::min(8, head_commit.len())]
                ),
            }
        }
        Err(e) => HarvestResult {
            binary: primal.into(),
            status: HarvestStatus::Failed,
            detail: e.to_string(),
        },
    }
}

pub(super) use toolchain::{
    ANDROID_TARGET, ENV_ANDROID_NDK_HOME, resolve_ndk_linker, validate_elf_arch,
};

/// Async depot staging: copy binary → atomic rename → BLAKE3 checksum.
/// Shared by both `plasmid.build` and `plasmid.harvest`.
pub(super) async fn stage_to_depot_async(
    primal: &str,
    bin_path: &Path,
    depot_dir: &Path,
    target: &str,
) -> crate::Result<(u64, String)> {
    let staging_dir = depot_dir.join("primals").join(target);
    tokio::fs::create_dir_all(&staging_dir).await.map_err(|e| {
        crate::error::ShadowError::Build(format!("depot staging dir create failed: {e}"))
    })?;
    let dest = staging_dir.join(primal);
    let tmp = staging_dir.join(format!(".{primal}.new"));

    tokio::fs::copy(bin_path, &tmp)
        .await
        .map_err(|e| crate::error::ShadowError::Build(format!("copy to depot failed: {e}")))?;
    tokio::fs::rename(&tmp, &dest)
        .await
        .map_err(|e| crate::error::ShadowError::Build(format!("atomic rename failed: {e}")))?;

    let size = tokio::fs::metadata(&dest).await.map_or(0, |m| m.len());
    let blake3 = super::compute_blake3_file_async(dest).await;
    Ok((size, blake3))
}

pub(super) use super::depot::{
    load_provenance, load_sources, resolve_depot, update_depot_metadata,
};

/// Load build configs from the ecosystem manifest for all primals.
///
/// Returns a map keyed by lowercase primal name. If the manifest is unavailable,
/// returns an empty map (graceful fallback to `sources.toml` only).
fn load_manifest_build_configs() -> BTreeMap<String, ManifestBuildConfig> {
    let workspace = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_ECOPRIMALS_ROOT,
        cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT,
    );
    let workspace_path = std::path::Path::new(&workspace);
    let manifest = match crate::manifest::load_from_workspace(workspace_path) {
        Ok(m) => m,
        Err(e) => {
            info!(error = %e, "manifest unavailable — using sources.toml only");
            return BTreeMap::new();
        }
    };

    let mut configs = BTreeMap::new();
    for (name, entry) in &manifest.repos {
        let cfg = ManifestBuildConfig {
            package: entry.package.clone(),
            linker: entry.linker.clone(),
            gpu: entry.gpu,
        };
        if cfg.package.is_some() || cfg.linker.is_some() || cfg.gpu {
            let lower = name.to_lowercase();
            configs.insert(lower, cfg);
        }
    }
    configs
}

/// Apply manifest build config overrides onto a `SourceEntry`.
///
/// Manifest `package` becomes `build_args = "-p <package>"` (overrides existing).
/// Manifest `gpu` overlays onto `source.gpu`.
fn apply_manifest_overrides(source: &mut SourceEntry, cfg: &ManifestBuildConfig) {
    if let Some(pkg) = &cfg.package {
        source.build_args = Some(format!("-p {pkg}"));
    }
    if cfg.gpu {
        source.gpu = true;
    }
}

fn format_harvest_outcome(results: &[HarvestResult]) -> ShadowOutcome {
    let built = results
        .iter()
        .filter(|r| matches!(r.status, HarvestStatus::Built))
        .count();
    let current = results
        .iter()
        .filter(|r| matches!(r.status, HarvestStatus::Current))
        .count();
    let failed = results
        .iter()
        .filter(|r| matches!(r.status, HarvestStatus::Failed))
        .count();
    let skipped = results
        .iter()
        .filter(|r| matches!(r.status, HarvestStatus::Skipped))
        .count();

    let msg =
        format!("harvest: {built} built, {current} current, {skipped} skipped, {failed} failed");

    ShadowOutcome {
        ok: failed == 0,
        message: msg,
        data: serde_json::to_value(results).ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_entry_deserialize() {
        let toml_str = r#"
[sources.beardog]
repo = "https://git.primals.eco/ecoPrimals/bearDog.git"
private = true
build_args = "--features server"
"#;
        let parsed: SourcesFile = toml::from_str(toml_str).unwrap();
        let entry = &parsed.sources["beardog"];
        assert_eq!(entry.repo, "https://git.primals.eco/ecoPrimals/bearDog.git");
        assert!(entry.private);
        assert_eq!(entry.build_args.as_deref(), Some("--features server"));
        assert!(entry.binary_name.is_none());
    }

    #[test]
    fn source_entry_minimal() {
        let toml_str = r#"
[sources.songbird]
repo = "https://git.primals.eco/ecoPrimals/songBird.git"
"#;
        let parsed: SourcesFile = toml::from_str(toml_str).unwrap();
        let entry = &parsed.sources["songbird"];
        assert!(!entry.private);
        assert!(entry.build_args.is_none());
    }

    #[test]
    fn provenance_file_roundtrip() {
        let mut entries = BTreeMap::new();
        entries.insert(
            "beardog".into(),
            ProvenanceEntry {
                version: Some("0.9.1".into()),
                commit: Some("abc123".into()),
                source: Some("forgejo".into()),
            },
        );
        let prov = ProvenanceFile {
            generated: Some("2026-06-07".into()),
            builder: Some("eastGate".into()),
            target: Some("x86_64-unknown-linux-musl".into()),
            rustc: Some("1.96.0".into()),
            entries,
        };
        let serialized = toml::to_string_pretty(&prov).unwrap();
        let deserialized: ProvenanceFile = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.generated.as_deref(), Some("2026-06-07"));
        assert_eq!(
            deserialized.entries["beardog"].commit.as_deref(),
            Some("abc123")
        );
    }

    #[test]
    fn checksum_entry_serde() {
        let entry = ChecksumEntry {
            blake3: "deadbeef".into(),
            size: 42_000,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let back: ChecksumEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back.blake3, "deadbeef");
        assert_eq!(back.size, 42_000);
    }

    #[test]
    fn harvest_result_status_display() {
        let result = HarvestResult {
            binary: "beardog".into(),
            status: HarvestStatus::Built,
            detail: "compiled OK".into(),
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["status"], "Built");
        assert_eq!(json["binary"], "beardog");
    }

    #[test]
    fn format_harvest_outcome_all_current() {
        let results = vec![
            HarvestResult {
                binary: "a".into(),
                status: HarvestStatus::Current,
                detail: "no change".into(),
            },
            HarvestResult {
                binary: "b".into(),
                status: HarvestStatus::Current,
                detail: "no change".into(),
            },
        ];
        let outcome = format_harvest_outcome(&results);
        assert!(outcome.ok);
        assert!(outcome.message.contains("0 built"));
        assert!(outcome.message.contains("2 current"));
    }

    #[test]
    fn format_harvest_outcome_with_failure() {
        let results = vec![
            HarvestResult {
                binary: "a".into(),
                status: HarvestStatus::Built,
                detail: "ok".into(),
            },
            HarvestResult {
                binary: "b".into(),
                status: HarvestStatus::Failed,
                detail: "build error".into(),
            },
        ];
        let outcome = format_harvest_outcome(&results);
        assert!(!outcome.ok);
        assert!(outcome.message.contains("1 built"));
        assert!(outcome.message.contains("1 failed"));
    }

    #[test]
    fn load_sources_from_tempdir() {
        let tmp = std::env::temp_dir().join("harvest_test_sources");
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("sources.toml"),
            r#"
[sources.beardog]
repo = "https://example.com/beardog.git"
[sources.songbird]
repo = "https://example.com/songbird.git"
private = true
"#,
        )
        .unwrap();

        let sources = load_sources(&tmp).unwrap();
        assert_eq!(sources.len(), 2);
        assert!(sources.contains_key("beardog"));
        assert!(sources["songbird"].private);

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn load_provenance_missing_returns_none() {
        let tmp = std::env::temp_dir().join("harvest_test_no_prov");
        std::fs::create_dir_all(&tmp).unwrap();
        assert!(load_provenance(&tmp).is_none());
        std::fs::remove_dir_all(&tmp).ok();
    }

    fn test_source_entry(repo: &str) -> SourceEntry {
        SourceEntry {
            repo: repo.into(),
            private: false,
            build_args: None,
            binary_name: None,
            gpu: false,
        }
    }

    #[test]
    fn determine_primals_single_valid() {
        let mut sources = BTreeMap::new();
        sources.insert(
            "beardog".to_string(),
            test_source_entry("ecoPrimals/bearDog"),
        );
        let args = HarvestArgs {
            primal: Some("beardog".into()),
            force: false,
            dry_run: false,
            depot_dir: None,
            target: None,
        };
        let result = determine_primals(&args, &sources).unwrap();
        assert_eq!(result, vec!["beardog"]);
    }

    #[test]
    fn determine_primals_single_invalid() {
        let sources = BTreeMap::new();
        let args = HarvestArgs {
            primal: Some("nonexistent".into()),
            force: false,
            dry_run: false,
            depot_dir: None,
            target: None,
        };
        assert!(determine_primals(&args, &sources).is_err());
    }

    #[test]
    fn determine_primals_all_filtered() {
        let mut sources = BTreeMap::new();
        sources.insert(
            "beardog".to_string(),
            test_source_entry("ecoPrimals/bearDog"),
        );
        sources.insert(
            "songbird".to_string(),
            test_source_entry("ecoPrimals/songbird"),
        );
        let args = HarvestArgs {
            primal: None,
            force: false,
            dry_run: false,
            depot_dir: None,
            target: None,
        };
        let result = determine_primals(&args, &sources).unwrap();
        assert!(result.contains(&"beardog".to_string()));
    }

    #[test]
    fn targets_for_regular_primal() {
        let source = test_source_entry("ecoPrimals/bearDog");
        let targets = targets_for_primal(None, &source);
        assert_eq!(targets.len(), 1);
        assert!(targets[0].contains("musl"));
    }

    #[test]
    fn targets_for_gpu_primal() {
        let mut source = test_source_entry("ecoPrimals/barracuda");
        source.gpu = true;
        let targets = targets_for_primal(None, &source);
        if cfg!(target_arch = "x86_64") {
            assert_eq!(targets.len(), 2);
            assert!(targets[0].contains("musl"));
            assert!(targets[1].contains("gnu"));
        }
    }

    #[test]
    fn targets_cli_override_ignores_gpu() {
        let mut source = test_source_entry("ecoPrimals/barracuda");
        source.gpu = true;
        let targets = targets_for_primal(Some("aarch64-unknown-linux-musl"), &source);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0], "aarch64-unknown-linux-musl");
    }

    #[test]
    fn apply_manifest_package_override() {
        let mut source = test_source_entry("ecoPrimals/biomeOS");
        let cfg = ManifestBuildConfig {
            package: Some("biomeos-unibin".into()),
            linker: None,
            gpu: false,
        };
        apply_manifest_overrides(&mut source, &cfg);
        assert_eq!(source.build_args.as_deref(), Some("-p biomeos-unibin"));
        assert!(!source.gpu);
    }

    #[test]
    fn apply_manifest_gpu_override() {
        let mut source = test_source_entry("ecoPrimals/barraCuda");
        assert!(!source.gpu);
        let cfg = ManifestBuildConfig {
            package: None,
            linker: None,
            gpu: true,
        };
        apply_manifest_overrides(&mut source, &cfg);
        assert!(source.gpu);
    }

    #[test]
    fn apply_manifest_no_override_preserves_source() {
        let mut source = test_source_entry("ecoPrimals/bearDog");
        source.build_args = Some("--features server".into());
        let cfg = ManifestBuildConfig::default();
        apply_manifest_overrides(&mut source, &cfg);
        assert_eq!(source.build_args.as_deref(), Some("--features server"));
    }

    #[test]
    fn source_entry_gpu_defaults_false() {
        let toml_str = r#"
[sources.beardog]
repo = "ecoPrimals/bearDog"
"#;
        let parsed: super::super::depot::SourcesFile = toml::from_str(toml_str).unwrap();
        assert!(!parsed.sources["beardog"].gpu);
    }

    #[test]
    fn source_entry_gpu_parses() {
        let toml_str = r#"
[sources.barracuda]
repo = "ecoPrimals/barracuda"
gpu = true
"#;
        let parsed: super::super::depot::SourcesFile = toml::from_str(toml_str).unwrap();
        assert!(parsed.sources["barracuda"].gpu);
    }
}
