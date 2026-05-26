// SPDX-License-Identifier: AGPL-3.0-or-later

use cellmembrane_types::envelope::{
    BondType, BoundaryPolicy, BraidPolicy, ChannelProtein, EnvelopeLayer, EnvelopeTopology,
};

// --- Topology layer structure ---

#[test]
fn monoderm_has_three_layers() {
    let layers = EnvelopeTopology::Monoderm.layers();
    assert_eq!(layers.len(), 3);
    assert_eq!(layers[0], EnvelopeLayer::Cytoplasm);
    assert_eq!(layers[1], EnvelopeLayer::PlasmaMembrane);
    assert_eq!(layers[2], EnvelopeLayer::Extracellular);
}

#[test]
fn diderm_has_five_layers() {
    let layers = EnvelopeTopology::Diderm.layers();
    assert_eq!(layers.len(), 5);
    assert_eq!(layers[0], EnvelopeLayer::Cytoplasm);
    assert_eq!(layers[1], EnvelopeLayer::PlasmaMembrane);
    assert_eq!(layers[2], EnvelopeLayer::Periplasm);
    assert_eq!(layers[3], EnvelopeLayer::OuterMembrane);
    assert_eq!(layers[4], EnvelopeLayer::Extracellular);
}

#[test]
fn monoderm_boundary_counts_derived_from_layers() {
    assert_eq!(EnvelopeTopology::Monoderm.boundary_count(), 1);
    assert_eq!(EnvelopeTopology::Monoderm.periplasm_count(), 0);
    assert!(!EnvelopeTopology::Monoderm.has_periplasm());
}

#[test]
fn diderm_boundary_counts_derived_from_layers() {
    assert_eq!(EnvelopeTopology::Diderm.boundary_count(), 2);
    assert_eq!(EnvelopeTopology::Diderm.periplasm_count(), 1);
    assert!(EnvelopeTopology::Diderm.has_periplasm());
}

#[test]
fn default_topology_is_diderm() {
    assert_eq!(EnvelopeTopology::default(), EnvelopeTopology::Diderm);
}

#[test]
fn layers_ordered_inside_out() {
    let all = EnvelopeLayer::all();
    for window in all.windows(2) {
        assert!(
            window[0] < window[1],
            "{} should be inside of {}",
            window[0],
            window[1]
        );
    }
}

#[test]
fn boundary_layers_are_boundaries() {
    assert!(EnvelopeLayer::PlasmaMembrane.is_boundary());
    assert!(EnvelopeLayer::OuterMembrane.is_boundary());
    assert!(!EnvelopeLayer::Cytoplasm.is_boundary());
    assert!(!EnvelopeLayer::Periplasm.is_boundary());
    assert!(!EnvelopeLayer::Extracellular.is_boundary());
}

#[test]
fn compartment_layers_are_compartments() {
    assert!(EnvelopeLayer::Cytoplasm.is_compartment());
    assert!(EnvelopeLayer::Periplasm.is_compartment());
    assert!(EnvelopeLayer::Extracellular.is_compartment());
    assert!(!EnvelopeLayer::PlasmaMembrane.is_compartment());
    assert!(!EnvelopeLayer::OuterMembrane.is_compartment());
}

// --- Bond type permissions per layer ---

#[test]
fn cytoplasm_only_permits_covalent() {
    let bonds = EnvelopeLayer::Cytoplasm.permitted_inbound_bonds();
    assert_eq!(bonds, &[BondType::Covalent]);
}

#[test]
fn outer_membrane_permits_weak_and_ionic() {
    let bonds = EnvelopeLayer::OuterMembrane.permitted_inbound_bonds();
    assert!(bonds.contains(&BondType::Weak));
    assert!(bonds.contains(&BondType::Ionic));
    assert!(!bonds.contains(&BondType::Covalent));
}

#[test]
fn periplasm_permits_ionic_and_metallic() {
    let bonds = EnvelopeLayer::Periplasm.permitted_inbound_bonds();
    assert!(bonds.contains(&BondType::Ionic));
    assert!(bonds.contains(&BondType::Metallic));
    assert!(!bonds.contains(&BondType::Covalent));
    assert!(!bonds.contains(&BondType::Weak));
}

// --- Bond type → channel protein mapping ---

#[test]
fn covalent_uses_aquaporin() {
    assert_eq!(BondType::Covalent.channel_protein(), ChannelProtein::Aquaporin);
}

#[test]
fn ionic_uses_gated_ion() {
    assert_eq!(BondType::Ionic.channel_protein(), ChannelProtein::GatedIon);
}

#[test]
fn ceremony_uses_voltage_gated() {
    assert_eq!(BondType::Ceremony.channel_protein(), ChannelProtein::VoltageGated);
}

