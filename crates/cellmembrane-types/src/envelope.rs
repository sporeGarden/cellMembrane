// SPDX-License-Identifier: AGPL-3.0-or-later

//! K-Derm cell envelope topology model.
//!
//! Defines the monoderm/diderm envelope topologies, absolute envelope layers,
//! bonding types, channel protein specificity, and per-boundary policies.
//! See `specs/K_DERM_TOPOLOGY.md` for the full specification.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Cell envelope topology — how many membrane boundaries exist between
/// the cytoplasm (NUCLEUS) and the extracellular environment (internet).
///
/// Monoderm = single boundary (gate firewall only).
/// Diderm = double boundary (gate firewall + VPS outer membrane, with periplasm).
///
/// ```
/// use cellmembrane_types::EnvelopeTopology;
///
/// let topo = EnvelopeTopology::Diderm;
/// assert_eq!(topo.boundary_count(), 2);
/// assert!(topo.has_periplasm());
/// assert_eq!(topo.layers().len(), 5);
/// ```
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvelopeTopology {
    /// Single boundary — gate directly on network, no VPS.
    Monoderm,
    /// Double boundary — gate + VPS with periplasm between them.
    #[default]
    Diderm,
}

impl EnvelopeTopology {
    /// Returns all topology variants.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[Self::Monoderm, Self::Diderm]
    }

    /// Layers present in this topology, ordered inside-out.
    #[must_use]
    pub const fn layers(&self) -> &'static [EnvelopeLayer] {
        match self {
            Self::Monoderm => &[
                EnvelopeLayer::Cytoplasm,
                EnvelopeLayer::PlasmaMembrane,
                EnvelopeLayer::Extracellular,
            ],
            Self::Diderm => &[
                EnvelopeLayer::Cytoplasm,
                EnvelopeLayer::PlasmaMembrane,
                EnvelopeLayer::Periplasm,
                EnvelopeLayer::OuterMembrane,
                EnvelopeLayer::Extracellular,
            ],
        }
    }

    /// Number of membrane boundaries (selectively permeable layers) in this topology.
    /// Derived from the layer list — not hardcoded per variant.
    #[must_use]
    pub fn boundary_count(&self) -> usize {
        self.layers().iter().filter(|l| l.is_boundary()).count()
    }

    /// Number of periplasmic spaces (compartments between adjacent boundaries).
    /// Derived from the layer list.
    #[must_use]
    pub fn periplasm_count(&self) -> usize {
        self.layers()
            .iter()
            .filter(|l| matches!(l, EnvelopeLayer::Periplasm))
            .count()
    }

    /// Whether a VPS relay/periplasm layer exists.
    /// Discovered from layer capabilities, not hardcoded.
    #[must_use]
    pub fn has_periplasm(&self) -> bool {
        self.layers().contains(&EnvelopeLayer::Periplasm)
    }

    /// Boundary layers present in this topology, ordered inside-out.
    #[must_use]
    pub fn boundaries(&self) -> Vec<EnvelopeLayer> {
        self.layers()
            .iter()
            .copied()
            .filter(EnvelopeLayer::is_boundary)
            .collect()
    }

    /// Default boundary policies for this topology.
    /// Each boundary layer derives its policy from its own capabilities.
    #[must_use]
    pub fn default_boundaries(&self) -> Vec<BoundaryPolicy> {
        self.boundaries()
            .into_iter()
            .map(BoundaryPolicy::for_layer)
            .collect()
    }
}

impl fmt::Display for EnvelopeTopology {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Monoderm => write!(f, "monoderm"),
            Self::Diderm => write!(f, "diderm"),
        }
    }
}

/// Absolute position within the cell envelope, ordered inside-out.
///
/// These names are fixed and never relative. "Inner" and "outer" are avoided
/// to prevent the Franklin's Current problem (see K-Derm spec §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvelopeLayer {
    /// Innermost — NUCLEUS processes, UDS IPC, shared memory.
    Cytoplasm,
    /// Gate firewall boundary — the always-present membrane.
    PlasmaMembrane,
    /// Space between plasma and outer membrane — VPS relay/routing/telemetry.
    /// Only present in diderm topologies.
    Periplasm,
    /// VPS-facing boundary — channels (Signal/Relay/Surface) to the internet.
    /// Only present in diderm topologies.
    OuterMembrane,
    /// Outermost — the public internet.
    Extracellular,
}

