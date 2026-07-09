// SPDX-License-Identifier: AGPL-3.0-or-later

use cellmembrane_types::composition::MembraneComposition;
use cellmembrane_types::service::MembraneService;

#[test]
fn composition_ladder_ordering() {
    assert!(MembraneComposition::Relay < MembraneComposition::RustDesk);
    assert!(MembraneComposition::RustDesk < MembraneComposition::Tower);
    assert!(MembraneComposition::Tower < MembraneComposition::Nest);
}

#[test]
fn composition_btsp_requirements() {
    assert!(!MembraneComposition::Relay.has_btsp());
    assert!(!MembraneComposition::RustDesk.has_btsp());
    assert!(MembraneComposition::Tower.has_btsp());
    assert!(MembraneComposition::Nest.has_btsp());
}

#[test]
fn tower_composition_spec() {
    let spec = MembraneComposition::Tower.spec();
    assert_eq!(spec.primals, vec!["beardog", "songbird", "skunkbat"]);
    assert_eq!(spec.symbiotic, vec!["hbbs", "hbbr"]);
    assert!(spec.boot_order[0] == "beardog");
}

#[test]
fn nest_composition_includes_all_tower_primals() {
    let tower = MembraneComposition::Tower.spec();
    let nest = MembraneComposition::Nest.spec();
    for primal in &tower.primals {
        assert!(
            nest.primals.contains(primal),
            "Nest should include Tower primal: {primal}"
        );
    }
}

#[test]
fn nest_composition_adds_storage_primals() {
    let spec = MembraneComposition::Nest.spec();
    assert!(spec.primals.contains(&"nestgate"));
    assert!(spec.primals.contains(&"rhizocrypt"));
    assert!(spec.primals.contains(&"loamspine"));
    assert!(spec.primals.contains(&"sweetgrass"));
    assert!(spec.symbiotic.contains(&"caddy"));
}

#[test]
fn composition_serde_roundtrip() {
    #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
    struct Wrapper {
        c: MembraneComposition,
    }
    for comp in MembraneComposition::all() {
        let w = Wrapper { c: *comp };
        let serialized = toml::to_string(&w).unwrap();
        let deserialized: Wrapper = toml::from_str(&serialized).unwrap();
        assert_eq!(w, deserialized);
    }
}

// === NUCLEUS composition tests ===

#[test]
fn nucleus_includes_all_thirteen_primals() {
    let spec = MembraneComposition::Nucleus.spec();
    assert_eq!(spec.primals.len(), 13, "NUCLEUS should have 13 primals");
    for expected in &[
        "beardog",
        "songbird",
        "skunkbat",
        "nestgate",
        "rhizocrypt",
        "loamspine",
        "sweetgrass",
        "toadstool",
        "barracuda",
        "coralreef",
        "biomeos",
        "squirrel",
        "petaltongue",
    ] {
        assert!(
            spec.primals.contains(expected),
            "NUCLEUS should include {expected}"
        );
    }
}

#[test]
fn nucleus_superset_of_nest() {
    let nest = MembraneComposition::Nest.spec();
    let nucleus = MembraneComposition::Nucleus.spec();
    for primal in &nest.primals {
        assert!(
            nucleus.primals.contains(primal),
            "NUCLEUS should include all Nest primals, missing: {primal}"
        );
    }
    assert!(nucleus.primals.len() > nest.primals.len());
}

#[test]
fn nucleus_has_biomeos() {
    assert!(MembraneComposition::Nucleus.has_biomeos());
    assert!(!MembraneComposition::Nest.has_biomeos());
    assert!(!MembraneComposition::Tower.has_biomeos());
}

#[test]
fn nucleus_has_btsp() {
    assert!(MembraneComposition::Nucleus.has_btsp());
    assert!(MembraneComposition::Nucleus.dark_forest_compliant());
    assert!(MembraneComposition::Nucleus.requires_tower_env());
}

#[test]
fn nucleus_channels_same_as_nest() {
    let nest_ch = MembraneComposition::Nest.active_channels();
    let nucleus_ch = MembraneComposition::Nucleus.active_channels();
    assert_eq!(nest_ch, nucleus_ch);
}

#[test]
fn nucleus_serde_roundtrip() {
    #[derive(serde::Serialize, serde::Deserialize)]
    struct Wrapper {
        comp: MembraneComposition,
    }
    let val = Wrapper {
        comp: MembraneComposition::Nucleus,
    };
    let serialized = toml::to_string(&val).unwrap();
    assert!(serialized.contains("nucleus"));
    let deserialized: Wrapper = toml::from_str(&serialized).unwrap();
    assert_eq!(deserialized.comp, MembraneComposition::Nucleus);
}

