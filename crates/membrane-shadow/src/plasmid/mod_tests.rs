use super::*;

#[test]
fn nucleus_primals_returns_13() {
    let primals = nucleus_primals();
    assert_eq!(primals.len(), 13, "expected 13 nucleus primals");
    assert!(primals.contains(&"beardog"));
    assert!(primals.contains(&"songbird"));
    assert!(primals.contains(&"squirrel"));
}

#[test]
fn detect_target_triple_contains_musl() {
    let triple = detect_target_triple();
    assert!(
        triple.ends_with("-unknown-linux-musl"),
        "expected musl target, got: {triple}"
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
fn parse_staleness_recent() {
    let ts = crate::utc_now_iso8601();
    let days = parse_staleness_days(&ts);
    assert_eq!(days, Some(0), "today's timestamp should be 0 days old");
}

#[test]
fn parse_staleness_old() {
    let days = parse_staleness_days("2020-01-01T00:00:00Z");
    assert!(days.is_some());
    assert!(days.unwrap() > 365, "2020 should be years ago");
}

#[test]
fn parse_staleness_unparseable() {
    assert!(parse_staleness_days("unknown").is_none());
    assert!(parse_staleness_days("").is_none());
    assert!(parse_staleness_days("not-a-date").is_none());
}

#[test]
fn stale_threshold_is_7_days() {
    assert_eq!(DEPOT_STALE_THRESHOLD_DAYS, 7);
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
