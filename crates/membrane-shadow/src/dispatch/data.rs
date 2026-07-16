// SPDX-License-Identifier: AGPL-3.0-or-later

//! Data domain dispatch — manifest, identity, context.
//!
//! Plasmid, relay, and content domains are in their own modules
//! (`plasmid_dispatch`, `relay_dispatch`, `content_dispatch`).

use crate::cli;
use crate::error::ShadowError;
use crate::{ShadowOutcome, context, identity, manifest, temporal, topology};
use cellmembrane_types::cytoplasm::{KNOWN_MESH_GATES, ZoneLabel, mesh_address};

// ── Manifest domain ──────────────────────────────────────────────────

pub(super) async fn dispatch_manifest(cmd: &str, args: &[&str]) -> crate::Result<ShadowOutcome> {
    let root = temporal::resolve_workspace_root()?;
    match cmd {
        "manifest.info" => {
            let m = manifest::load_from_workspace_async(&root).await?;
            let topo = m.topology.as_ref().map_or_else(
                || "monoderm (no topology section)".to_string(),
                |t| {
                    let roles = t.roles.as_ref().map_or_else(
                        || "no roles assigned".to_string(),
                        |r| {
                            format!(
                                "receiver={} mediator={} publisher={}",
                                r.push_receiver, r.sync_mediator, r.external_publisher
                            )
                        },
                    );
                    format!(
                        "{}: {} → {} → {} ({})",
                        t.model, t.inner_membrane, t.peptidoglycan, t.outer_membrane, roles
                    )
                },
            );
            let msg = format!(
                "manifest v{} wave {} ({} repos)\n\
                 sync: source={} branch={} push_target={} divergence={}\n\
                 topology: {}",
                m.meta.version,
                m.meta.wave,
                m.meta.total_repos,
                m.sync.default_source,
                m.sync.default_branch,
                m.sync.push_target,
                m.sync.divergence_policy,
                topo,
            );
            Ok(ShadowOutcome::ok_with(msg, serde_json::to_value(&m)?))
        }
        "manifest.repos" => {
            let m = manifest::load_from_workspace_async(&root).await?;
            let repos: Vec<(&str, &manifest::RepoEntry)> = if let Some(gate_name) = args.first() {
                m.gate_repos(gate_name)
            } else {
                m.repos.iter().map(|(n, e)| (n.as_str(), e)).collect()
            };
            let lines: Vec<String> = repos
                .iter()
                .map(|(name, e)| {
                    format!(
                        "  {:<25} {:<30} {:<18} {}",
                        name, e.local_path, e.membrane, e.category
                    )
                })
                .collect();
            let header = args.first().map_or_else(
                || format!("{} repos total", repos.len()),
                |g| format!("{} repos for gate {g}", repos.len()),
            );
            Ok(ShadowOutcome::ok(format!("{header}\n{}", lines.join("\n"))))
        }
        "manifest.orgs" => {
            let m = manifest::load_from_workspace_async(&root).await?;
            let orgs = m.orgs();
            Ok(ShadowOutcome::ok(format!(
                "{} orgs: {}",
                orgs.len(),
                orgs.join(", ")
            )))
        }
        "manifest.validate" => manifest_validate(&root).await,
        _ => Ok(ShadowOutcome::fail(format!(
            "unknown manifest command: {cmd}"
        ))),
    }
}

async fn manifest_validate(root: &std::path::Path) -> crate::Result<ShadowOutcome> {
    let m = manifest::load_from_workspace_async(root).await?;
    let issues = m.validate();
    if issues.is_empty() {
        Ok(ShadowOutcome::ok_with(
            format!(
                "manifest v{} VALID ({} repos, {} gates)",
                m.meta.version,
                m.repos.len(),
                m.gates.len(),
            ),
            serde_json::json!({
                "valid": true,
                "repos": m.repos.len(),
                "gates": m.gates.len(),
                "version": m.meta.version,
            }),
        ))
    } else {
        let lines: Vec<String> = issues.iter().map(|i| format!("  \u{26a0} {i}")).collect();
        Ok(ShadowOutcome::ok_with(
            format!(
                "manifest v{} — {} issue(s)\n{}",
                m.meta.version,
                issues.len(),
                lines.join("\n"),
            ),
            serde_json::json!({
                "valid": false,
                "issues": issues,
                "repos": m.repos.len(),
                "gates": m.gates.len(),
                "version": m.meta.version,
            }),
        ))
    }
}

