// SPDX-License-Identifier: AGPL-3.0-or-later

//! Sovereign DNS configuration — manifest-driven knot-dns zone generation.
//!
//! `dns.configure` generates knot-dns zone files and `knot.conf` from the
//! ecosystem manifest. `dns.apply` writes them to disk and reloads knot-dns.
//!
//! Records are derived from gate profiles (IPs, roles, domains) and the
//! surface domain constants — no manual zone editing required.

use crate::error::Result;
use crate::ShadowOutcome;
use cellmembrane_types::dns::{DnsRecord, DnsZone, KnotConfig};

/// Dispatch entry point for `dns.*` commands.
pub async fn dispatch(
    _config: &crate::ShadowConfig,
    command: &str,
    args: &[&str],
) -> Result<ShadowOutcome> {
    match command {
        "dns.configure" => dispatch_configure(args).await,
        "dns.apply" => dispatch_apply(args).await,
        _ => Ok(ShadowOutcome::ok(format!("unknown dns command: {command}"))),
    }
}

/// Generate DNS configuration from the ecosystem manifest (preview only).
///
/// Derives A records from gate WG IPs, CNAME/subdomain records from role
/// assignments, and SOA/NS records from the DNS primary gate.
async fn dispatch_configure(args: &[&str]) -> Result<ShadowOutcome> {
    let root = crate::temporal::resolve_workspace_root()?;
    let m = crate::manifest::load_from_workspace_async(&root).await?;

    let gate_name = crate::cli::extract_flag_value(args, "--gate")
        .map_or_else(crate::gate::resolve_local_gate_identity, str::to_string);

    let topo = m.topology.as_ref();

    let hub_ip = topo
        .and_then(|t| {
            m.gates
                .get(&t.inner_membrane)
                .and_then(|p| p.wg_ip.as_deref())
        })
        .unwrap_or(cellmembrane_types::service::DEFAULT_HUB_MESH_IP);

    let dnssec = resolve_dnssec_flag(&root);

    let public_zone = build_public_zone(&m, hub_ip);
    let mesh_zone = build_mesh_zone(&m);

    let config = KnotConfig {
        gate_name: gate_name.clone(),
        zones: vec![public_zone, mesh_zone],
        dnssec,
        listen: vec!["0.0.0.0@53".into(), "::@53".into()],
    };

    let mut preview = String::new();
    preview.push_str("# --- knot.conf ---\n");
    preview.push_str(&config.to_knot_conf());
    for zone in &config.zones {
        use std::fmt::Write;
        let _ = writeln!(preview, "\n# --- {}.zone ---", zone.origin);
        preview.push_str(&zone.to_zonefile());
    }

    Ok(ShadowOutcome::ok_with(
        format!(
            "dns.configure: {} zones, {} total records for gate {gate_name}",
            config.zones.len(),
            config.zones.iter().map(|z| z.records.len()).sum::<usize>(),
        ),
        serde_json::json!({
            "gate": gate_name,
            "zones": config.zones.iter().map(|z| &z.origin).collect::<Vec<_>>(),
            "record_count": config.zones.iter().map(|z| z.records.len()).sum::<usize>(),
            "dnssec": dnssec,
            "preview": preview,
        }),
    ))
}

