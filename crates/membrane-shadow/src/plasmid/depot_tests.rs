use super::*;

#[test]
fn staleness_report_display_all_current() {
    let report = StalenessReport {
        entries: vec![StalenessEntry {
            name: "beardog".into(),
            binary_exists: true,
            provenance_commit: Some("abc123".into()),
            source_commit: Some("abc123".into()),
            stale: false,
            reason: None,
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
                source_commit: Some("abc".into()),
                stale: false,
                reason: None,
            },
            StalenessEntry {
                name: "songbird".into(),
                binary_exists: false,
                provenance_commit: None,
                source_commit: None,
                stale: true,
                reason: Some("binary missing".into()),
            },
        ],
        total: 2,
        stale_count: 1,
        current_count: 1,
    };
    let s = report.to_string();
    assert!(s.contains("1/2 current"));
    assert!(s.contains("1 stale"));
    assert!(s.contains("songbird (binary missing)"));
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
    // beardog: binary exists + provenance present. May be stale if workspace
    // is available and source HEAD != "aaa" (drift detection).
    // songbird: no binary, no provenance → always stale.
    let songbird = report
        .entries
        .iter()
        .find(|e| e.name == "songbird")
        .unwrap();
    assert!(songbird.stale, "songbird should be stale (no binary)");
    let beardog = report.entries.iter().find(|e| e.name == "beardog").unwrap();
    assert!(beardog.binary_exists, "beardog binary should exist");
    assert!(
        beardog.provenance_commit.is_some(),
        "beardog should have provenance"
    );
    assert!(report.stale_count >= 1, "at least songbird should be stale");

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
    match result {
        Err(ShadowError::Config(msg)) => {
            assert!(
                msg.contains("auto-provision"),
                "error should mention auto-provision: {msg}"
            );
        }
        Ok(sources) => {
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
    use crate::plasmid::harvest::{HarvestResult, HarvestStatus};

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

#[test]
fn prune_removes_unknown_binaries() {
    let tmp = std::env::temp_dir().join(format!("prune_test_{}", std::process::id()));
    let arch_dir = tmp.join("primals").join("x86_64-unknown-linux-musl");
    std::fs::create_dir_all(&arch_dir).unwrap();

    std::fs::write(arch_dir.join("beardog"), b"known").unwrap();
    std::fs::write(arch_dir.join("songbird"), b"known").unwrap();
    std::fs::write(arch_dir.join("test-demo-binary"), b"junk").unwrap();
    std::fs::write(arch_dir.join("bench-tool"), b"junk").unwrap();

    let report = prune_depot(&tmp, &[], false).unwrap();

    assert_eq!(report.scanned, 4);
    assert_eq!(report.retained, 2);
    assert_eq!(report.pruned.len(), 2);
    assert!(!arch_dir.join("test-demo-binary").exists());
    assert!(!arch_dir.join("bench-tool").exists());
    assert!(arch_dir.join("beardog").exists());
    assert!(arch_dir.join("songbird").exists());

    let display = report.to_string();
    assert!(display.contains("pruned=2"));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn prune_dry_run_does_not_delete() {
    let tmp = std::env::temp_dir().join(format!("prune_dry_{}", std::process::id()));
    let arch_dir = tmp.join("primals").join("x86_64-unknown-linux-musl");
    std::fs::create_dir_all(&arch_dir).unwrap();

    std::fs::write(arch_dir.join("beardog"), b"known").unwrap();
    std::fs::write(arch_dir.join("unknown-bin"), b"junk").unwrap();

    let report = prune_depot(&tmp, &[], true).unwrap();

    assert_eq!(report.pruned.len(), 1);
    assert!(
        arch_dir.join("unknown-bin").exists(),
        "dry-run must not delete"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn prune_respects_allow_list() {
    let tmp = std::env::temp_dir().join(format!("prune_allow_{}", std::process::id()));
    let arch_dir = tmp.join("primals").join("x86_64-unknown-linux-musl");
    std::fs::create_dir_all(&arch_dir).unwrap();

    std::fs::write(arch_dir.join("swarmvine"), b"extra").unwrap();
    std::fs::write(arch_dir.join("beardog"), b"known").unwrap();

    let report = prune_depot(&tmp, &["swarmvine"], false).unwrap();

    assert_eq!(report.pruned.len(), 0);
    assert_eq!(report.retained, 2);
    assert!(arch_dir.join("swarmvine").exists());

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn prune_skips_blake3sums_file() {
    let tmp = std::env::temp_dir().join(format!("prune_b3_{}", std::process::id()));
    let arch_dir = tmp.join("primals").join("x86_64-unknown-linux-musl");
    std::fs::create_dir_all(&arch_dir).unwrap();

    std::fs::write(
        arch_dir.join(cellmembrane_types::service::BLAKE3SUMS_FILE),
        b"hash file",
    )
    .unwrap();
    std::fs::write(arch_dir.join("unknown-bin"), b"junk").unwrap();

    let report = prune_depot(&tmp, &[], true).unwrap();

    assert_eq!(
        report.scanned, 1,
        "BLAKE3SUMS must not be counted or pruned"
    );
    assert_eq!(report.pruned.len(), 1);

    let _ = std::fs::remove_dir_all(&tmp);
}