impl EnvelopeLayer {
    /// Returns all layers in inside-out order.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::Cytoplasm,
            Self::PlasmaMembrane,
            Self::Periplasm,
            Self::OuterMembrane,
            Self::Extracellular,
        ]
    }

    /// Whether this layer is a membrane boundary (selectively permeable).
    #[must_use]
    pub const fn is_boundary(&self) -> bool {
        matches!(self, Self::PlasmaMembrane | Self::OuterMembrane)
    }

    /// Whether this layer is a compartment (contains processes/routing).
    #[must_use]
    pub const fn is_compartment(&self) -> bool {
        matches!(
            self,
            Self::Cytoplasm | Self::Periplasm | Self::Extracellular
        )
    }

    /// Bond types that may cross into this layer from outside.
    #[must_use]
    pub const fn permitted_inbound_bonds(&self) -> &'static [BondType] {
        match self {
            Self::Cytoplasm => &[BondType::Covalent],
            Self::PlasmaMembrane => &[BondType::Covalent, BondType::Metallic],
            Self::Periplasm => &[BondType::Ionic, BondType::Metallic],
            Self::OuterMembrane => &[BondType::Weak, BondType::Ionic],
            Self::Extracellular => &[BondType::Weak],
        }
    }
}

impl fmt::Display for EnvelopeLayer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cytoplasm => write!(f, "cytoplasm"),
            Self::PlasmaMembrane => write!(f, "plasma_membrane"),
            Self::Periplasm => write!(f, "periplasm"),
            Self::OuterMembrane => write!(f, "outer_membrane"),
            Self::Extracellular => write!(f, "extracellular"),
        }
    }
}

/// Bond type from the organo-metallo-salt model.
///
/// Maps to `primals/biomeOS/specs/NUCLEUS_BONDING_MODEL.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BondType {
    /// Shared family seed, full capability, UDS IPC.
    Covalent,
    /// Delocalized fleet compute, specialized but coordinated.
    Metallic,
    /// Contract-based, BTSP scoped tokens, capability masks.
    Ionic,
    /// Time-bound decay: covalent → ionic → weak.
    Ceremony,
    /// Read-only, no active transport, passive API.
    Weak,
}

impl BondType {
    /// Returns all bond types ordered by trust level (highest first).
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::Covalent,
            Self::Metallic,
            Self::Ionic,
            Self::Ceremony,
            Self::Weak,
        ]
    }

    /// Channel protein that mediates this bond type at a membrane boundary.
    #[must_use]
    pub const fn channel_protein(&self) -> ChannelProtein {
        match self {
            Self::Covalent | Self::Metallic => ChannelProtein::Aquaporin,
            Self::Ionic => ChannelProtein::GatedIon,
            Self::Ceremony => ChannelProtein::VoltageGated,
            Self::Weak => ChannelProtein::PassiveDiffusion,
        }
    }
}

impl fmt::Display for BondType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Covalent => write!(f, "covalent"),
            Self::Metallic => write!(f, "metallic"),
            Self::Ionic => write!(f, "ionic"),
            Self::Ceremony => write!(f, "ceremony"),
            Self::Weak => write!(f, "weak"),
        }
    }
}

/// Channel protein type — determines how traffic crosses a membrane boundary.
///
/// Maps to `SOVEREIGN_TRANSACTION_MEMBRANE.md` channel protein taxonomy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelProtein {
    /// Always open — covalent/metallic traffic, shared family seed.
    Aquaporin,
    /// Gated by BTSP scoped token — ionic traffic, method-level filtering.
    GatedIon,
    /// Time-bound gate — ceremony traffic, covalent → ionic → weak decay.
    VoltageGated,
    /// No active transport — weak/read-only traffic.
    PassiveDiffusion,
}