// ── Topology domain ─────────────────────────────────────────────────

pub(super) async fn dispatch_topology(cmd: &str, args: &[&str]) -> crate::Result<ShadowOutcome> {
    match cmd {
        "topology.resolve" => {
            let gate_name = cli::require_arg(args, 0, "gate_name")?;
            topology_resolve(gate_name).await
        }
        "topology.zones" => topology_zones().await,
        "topology.mesh" => Ok(topology_mesh()),
        "topology.summary" => {
            let root = temporal::resolve_workspace_root()?;
            let map = topology::load_topology_map(&root)?;
            let summary = topology::format_topology_summary(&map);
            let data = serde_json::to_value(&map)?;
            Ok(ShadowOutcome::ok_with(summary, data))
        }
        _ => Ok(ShadowOutcome::fail(format!(
            "unknown topology command: {cmd}"
        ))),
    }
}

async fn topology_resolve(gate_name: &str) -> crate::Result<ShadowOutcome> {
    let root = temporal::resolve_workspace_root()?;
    let m = manifest::load_from_workspace_async(&root).await?;

    let zone = m
        .gates
        .get(gate_name)
        .and_then(|p| p.zone)
        .unwrap_or_else(|| ZoneLabel::for_gate(gate_name));

    let profile = m.gates.get(gate_name);

    let transport = profile.and_then(|p| p.transport).unwrap_or_default();
    let composition = profile
        .and_then(|p| p.composition.as_deref())
        .unwrap_or("unknown");
    let target = profile
        .and_then(|p| p.target.as_deref())
        .unwrap_or("unknown");
    let mobility = profile
        .and_then(|p| p.mobility)
        .map_or_else(|| "unknown".to_string(), |m| m.to_string());
    let mesh_ip = profile
        .and_then(|p| p.wg_ip.as_deref())
        .or_else(|| mesh_address(gate_name))
        .unwrap_or("unpeered");
    let lan_ip = profile.and_then(|p| p.lan_ip.as_deref());
    let dns_name = cellmembrane_types::service::lan_dns_name(gate_name);
    let mesh_peer = profile.and_then(|p| p.mesh_peer.as_deref());
    let hub_port = profile.and_then(|p| p.hub_port.as_deref());
    let link_speed = profile.and_then(|p| p.link_speed_mbps);

    let envelope = if zone.requires_overlay() {
        cellmembrane_types::EnvelopeTopology::Diderm
    } else {
        cellmembrane_types::EnvelopeTopology::Monoderm
    };

    let mut lines = vec![
        format!("=== Topology: {gate_name} ==="),
        format!("  zone:        {zone}"),
        format!("  transport:   {transport}"),
        format!("  composition: {composition}"),
        format!("  target:      {target}"),
        format!("  mobility:    {mobility}"),
        format!(
            "  envelope:    {envelope} ({} boundaries)",
            envelope.boundary_count()
        ),
        format!("  mesh_ip:     {mesh_ip}"),
        format!("  dns_name:    {dns_name}"),
    ];

    if let Some(lip) = lan_ip {
        lines.push(format!("  lan_ip:      {lip}"));
    }
    if let Some(peer) = mesh_peer {
        lines.push(format!("  mesh_peer:   {peer}"));
    }
    if let Some(port) = hub_port {
        lines.push(format!("  hub_port:    {port}"));
    }
    if let Some(speed) = link_speed {
        lines.push(format!("  link_speed:  {speed} Mbps"));
    }
    if zone.has_l2_backbone() {
        lines.push("  l2_backbone: yes (direct switched)".into());
    }
    if zone.requires_overlay() {
        lines.push("  overlay:     required (WireGuard)".into());
    }
    if profile.is_none() {
        lines.push(format!(
            "  ⚠ {gate_name} not in ecosystem manifest — zone derived from name"
        ));
    }

    let mut data = serde_json::json!({
        "gate": gate_name,
        "zone": zone.label(),
        "transport": transport,
        "composition": composition,
        "target": target,
        "mobility": mobility,
        "envelope": envelope.to_string(),
        "mesh_ip": mesh_ip,
        "lan_ip": lan_ip,
        "dns_name": dns_name,
        "mesh_peer": mesh_peer,
        "hub_port": hub_port,
        "link_speed_mbps": link_speed,
        "l2_backbone": zone.has_l2_backbone(),
        "requires_overlay": zone.requires_overlay(),
    });

    enrich_with_physical_topology(&root, gate_name, &mut lines, &mut data);

    Ok(ShadowOutcome::ok_with(lines.join("\n"), data))
}

