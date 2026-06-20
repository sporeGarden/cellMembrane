// SPDX-License-Identifier: AGPL-3.0-or-later

use cellmembrane_types::identity::{GateMobility, MembraneIdentity};

#[test]
fn identity_gate_id_returns_explicit() {
    let id: MembraneIdentity = toml::from_str(
        r#"
        family_id = "alpha"
        gate_id = "nyc-01"
        "#,
    )
    .unwrap();
    assert_eq!(id.gate_id_or_default(), "nyc-01");
}

#[test]
fn identity_gate_id_generates_default() {
    let id: MembraneIdentity = toml::from_str(
        r#"
        family_id = "membrane-alpha"
        "#,
    )
    .unwrap();
    assert_eq!(id.gate_id_or_default(), "membrane-alpha-membrane");
}

#[test]
fn identity_serde_roundtrip() {
    let id: MembraneIdentity = toml::from_str(
        r#"
        family_id = "eco-01"
        gate_id = "west-gate"
        "#,
    )
    .unwrap();
    let serialized = toml::to_string(&id).unwrap();
    let deserialized: MembraneIdentity = toml::from_str(&serialized).unwrap();
    assert_eq!(deserialized.family_id, "eco-01");
    assert_eq!(deserialized.gate_id.as_deref(), Some("west-gate"));
}

#[test]
fn identity_extra_fields_preserved() {
    let id: MembraneIdentity = toml::from_str(
        r#"
        family_id = "test"
        custom_field = "value"
        "#,
    )
    .unwrap();
    assert!(id.extra.contains_key("custom_field"));
}

#[test]
fn identity_mobility_default_is_fixed() {
    let id: MembraneIdentity = toml::from_str(
        r#"
        family_id = "eco"
        "#,
    )
    .unwrap();
    assert_eq!(id.mobility, GateMobility::Fixed);
}

#[test]
fn identity_mobility_mobile_parse() {
    let id: MembraneIdentity = toml::from_str(
        r#"
        family_id = "eco"
        gate_id = "golgiAlpha"
        mobility = "mobile"
        "#,
    )
    .unwrap();
    assert_eq!(id.mobility, GateMobility::Mobile);
    assert!(id.mobility.needs_reconnect_hook());
    assert!(!id.mobility.is_mesh_anchor());
}

#[test]
fn identity_mobility_fixed_attributes() {
    assert!(!GateMobility::Fixed.needs_reconnect_hook());
    assert!(GateMobility::Fixed.is_mesh_anchor());
    assert_eq!(GateMobility::Fixed.to_string(), "fixed");
    assert_eq!(GateMobility::Mobile.to_string(), "mobile");
}
