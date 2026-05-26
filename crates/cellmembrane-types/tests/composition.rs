// SPDX-License-Identifier: AGPL-3.0-or-later

use cellmembrane_types::composition::MembraneComposition;

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