/// Write DNS configuration to disk and reload knot-dns.
async fn dispatch_apply(args: &[&str]) -> Result<ShadowOutcome> {
    let dry_run = args.contains(&"--dry-run");

    let root = crate::temporal::resolve_workspace_root()?;
    let m = crate::manifest::load_from_workspace_async(&root).await?;

    let gate_name = crate::cli::extract_flag_value(args, "--gate")
        .map_or_else(crate::gate::resolve_local_gate_identity, str::to_string);

    let topo = m.topology.as_ref();
    let hub_ip = topo
        .and_then(|t| {
            m.gates
                .get(&t.inner_membrane)
                .and_then(|p| p.wg_ip.as_deref())
        })
        .unwrap_or(cellmembrane_types::service::DEFAULT_HUB_MESH_IP);

    let dnssec = resolve_dnssec_flag(&root);

    let public_zone = build_public_zone(&m, hub_ip);
    let mesh_zone = build_mesh_zone(&m);

    let config = KnotConfig {
        gate_name: gate_name.clone(),
        zones: vec![public_zone, mesh_zone],
        dnssec,
        listen: vec!["0.0.0.0@53".into(), "::@53".into()],
    };

    if dry_run {
        return Ok(ShadowOutcome::ok(format!(
            "dns.apply dry-run: would write {} zones for gate {gate_name}",
            config.zones.len()
        )));
    }

    let conf_path = std::path::Path::new(KNOT_CONF_PATH);
    let zone_dir = std::path::Path::new(KNOT_ZONE_DIR);

    if let Err(e) = std::fs::create_dir_all(zone_dir) {
        return Ok(ShadowOutcome::fail(format!(
            "failed to create zone directory {}: {e}",
            zone_dir.display()
        )));
    }

    if let Err(e) = std::fs::write(conf_path, config.to_knot_conf()) {
        return Ok(ShadowOutcome::fail(format!(
            "failed to write {}: {e}",
            conf_path.display()
        )));
    }

    let mut written = 0u32;
    for zone in &config.zones {
        let zone_path = zone_dir.join(format!("{}.zone", zone.origin));
        if let Err(e) = std::fs::write(&zone_path, zone.to_zonefile()) {
            return Ok(ShadowOutcome::fail(format!(
                "failed to write {}: {e}",
                zone_path.display()
            )));
        }
        written += 1;
    }

    let reload_ok = reload_knot();

    Ok(ShadowOutcome::ok(format!(
        "dns.apply: wrote {written} zone files + knot.conf for gate {gate_name}{}",
        if reload_ok {
            " — knot reloaded"
        } else {
            " — knot reload failed (manual reload needed)"
        }
    )))
}

// ── Constants ──────────────────────────────────────────────────────

const KNOT_CONF_PATH: &str = cellmembrane_types::service::DEFAULT_KNOT_CONF_PATH;
const KNOT_ZONE_DIR: &str = cellmembrane_types::service::DEFAULT_KNOT_ZONE_DIR;

// ── Zone builders ──────────────────────────────────────────────────

/// Build the public zone (e.g. `primals.eco`) from manifest gates and roles.
fn build_public_zone(
    m: &crate::manifest::EcosystemManifest,
    hub_ip: &str,
) -> DnsZone {
    use cellmembrane_types::service;

    let origin = service::SURFACE_DOMAIN;
    let ns_fqdn = format!("ns1.{origin}.");
    let admin_fqdn = format!("admin.{origin}.");

    let serial = generate_serial();

    let mut records = Vec::new();

    records.push(DnsRecord {
        name: "@".into(),
        ttl: 0,
        rtype: "NS".into(),
        rdata: ns_fqdn.clone(),
    });

    records.push(DnsRecord {
        name: "ns1".into(),
        ttl: 0,
        rtype: "A".into(),
        rdata: hub_ip.into(),
    });

    records.push(DnsRecord {
        name: "@".into(),
        ttl: 0,
        rtype: "A".into(),
        rdata: hub_ip.into(),
    });

    let subdomain_roles: &[(&str, &str)] = &[
        ("forgejo", "git"),
        ("depot", "depot"),
        ("relay", "mesh"),
        ("caddy_tls", "sporeprint"),
        ("footprint", "footprint"),
        ("tideglass", "tideglass"),
        ("esotericwebb", "webb"),
        ("lab", "lab"),
    ];

    for &(role, subdomain) in subdomain_roles {
        let gates = m.gates_for_role(role);
        if let Some((_name, profile)) = gates.first() {
            let ip = profile
                .wg_ip
                .as_deref()
                .or(profile.host.as_deref())
                .unwrap_or(hub_ip);
            records.push(DnsRecord {
                name: subdomain.into(),
                ttl: 0,
                rtype: "A".into(),
                rdata: ip.into(),
            });
        }
    }

    if let Some(topo) = m.topology.as_ref() {
        if let Some(ca_host) = topo.hosts.get("step-ca") {
            records.push(DnsRecord {
                name: "ca".into(),
                ttl: 0,
                rtype: "A".into(),
                rdata: ca_host.clone(),
            });
        }
    }

    DnsZone {
        origin: origin.into(),
        soa_mname: ns_fqdn,
        soa_rname: admin_fqdn,
        soa_serial: serial,
        records,
        ..Default::default()
    }
}

