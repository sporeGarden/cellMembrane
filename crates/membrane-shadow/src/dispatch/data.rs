// SPDX-License-Identifier: AGPL-3.0-or-later

//! Data domain dispatch — manifest, identity, context.
//!
//! Plasmid, relay, and content domains are in their own modules
//! (`plasmid_dispatch`, `relay_dispatch`, `content_dispatch`).

use crate::cli;
use crate::error::ShadowError;
use crate::{ShadowOutcome, context, identity, manifest, temporal, topology};
use cellmembrane_types::cytoplasm::{ZoneLabel, mesh_address};

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
        _ => Ok(ShadowOutcome::fail(format!(
            "unknown manifest command: {cmd}"
        ))),
    }
}

// ── Topology domain ─────────────────────────────────────────────────

pub(super) async fn dispatch_topology(cmd: &str, args: &[&str]) -> crate::Result<ShadowOutcome> {
    match cmd {
        "topology.resolve" => {
            let gate_name = cli::require_arg(args, 0, "gate_name")?;
            topology_resolve(gate_name).await
        }
        "topology.service" => {
            let role = cli::require_arg(args, 0, "role")?;
            topology_service_resolve(role).await
        }
        "topology.roles" => topology_roles().await,
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
        .and_then(|p| p.mobility.as_deref())
        .unwrap_or("unknown");
    let mesh_ip_owned = m
        .mesh_ip_for(gate_name)
        .unwrap_or_else(|| "unpeered".into());
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
        format!("  mesh_ip:     {mesh_ip_owned}"),
    ];

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
        "mesh_ip": mesh_ip_owned,
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
        if let Some(ref seg) = physical.segment {
            if let Some(subnet) = &seg.subnet {
                lines.push(format!("  subnet:      {subnet}"));
            }
        }
    }
    for issue in &physical.issues {
        lines.push(format!("  ⚠ {issue}"));
    }
    if let Ok(val) = serde_json::to_value(&physical) {
        if let Some(obj) = data.as_object_mut() {
            obj.insert("physical".into(), val);
        }
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

        if let Some(ref map) = physical_map {
            let physical = map.zones.iter().find(|(id, pzone)| {
                *id == zone_label || pzone.gates.iter().any(|g| gates.contains(g))
            });
            if let Some((id, pzone)) = physical {
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
        }

        lines.push(line);
        data.insert(zone_label.clone(), entry);
    }

    Ok(ShadowOutcome::ok_with(
        lines.join("\n"),
        serde_json::Value::Object(data),
    ))
}

/// Resolve which gate(s) provide a service role — identity-based discovery.
///
/// `membrane topology.service forgejo` → finds the gate with `roles = ["forgejo"]`
/// and returns its mesh IP, zone, transport.
async fn topology_service_resolve(role: &str) -> crate::Result<ShadowOutcome> {
    let root = temporal::resolve_workspace_root()?;
    let m = manifest::load_from_workspace_async(&root).await?;

    let providers = m.gates_for_role(role);
    if providers.is_empty() {
        return Ok(ShadowOutcome::fail(format!(
            "no gate provides role '{role}' — add roles = [\"{role}\"] to gate profile in ecosystem_manifest.toml"
        )));
    }

    let mut lines = vec![format!("=== Service: {role} ===")];
    let mut data_entries = Vec::new();

    for (gate_name, profile) in &providers {
        let mesh_ip = m
            .mesh_ip_for(gate_name)
            .unwrap_or_else(|| "unpeered".into());
        let zone = profile
            .zone
            .unwrap_or_else(|| ZoneLabel::for_gate(gate_name));
        let transport = profile.transport.unwrap_or_default();

        lines.push(format!(
            "  {gate_name:<14} {mesh_ip:<14} {zone} ({transport})"
        ));

        data_entries.push(serde_json::json!({
            "gate": gate_name,
            "mesh_ip": mesh_ip,
            "zone": zone.label(),
            "transport": transport.to_string(),
            "roles": profile.roles,
            "mesh_peer": profile.mesh_peer,
            "composition": profile.composition,
        }));
    }

    if providers.len() == 1 {
        lines.push(format!(
            "\n  → {role} is hosted on {} (single provider)",
            providers[0].0
        ));
    } else {
        lines.push(format!(
            "\n  → {role} available on {} gates (multi-provider)",
            providers.len()
        ));
    }

    Ok(ShadowOutcome::ok_with(
        lines.join("\n"),
        serde_json::json!(data_entries),
    ))
}