impl ChannelProtein {
    /// Returns all channel protein variants.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::Aquaporin,
            Self::GatedIon,
            Self::VoltageGated,
            Self::PassiveDiffusion,
        ]
    }

    /// Bond types this channel protein permits.
    #[must_use]
    pub const fn permitted_bonds(&self) -> &'static [BondType] {
        match self {
            Self::Aquaporin => &[BondType::Covalent, BondType::Metallic],
            Self::GatedIon => &[BondType::Ionic],
            Self::VoltageGated => &[BondType::Ceremony],
            Self::PassiveDiffusion => &[BondType::Weak],
        }
    }
}

impl fmt::Display for ChannelProtein {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Aquaporin => write!(f, "aquaporin"),
            Self::GatedIon => write!(f, "gated_ion"),
            Self::VoltageGated => write!(f, "voltage_gated"),
            Self::PassiveDiffusion => write!(f, "passive_diffusion"),
        }
    }
}

/// Cytoplasm zone — physical topology grouping within the plasma membrane.
///
/// Maps to the K-Derm model where the cytoplasm is segmented into zones by
/// physical switching fabric. Gates in the same zone share L2 connectivity;
/// cross-zone traffic traverses backbone links or `WireGuard` overlay.
///
/// Zone assignments are authoritative in the ecosystem manifest
/// (`ecosystem_manifest.toml` `[gates.<name>] zone = "..."`) but can be
/// derived from gate name as a fallback.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CytoplasmZone {
    /// Hub 1: CRS310 backbone, sporeGate plasma membrane, 10G fabric.
    Backbone,
    /// Hub 2: Omada `SX3008F` (standalone L2), Flint 2 `WiFi`, house 2 gates.
    House2,
    /// Hub 3: Garage, planned compute + outdoor `WiFi`.
    Garage,
    /// WAN: gates outside the plasma membrane (VPS, offsite, public internet).
    Wan,
    /// Unknown or unassigned zone.
    #[default]
    Unassigned,
}

impl CytoplasmZone {
    /// Parse a zone string from the manifest. Falls back to `Unassigned`.
    #[must_use]
    pub fn from_manifest(s: &str) -> Self {
        match s {
            "backbone" => Self::Backbone,
            "house2" => Self::House2,
            "garage" => Self::Garage,
            "wan" => Self::Wan,
            _ => Self::Unassigned,
        }
    }

    /// Derive zone from gate name using known topology assignments.
    #[must_use]
    pub fn for_gate(gate_name: &str) -> Self {
        match gate_name {
            "eastGate" | "sporeGate" | "northGate" | "ironGate" => Self::Backbone,
            "strandGate" | "southGate" | "swiftGate" | "fieldGate" => Self::House2,
            "golgi" | "pepti" | "flockGate" => Self::Wan,
            _ => Self::Unassigned,
        }
    }

    /// Short label suitable for display and serialization.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Backbone => "backbone",
            Self::House2 => "house2",
            Self::Garage => "garage",
            Self::Wan => "wan",
            Self::Unassigned => "unassigned",
        }
    }

    /// Whether this zone has L2 (direct switched) connectivity to the backbone.
    #[must_use]
    pub const fn has_l2_backbone(self) -> bool {
        matches!(self, Self::Backbone)
    }

    /// Whether gates in this zone require `WireGuard` overlay for inter-zone traffic.
    #[must_use]
    pub const fn requires_overlay(self) -> bool {
        matches!(self, Self::Wan | Self::Garage)
    }
}

impl fmt::Display for CytoplasmZone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// `WireGuard` mesh address assignments (10.13.37.0/24 overlay).
///
/// Static registry of assigned mesh IPs. Once assigned, an address is permanent.
/// Gates without an entry have not yet been peered.
#[must_use]
pub fn mesh_address(gate_name: &str) -> Option<&'static str> {
    match gate_name {
        "golgi" => Some("10.13.37.1"),
        "sporeGate" => Some("10.13.37.2"),
        "pepti" => Some("10.13.37.4"),
        "eastGate" => Some("10.13.37.5"),
        "flockGate" => Some("10.13.37.6"),
        _ => None,
    }
}

