// SPDX-License-Identifier: AGPL-3.0-or-later

//! Membrane composition model.
//!
//! Compositions form a monotonic ladder — each tier includes everything from
//! the tier below. See `specs/MEMBRANE_COMPOSITION_MODEL.md`.

use crate::channels::MembraneChannel;
use crate::service::MembraneService;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Membrane composition tier.
///
/// Each composition is a strict superset of the one below:
/// `relay < rustdesk < tower < nest`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MembraneComposition {
    /// Tier 1: Songbird TURN relay only.
    Relay,
    /// Tier 2: Relay + RustDesk remote desktop.
    #[serde(alias = "rust_desk")]
    RustDesk,
    /// Tier 3: RustDesk + BearDog/SkunkBat BTSP identity boundary.
    Tower,
    /// Tier 4: Tower + NestGate + provenance trio + Caddy TLS.
    Nest,
}

impl MembraneComposition {
    /// Returns all composition variants in ladder order.
    pub fn all() -> &'static [Self] {
        &[Self::Relay, Self::RustDesk, Self::Tower, Self::Nest]
    }

    /// Whether this composition includes BTSP identity (Tower+).
    pub fn has_btsp(&self) -> bool {
        matches!(self, Self::Tower | Self::Nest)
    }

    /// Whether this composition requires a `tower.env` identity file.
    pub fn requires_tower_env(&self) -> bool {
        self.has_btsp()
    }

    /// Whether this composition satisfies Dark Forest full compliance.
    pub fn dark_forest_compliant(&self) -> bool {
        self.has_btsp()
    }

    /// Active channels for this composition.
    pub fn active_channels(&self) -> Vec<MembraneChannel> {
        match self {
            Self::Relay | Self::RustDesk | Self::Tower => {
                vec![MembraneChannel::Relay]
            }
            Self::Nest => {
                vec![
                    MembraneChannel::Signal,
                    MembraneChannel::Relay,
                    MembraneChannel::Surface,
                ]
            }
        }
    }

    /// Returns the full specification for this composition.
    pub fn spec(&self) -> CompositionSpec {
        match self {
            Self::Relay => CompositionSpec::relay(),
            Self::RustDesk => CompositionSpec::rustdesk(),
            Self::Tower => CompositionSpec::tower(),
            Self::Nest => CompositionSpec::nest(),
        }
    }
}

impl fmt::Display for MembraneComposition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Relay => write!(f, "relay"),
            Self::RustDesk => write!(f, "rustdesk"),
            Self::Tower => write!(f, "tower"),
            Self::Nest => write!(f, "nest"),
        }
    }
}

/// Full specification for a composition tier: which primals, symbiotic
/// partners, ports, and systemd units are required.
#[derive(Debug, Clone)]
pub struct CompositionSpec {
    /// Composition this spec describes.
    pub composition: MembraneComposition,
    /// ecoPrimals binaries required.
    pub primals: Vec<&'static str>,
    /// Non-ecoPrimal binaries (RustDesk, Caddy, knot-dns).
    pub symbiotic: Vec<&'static str>,
    /// TCP ports that must be open in the firewall.
    pub tcp_ports: Vec<u16>,
    /// UDP ports that must be open in the firewall.
    pub udp_ports: Vec<u16>,
    /// Systemd unit names.
    pub systemd_units: Vec<&'static str>,
    /// Boot order: earlier entries must start before later ones.
    pub boot_order: Vec<&'static str>,
}

impl CompositionSpec {
    fn relay() -> Self {
        Self {
            composition: MembraneComposition::Relay,
            primals: vec!["songbird"],
            symbiotic: vec![],
            tcp_ports: vec![22, 3478],
            udp_ports: vec![3478],
            systemd_units: vec!["songbird-relay.service"],
            boot_order: vec!["songbird"],
        }
    }

    fn rustdesk() -> Self {
        Self {
            composition: MembraneComposition::RustDesk,
            primals: vec!["songbird"],
            symbiotic: vec!["hbbs", "hbbr"],
            tcp_ports: vec![22, 3478, 21115, 21116, 21117],
            udp_ports: vec![3478, 21116],
            systemd_units: vec![
                "songbird-relay.service",
                "hbbs-membrane.service",
                "hbbr-membrane.service",
            ],
            boot_order: vec!["songbird", "hbbs", "hbbr"],
        }
    }

    fn tower() -> Self {
        Self {
            composition: MembraneComposition::Tower,
            primals: vec!["beardog", "songbird", "skunkbat"],
            symbiotic: vec!["hbbs", "hbbr"],
            tcp_ports: vec![22, 3478, 21115, 21116, 21117],
            udp_ports: vec![3478, 21116],
            systemd_units: vec![
                "beardog-membrane.service",
                "songbird-relay.service",
                "skunkbat-membrane.service",
                "hbbs-membrane.service",
                "hbbr-membrane.service",
            ],
            boot_order: vec!["beardog", "songbird", "skunkbat", "hbbs", "hbbr"],
        }
    }

    fn nest() -> Self {
        Self {
            composition: MembraneComposition::Nest,
            primals: vec![
                "beardog",
                "songbird",
                "skunkbat",
                "nestgate",
                "rhizocrypt",
                "loamspine",
                "sweetgrass",
            ],
            symbiotic: vec!["hbbs", "hbbr", "caddy", "knot-dns"],
            tcp_ports: vec![
                22, 53, 80, 443, 3478, 8443, 9500, 9602, 9700, 9850, 21115, 21116,
                21117,
            ],
            udp_ports: vec![53, 3478, 21116],
            systemd_units: vec![
                "beardog-membrane.service",
                "songbird-relay.service",
                "skunkbat-membrane.service",
                "nestgate-membrane.service",
                "rhizocrypt-membrane.service",
                "loamspine-membrane.service",
                "sweetgrass-membrane.service",
                "hbbs-membrane.service",
                "hbbr-membrane.service",
                "caddy-tls.service",
                "knot.service",
            ],
            boot_order: vec![
                "beardog",
                "songbird",
                "skunkbat",
                "nestgate",
                "rhizocrypt",
                "loamspine",
                "sweetgrass",
                "hbbs",
                "hbbr",
                "caddy",
                "knot-dns",
            ],
        }
    }

    /// All binaries required (primals + symbiotic).
    pub fn all_binaries(&self) -> Vec<&str> {
        let mut bins = self.primals.to_vec();
        bins.extend_from_slice(&self.symbiotic);
        bins
    }

    /// All ports (TCP + UDP deduplicated).
    pub fn all_ports(&self) -> Vec<u16> {
        let mut ports = self.tcp_ports.clone();
        for p in &self.udp_ports {
            if !ports.contains(p) {
                ports.push(*p);
            }
        }
        ports.sort();
        ports
    }

    /// Lookup a service definition by binary name.
    pub fn service_for(&self, binary: &str) -> Option<&'static MembraneService> {
        MembraneService::for_binary(binary)
    }
}
