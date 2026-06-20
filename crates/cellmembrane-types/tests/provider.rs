// SPDX-License-Identifier: AGPL-3.0-or-later

use cellmembrane_types::provider::{ProviderConfig, ProviderType, SubstrateProfile};

#[test]
fn provider_type_display_all() {
    assert_eq!(format!("{}", ProviderType::DigitalOcean), "digitalocean");
    assert_eq!(format!("{}", ProviderType::Hetzner), "hetzner");
    assert_eq!(format!("{}", ProviderType::BareMetal), "bare_metal");
    assert_eq!(format!("{}", ProviderType::GateLocal), "gate_local");
    assert_eq!(format!("{}", ProviderType::Custom), "custom");
}

#[test]
fn provider_ssh_defaults() {
    let cfg: ProviderConfig = toml::from_str(r#"type = "bare_metal""#).unwrap();
    assert_eq!(cfg.ssh_user_or_default(), "root");
    assert_eq!(cfg.ssh_port_or_default(), 22);
}

#[test]
fn provider_ssh_overrides() {
    let cfg: ProviderConfig = toml::from_str(
        r#"
        type = "bare_metal"
        ssh_user = "deploy"
        ssh_port = 2222
        "#,
    )
    .unwrap();
    assert_eq!(cfg.ssh_user_or_default(), "deploy");
    assert_eq!(cfg.ssh_port_or_default(), 2222);
}

#[test]
fn provider_requires_ssh() {
    let do_cfg: ProviderConfig = toml::from_str(r#"type = "digitalocean""#).unwrap();
    let gate_cfg: ProviderConfig = toml::from_str(r#"type = "gate_local""#).unwrap();
    let bare_cfg: ProviderConfig = toml::from_str(r#"type = "bare_metal""#).unwrap();
    assert!(do_cfg.requires_ssh());
    assert!(!gate_cfg.requires_ssh());
    assert!(bare_cfg.requires_ssh());
}

#[test]
fn provider_supports_provisioning() {
    let do_cfg: ProviderConfig = toml::from_str(r#"type = "digitalocean""#).unwrap();
    let hz_cfg: ProviderConfig = toml::from_str(r#"type = "hetzner""#).unwrap();
    let bare_cfg: ProviderConfig = toml::from_str(r#"type = "bare_metal""#).unwrap();
    let custom: ProviderConfig = toml::from_str(r#"type = "custom""#).unwrap();
    assert!(do_cfg.supports_provisioning());
    assert!(hz_cfg.supports_provisioning());
    assert!(!bare_cfg.supports_provisioning());
    assert!(!custom.supports_provisioning());
}

#[test]
fn substrate_profile_all_variants() {
    let do_cfg: ProviderConfig = toml::from_str(r#"type = "digitalocean""#).unwrap();
    let bare_cfg: ProviderConfig = toml::from_str(r#"type = "bare_metal""#).unwrap();
    let gate_cfg: ProviderConfig = toml::from_str(r#"type = "gate_local""#).unwrap();
    assert_eq!(do_cfg.substrate_profile(), SubstrateProfile::VpsFieldMouse);
    assert_eq!(
        bare_cfg.substrate_profile(),
        SubstrateProfile::RemoteCovalent
    );
    assert_eq!(gate_cfg.substrate_profile(), SubstrateProfile::GateLocal);
}

#[test]
fn substrate_profile_display() {
    assert_eq!(
        format!("{}", SubstrateProfile::VpsFieldMouse),
        "vps_fieldmouse"
    );
    assert_eq!(
        format!("{}", SubstrateProfile::RemoteCovalent),
        "remote_covalent"
    );
    assert_eq!(format!("{}", SubstrateProfile::GateLocal), "gate_local");
}

#[test]
fn substrate_biomeos_and_hardening() {
    assert!(!SubstrateProfile::VpsFieldMouse.has_biomeos());
    assert!(SubstrateProfile::VpsFieldMouse.requires_full_hardening());
    assert!(SubstrateProfile::GateLocal.has_biomeos());
    assert!(!SubstrateProfile::GateLocal.requires_full_hardening());
    assert!(!SubstrateProfile::RemoteCovalent.has_biomeos());
    assert!(!SubstrateProfile::RemoteCovalent.requires_full_hardening());
}

#[test]
fn provider_extra_fields() {
    let cfg: ProviderConfig = toml::from_str(
        r#"
        type = "digitalocean"
        region = "nyc1"
        size = "s-1vcpu-2gb"
        image = "debian-12-x64"
        custom_tag = "test"
        "#,
    )
    .unwrap();
    assert_eq!(cfg.region.as_deref(), Some("nyc1"));
    assert_eq!(cfg.size.as_deref(), Some("s-1vcpu-2gb"));
    assert_eq!(cfg.image.as_deref(), Some("debian-12-x64"));
    assert!(cfg.extra.contains_key("custom_tag"));
}
