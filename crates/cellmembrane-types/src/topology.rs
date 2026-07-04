// SPDX-License-Identifier: AGPL-3.0-or-later

//! Physical network topology model — cytoplasm zones, segments, and backbone links.
//!
//! Distinct from [`crate::envelope`] (K-Derm logical model). This module types
//! the physical L2/L3 switching domains parsed from `TOPOLOGY_MAP.toml`.
//!
//! ```
//! use cellmembrane_types::topology::{PhysicalZone, ZoneStatus};
//!
//! let zone = PhysicalZone {
//!     hub_device: "MikroTik CRS310-1G-5S-4S+IN".into(),
//!     hub_role: "backbone".into(),
//!     max_speed_mbps: 10_000,
//!     site: "house1".into(),
//!     gates: vec!["sporeGate".into(), "eastGate".into()],
//!     status: ZoneStatus::Live,
//!     ..Default::default()
//! };
//! assert_eq!(zone.max_speed_mbps, 10_000);
//! ```

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// Operational status of a zone or backbone link.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZoneStatus {
    /// Zone is operational and passing traffic.
    #[default]
    Live,
    /// Zone exists but is degraded or running a temporary workaround.
    Workaround,
    /// Zone is ordered/planned but not yet deployed.
    Planned,
    /// Zone hardware ordered, awaiting physical install.
    Ordered,
    /// Zone is being retired.
    Retiring,
}

impl fmt::Display for ZoneStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Live => write!(f, "live"),
            Self::Workaround => write!(f, "workaround"),
            Self::Planned => write!(f, "planned"),
            Self::Ordered => write!(f, "ordered"),
            Self::Retiring => write!(f, "retiring"),
        }
    }
}

/// A cytoplasm zone — an L2 switching domain within the plasma membrane.
///
/// Each zone has a hub device (switch), a physical site, a set of gates,
/// and bandwidth/uplink characteristics. Maps to `[cytoplasm.zones.*]`
/// in `TOPOLOGY_MAP.toml`.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct PhysicalZone {
    /// Hub switch or AP (e.g., "`MikroTik` CRS310-1G-5S-4S+IN").
    #[serde(default)]
    pub hub_device: String,
    /// Role: "backbone", "extension", "`wifi_bridge`", "`compute_and_wifi`".
    #[serde(default)]
    pub hub_role: String,
    /// Maximum link speed available from this hub (Mbps).
    #[serde(default)]
    pub max_speed_mbps: u32,
    /// Physical site: "house1", "house2", "garage".
    #[serde(default)]
    pub site: String,
    /// Gates physically connected to this zone's hub.
    #[serde(default)]
    pub gates: Vec<String>,
    /// Zone this hub uplinks to (e.g., "backbone" for house2).
    #[serde(default)]
    pub uplink_zone: Option<String>,
    /// Uplink speed to parent zone (Mbps).
    #[serde(default)]
    pub uplink_speed_mbps: Option<u32>,
    /// Operational status.
    #[serde(default)]
    pub status: ZoneStatus,
    /// Operational notes.
    #[serde(default)]
    pub note: Option<String>,
}

/// A backbone link between two sites in the triangle topology.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct BackboneLink {
    /// Origin site.
    #[serde(default)]
    pub from: String,
    /// Destination site.
    #[serde(default)]
    pub to: String,
    /// Cable/fiber medium.
    #[serde(default)]
    pub medium: String,
    /// Link speed (Mbps).
    #[serde(default)]
    pub speed_mbps: u32,
    /// Physical distance (meters).
    #[serde(default)]
    pub distance_m: u32,
    /// Operational status.
    #[serde(default)]
    pub status: ZoneStatus,
}

/// A network segment — an L3 boundary with subnet, gateway, and routing scope.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct NetworkSegment {
    /// Subnet CIDR (e.g., "192.168.4.0/22").
    #[serde(default)]
    pub subnet: Option<String>,
    /// Gateway IP.
    #[serde(default)]
    pub gateway: Option<String>,
    /// Transport type descriptor.
    #[serde(default)]
    pub transport: String,
    /// Human-readable description.
    #[serde(default)]
    pub description: String,
    /// Gates in this segment.
    #[serde(default)]
    pub gates: Vec<String>,
    /// Site this segment is at.
    #[serde(default)]
    pub site: Option<String>,
}