#[test]
fn weak_uses_passive_diffusion() {
    assert_eq!(BondType::Weak.channel_protein(), ChannelProtein::PassiveDiffusion);
}

#[test]
fn channel_protein_round_trip_to_bonds() {
    for protein in ChannelProtein::all() {
        let bonds = protein.permitted_bonds();
        assert!(!bonds.is_empty(), "{protein} should permit at least one bond type");
        for bond in bonds {
            assert_eq!(
                bond.channel_protein(),
                *protein,
                "{bond} should map back to {protein}"
            );
        }
    }
}

// --- Braid policy ---

#[test]
fn covalent_braid_passes_through() {
    assert_eq!(BraidPolicy::for_bond(BondType::Covalent), BraidPolicy::PassThrough);
    assert_eq!(BraidPolicy::for_bond(BondType::Metallic), BraidPolicy::PassThrough);
}

#[test]
fn ionic_braid_is_verified() {
    assert_eq!(BraidPolicy::for_bond(BondType::Ionic), BraidPolicy::Verify);
    assert_eq!(BraidPolicy::for_bond(BondType::Ceremony), BraidPolicy::Verify);
}

#[test]
fn weak_braid_is_blocked() {
    assert_eq!(BraidPolicy::for_bond(BondType::Weak), BraidPolicy::Block);
}

// --- Boundary policies (derived from layer capabilities) ---

#[test]
fn plasma_membrane_policy_derived_from_layer() {
    let policy = BoundaryPolicy::plasma_membrane();
    assert_eq!(policy.layer, EnvelopeLayer::PlasmaMembrane);
    assert!(policy.permits_bond(BondType::Covalent));
    assert!(policy.permits_bond(BondType::Metallic));
    assert!(!policy.permits_bond(BondType::Ionic));
    assert!(!policy.permits_bond(BondType::Weak));
    assert_eq!(policy.braid_policy, BraidPolicy::PassThrough);
}

#[test]
fn outer_membrane_policy_derived_from_layer() {
    let policy = BoundaryPolicy::outer_membrane();
    assert_eq!(policy.layer, EnvelopeLayer::OuterMembrane);
    assert!(policy.permits_bond(BondType::Weak));
    assert!(policy.permits_bond(BondType::Ionic));
    assert!(!policy.permits_bond(BondType::Covalent));
    assert_eq!(policy.braid_policy, BraidPolicy::Block);
}

#[test]
fn for_layer_matches_named_constructors() {
    assert_eq!(
        BoundaryPolicy::for_layer(EnvelopeLayer::PlasmaMembrane),
        BoundaryPolicy::plasma_membrane(),
    );
    assert_eq!(
        BoundaryPolicy::for_layer(EnvelopeLayer::OuterMembrane),
        BoundaryPolicy::outer_membrane(),
    );
}

#[test]
fn diderm_default_boundaries_has_two_policies() {
    let boundaries = EnvelopeTopology::Diderm.default_boundaries();
    assert_eq!(boundaries.len(), 2);
    assert_eq!(boundaries[0].layer, EnvelopeLayer::PlasmaMembrane);
    assert_eq!(boundaries[1].layer, EnvelopeLayer::OuterMembrane);
}

#[test]
fn monoderm_default_boundaries_has_one_policy() {
    let boundaries = EnvelopeTopology::Monoderm.default_boundaries();
    assert_eq!(boundaries.len(), 1);
    assert_eq!(boundaries[0].layer, EnvelopeLayer::PlasmaMembrane);
}

#[test]
fn outer_membrane_has_passive_and_gated_proteins() {
    let policy = BoundaryPolicy::outer_membrane();
    assert!(policy.has_channel_protein(ChannelProtein::PassiveDiffusion));
    assert!(policy.has_channel_protein(ChannelProtein::GatedIon));
    assert!(!policy.has_channel_protein(ChannelProtein::Aquaporin));
}

// --- Serde round-trip ---

#[test]
fn topology_serde_roundtrip() {
    #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
    struct Wrapper {
        t: EnvelopeTopology,
    }
    for topo in EnvelopeTopology::all() {
        let w = Wrapper { t: *topo };
        let serialized = toml::to_string(&w).unwrap();
        let deserialized: Wrapper = toml::from_str(&serialized).unwrap();
        assert_eq!(w, deserialized);
    }
}

#[test]
fn bond_type_serde_roundtrip() {
    #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
    struct Wrapper {
        b: BondType,
    }
    for bond in BondType::all() {
        let w = Wrapper { b: *bond };
        let serialized = toml::to_string(&w).unwrap();
        let deserialized: Wrapper = toml::from_str(&serialized).unwrap();
        assert_eq!(w, deserialized);
    }
}