fn enrich_with_physical_topology(
    root: &std::path::Path,
    gate_name: &str,
    lines: &mut Vec<String>,
    data: &mut serde_json::Value,
) {
    let Ok(map) = topology::load_topology_map(root) else {
        return;
    };
    let physical = map.resolve_gate(gate_name);
    if let Some(ref pzone) = physical.zone {
        lines.push(format!("  hub_device:  {}", pzone.hub_device));
        lines.push(format!("  site:        {}", pzone.site));
        if !pzone.hub_role.is_empty() {
            lines.push(format!("  hub_role:    {}", pzone.hub_role));
        }
        if let Some(speed) = physical.expected_speed_mbps {
            lines.push(format!("  max_speed:   {} Mbps ({}G)", speed, speed / 1000));
        }
    }
    if let Some(ref seg_id) = physical.segment_id {
        lines.push(format!("  segment:     {seg_id}"));
        if let Some(ref seg) = physical.segment
            && let Some(subnet) = &seg.subnet
        {
            lines.push(format!("  subnet:      {subnet}"));
        }
    }
    for issue in &physical.issues {
        lines.push(format!("  ⚠ {issue}"));
    }
    if let Ok(val) = serde_json::to_value(&physical)
        && let Some(obj) = data.as_object_mut()
    {
        obj.insert("physical".into(), val);
    }
}

async fn topology_zones() -> crate::Result<ShadowOutcome> {
    let root = temporal::resolve_workspace_root()?;
    let m = manifest::load_from_workspace_async(&root).await?;
    let physical_map = topology::load_topology_map(&root).ok();

    let mut zone_gates: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();

    for (name, profile) in &m.gates {
        let zone = profile.zone.unwrap_or_else(|| ZoneLabel::for_gate(name));
        zone_gates
            .entry(zone.label().to_owned())
            .or_default()
            .push(name.clone());
    }

    let mut lines = vec!["=== Cytoplasm Zone Map ===".to_owned()];
    let mut data = serde_json::Map::new();

    for (zone_label, gates) in &zone_gates {
        let mut line = format!(
            "  {zone_label:<12} {} gate(s): {}",
            gates.len(),
            gates.join(", ")
        );

        let mut entry = serde_json::json!({ "gates": gates });

        if let Some(ref map) = physical_map
            && let Some((id, pzone)) = map.zones.iter().find(|(id, pzone)| {
                *id == zone_label || pzone.gates.iter().any(|g| gates.contains(g))
            })
        {
            use std::fmt::Write;
            let _ = write!(
                line,
                "\n    hub: {} @ {} ({}G max, {} physical gates)",
                pzone.hub_device,
                pzone.site,
                pzone.max_speed_mbps / 1000,
                pzone.gates.len()
            );
            if let Some(obj) = entry.as_object_mut() {
                obj.insert("physical_zone_id".into(), serde_json::json!(id));
                obj.insert("hub_device".into(), serde_json::json!(&pzone.hub_device));
                obj.insert("site".into(), serde_json::json!(&pzone.site));
                obj.insert(
                    "max_speed_mbps".into(),
                    serde_json::json!(pzone.max_speed_mbps),
                );
                obj.insert("physical_gates".into(), serde_json::json!(&pzone.gates));
            }
        }

        lines.push(line);
        data.insert(zone_label.clone(), entry);
    }

    Ok(ShadowOutcome::ok_with(
        lines.join("\n"),
        serde_json::Value::Object(data),
    ))
}