/// How braid (`sweetGrass` provenance attribution) is handled when crossing
/// a membrane boundary — the vesicle transport policy.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BraidPolicy {
    /// Braid passes through without inspection (covalent/internal).
    PassThrough,
    /// Braid metadata is verified at the boundary (ionic/periplasm).
    #[default]
    Verify,
    /// Braid is stripped — only results cross, not provenance (weak/external).
    Block,
}

impl BraidPolicy {
    /// Default braid policy for a given bond type.
    #[must_use]
    pub const fn for_bond(bond: BondType) -> Self {
        match bond {
            BondType::Covalent | BondType::Metallic => Self::PassThrough,
            BondType::Ionic | BondType::Ceremony => Self::Verify,
            BondType::Weak => Self::Block,
        }
    }
}

impl fmt::Display for BraidPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PassThrough => write!(f, "pass_through"),
            Self::Verify => write!(f, "verify"),
            Self::Block => write!(f, "block"),
        }
    }
}

/// Policy for a single membrane boundary — which bond types, channel proteins,
/// and braid handling are permitted at this layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundaryPolicy {
    /// Which envelope layer this policy governs.
    pub layer: EnvelopeLayer,
    /// Bond types permitted to cross this boundary.
    pub permitted_bonds: Vec<BondType>,
    /// Channel proteins active at this boundary.
    pub channel_proteins: Vec<ChannelProtein>,
    /// How braid is handled at this boundary.
    pub braid_policy: BraidPolicy,
}

impl BoundaryPolicy {
    /// Derive the default policy for any boundary layer from its own properties.
    /// The layer itself declares what bonds it permits; the policy is assembled
    /// from those capabilities rather than hardcoded per named membrane.
    #[must_use]
    pub fn for_layer(layer: EnvelopeLayer) -> Self {
        let permitted_bonds = layer.permitted_inbound_bonds().to_vec();

        let mut proteins: Vec<ChannelProtein> = permitted_bonds
            .iter()
            .map(BondType::channel_protein)
            .collect();
        proteins.dedup();

        let strongest_bond = permitted_bonds.first().copied().unwrap_or(BondType::Weak);

        Self {
            layer,
            permitted_bonds,
            channel_proteins: proteins,
            braid_policy: BraidPolicy::for_bond(strongest_bond),
        }
    }

    /// Named constructor preserved for readability — delegates to `for_layer`.
    #[must_use]
    pub fn plasma_membrane() -> Self {
        Self::for_layer(EnvelopeLayer::PlasmaMembrane)
    }

    /// Named constructor preserved for readability — delegates to `for_layer`.
    #[must_use]
    pub fn outer_membrane() -> Self {
        Self::for_layer(EnvelopeLayer::OuterMembrane)
    }

    /// Whether a given bond type is permitted at this boundary.
    #[must_use]
    pub fn permits_bond(&self, bond: BondType) -> bool {
        self.permitted_bonds.contains(&bond)
    }

