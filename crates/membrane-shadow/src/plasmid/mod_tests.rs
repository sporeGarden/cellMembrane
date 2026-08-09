use super::*;

#[test]
fn spawn_resolves_biomeos_contract() {
    let svc = cellmembrane_types::MembraneService::for_binary("biomeos").unwrap();
    assert_eq!(
        svc.server_contract,
        cellmembrane_types::service::ServerContract::BiomeosApi,
        "biomeOS must use BiomeosApi, not generic server"
    );
}

#[test]
fn spawn_resolves_generic_server_contract() {
    for bin in ["beardog", "songbird", "nestgate", "squirrel"] {
        let svc = cellmembrane_types::MembraneService::for_binary(bin).unwrap();
        assert!(
            !matches!(
                svc.server_contract,
                cellmembrane_types::service::ServerContract::BiomeosApi
            ),
            "{bin} should not use BiomeosApi contract"
        );
    }
}

#[test]
fn nucleus_primals_returns_13() {
    let primals = nucleus_primals();
    assert_eq!(primals.len(), 13, "expected 13 nucleus primals");
    assert!(primals.contains(&"beardog"));
    assert!(primals.contains(&"songbird"));
    assert!(primals.contains(&"squirrel"));
}

#[test]
fn detect_target_triple_matches_platform() {
    let triple = detect_target_triple();
    let platform = cellmembrane_types::Platform::detect();
    assert_eq!(
        triple,
        platform.triple(),
        "detect_target_triple must agree with Platform::detect"
    );
    assert!(
        !triple.is_empty(),
        "triple should be a non-empty target string"
    );
}

#[test]
fn resolve_path_explicit_overrides_env() {
    let result = resolve_path(Some("/explicit"), "NONEXISTENT_VAR_XYZ", || {
        PathBuf::from("/default")
    });
    assert_eq!(result, PathBuf::from("/explicit"));
}

#[test]
fn resolve_path_uses_default_when_no_env() {
    let result = resolve_path(None, "NONEXISTENT_VAR_XYZ_ABC", || {
        PathBuf::from("/fallback")
    });
    assert_eq!(result, PathBuf::from("/fallback"));
}

#[test]
fn validate_lineage_missing_depot_warns_non_critical() {
    let tmp = std::env::temp_dir().join("lineage_test_warn");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    let result = validate_lineage("squirrel", &tmp);
    assert!(
        matches!(result, LineageResult::Warned(_)),
        "non-critical primal with no depot should warn, got: {result:?}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn validate_lineage_missing_depot_blocks_critical() {
    let tmp = std::env::temp_dir().join("lineage_test_block");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    let result = validate_lineage("beardog", &tmp);
    assert!(
        matches!(result, LineageResult::Blocked(_)),
        "postPrimordial primal with no depot should be blocked, got: {result:?}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn lineage_result_variants_exhaustive() {
    let v = LineageResult::Verified;
    assert!(matches!(v, LineageResult::Verified));
    let b = LineageResult::Blocked("test".into());
    assert!(matches!(b, LineageResult::Blocked(_)));
    let w = LineageResult::Warned("test".into());
    assert!(matches!(w, LineageResult::Warned(_)));
}

#[test]
fn validate_lineage_per_entry_builder_fallback() {
    use super::harvest::{ProvenanceEntry, ProvenanceFile};
    use std::collections::BTreeMap;

    let tmp = std::env::temp_dir().join("lineage_per_entry_builder");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    let mut entries = BTreeMap::new();
    entries.insert(
        "squirrel".to_string(),
        ProvenanceEntry {
            version: None,
            commit: Some("abc123".into()),
            source: None,
            blake3: None,
            built_at: Some("2026-08-09T18:00:00Z".into()),
            target: Some("x86_64-unknown-linux-musl".into()),
            builder: Some("blueGate".into()),
        },
    );
    let prov = ProvenanceFile {
        generated: Some("2026-08-09".into()),
        builder: None,
        target: None,
        rustc: None,
        entries,
    };
    let prov_toml = toml::to_string_pretty(&prov).unwrap();
    std::fs::write(
        tmp.join(cellmembrane_types::service::PROVENANCE_FILE),
        &prov_toml,
    )
    .unwrap();

    let result = validate_lineage("squirrel", &tmp);
    match &result {
        LineageResult::Warned(detail) => {
            assert!(
                !detail.contains("no builder identity"),
                "per-entry builder should satisfy the builder check, got: {detail}"
            );
        }
        other => {
            assert!(
                !matches!(other, LineageResult::Blocked(_)),
                "squirrel is not postPrimordial: {other:?}"
            );
        }
    }

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn strip_sandbox_suffix_removes_commit_hash() {
    assert_eq!(strip_sandbox_suffix("biomeos-abc12345"), "biomeos");
    assert_eq!(strip_sandbox_suffix("beardog-deadbeef"), "beardog");
    assert_eq!(strip_sandbox_suffix("squirrel-0a1b2c3d"), "squirrel");
}

#[test]
fn strip_sandbox_suffix_preserves_bare_names() {
    assert_eq!(strip_sandbox_suffix("biomeos"), "biomeos");
    assert_eq!(strip_sandbox_suffix("beardog"), "beardog");
    assert_eq!(strip_sandbox_suffix("membrane"), "membrane");
}

#[test]
fn strip_sandbox_suffix_preserves_short_or_non_hex() {
    assert_eq!(strip_sandbox_suffix("coral-reef"), "coral-reef");
    assert_eq!(strip_sandbox_suffix("sweet-grass"), "sweet-grass");
    assert_eq!(strip_sandbox_suffix("beardog-abc"), "beardog-abc");
}

#[test]
fn strip_sandbox_suffix_resolves_biomeos_contract() {
    let stripped = strip_sandbox_suffix("biomeos-abc12345");
    let svc = cellmembrane_types::MembraneService::for_binary(stripped).unwrap();
    assert_eq!(
        svc.server_contract,
        cellmembrane_types::service::ServerContract::BiomeosApi,
        "commit-suffixed biomeos must still resolve to BiomeosApi"
    );
}

#[tokio::test]
async fn status_reports_depot_state() {
    let result = status().await;
    match result {
        Ok(outcome) => {
            assert!(outcome.message.contains("depot:"));
            assert!(outcome.message.contains("current"));
            let data = outcome.data.unwrap();
            assert!(data.get("total").is_some());
            assert!(data.get("drifted").is_some());
            assert!(data.get("stale_days").is_some());
            assert!(data.get("stale").is_some());
        }
        Err(e) => {
            let msg = e.to_string();
            assert!(
                msg.contains("depot not found")
                    || msg.contains("cannot read")
                    || msg.contains("No such file"),
                "unexpected error: {msg}"
            );
        }
    }
}