fn topology_mesh() -> ShadowOutcome {
    let manifest = temporal::resolve_workspace_root()
        .ok()
        .and_then(|root| manifest::EcosystemManifest::find_in_workspace(&root))
        .and_then(|path| manifest::EcosystemManifest::load(&path).ok());

    let gate_names: Vec<String> = manifest.as_ref().map_or_else(
        || KNOWN_MESH_GATES.iter().map(|&s| s.to_owned()).collect(),
        |m| m.gates.keys().cloned().collect(),
    );

    let mut lines = vec![format!(
        "=== WireGuard Mesh ({}) ===",
        cellmembrane_types::service::DEFAULT_WG_MESH_SUBNET
    )];

    for gate in &gate_names {
        let ip = resolve_mesh_ip(manifest.as_ref(), gate);
        if let Some(ip) = ip {
            let zone = manifest
                .as_ref()
                .and_then(|m| m.gates.get(gate.as_str()))
                .and_then(|p| p.zone)
                .unwrap_or_else(|| ZoneLabel::for_gate(gate));
            lines.push(format_mesh_line(gate, ip, zone));
        }
    }

    let data = build_mesh_data(manifest.as_ref(), &gate_names);
    ShadowOutcome::ok_with(lines.join("\n"), serde_json::json!(data))
}

/// Resolve mesh IP: manifest `wg_ip` first, then static fallback.
fn resolve_mesh_ip<'a>(
    manifest: Option<&'a manifest::EcosystemManifest>,
    gate: &str,
) -> Option<&'a str> {
    manifest
        .and_then(|m| m.gates.get(gate))
        .and_then(|p| p.wg_ip.as_deref())
        .or_else(|| mesh_address(gate))
}

/// Format a single mesh entry line for display.
fn format_mesh_line(gate: &str, ip: &str, zone: ZoneLabel) -> String {
    format!("  {gate:<14} {ip:<14} {zone}")
}

/// Build the JSON data array for mesh entries.
fn build_mesh_data(
    manifest: Option<&manifest::EcosystemManifest>,
    gate_names: &[String],
) -> Vec<serde_json::Value> {
    gate_names
        .iter()
        .filter_map(|g| {
            resolve_mesh_ip(manifest, g).map(|ip| {
                let zone = manifest
                    .and_then(|m| m.gates.get(g.as_str()))
                    .and_then(|p| p.zone)
                    .unwrap_or_else(|| ZoneLabel::for_gate(g));
                serde_json::json!({
                    "gate": g,
                    "ip": ip,
                    "zone": zone.label(),
                })
            })
        })
        .collect()
}

// ── Identity domain ──────────────────────────────────────────────────

pub(super) async fn dispatch_identity() -> crate::Result<ShadowOutcome> {
    let root = temporal::resolve_workspace_root()?;
    match identity::resolve_async(&root).await {
        Ok(id) => Ok(ShadowOutcome::ok_with(
            format!("{} (via {:?})", id.name, id.source),
            serde_json::to_value(&id)?,
        )),
        Err(e) => Ok(ShadowOutcome::fail(e)),
    }
}

// ── Context domain (sweetGrass-external braids) ──────────────────────

