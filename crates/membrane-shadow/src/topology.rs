// SPDX-License-Identifier: AGPL-3.0-or-later

//! Topology resolution — load `TOPOLOGY_MAP.toml` and resolve gate context.
//!
//! Parses the physical network topology map (cytoplasm zones, segments,
//! backbone links) and cross-references with the ecosystem manifest to
//! produce a `ResolvedTopology` for any named gate.

use cellmembrane_types::topology::{
    BackboneLink, LatencyEstimate, NetworkSegment, PhysicalZone, TopologyMap, TopologyMeta,
};
use std::collections::BTreeMap;
use std::path::Path;

use crate::error::{Result, ShadowError};

/// Load `TOPOLOGY_MAP.toml` from the workspace's `infra/wateringHole/` directory.
///
/// # Errors
///
/// Returns `ShadowError::Io` if the file is missing, or `ShadowError::Toml`
/// if parsing fails.
pub(crate) fn load_topology_map(workspace_root: &Path) -> Result<TopologyMap> {
    let path = workspace_root
        .join(cellmembrane_types::service::INFRA_WATERING_HOLE)
        .join(cellmembrane_types::service::TOPOLOGY_MAP_FILENAME);
    let contents = std::fs::read_to_string(&path).map_err(ShadowError::Io)?;
    parse_topology_map(&contents)
}

/// Parse a `TOPOLOGY_MAP.toml` string into a typed `TopologyMap`.
///
/// The TOML uses nested `[cytoplasm.zones.<id>]` and `[segments.<id>]` sections
/// which don't map to a flat serde struct directly. This function extracts each
/// section manually for resilience against upstream TOML schema evolution.
pub(crate) fn parse_topology_map(contents: &str) -> Result<TopologyMap> {
    let table: toml::Table = contents.parse().map_err(ShadowError::Toml)?;

    let meta = table
        .get("meta")
        .and_then(try_deserialize::<TopologyMeta>)
        .unwrap_or_default();

    let zones = extract_zones(&table);
    let segments = extract_segments(&table);
    let backbone = extract_backbone(&table);
    let latency = extract_latency(&table);
    let affinity = table
        .get("affinity")
        .and_then(|v| toml::from_str(&v.to_string()).ok())
        .unwrap_or_default();

    Ok(TopologyMap {
        meta,
        zones,
        backbone,
        segments,
        latency,
        affinity,
    })
}

fn try_deserialize<T: serde::de::DeserializeOwned>(value: &toml::Value) -> Option<T> {
    value.clone().try_into().ok()
}

fn extract_zones(table: &toml::Table) -> BTreeMap<String, PhysicalZone> {
    let mut zones = BTreeMap::new();
    let Some(cytoplasm) = table.get("cytoplasm").and_then(|v| v.as_table()) else {
        return zones;
    };
    let Some(zone_table) = cytoplasm.get("zones").and_then(|v| v.as_table()) else {
        return zones;
    };
    for (id, value) in zone_table {
        if let Some(zone) = try_deserialize::<PhysicalZone>(value) {
            zones.insert(id.clone(), zone);
        }
    }
    zones
}

fn extract_segments(table: &toml::Table) -> BTreeMap<String, NetworkSegment> {
    let mut segments = BTreeMap::new();
    let Some(seg_table) = table.get("segments").and_then(|v| v.as_table()) else {
        return segments;
    };
    for (id, value) in seg_table {
        if let Some(seg) = try_deserialize::<NetworkSegment>(value) {
            segments.insert(id.clone(), seg);
        }
    }
    segments
}

fn extract_backbone(table: &toml::Table) -> Vec<BackboneLink> {
    let Some(bt) = table.get("backbone_triangle").and_then(|v| v.as_table()) else {
        return Vec::new();
    };
    let mut links = Vec::new();
    for (key, value) in bt {
        if key.starts_with("leg_") {
            if let Some(link) = try_deserialize::<BackboneLink>(value) {
                links.push(link);
            }
        }
    }
    links
}

fn extract_latency(table: &toml::Table) -> BTreeMap<String, LatencyEstimate> {
    let mut latencies = BTreeMap::new();
    let Some(lat_table) = table.get("latency").and_then(|v| v.as_table()) else {
        return latencies;
    };
    for (id, value) in lat_table {
        if let Some(est) = try_deserialize::<LatencyEstimate>(value) {
            latencies.insert(id.clone(), est);
        }
    }
    latencies
}

/// Format a `TopologyMap` summary for human output.
#[must_use]
pub(crate) fn format_topology_summary(map: &TopologyMap) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "Topology v{} (wave {})",
        map.meta.version, map.meta.wave
    );
    let _ = writeln!(out, "Model: {}\n", map.meta.site_topology);

    out.push_str("Zones:\n");
    for (id, zone) in &map.zones {
        let gc = zone.gates.len();
        let _ = writeln!(
            out,
            "  {id}: {} ({}, {}G max, {gc} gates, {})",
            zone.hub_device,
            zone.site,
            zone.max_speed_mbps / 1000,
            zone.status
        );
    }

    if !map.backbone.is_empty() {
        out.push_str("\nBackbone:\n");
        for link in &map.backbone {
            let _ = writeln!(
                out,
                "  {} <-> {} ({}G, {}, {})",
                link.from,
                link.to,
                link.speed_mbps / 1000,
                link.medium,
                link.status
            );
        }
    }

    let _ = writeln!(out, "\nSegments: {}", map.segments.len());
    for (id, seg) in &map.segments {
        let subnet = seg.subnet.as_deref().unwrap_or("(no subnet)");
        let gc = seg.gates.len();
        let _ = writeln!(out, "  {id}: {subnet} ({gc} gates)");
    }

    let _ = writeln!(out, "Latency paths: {}", map.latency.len());
    out
}