/// List all roles across all gates — shows the role→gate mapping.
async fn topology_roles() -> crate::Result<ShadowOutcome> {
    let root = temporal::resolve_workspace_root()?;
    let m = manifest::load_from_workspace_async(&root).await?;

    let mut role_map: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();

    for (gate_name, profile) in &m.gates {
        for role in &profile.roles {
            role_map
                .entry(role.clone())
                .or_default()
                .push(gate_name.clone());
        }
    }

    if role_map.is_empty() {
        return Ok(ShadowOutcome::ok(String::from(
            "no roles defined — add roles = [...] to gate profiles in ecosystem_manifest.toml",
        )));
    }

    let mut lines = vec!["=== Service Role Map ===".to_owned()];
    let mut data = serde_json::Map::new();

    for (role, gates) in &role_map {
        lines.push(format!("  {role:<24} → {}", gates.join(", ")));
        data.insert(role.clone(), serde_json::json!(gates));
    }

    lines.push(format!(
        "\n{} roles across {} gates",
        role_map.len(),
        m.gates.len()
    ));

    Ok(ShadowOutcome::ok_with(
        lines.join("\n"),
        serde_json::Value::Object(data),
    ))
}

fn topology_mesh() -> ShadowOutcome {
    let gates = discover_mesh_gates();
    let subnet = cellmembrane_types::service::DEFAULT_WG_MESH_SUBNET;
    let mut lines = vec![format!("=== WireGuard Mesh ({subnet}) ===")];

    let manifest = temporal::resolve_workspace_root().ok().and_then(|root| {
        let path = root
            .join(cellmembrane_types::service::INFRA_WATERING_HOLE)
            .join("ecosystem_manifest.toml");
        crate::manifest::EcosystemManifest::load(&path).ok()
    });

    for gate in &gates {
        let ip = manifest
            .as_ref()
            .and_then(|m| m.mesh_ip_for(gate))
            .or_else(|| mesh_address(gate).map(String::from));
        if let Some(ip) = ip {
            let zone = ZoneLabel::for_gate(gate);
            let source = if manifest
                .as_ref()
                .and_then(|m| m.gates.get(gate.as_str()))
                .and_then(|p| p.wg_ip.as_ref())
                .is_some()
            {
                "manifest"
            } else {
                "bootstrap"
            };
            lines.push(format!("  {gate:<14} {ip:<14} {zone:<12} ({source})"));
        }
    }

    let data: Vec<serde_json::Value> = gates
        .iter()
        .filter_map(|g| {
            let ip = manifest
                .as_ref()
                .and_then(|m| m.mesh_ip_for(g))
                .or_else(|| mesh_address(g).map(String::from));
            ip.map(|ip| {
                serde_json::json!({
                    "gate": g,
                    "ip": ip,
                    "zone": ZoneLabel::for_gate(g).label(),
                })
            })
        })
        .collect();

    ShadowOutcome::ok_with(lines.join("\n"), serde_json::json!(data))
}

/// Discover mesh gates from topology map, falling back to cytoplasm bootstrap set.
fn discover_mesh_gates() -> Vec<String> {
    if let Ok(root) = temporal::resolve_workspace_root() {
        match topology::load_topology_map(&root) {
            Ok(map) => {
                let mut gates: Vec<String> = map
                    .segments
                    .values()
                    .filter(|s| {
                        s.transport.contains("wireguard") || s.transport.contains("overlay")
                    })
                    .flat_map(|s| s.gates.iter().cloned())
                    .collect();
                gates.sort();
                gates.dedup();
                if !gates.is_empty() {
                    return gates;
                }
            }
            Err(e) => {
                tracing::debug!(error = %e, "topology map unavailable, using bootstrap gates");
            }
        }
    }
    cellmembrane_types::cytoplasm::BOOTSTRAP_GATES
        .iter()
        .map(|&(name, _)| name.to_string())
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
                .map_err(|e| ShadowError::Config(format!("spawn_blocking: {e}")))?
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
