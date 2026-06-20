// SPDX-License-Identifier: AGPL-3.0-or-later

//! Cytoplasm zone model and `WireGuard` mesh address resolution.
//!
//! Physical topology groupings within the plasma membrane. Distinct from the
//! K-Derm logical envelope model in [`crate::envelope`] — this module types
//! the L2 switching domains and overlay addressing.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Cytoplasm zone label — classifies a gate's physical position within
/// the plasma membrane switching fabric.
///
/// Distinct from [`crate::topology::PhysicalZone`] which is the full
/// zone data record. This enum is the label/classification used for routing
/// and overlay decisions.
///
/// Zone assignments are authoritative in the ecosystem manifest
/// (`ecosystem_manifest.toml` `[gates.<name>] zone = "..."`) but can be
/// derived from gate name as a fallback.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZoneLabel {
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

impl ZoneLabel {
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

    /// Resolve zone for a gate from a loaded [`TopologyMap`](crate::topology::TopologyMap),
    /// falling back to built-in defaults if the gate is not found in the runtime topology.
    ///
    /// Prefer this over [`Self::for_gate`] when topology data is available.
    #[must_use]
    pub fn from_topology(gate_name: &str, topology: &crate::topology::TopologyMap) -> Self {
        for (zone_id, zone) in &topology.zones {
            if zone.gates.iter().any(|g| g == gate_name) {
                return Self::from_manifest(zone_id);
            }
        }
        Self::for_gate(gate_name)
    }

