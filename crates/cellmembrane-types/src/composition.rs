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
/// `relay < rustdesk < tower < nest < nucleus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MembraneComposition {
    /// Tier 1: Songbird TURN relay only.
    Relay,
    /// Tier 2: Relay + `RustDesk` remote desktop.
    #[serde(alias = "rust_desk")]
    RustDesk,
    /// Tier 3: `RustDesk` + `BearDog`/`SkunkBat` BTSP identity boundary.
    Tower,
    /// Tier 4: Tower + `NestGate` + provenance trio + Caddy TLS.
    Nest,
    /// Tier 5: Nest + compute (toadStool/`barraCuda`/`coralReef`) + meta (`biomeOS`/squirrel/`petalTongue`).
    ///
    /// Full 13-primal NUCLEUS runtime. Springs overlay onto this via `biomeOS` deploy.
    Nucleus,
}

impl MembraneComposition {
    /// Returns all composition variants in ladder order.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::Relay,
            Self::RustDesk,
            Self::Tower,
            Self::Nest,
            Self::Nucleus,
        ]
    }

    /// Whether this composition includes BTSP identity (Tower+).
    #[must_use]
    pub const fn has_btsp(&self) -> bool {
        matches!(self, Self::Tower | Self::Nest | Self::Nucleus)
    }

    /// Whether this composition requires a `tower.env` identity file.
    #[must_use]
    pub const fn requires_tower_env(&self) -> bool {
        self.has_btsp()
    }

    /// Whether this composition satisfies Dark Forest full compliance.
    #[must_use]
    pub const fn dark_forest_compliant(&self) -> bool {
        self.has_btsp()
    }

    /// Whether this composition includes the `biomeOS` Neural API orchestrator.
    #[must_use]
    pub const fn has_biomeos(&self) -> bool {
        matches!(self, Self::Nucleus)
    }

    /// Active channels for this composition.
    #[must_use]
    pub fn active_channels(&self) -> Vec<MembraneChannel> {
        match self {
            Self::Relay | Self::RustDesk | Self::Tower => {
                vec![MembraneChannel::Relay]
            }
            Self::Nest | Self::Nucleus => {
                vec![
                    MembraneChannel::Signal,
                    MembraneChannel::Relay,
                    MembraneChannel::Surface,
                ]
            }
        }
    }

    /// Returns the full specification for this composition, derived from the
    /// service registry. No duplication — the registry is the single source
    /// of truth for binaries, ports, units, and tier membership.
    #[must_use]
    pub fn spec(&self) -> CompositionSpec {
        CompositionSpec::from_registry(*self)
    }
}

impl fmt::Display for MembraneComposition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Relay => write!(f, "relay"),
            Self::RustDesk => write!(f, "rustdesk"),
            Self::Tower => write!(f, "tower"),
            Self::Nest => write!(f, "nest"),
            Self::Nucleus => write!(f, "nucleus"),
        }
    }
}

/// SSH port — always open regardless of composition.
pub const SSH_PORT: u16 = 22;

/// Full specification for a composition tier: which primals, symbiotic
/// partners, ports, and systemd units are required.
///
/// Derived entirely from the [`MembraneService`] registry — no duplication.
/// Each service declares its `min_composition`; the spec collects services
/// whose tier is at or below the requested composition.
#[derive(Debug, Clone)]
pub struct CompositionSpec {
    /// Composition this spec describes.
    pub composition: MembraneComposition,
    /// ecoPrimals binaries required.
    pub primals: Vec<&'static str>,
    /// Non-ecoPrimal binaries (`RustDesk`, Caddy, knot-dns).
    pub symbiotic: Vec<&'static str>,
    /// TCP ports that must be open in the firewall.
    pub tcp_ports: Vec<u16>,
    /// UDP ports that must be open in the firewall.
    pub udp_ports: Vec<u16>,
    /// Systemd unit names.
    pub systemd_units: Vec<&'static str>,
    /// Boot order: primals first, then symbiotic, in registry order.
    pub boot_order: Vec<&'static str>,
}