#[test]
fn nucleus_display() {
    assert_eq!(MembraneComposition::Nucleus.to_string(), "nucleus");
}

#[test]
fn nucleus_all_includes_five_tiers() {
    let all = MembraneComposition::all();
    assert_eq!(all.len(), 5);
    assert_eq!(all[4], MembraneComposition::Nucleus);
}

#[test]
fn nucleus_ordering() {
    assert!(MembraneComposition::Nucleus > MembraneComposition::Nest);
    assert!(MembraneComposition::Nucleus > MembraneComposition::Tower);
}

#[test]
fn nucleus_compute_primals_are_uds_only() {
    for name in &["toadstool", "barracuda", "coralreef", "biomeos", "squirrel"] {
        let svc = MembraneService::for_binary(name).unwrap_or_else(|| panic!("{name} not found"));
        assert!(svc.is_uds_only(), "{name} should be UDS-only");
        assert!(svc.has_socket, "{name} should have socket");
    }
}

#[test]
fn nucleus_services_no_external_ports() {
    for name in &["toadstool", "barracuda", "coralreef", "biomeos", "squirrel"] {
        let svc = MembraneService::for_binary(name).unwrap();
        assert!(
            !svc.is_externally_reachable(),
            "{name} should not be externally reachable"
        );
    }
}

#[test]
fn petaltongue_loopback_not_external() {
    let svc = MembraneService::for_binary("petaltongue").unwrap();
    assert!(!svc.is_externally_reachable());
    assert_eq!(svc.port, Some(8080));
    assert!(svc.is_uds_only());
}

#[test]
fn nucleus_firewall_same_ports_as_nest() {
    use cellmembrane_types::firewall::FirewallRuleset;
    let nest_rules = FirewallRuleset::for_composition(MembraneComposition::Nest);
    let nucleus_rules = FirewallRuleset::for_composition(MembraneComposition::Nucleus);
    assert_eq!(
        nest_rules.rules.len(),
        nucleus_rules.rules.len(),
        "NUCLEUS adds no new firewall ports (all new services are UDS-only)"
    );
}

#[test]
fn nucleus_uds_socket_paths() {
    let spec = MembraneComposition::Nucleus.spec();
    let paths = spec.uds_socket_paths();
    assert!(paths.len() >= 10, "NUCLEUS should have many UDS sockets");
    let binaries: Vec<&str> = paths.iter().map(|(b, _)| *b).collect();
    assert!(binaries.contains(&"biomeos"));
    assert!(binaries.contains(&"toadstool"));
    assert!(binaries.contains(&"barracuda"));
}

#[test]
fn parse_name_resolves_all_variants() {
    assert_eq!(
        MembraneComposition::parse_name("relay"),
        Some(MembraneComposition::Relay)
    );
    assert_eq!(
        MembraneComposition::parse_name("NUCLEUS"),
        Some(MembraneComposition::Nucleus)
    );
    assert_eq!(
        MembraneComposition::parse_name("pepti"),
        Some(MembraneComposition::Peptidoglycan)
    );
    assert_eq!(
        MembraneComposition::parse_name("rust_desk"),
        Some(MembraneComposition::RustDesk)
    );
    assert!(MembraneComposition::parse_name("unknown").is_none());
}

#[test]
fn parse_name_manifest_aliases() {
    assert_eq!(
        MembraneComposition::parse_name("thin-relay"),
        Some(MembraneComposition::Relay)
    );
    assert_eq!(
        MembraneComposition::parse_name("full"),
        Some(MembraneComposition::Nucleus)
    );
    assert_eq!(
        MembraneComposition::parse_name("compute"),
        Some(MembraneComposition::Tower)
    );
    assert_eq!(
        MembraneComposition::parse_name("cold_storage"),
        Some(MembraneComposition::Nest)
    );
    assert_eq!(
        MembraneComposition::parse_name("THIN-RELAY"),
        Some(MembraneComposition::Relay)
    );
}

#[test]
fn config_nucleus_composition_parses() {
    let toml = r#"
        [membrane]
        name = "test-nucleus"
        composition = "nucleus"
        domain = "nucleus.primals.eco"
        [membrane.channels.signal]
        enabled = true
    "#;
    let file: cellmembrane_types::config::MembraneConfigFile = toml::from_str(toml).unwrap();
    assert_eq!(file.membrane.composition, MembraneComposition::Nucleus);
}