pub(super) async fn dispatch_context(cmd: &str, args: &[&str]) -> crate::Result<ShadowOutcome> {
    let root = temporal::resolve_workspace_root()?;
    match cmd {
        "context.weave" => {
            let weave_args = cli::parse_context_weave_args(args)?;
            let braid = context::weave(&root, &weave_args).await?;
            Ok(ShadowOutcome::ok_with(
                format!(
                    "WOVEN [{status}] {gate}/{slug}: {summary}",
                    status = braid.strands.focus.status,
                    gate = braid.braid.gate,
                    slug = cli::context_slug(&braid.braid.project),
                    summary = braid.strands.focus.summary,
                ),
                serde_json::to_value(&braid)?,
            ))
        }
        "context.sense" => {
            let all = args.contains(&"--all");
            let filter_gate = cli::extract_flag_value(args, "--gate").map(str::to_owned);
            let filter_project = cli::extract_flag_value(args, "--project").map(str::to_owned);
            let braids = {
                let root = root.clone();
                let fg = filter_gate.clone();
                let fp = filter_project.clone();
                tokio::task::spawn_blocking(move || {
                    context::sense(&root, fg.as_deref(), fp.as_deref(), all)
                })
                .await
                .map_err(|e| {
                    ShadowError::Io(std::io::Error::other(format!(
                        "spawn_blocking panicked: {e}"
                    )))
                })?
            }?;
            if braids.is_empty() {
                Ok(ShadowOutcome::ok(
                    "No context braids woven (resting state).".to_string(),
                ))
            } else {
                let lines: Vec<String> = braids
                    .iter()
                    .map(|b| {
                        format!(
                            "  [{status}] {gate}/{project}: {summary}",
                            status = b.strands.focus.status,
                            gate = b.braid.gate,
                            project = cli::context_slug(&b.braid.project),
                            summary = b.strands.focus.summary,
                        )
                    })
                    .collect();
                Ok(ShadowOutcome::ok_with(
                    format!("{} context braid(s)\n{}", braids.len(), lines.join("\n")),
                    serde_json::to_value(&braids)?,
                ))
            }
        }
        "context.clear" => {
            let project = cli::extract_flag_value(args, "--project");
            let expired = args.contains(&"--expired");
            let cleared = context::clear(&root, project, expired).await?;
            if cleared.is_empty() {
                Ok(ShadowOutcome::ok("No braids to clear.".to_string()))
            } else {
                Ok(ShadowOutcome::ok(format!(
                    "Cleared {} braid(s): {}",
                    cleared.len(),
                    cleared.join(", "),
                )))
            }
        }
        _ => Ok(ShadowOutcome::fail(format!(
            "unknown context command: {cmd}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_mesh_line_aligned() {
        let line = format_mesh_line("golgi", "10.13.37.1", ZoneLabel::Wan);
        assert!(line.contains("golgi"));
        assert!(line.contains("10.13.37.1"));
        assert!(line.contains("wan"));
    }

    #[test]
    fn build_mesh_data_includes_known_gates() {
        let gates: Vec<String> = vec!["golgi".into(), "sporeGate".into(), "unknownGate".into()];
        let data = build_mesh_data(None, &gates);
        assert_eq!(data.len(), 2, "unknownGate should be filtered out");
        assert_eq!(data[0]["gate"], "golgi");
        assert_eq!(data[1]["gate"], "sporeGate");
    }

    #[test]
    fn build_mesh_data_empty_input() {
        let data = build_mesh_data(None, &[]);
        assert!(data.is_empty());
    }

    #[test]
    fn build_mesh_data_includes_zone_label() {
        let gates = vec!["eastGate".into()];
        let data = build_mesh_data(None, &gates);
        assert_eq!(data.len(), 1);
        assert_eq!(data[0]["zone"], "backbone");
    }

    #[test]
    fn topology_mesh_uses_known_gates_fallback() {
        let outcome = topology_mesh();
        let msg = outcome.message;
        assert!(msg.contains("WireGuard Mesh"));
        for gate in KNOWN_MESH_GATES {
            if mesh_address(gate).is_some() {
                assert!(msg.contains(gate), "mesh output should include {gate}");
            }
        }
    }

    #[tokio::test]
    async fn dispatch_manifest_unknown_command() {
        let result = dispatch_manifest("manifest.bogus", &[]).await.unwrap();
        assert!(!result.ok);
        assert!(result.message.contains("unknown manifest command"));
    }
}
