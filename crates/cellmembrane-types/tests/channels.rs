// SPDX-License-Identifier: AGPL-3.0-or-later

use cellmembrane_types::channels::{CryptoLayer, MembraneChannel};

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