impl CompositionSpec {
    /// Build the spec for `composition` by querying the service registry.
    fn from_registry(composition: MembraneComposition) -> Self {
        let services = MembraneService::for_composition(composition);

        let mut primals = Vec::new();
        let mut symbiotic = Vec::new();
        let mut tcp_ports = vec![SSH_PORT];
        let mut udp_ports: Vec<u16> = Vec::new();
        let mut systemd_units = Vec::new();
        let mut boot_order = Vec::new();

        for svc in &services {
            if svc.is_primal {
                primals.push(svc.binary);
            } else {
                symbiotic.push(svc.binary);
            }
            systemd_units.push(svc.systemd_unit);
            boot_order.push(svc.binary);

            if let Some(port) = svc.port {
                match svc.protocol {
                    crate::service::Protocol::Tcp => tcp_ports.push(port),
                    crate::service::Protocol::Udp => udp_ports.push(port),
                    crate::service::Protocol::TcpAndUdp => {
                        tcp_ports.push(port);
                        udp_ports.push(port);
                    }
                    crate::service::Protocol::Uds => {}
                }
            }
            for &(port, proto, _) in svc.extra_ports {
                match proto {
                    crate::service::Protocol::Tcp => tcp_ports.push(port),
                    crate::service::Protocol::Udp => udp_ports.push(port),
                    crate::service::Protocol::TcpAndUdp => {
                        tcp_ports.push(port);
                        udp_ports.push(port);
                    }
                    crate::service::Protocol::Uds => {}
                }
            }
        }

        tcp_ports.sort_unstable();
        tcp_ports.dedup();
        udp_ports.sort_unstable();
        udp_ports.dedup();

        Self {
            composition,
            primals,
            symbiotic,
            tcp_ports,
            udp_ports,
            systemd_units,
            boot_order,
        }
    }

    /// All binaries required (primals + symbiotic), zero-allocation iterator.
    pub fn iter_binaries(&self) -> impl Iterator<Item = &str> {
        self.primals.iter().chain(self.symbiotic.iter()).copied()
    }

    /// All binaries required (primals + symbiotic) as a collected `Vec`.
    #[must_use]
    pub fn all_binaries(&self) -> Vec<&str> {
        self.iter_binaries().collect()
    }

    /// All listening ports (TCP + UDP deduplicated).
    #[must_use]
    pub fn all_ports(&self) -> Vec<u16> {
        let mut ports = self.tcp_ports.clone();
        for &p in &self.udp_ports {
            if !ports.contains(&p) {
                ports.push(p);
            }
        }
        ports.sort_unstable();
        ports
    }

    /// Lookup a service definition by binary name.
    #[must_use]
    pub fn service_for(&self, binary: &str) -> Option<&'static MembraneService> {
        MembraneService::for_binary(binary)
    }

    /// UDS socket paths for services in UDS-only mode (VPS standard).
    /// Returns `(binary, socket_path)` pairs for all primals that use UDS transport.
    #[must_use]
    pub fn uds_socket_paths(&self) -> Vec<(&'static str, &'static str)> {
        let services = MembraneService::for_composition(self.composition);
        services
            .iter()
            .filter(|s| s.is_uds_only())
            .filter_map(|s| s.socket_path.map(|path| (s.binary, path)))
            .collect()
    }

    /// TCP ports still required in UDS-only mode (symbiotic services + relay).
    /// These are services that must bind to TCP regardless of transport mode.
    #[must_use]
    pub fn tcp_ports_uds_mode(&self) -> Vec<u16> {
        let services = MembraneService::for_composition(self.composition);
        let mut ports = vec![SSH_PORT];

        for svc in &services {
            if svc.requires_tcp_in_uds_mode() {
                if let Some(port) = svc.port {
                    match svc.protocol {
                        crate::service::Protocol::Tcp | crate::service::Protocol::TcpAndUdp => {
                            ports.push(port);
                        }
                        _ => {}
                    }
                }
                for &(port, proto, _) in svc.extra_ports {
                    match proto {
                        crate::service::Protocol::Tcp | crate::service::Protocol::TcpAndUdp => {
                            ports.push(port);
                        }
                        _ => {}
                    }
                }
            }
        }

        ports.sort_unstable();
        ports.dedup();
        ports
    }
}