/// Latency estimate between two network domains.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct LatencyEstimate {
    /// Estimated one-way latency (ms).
    #[serde(default)]
    pub estimate_ms: f64,
    /// Confidence: "high", "medium", "low".
    #[serde(default)]
    pub confidence: String,
    /// Notes on measurement method.
    #[serde(default)]
    pub notes: String,
}

/// Affinity weights for Neural API routing bias.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct AffinityTable {
    /// Same gate (UDS IPC).
    #[serde(default)]
    pub same_gate: f64,
    /// Same L2 segment.
    #[serde(default)]
    pub same_segment: f64,
    /// Cross-segment bridged (same site).
    #[serde(default)]
    pub cross_segment_bridged: f64,
    /// Cross-site 10G trunk.
    #[serde(default)]
    pub cross_site_10g_trunk: f64,
    /// VPS relay path.
    #[serde(default)]
    pub vps_relay: f64,
    /// WAN TURN relay.
    #[serde(default)]
    pub wan_turn: f64,
    /// Remote contract (external compute).
    #[serde(default)]
    pub remote_contract: f64,
    /// Portable via `WiFi`.
    #[serde(default)]
    pub portable_wifi: f64,
    /// Portable via cellular.
    #[serde(default)]
    pub portable_cellular: f64,
    /// Portable via ADB USB (sub-1ms, high reliability).
    #[serde(default)]
    pub portable_adb: f64,
}

/// Full topology map parsed from `TOPOLOGY_MAP.toml`.
///
/// Represents the entire physical network: zones, segments, backbone links,
/// latency estimates, and affinity weights.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct TopologyMap {
    /// Map metadata.
    #[serde(default)]
    pub meta: TopologyMeta,
    /// Cytoplasm zones keyed by zone ID.
    #[serde(default)]
    pub zones: BTreeMap<String, PhysicalZone>,
    /// Backbone links.
    #[serde(default)]
    pub backbone: Vec<BackboneLink>,
    /// Network segments keyed by segment ID.
    #[serde(default)]
    pub segments: BTreeMap<String, NetworkSegment>,
    /// Latency estimates keyed by path name.
    #[serde(default)]
    pub latency: BTreeMap<String, LatencyEstimate>,
    /// Affinity table for routing bias.
    #[serde(default)]
    pub affinity: AffinityTable,
}

/// Topology map metadata.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct TopologyMeta {
    /// Schema version (e.g., "5.0.0").
    #[serde(default)]
    pub version: String,
    /// Wave at which this topology was last updated.
    #[serde(default)]
    pub wave: u32,
    /// Site topology model name.
    #[serde(default)]
    pub site_topology: String,
    /// WAN model.
    #[serde(default)]
    pub wan_model: String,
}

/// Resolved topology for a specific gate — the merger of manifest profile,
/// zone assignment, segment info, and detected interfaces.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ResolvedTopology {
    /// Gate name.
    pub gate: String,
    /// Zone ID this gate belongs to (from manifest or zone.gates).
    pub zone_id: Option<String>,
    /// Zone definition (if resolved).
    pub zone: Option<PhysicalZone>,
    /// Segment ID this gate belongs to.
    pub segment_id: Option<String>,
    /// Segment definition (if resolved).
    pub segment: Option<NetworkSegment>,
    /// Expected link speed from zone hub (Mbps).
    pub expected_speed_mbps: Option<u32>,
    /// Expected hub port annotation.
    pub hub_port: Option<String>,
    /// Site this gate is at.
    pub site: Option<String>,
    /// Validation issues found during resolution.
    pub issues: Vec<String>,
}

impl TopologyMap {
    /// Find which zone a gate belongs to by searching zone gate lists.
    #[must_use]
    pub fn zone_for_gate(&self, gate_name: &str) -> Option<(&str, &PhysicalZone)> {
        self.zones.iter().find_map(|(id, zone)| {
            if zone.gates.iter().any(|g| g == gate_name) {
                Some((id.as_str(), zone))
            } else {
                None
            }
        })
    }

    /// Find which segment a gate belongs to by searching segment gate lists.
    #[must_use]
    pub fn segment_for_gate(&self, gate_name: &str) -> Option<(&str, &NetworkSegment)> {
        self.segments.iter().find_map(|(id, seg)| {
            if seg.gates.iter().any(|g| g == gate_name) {
                Some((id.as_str(), seg))
            } else {
                None
            }
        })
    }