/// Build the mesh zone (`primals.local`) from gate WG IPs.
fn build_mesh_zone(
    m: &crate::manifest::EcosystemManifest,
) -> DnsZone {
    use cellmembrane_types::service;

    let origin = service::LAN_DNS_DOMAIN;
    let ns_fqdn = format!("ns1.{origin}.");
    let admin_fqdn = format!("admin.{origin}.");

    let serial = generate_serial();

    let mut records = Vec::new();

    records.push(DnsRecord {
        name: "@".into(),
        ttl: 0,
        rtype: "NS".into(),
        rdata: ns_fqdn.clone(),
    });

    let hub_ip = m
        .topology
        .as_ref()
        .and_then(|t| {
            m.gates
                .get(&t.inner_membrane)
                .and_then(|p| p.wg_ip.as_deref())
        })
        .unwrap_or(service::DEFAULT_HUB_MESH_IP);

    records.push(DnsRecord {
        name: "ns1".into(),
        ttl: 0,
        rtype: "A".into(),
        rdata: hub_ip.into(),
    });

    let mut gate_entries: Vec<(&String, &crate::manifest::GateProfile)> =
        m.gates.iter().collect();
    gate_entries.sort_by_key(|(name, _)| name.to_lowercase());

    for (gate_name, profile) in gate_entries {
        if let Some(wg_ip) = &profile.wg_ip {
            records.push(DnsRecord {
                name: gate_name.to_lowercase(),
                ttl: 0,
                rtype: "A".into(),
                rdata: wg_ip.clone(),
            });
        }
    }

    DnsZone {
        origin: origin.into(),
        soa_mname: ns_fqdn,
        soa_rname: admin_fqdn,
        soa_serial: serial,
        records,
        ..Default::default()
    }
}

// ── Helpers ────────────────────────────────────────────────────────

/// Generate a SOA serial in YYYYMMDD00 format from current date.
fn generate_serial() -> u32 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let days = now / 86400;
    let year = 1970 + (days / 365);
    let day_of_year = days % 365;
    let month = day_of_year / 30 + 1;
    let day = day_of_year % 30 + 1;

    #[allow(clippy::cast_possible_truncation)]
    let serial = (year * 1_000_000 + month * 10_000 + day * 100) as u32;
    serial
}

/// Read the DNSSEC flag from the local `membrane.toml` channel config.
fn resolve_dnssec_flag(workspace_root: &std::path::Path) -> bool {
    let config_path = workspace_root.join("membrane.toml");
    let Ok(content) = std::fs::read_to_string(&config_path) else {
        return true;
    };
    let Ok(config) = toml::from_str::<cellmembrane_types::config::MembraneConfigFile>(&content)
    else {
        return true;
    };
    config
        .membrane
        .channels
        .signal
        .as_ref()
        .and_then(|s| s.dnssec)
        .unwrap_or(true)
}

