// SPDX-License-Identifier: AGPL-3.0-or-later

//! Gate network configuration generators — firewall rulesets and `WireGuard` configs.
//!
//! Extracted from `gate.rs` for independent testability. These are pure
//! manifest-to-config renderers with no async I/O beyond manifest loading.

use crate::cli;
use crate::ShadowOutcome;

pub(super) fn dispatch_firewall_generate(args: &[&str]) -> crate::Result<ShadowOutcome> {
    use cellmembrane_types::composition::MembraneComposition;
    use cellmembrane_types::firewall::{FirewallRuleset, NftablesConfig};

    let gate_name_owned = cli::extract_flag_value(args, "--gate-name")
        .or_else(|| cli::extract_flag_value(args, "--gate"))
        .map_or_else(crate::gate::resolve_local_gate_identity, String::from);
    let gate_name: &str = &gate_name_owned;

    let manifest = crate::temporal::resolve_workspace_root()
        .ok()
        .and_then(|root| crate::manifest::load_from_workspace(&root).ok());

    let profile = manifest.as_ref().and_then(|m| m.gates.get(gate_name));

    let comp_str = cli::extract_flag_value(args, "--composition")
        .or_else(|| args.first().filter(|a| !a.starts_with("--")).copied())
        .or_else(|| profile.and_then(|p| p.composition.as_deref()))
        .unwrap_or("relay");
    let composition = MembraneComposition::parse_name(comp_str).ok_or_else(|| {
        crate::error::ShadowError::Config(format!(
            "unknown composition: {comp_str} (expected: {})",
            MembraneComposition::all()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ))
    })?;

    let fw = FirewallRuleset::for_composition(composition);
    let format = cli::extract_flag_value(args, "--format").unwrap_or("nftables");

    let is_plasma_membrane = args.contains(&"--plasma-membrane")
        || profile.is_some_and(|p| p.roles.contains(&cellmembrane_types::GateRole::NatFirewall));

    let nft_config = if is_plasma_membrane {
        let wan = cli::extract_flag_value(args, "--wan")
            .or_else(|| profile.and_then(|p| p.wan_interface.as_deref()))
            .unwrap_or(cellmembrane_types::service::DEFAULT_WAN_IFACE);
        let lan = cli::extract_flag_value(args, "--lan")
            .or_else(|| profile.and_then(|p| p.lan_interface.as_deref()))
            .unwrap_or(cellmembrane_types::service::DEFAULT_LAN_IFACE);
        let subnet = cli::extract_flag_value(args, "--subnet")
            .or_else(|| profile.and_then(|p| p.lan_subnet.as_deref()))
            .unwrap_or(cellmembrane_types::service::DEFAULT_LAN_SUBNET);
        let has_wg = profile.is_some_and(|p| p.wg_ip.is_some());
        Some(NftablesConfig {
            wan_interface: wan.into(),
            lan_interface: lan.into(),
            lan_subnet: subnet.into(),
            gate_name: gate_name.into(),
            enable_nat: !args.contains(&"--no-nat"),
            enable_dhcp: !args.contains(&"--no-dhcp")
                && profile.is_some_and(|p| p.roles.contains(&cellmembrane_types::GateRole::Dhcp)),
            trust_lan_input: args.contains(&"--trust-lan")
                || profile
                    .is_some_and(|p| p.roles.contains(&cellmembrane_types::GateRole::NatFirewall)),
            wireguard_interface: cli::extract_flag_value(args, "--wg-iface")
                .map(Into::into)
                .or_else(|| if has_wg { Some("wg0".into()) } else { None }),
            wireguard_port: cli::extract_flag_value(args, "--wg-port")
                .and_then(|p| p.parse().ok())
                .unwrap_or(cellmembrane_types::firewall::default_wg_port()),
            drop_ipv6_forward: !args.contains(&"--allow-ipv6-forward"),
        })
    } else {
        None
    };

    let script = match format {
        "ufw" => fw.to_ufw_script(),
        "nftables" | "nft" => fw.to_nftables_script(nft_config.as_ref()),
        other => {
            return Err(crate::error::ShadowError::Config(format!(
                "unknown format: {other} (expected: nftables, ufw)"
            )));
        }
    };

    Ok(ShadowOutcome::ok(script))
}

pub(super) async fn dispatch_wireguard_generate(args: &[&str]) -> crate::Result<ShadowOutcome> {
    use cellmembrane_types::wireguard::{DEFAULT_WG_PORT, WgConfig, WgPeer};

    let root = crate::temporal::resolve_workspace_root()?;
    let m = crate::manifest::load_from_workspace_async(&root).await?;

    let gate_name = cli::extract_flag_value(args, "--gate")
        .map_or_else(crate::gate::resolve_local_gate_identity, str::to_string);

    let listen_port: u16 = cli::extract_flag_value(args, "--port")
        .and_then(|p| p.parse().ok())
        .unwrap_or(DEFAULT_WG_PORT);

    let subnet = cli::extract_flag_value(args, "--subnet")
        .unwrap_or(cellmembrane_types::service::DEFAULT_WG_MESH_SUBNET);

    let local_ip = m.mesh_ip_for(&gate_name).map(String::from).ok_or_else(|| {
        crate::error::ShadowError::Config(format!(
            "gate '{gate_name}' has no WG mesh IP — add wg_ip to its manifest profile"
        ))
    })?;

    let keepalive: u16 = cli::extract_flag_value(args, "--keepalive")
        .and_then(|k| k.parse().ok())
        .unwrap_or(25);

    let hub_mode = cli::extract_flag_value(args, "--hub").unwrap_or_else(|| {
        m.gates_for_role("wg_hub")
            .first()
            .map_or("", |&(name, _)| name)
    });

    let is_local_hub = m
        .gates
        .get(&gate_name)
        .is_some_and(|p| p.roles.contains(&cellmembrane_types::GateRole::WgHub));

    let mut peers = Vec::new();
    for (name, profile) in &m.gates {
        if *name == gate_name {
            continue;
        }
        let Some(mesh_ip) = m.mesh_ip_for(name).map(String::from) else {
            continue;
        };

        let is_hub = profile.roles.contains(&cellmembrane_types::GateRole::WgHub);

        let endpoint = profile
            .wan_endpoint
            .as_deref()
            .or(profile.host.as_deref())
            .map(String::from);

        let allowed_ips = if is_hub && !is_local_hub {
            vec![format!("{subnet}")]
        } else if is_local_hub {
            let mut ips = vec![format!("{mesh_ip}/32")];
            if let Some(ref lan) = profile.lan_subnet {
                ips.push(lan.clone());
            }
            ips
        } else {
            vec![format!("{mesh_ip}/32")]
        };

        peers.push(WgPeer {
            name: name.clone(),
            mesh_ip,
            public_key: profile.wg_pubkey.clone(),
            endpoint,
            allowed_ips,
            keepalive,
        });
    }

    if !is_local_hub && !hub_mode.is_empty() && peers.iter().any(|p| p.name != hub_mode) {
        peers.retain(|p| p.name == hub_mode);
    }

    peers.sort_by(|a, b| a.name.cmp(&b.name));

    let config = WgConfig {
        gate_name,
        address: local_ip,
        listen_port,
        subnet: subnet.into(),
        dns: Some(cellmembrane_types::service::DEFAULT_HUB_MESH_IP.into()),
        peers,
    };

    let output = config.to_wg_quick();
    let data = serde_json::to_value(&config)?;

    Ok(ShadowOutcome::ok_with(output, data))
}