    /// Derive zone from gate name using built-in fallback assignments.
    ///
    /// This is a bootstrap fallback for when `TOPOLOGY_MAP.toml` is not loaded.
    /// Once topology is available, prefer [`Self::from_topology`].
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

impl fmt::Display for ZoneLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Resolve `WireGuard` mesh address from a loaded [`TopologyMap`](crate::topology::TopologyMap),
/// falling back to the built-in registry if the topology lacks mesh address data.
///
/// Prefer this when topology data is available.
#[must_use]
pub fn mesh_address_from_topology(
    gate_name: &str,
    topology: &crate::topology::TopologyMap,
) -> Option<String> {
    for segment in topology.segments.values() {
        if segment.transport.contains("wireguard") || segment.transport.contains("overlay") {
            if let Some(pos) = segment.gates.iter().position(|g| g == gate_name) {
                if let Some(subnet) = &segment.subnet {
                    if let Some(base) = subnet.split('/').next() {
                        if let Some((prefix, last_octet)) = base.rsplit_once('.') {
                            if let Ok(start) = last_octet.parse::<u32>() {
                                if let Ok(offset) = u32::try_from(pos) {
                                    return Some(format!(
                                        "{prefix}.{}",
                                        start.saturating_add(offset)
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    mesh_address(gate_name).map(String::from)
}

/// Bootstrap gate list for mesh address resolution before topology is loaded.
///
/// Format: `(gate_name, mesh_ip)`. These are permanent WG assignments.
/// For runtime gate discovery, load topology data from `TOPOLOGY_MAP.toml`.
pub const BOOTSTRAP_GATES: &[(&str, &str)] = &[
    ("golgi", "10.13.37.1"),
    ("sporeGate", "10.13.37.2"),
    ("eastGate", "10.13.37.5"),
    ("flockGate", "10.13.37.6"),
];

/// `WireGuard` mesh address assignments (10.13.37.0/24 overlay).
///
/// Built-in fallback registry. Once assigned, an address is permanent.
/// Once topology data is loaded, prefer [`mesh_address_from_topology`].
#[must_use]
pub fn mesh_address(gate_name: &str) -> Option<&'static str> {
    BOOTSTRAP_GATES
        .iter()
        .find(|(name, _)| *name == gate_name)
        .map(|(_, ip)| *ip)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zone_for_gate_backbone() {
        assert_eq!(ZoneLabel::for_gate("eastGate"), ZoneLabel::Backbone);
        assert_eq!(ZoneLabel::for_gate("sporeGate"), ZoneLabel::Backbone);
        assert_eq!(ZoneLabel::for_gate("northGate"), ZoneLabel::Backbone);
        assert_eq!(ZoneLabel::for_gate("ironGate"), ZoneLabel::Backbone);
    }

    #[test]
    fn zone_for_gate_house2() {
        assert_eq!(ZoneLabel::for_gate("strandGate"), ZoneLabel::House2);
        assert_eq!(ZoneLabel::for_gate("southGate"), ZoneLabel::House2);
        assert_eq!(ZoneLabel::for_gate("swiftGate"), ZoneLabel::House2);
        assert_eq!(ZoneLabel::for_gate("fieldGate"), ZoneLabel::House2);
    }

    #[test]
    fn zone_for_gate_wan() {
        assert_eq!(ZoneLabel::for_gate("golgi"), ZoneLabel::Wan);
        assert_eq!(ZoneLabel::for_gate("flockGate"), ZoneLabel::Wan);
    }

    #[test]
    fn zone_unknown_gate() {
        assert_eq!(ZoneLabel::for_gate("newGate"), ZoneLabel::Unassigned);
    }

    #[test]
    fn zone_from_manifest_string() {
        assert_eq!(ZoneLabel::from_manifest("backbone"), ZoneLabel::Backbone);
        assert_eq!(ZoneLabel::from_manifest("house2"), ZoneLabel::House2);
        assert_eq!(ZoneLabel::from_manifest("garage"), ZoneLabel::Garage);
        assert_eq!(ZoneLabel::from_manifest("wan"), ZoneLabel::Wan);
        assert_eq!(ZoneLabel::from_manifest("bogus"), ZoneLabel::Unassigned);
    }

    #[test]
    fn zone_display() {
        assert_eq!(ZoneLabel::Backbone.to_string(), "backbone");
        assert_eq!(ZoneLabel::Wan.to_string(), "wan");
        assert_eq!(ZoneLabel::Unassigned.to_string(), "unassigned");
    }

    #[test]
    fn zone_l2_and_overlay() {
        assert!(ZoneLabel::Backbone.has_l2_backbone());
        assert!(!ZoneLabel::Wan.has_l2_backbone());
        assert!(ZoneLabel::Wan.requires_overlay());
        assert!(ZoneLabel::Garage.requires_overlay());
        assert!(!ZoneLabel::Backbone.requires_overlay());
        assert!(!ZoneLabel::House2.requires_overlay());
    }

    #[test]
    fn zone_serde_roundtrip() {
        let zone = ZoneLabel::Backbone;
        let json = serde_json::to_string(&zone).unwrap();
        assert_eq!(json, "\"backbone\"");
        let parsed: ZoneLabel = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, zone);
    }

    #[test]
    fn zone_default_is_unassigned() {
        assert_eq!(ZoneLabel::default(), ZoneLabel::Unassigned);
    }

    #[test]
    fn mesh_address_known_gates() {
        assert_eq!(mesh_address("golgi"), Some("10.13.37.1"));
        assert_eq!(mesh_address("sporeGate"), Some("10.13.37.2"));
        assert_eq!(mesh_address("eastGate"), Some("10.13.37.5"));
        assert_eq!(mesh_address("flockGate"), Some("10.13.37.6"));
    }

    #[test]
    fn mesh_address_decommissioned_returns_none() {
        assert_eq!(mesh_address("pepti"), None);
    }

    #[test]
    fn mesh_address_unpeered_returns_none() {
        assert_eq!(mesh_address("ironGate"), None);
        assert_eq!(mesh_address("northGate"), None);
        assert_eq!(mesh_address("newGate"), None);
    }

    #[test]
    fn mesh_addresses_unique() {
        let known = ["golgi", "sporeGate", "eastGate", "flockGate"];
        let addrs: Vec<_> = known.iter().filter_map(|g| mesh_address(g)).collect();
        let mut seen = std::collections::HashSet::new();
        assert!(addrs.iter().all(|a| seen.insert(a)));
    }

    #[test]
    fn mesh_addresses_in_subnet() {
        let known = ["golgi", "sporeGate", "eastGate", "flockGate"];
        for gate in &known {
            let ip = mesh_address(gate).unwrap();
            assert!(
                ip.starts_with("10.13.37."),
                "{gate} address not in 10.13.37.0/24"
            );
        }
    }

    #[test]
    fn from_topology_fallback_when_empty() {
        let topo = crate::topology::TopologyMap::default();
        assert_eq!(
            ZoneLabel::from_topology("eastGate", &topo),
            ZoneLabel::Backbone
        );
    }

    #[test]
    fn from_topology_resolves_from_zone_gates() {
        let mut topo = crate::topology::TopologyMap::default();
        topo.zones.insert(
            "garage".into(),
            crate::topology::PhysicalZone {
                gates: vec!["testGate".into()],
                ..Default::default()
            },
        );
        assert_eq!(
            ZoneLabel::from_topology("testGate", &topo),
            ZoneLabel::Garage
        );
    }
}
