use super::*;
use crate::manifest::ManifestBuildConfig;

use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct SourcesFile {
    sources: BTreeMap<String, SourceEntry>,
}

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
        local: false,
        push: false,
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
        local: false,
        push: false,
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
        local: false,
        push: false,
    };
    let result = determine_primals(&args, &sources).unwrap();
    assert!(result.contains(&"beardog".to_string()));
}

#[test]
fn targets_for_regular_primal() {
    let source = test_source_entry("ecoPrimals/bearDog");
    let targets = targets_for_primal(None, &source, &[]);
    assert_eq!(targets.len(), 1);
    assert!(targets[0].contains("musl"));
}

#[test]
fn targets_for_gpu_primal() {
    let mut source = test_source_entry("ecoPrimals/barracuda");
    source.gpu = true;
    let targets = targets_for_primal(None, &source, &[]);
    if cfg!(target_arch = "x86_64") {
        assert_eq!(targets.len(), 2);
        assert!(targets[0].contains("musl"));
        assert!(targets[1].contains("gnu"));
    }
}

#[test]
fn targets_from_manifest_overrides_host() {
    let source = test_source_entry("ecoPrimals/bearDog");
    let manifest_targets = vec![
        "x86_64-unknown-linux-musl".to_string(),
        "aarch64-unknown-linux-musl".to_string(),
    ];
    let targets = targets_for_primal(None, &source, &manifest_targets);
    assert_eq!(targets.len(), 2);
    assert!(targets.contains(&"x86_64-unknown-linux-musl".to_string()));
    assert!(targets.contains(&"aarch64-unknown-linux-musl".to_string()));
}

#[test]
fn targets_cli_overrides_manifest() {
    let source = test_source_entry("ecoPrimals/bearDog");
    let manifest_targets = vec![
        "x86_64-unknown-linux-musl".to_string(),
        "aarch64-unknown-linux-musl".to_string(),
    ];
    let targets = targets_for_primal(
        Some("aarch64-unknown-linux-musl"),
        &source,
        &manifest_targets,
    );
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0], "aarch64-unknown-linux-musl");
}

#[test]
fn targets_cli_override_ignores_gpu() {
    let mut source = test_source_entry("ecoPrimals/barracuda");
    source.gpu = true;
    let targets = targets_for_primal(Some("aarch64-unknown-linux-musl"), &source, &[]);
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
        targets: Vec::new(),
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
        targets: Vec::new(),
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

#[test]
fn resolve_local_source_dir_unknown_primal() {
    let result = resolve_local_source_dir("nonexistent_primal_xyz");
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("--local"),
        "error should mention --local flag: {err}"
    );
}

#[test]
fn harvest_args_local_flag() {
    let args = HarvestArgs {
        primal: None,
        force: true,
        dry_run: false,
        depot_dir: None,
        target: None,
        local: true,
        push: false,
    };
    assert!(args.local);
    assert!(args.force);
}

#[test]
fn checksum_entry_deserialize_struct() {
    let toml_str = r#"blake3 = "abc123"
size = 42"#;
    let entry: ChecksumEntry = toml::from_str(toml_str).unwrap();
    assert_eq!(entry.blake3, "abc123");
    assert_eq!(entry.size, 42);
}

#[test]
fn checksum_entry_deserialize_plain_string() {
    let val = toml::Value::String("abc123".into());
    let entry: ChecksumEntry = val.try_into().unwrap();
    assert_eq!(entry.blake3, "abc123");
    assert_eq!(entry.size, 0);
}

#[test]
fn checksum_entry_deserialize_struct_without_size() {
    let toml_str = r#"blake3 = "abc123""#;
    let entry: ChecksumEntry = toml::from_str(toml_str).unwrap();
    assert_eq!(entry.blake3, "abc123");
    assert_eq!(entry.size, 0);
}

#[test]
fn checksum_file_mixed_format() {
    use std::collections::BTreeMap;
    #[derive(serde::Deserialize)]
    struct ChecksumFile {
        #[serde(flatten)]
        targets: BTreeMap<String, BTreeMap<String, ChecksumEntry>>,
    }
    let content = r#"
[x86_64-unknown-linux-musl]
beardog = { blake3 = "abc123", size = 1000 }
songbird = "def456"
"#;
    let parsed: ChecksumFile = toml::from_str(content).unwrap();
    let arch = &parsed.targets["x86_64-unknown-linux-musl"];
    assert_eq!(arch["beardog"].blake3, "abc123");
    assert_eq!(arch["beardog"].size, 1000);
    assert_eq!(arch["songbird"].blake3, "def456");
    assert_eq!(arch["songbird"].size, 0);
}