    /// Resolve the full topology context for a named gate.
    #[must_use]
    pub fn resolve_gate(&self, gate_name: &str) -> ResolvedTopology {
        let mut resolved = ResolvedTopology {
            gate: gate_name.to_string(),
            ..Default::default()
        };

        if let Some((zone_id, zone)) = self.zone_for_gate(gate_name) {
            resolved.zone_id = Some(zone_id.to_string());
            resolved.site = Some(zone.site.clone());
            resolved.expected_speed_mbps = Some(zone.max_speed_mbps);
            resolved.zone = Some(zone.clone());
        } else {
            resolved.issues.push(format!(
                "gate '{gate_name}' not found in any cytoplasm zone"
            ));
        }

        if let Some((seg_id, seg)) = self.segment_for_gate(gate_name) {
            resolved.segment_id = Some(seg_id.to_string());
            resolved.segment = Some(seg.clone());
        }

        resolved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_map() -> TopologyMap {
        let mut zones = BTreeMap::new();
        zones.insert(
            "backbone".to_string(),
            PhysicalZone {
                hub_device: "CRS310".into(),
                hub_role: "backbone".into(),
                max_speed_mbps: 10_000,
                site: "house1".into(),
                gates: vec!["sporeGate".into(), "eastGate".into()],
                status: ZoneStatus::Live,
                ..Default::default()
            },
        );
        zones.insert(
            "house2".to_string(),
            PhysicalZone {
                hub_device: "Omada SX3008F".into(),
                hub_role: "extension".into(),
                max_speed_mbps: 10_000,
                site: "house2".into(),
                gates: vec!["fieldGate".into()],
                uplink_zone: Some("backbone".into()),
                status: ZoneStatus::Live,
                ..Default::default()
            },
        );
        let mut segments = BTreeMap::new();
        segments.insert(
            "periplasm".to_string(),
            NetworkSegment {
                subnet: Some("192.168.4.0/22".into()),
                gateway: Some("192.168.4.1".into()),
                transport: "cat6_2.5g".into(),
                gates: vec!["sporeGate".into(), "eastGate".into()],
                site: Some("house1".into()),
                ..Default::default()
            },
        );
        TopologyMap {
            zones,
            segments,
            ..Default::default()
        }
    }

    #[test]
    fn zone_for_gate_finds_backbone() {
        let map = sample_map();
        let (id, zone) = map.zone_for_gate("sporeGate").unwrap();
        assert_eq!(id, "backbone");
        assert_eq!(zone.max_speed_mbps, 10_000);
    }

    #[test]
    fn zone_for_gate_finds_house2() {
        let map = sample_map();
        let (id, _) = map.zone_for_gate("fieldGate").unwrap();
        assert_eq!(id, "house2");
    }

    #[test]
    fn zone_for_gate_returns_none_for_unknown() {
        let map = sample_map();
        assert!(map.zone_for_gate("missingGate").is_none());
    }

    #[test]
    fn segment_for_gate_finds_periplasm() {
        let map = sample_map();
        let (id, seg) = map.segment_for_gate("eastGate").unwrap();
        assert_eq!(id, "periplasm");
        assert_eq!(seg.subnet.as_deref(), Some("192.168.4.0/22"));
    }

    #[test]
    fn resolve_gate_populates_zone_and_segment() {
        let map = sample_map();
        let resolved = map.resolve_gate("sporeGate");
        assert_eq!(resolved.zone_id.as_deref(), Some("backbone"));
        assert_eq!(resolved.segment_id.as_deref(), Some("periplasm"));
        assert_eq!(resolved.expected_speed_mbps, Some(10_000));
        assert_eq!(resolved.site.as_deref(), Some("house1"));
        assert!(resolved.issues.is_empty());
    }

    #[test]
    fn resolve_gate_flags_unknown_gate() {
        let map = sample_map();
        let resolved = map.resolve_gate("missingGate");
        assert!(resolved.zone_id.is_none());
        assert_eq!(resolved.issues.len(), 1);
        assert!(resolved.issues[0].contains("not found"));
    }

    #[test]
    fn zone_status_display() {
        assert_eq!(ZoneStatus::Live.to_string(), "live");
        assert_eq!(ZoneStatus::Workaround.to_string(), "workaround");
        assert_eq!(ZoneStatus::Planned.to_string(), "planned");
    }

    #[test]
    fn zone_status_default_is_live() {
        assert_eq!(ZoneStatus::default(), ZoneStatus::Live);
    }
}
