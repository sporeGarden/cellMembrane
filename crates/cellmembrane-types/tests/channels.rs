// SPDX-License-Identifier: AGPL-3.0-or-later

use cellmembrane_types::channels::{CryptoLayer, MembraneChannel, TlsProvider};

#[test]
fn channel_trust_ordering() {
    assert!(MembraneChannel::Signal.trust_level() < MembraneChannel::Relay.trust_level());
    assert!(MembraneChannel::Relay.trust_level() < MembraneChannel::Surface.trust_level());
}

#[test]
fn channel_default_ports() {
    assert_eq!(MembraneChannel::Signal.default_ports(), &[53]);
    assert_eq!(MembraneChannel::Relay.default_ports(), &[3478]);
    assert_eq!(MembraneChannel::Surface.default_ports(), &[80, 443]);
}

#[test]
fn channel_crypto_layers() {
    assert_eq!(MembraneChannel::Signal.default_crypto(), CryptoLayer::None);
    assert_eq!(
        MembraneChannel::Relay.default_crypto(),
        CryptoLayer::TurnHmac
    );
    assert_eq!(MembraneChannel::Surface.default_crypto(), CryptoLayer::Tls);
}

#[test]
fn channel_serde_roundtrip() {
    #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
    struct Wrapper {
        c: MembraneChannel,
    }
    for ch in MembraneChannel::all() {
        let w = Wrapper { c: *ch };
        let serialized = toml::to_string(&w).unwrap();
        let deserialized: Wrapper = toml::from_str(&serialized).unwrap();
        assert_eq!(w, deserialized);
    }
}

#[test]
fn tls_provider_default_is_caddy_acme() {
    assert_eq!(TlsProvider::default(), TlsProvider::CaddyAcme);
}

#[test]
fn tls_provider_self_managed() {
    assert!(TlsProvider::CaddyAcme.is_self_managed());
    assert!(!TlsProvider::BearDog.is_self_managed());
}

#[test]
fn tls_provider_serde_roundtrip() {
    #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
    struct Wrapper {
        p: TlsProvider,
    }
    let caddy = Wrapper {
        p: TlsProvider::CaddyAcme,
    };
    let bear = Wrapper {
        p: TlsProvider::BearDog,
    };

    let s = toml::to_string(&caddy).unwrap();
    assert!(s.contains("caddy_acme"));
    let d: Wrapper = toml::from_str(&s).unwrap();
    assert_eq!(d, caddy);

    let s = toml::to_string(&bear).unwrap();
    assert!(s.contains("bear_dog"), "serialized BearDog as: {s}");
    let d: Wrapper = toml::from_str(&s).unwrap();
    assert_eq!(d, bear);
}

#[test]
fn tls_provider_display() {
    assert_eq!(TlsProvider::CaddyAcme.to_string(), "caddy_acme");
    assert_eq!(TlsProvider::BearDog.to_string(), "beardog");
}

#[test]
fn channel_config_with_tls_provider() {
    let toml_str = r#"
enabled = true
tls_domain = "membrane.primals.eco"
tls_provider = "bear_dog"
"#;
    let cfg: cellmembrane_types::ChannelConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(cfg.tls_provider, Some(TlsProvider::BearDog));
    assert_eq!(cfg.tls_domain.as_deref(), Some("membrane.primals.eco"));
}

#[test]
fn tls_provider_cert_dir() {
    assert_eq!(TlsProvider::default_cert_dir(), "/etc/membrane/tls");
}