    /// Whether a given channel protein is active at this boundary.
    #[must_use]
    pub fn has_channel_protein(&self, protein: ChannelProtein) -> bool {
        self.channel_proteins.contains(&protein)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cytoplasm_zone_for_gate_backbone() {
        assert_eq!(CytoplasmZone::for_gate("eastGate"), CytoplasmZone::Backbone);
        assert_eq!(CytoplasmZone::for_gate("sporeGate"), CytoplasmZone::Backbone);
        assert_eq!(CytoplasmZone::for_gate("northGate"), CytoplasmZone::Backbone);
        assert_eq!(CytoplasmZone::for_gate("ironGate"), CytoplasmZone::Backbone);
    }

    #[test]
    fn cytoplasm_zone_for_gate_house2() {
        assert_eq!(CytoplasmZone::for_gate("strandGate"), CytoplasmZone::House2);
        assert_eq!(CytoplasmZone::for_gate("southGate"), CytoplasmZone::House2);
        assert_eq!(CytoplasmZone::for_gate("swiftGate"), CytoplasmZone::House2);
        assert_eq!(CytoplasmZone::for_gate("fieldGate"), CytoplasmZone::House2);
    }

    #[test]
    fn cytoplasm_zone_for_gate_wan() {
        assert_eq!(CytoplasmZone::for_gate("golgi"), CytoplasmZone::Wan);
        assert_eq!(CytoplasmZone::for_gate("pepti"), CytoplasmZone::Wan);
        assert_eq!(CytoplasmZone::for_gate("flockGate"), CytoplasmZone::Wan);
    }

    #[test]
    fn cytoplasm_zone_unknown_gate() {
        assert_eq!(CytoplasmZone::for_gate("newGate"), CytoplasmZone::Unassigned);
    }

    #[test]
    fn cytoplasm_zone_from_manifest_string() {
        assert_eq!(CytoplasmZone::from_manifest("backbone"), CytoplasmZone::Backbone);
        assert_eq!(CytoplasmZone::from_manifest("house2"), CytoplasmZone::House2);
        assert_eq!(CytoplasmZone::from_manifest("garage"), CytoplasmZone::Garage);
        assert_eq!(CytoplasmZone::from_manifest("wan"), CytoplasmZone::Wan);
        assert_eq!(CytoplasmZone::from_manifest("bogus"), CytoplasmZone::Unassigned);
    }

    #[test]
    fn cytoplasm_zone_display() {
        assert_eq!(CytoplasmZone::Backbone.to_string(), "backbone");
        assert_eq!(CytoplasmZone::Wan.to_string(), "wan");
        assert_eq!(CytoplasmZone::Unassigned.to_string(), "unassigned");
    }

    #[test]
    fn cytoplasm_zone_l2_and_overlay() {
        assert!(CytoplasmZone::Backbone.has_l2_backbone());
        assert!(!CytoplasmZone::Wan.has_l2_backbone());
        assert!(CytoplasmZone::Wan.requires_overlay());
        assert!(CytoplasmZone::Garage.requires_overlay());
        assert!(!CytoplasmZone::Backbone.requires_overlay());
        assert!(!CytoplasmZone::House2.requires_overlay());
    }

    #[test]
    fn cytoplasm_zone_serde_roundtrip() {
        let zone = CytoplasmZone::Backbone;
        let json = serde_json::to_string(&zone).unwrap();
        assert_eq!(json, "\"backbone\"");
        let parsed: CytoplasmZone = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, zone);
    }

    #[test]
    fn cytoplasm_zone_default_is_unassigned() {
        assert_eq!(CytoplasmZone::default(), CytoplasmZone::Unassigned);
    }

    #[test]
    fn mesh_address_known_gates() {
        assert_eq!(mesh_address("golgi"), Some("10.13.37.1"));
        assert_eq!(mesh_address("sporeGate"), Some("10.13.37.2"));
        assert_eq!(mesh_address("pepti"), Some("10.13.37.4"));
        assert_eq!(mesh_address("eastGate"), Some("10.13.37.5"));
        assert_eq!(mesh_address("flockGate"), Some("10.13.37.6"));
    }

    #[test]
    fn mesh_address_unpeered_returns_none() {
        assert_eq!(mesh_address("ironGate"), None);
        assert_eq!(mesh_address("northGate"), None);
        assert_eq!(mesh_address("newGate"), None);
    }

    #[test]
    fn mesh_addresses_unique() {
        let known = ["golgi", "sporeGate", "pepti", "eastGate", "flockGate"];
        let addrs: Vec<_> = known.iter().filter_map(|g| mesh_address(g)).collect();
        let mut seen = std::collections::HashSet::new();
        assert!(addrs.iter().all(|a| seen.insert(a)));
    }

    #[test]
    fn mesh_addresses_in_subnet() {
        let known = ["golgi", "sporeGate", "pepti", "eastGate", "flockGate"];
        for gate in &known {
            let ip = mesh_address(gate).unwrap();
            assert!(ip.starts_with("10.13.37."), "{gate} address not in 10.13.37.0/24");
        }
    }

    #[test]
    fn envelope_monoderm_boundary_count() {
        assert_eq!(EnvelopeTopology::Monoderm.boundary_count(), 1);
    }

    #[test]
    fn envelope_diderm_boundary_count() {
        assert_eq!(EnvelopeTopology::Diderm.boundary_count(), 2);
    }
}
