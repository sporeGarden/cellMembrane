use super::*;

#[test]
fn plasmidbin_was_pulled_positive() {
    let lines = vec![
        "  [parity] primals/bearDog — pull".into(),
        "  [parity] infra/plasmidBin — pull".into(),
    ];
    assert!(plasmidbin_was_pulled(&lines));
}

#[test]
fn plasmidbin_was_pulled_negative() {
    let lines = vec![
        "  [parity] primals/bearDog — pull".into(),
        "  [parity] infra/wateringHole — pull".into(),
    ];
    assert!(!plasmidbin_was_pulled(&lines));
}

#[test]
fn rootpulse_state_toml_roundtrip() {
    let toml_content = r#"wave = 116
gate = "sporeGate"
session = "rp-116-abc123"
timestamp = "2026-06-19T12:00:00Z"
"#;
    let table: toml::Table = toml_content.parse().unwrap();
    let session = table
        .get("session")
        .and_then(|v| v.as_str())
        .map(String::from);
    assert_eq!(session.as_deref(), Some("rp-116-abc123"));

    let wave = table.get("wave").and_then(toml::Value::as_integer).unwrap();
    assert_eq!(wave, 116);

    let gate = table.get("gate").and_then(toml::Value::as_str).unwrap();
    assert_eq!(gate, "sporeGate");
}

#[test]
fn is_build_authority_defaults_false() {
    if std::env::var(cellmembrane_types::service::ENV_BUILD_AUTHORITY).is_err() {
        assert!(!is_build_authority());
    }
}

#[test]
fn summarize_depot_suffix_formatting() {
    let missing = 2u32;
    let stale = 3u32;
    let suffix = match (missing, stale) {
        (0, 0) => String::new(),
        (0, s) => format!(" ({s} stale — run with --with-rebuild to auto-fix)"),
        (m, 0) => format!(" ({m} missing)"),
        (m, s) => format!(" ({m} missing, {s} stale)"),
    };
    assert_eq!(suffix, " (2 missing, 3 stale)");

    let suffix_none = match (0u32, 0u32) {
        (0, 0) => String::new(),
        (0, s) => format!(" ({s} stale — run with --with-rebuild to auto-fix)"),
        (m, 0) => format!(" ({m} missing)"),
        (m, s) => format!(" ({m} missing, {s} stale)"),
    };
    assert!(suffix_none.is_empty());
}