/// Reload knot-dns via `knotc reload` or `systemctl reload knot`.
fn reload_knot() -> bool {
    if !matches!(
        cellmembrane_types::InitSystem::detect(),
        cellmembrane_types::InitSystem::Systemd
    ) {
        return false;
    }

    std::process::Command::new("knotc")
        .arg("reload")
        .output()
        .is_ok_and(|o| o.status.success())
        || std::process::Command::new("systemctl")
            .args(["reload", "knot"])
            .output()
            .is_ok_and(|o| o.status.success())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::EcosystemManifest;

    fn empty_manifest() -> EcosystemManifest {
        toml::from_str(
            r#"
            [meta]
            version = "1.0"
            wave = 155

            [sync]
            "#,
        )
        .expect("test manifest should parse")
    }

    fn manifest_with_gate(name: &str, wg_ip: &str) -> EcosystemManifest {
        let toml_str = format!(
            r#"
            [meta]
            version = "1.0"
            wave = 155

            [sync]

            [gates.{name}]
            wg_ip = "{wg_ip}"
            "#,
        );
        toml::from_str(&toml_str).expect("test manifest should parse")
    }

    #[test]
    fn generate_serial_is_reasonable() {
        let serial = generate_serial();
        assert!(serial >= 2_026_010_100, "serial should be >= 2026: {serial}");
        assert!(serial < 2_100_123_199, "serial should be < 2100: {serial}");
    }

    #[test]
    fn resolve_dnssec_flag_defaults_true() {
        let result = resolve_dnssec_flag(std::path::Path::new("/nonexistent"));
        assert!(result, "should default to true when config missing");
    }

    #[test]
    fn build_public_zone_has_essential_records() {
        let m = empty_manifest();
        let zone = build_public_zone(&m, "10.13.37.1");
        assert_eq!(zone.origin, cellmembrane_types::service::SURFACE_DOMAIN);
        assert!(
            zone.records.iter().any(|r| r.rtype == "NS"),
            "public zone must have NS record"
        );
        assert!(
            zone.records.iter().any(|r| r.name == "ns1" && r.rtype == "A"),
            "public zone must have ns1 A record"
        );
        assert!(
            zone.records.iter().any(|r| r.name == "@" && r.rtype == "A"),
            "public zone must have apex A record"
        );
    }

    #[test]
    fn build_mesh_zone_has_ns() {
        let m = empty_manifest();
        let zone = build_mesh_zone(&m);
        assert_eq!(zone.origin, cellmembrane_types::service::LAN_DNS_DOMAIN);
        assert!(
            zone.records.iter().any(|r| r.rtype == "NS"),
            "mesh zone must have NS record"
        );
    }

    #[test]
    fn build_mesh_zone_includes_gates_with_wg_ip() {
        let m = manifest_with_gate("eastGate", "10.13.37.5");

        let zone = build_mesh_zone(&m);
        assert!(
            zone.records
                .iter()
                .any(|r| r.name == "eastgate" && r.rdata == "10.13.37.5"),
            "mesh zone should include gate with WG IP"
        );
    }

    #[test]
    fn build_mesh_zone_skips_gates_without_wg_ip() {
        let m: EcosystemManifest = toml::from_str(
            r#"
            [meta]
            version = "1.0"
            wave = 155
            [sync]
            [gates.noIpGate]
            "#,
        )
        .unwrap();

        let zone = build_mesh_zone(&m);
        assert!(
            !zone.records.iter().any(|r| r.name == "noipgate"),
            "mesh zone should not include gate without WG IP"
        );
    }

    #[test]
    fn build_mesh_zone_sorts_gates_alphabetically() {
        let m: EcosystemManifest = toml::from_str(
            r#"
            [meta]
            version = "1.0"
            wave = 155
            [sync]
            [gates.zGate]
            wg_ip = "10.13.37.9"
            [gates.aGate]
            wg_ip = "10.13.37.2"
            "#,
        )
        .unwrap();

        let zone = build_mesh_zone(&m);
        let gate_records: Vec<&str> = zone
            .records
            .iter()
            .filter(|r| r.name != "@" && r.name != "ns1")
            .map(|r| r.name.as_str())
            .collect();
        assert_eq!(gate_records, vec!["agate", "zgate"]);
    }

    #[test]
    fn build_public_zone_has_hub_apex() {
        let m = empty_manifest();
        let zone = build_public_zone(&m, "10.13.37.1");
        assert!(
            zone.records
                .iter()
                .any(|r| r.name == "@" && r.rtype == "A" && r.rdata == "10.13.37.1"),
            "public zone apex A should point to hub IP"
        );
    }
}