/// Format a resolved gate topology for human output.
#[cfg(test)]
#[must_use]
fn format_resolved(resolved: &cellmembrane_types::topology::ResolvedTopology) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(out, "Gate: {}", resolved.gate);

    if let Some(zone_id) = &resolved.zone_id {
        let _ = write!(out, "Zone: {zone_id}");
        if let Some(site) = &resolved.site {
            let _ = write!(out, " (site: {site})");
        }
        out.push('\n');
    } else {
        out.push_str("Zone: (not assigned)\n");
    }

    if let Some(speed) = resolved.expected_speed_mbps {
        let _ = writeln!(out, "Expected speed: {}G", speed / 1000);
    }

    if let Some(hub_port) = &resolved.hub_port {
        let _ = writeln!(out, "Hub port: {hub_port}");
    }

    if let Some(seg_id) = &resolved.segment_id {
        let _ = write!(out, "Segment: {seg_id}");
        if let Some(seg) = &resolved.segment {
            if let Some(subnet) = &seg.subnet {
                let _ = write!(out, " ({subnet})");
            }
        }
        out.push('\n');
    }

    if !resolved.issues.is_empty() {
        out.push_str("\nIssues:\n");
        for issue in &resolved.issues {
            let _ = writeln!(out, "  - {issue}");
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_TOML: &str = r#"
[meta]
version = "5.0.0"
wave = 116
site_topology = "triangle_3hub_backbone"
wan_model = "single_exit_star"

[cytoplasm.zones.backbone]
hub_device = "MikroTik CRS310"
hub_role = "backbone"
max_speed_mbps = 10000
site = "house1"
gates = ["sporeGate", "eastGate"]

[cytoplasm.zones.house2]
hub_device = "Omada SX3008F"
hub_role = "extension"
max_speed_mbps = 10000
site = "house2"
gates = ["fieldGate"]
uplink_zone = "backbone"

[segments.periplasm]
subnet = "192.168.4.0/22"
gateway = "192.168.4.1"
transport = "cat6_2.5g"
gates = ["sporeGate", "eastGate"]

[backbone_triangle.leg_b]
from = "house1"
to = "house2"
medium = "80m AOC (10G SFP+)"
speed_mbps = 10000
distance_m = 80
status = "live"

[latency.backbone_to_backbone]
estimate_ms = 0.5
confidence = "high"
notes = "Same switch"

[affinity]
same_gate = 1.0
same_segment = 0.9
vps_relay = 0.4
wan_turn = 0.3
"#;

    #[test]
    fn parse_topology_map_extracts_zones() {
        let map = parse_topology_map(SAMPLE_TOML).unwrap();
        assert_eq!(map.zones.len(), 2);
        assert!(map.zones.contains_key("backbone"));
        assert!(map.zones.contains_key("house2"));
        let bb = &map.zones["backbone"];
        assert_eq!(bb.max_speed_mbps, 10_000);
        assert_eq!(bb.gates, vec!["sporeGate", "eastGate"]);
    }

    #[test]
    fn parse_topology_map_extracts_segments() {
        let map = parse_topology_map(SAMPLE_TOML).unwrap();
        assert!(map.segments.contains_key("periplasm"));
        assert_eq!(
            map.segments["periplasm"].subnet.as_deref(),
            Some("192.168.4.0/22")
        );
    }

    #[test]
    fn parse_topology_map_extracts_backbone() {
        let map = parse_topology_map(SAMPLE_TOML).unwrap();
        assert_eq!(map.backbone.len(), 1);
        assert_eq!(map.backbone[0].speed_mbps, 10_000);
    }

    #[test]
    fn parse_topology_map_extracts_meta() {
        let map = parse_topology_map(SAMPLE_TOML).unwrap();
        assert_eq!(map.meta.version, "5.0.0");
        assert_eq!(map.meta.wave, 116);
    }

    #[test]
    fn parse_topology_map_extracts_latency() {
        let map = parse_topology_map(SAMPLE_TOML).unwrap();
        assert!(map.latency.contains_key("backbone_to_backbone"));
    }

    #[test]
    fn resolve_gate_via_parsed_map() {
        let map = parse_topology_map(SAMPLE_TOML).unwrap();
        let resolved = map.resolve_gate("eastGate");
        assert_eq!(resolved.zone_id.as_deref(), Some("backbone"));
        assert_eq!(resolved.segment_id.as_deref(), Some("periplasm"));
        assert_eq!(resolved.expected_speed_mbps, Some(10_000));
        assert!(resolved.issues.is_empty());
    }

    #[test]
    fn format_topology_summary_output() {
        let map = parse_topology_map(SAMPLE_TOML).unwrap();
        let summary = format_topology_summary(&map);
        assert!(summary.contains("v5.0.0"));
        assert!(summary.contains("backbone"));
        assert!(summary.contains("periplasm"));
    }

    #[test]
    fn format_resolved_output() {
        let map = parse_topology_map(SAMPLE_TOML).unwrap();
        let resolved = map.resolve_gate("eastGate");
        let output = format_resolved(&resolved);
        assert!(output.contains("eastGate"));
        assert!(output.contains("backbone"));
        assert!(output.contains("192.168.4.0/22"));
    }
}
