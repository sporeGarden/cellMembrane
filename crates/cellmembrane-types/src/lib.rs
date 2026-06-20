// SPDX-License-Identifier: AGPL-3.0-or-later

//! Typed domain models for cellMembrane sovereign infrastructure deployment.
//!
//! This crate defines the membrane architecture as Rust types: channels,
//! compositions, firewall rules, provider configs, and validation. It parses
//! `membrane.toml` configuration files and validates them against the
//! cellMembrane specification.
//!
//! No async runtime, no network I/O — pure data types, serde, and validation.
//!
//! # Quick Start
//!
//! ```
//! use cellmembrane_types::{MembraneComposition, FirewallRuleset};
//!
//! // Derive firewall rules from composition tier
//! let fw = FirewallRuleset::for_composition(MembraneComposition::Nest);
//! assert!(fw.ports().contains(&443));
//! assert!(fw.ports().contains(&22));
//! ```
//!
//! ```
//! use cellmembrane_types::MembraneComposition;
//!
//! // Composition specs are derived from the static service registry
//! let spec = MembraneComposition::Tower.spec();
//! assert!(spec.primals.contains(&"beardog"));
//! assert!(spec.primals.contains(&"songbird"));
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod caddy;
pub mod channels;
pub mod composition;
pub mod config;
pub mod credentials;
pub mod cytoplasm;
pub mod envelope;
pub mod error;
pub mod firewall;
pub mod identity;
pub mod provider;
pub mod service;
pub mod signal;
pub mod sync;
pub mod topology;
pub mod transport;
pub mod validation;
pub mod wireguard;

pub use caddy::{CaddyConfig, CaddyVhost};
pub use channels::{ChannelConfig, CryptoLayer, MembraneChannel, TlsProvider, TrustLevel};
pub use composition::{CompositionSpec, MembraneComposition};
pub use config::{DeployPaths, MembraneConfig, ShadowMode};
pub use credentials::{CredentialFile, CredentialModel, credential_files_for};
pub use cytoplasm::{ZoneLabel, mesh_address, mesh_address_from_topology};
pub use envelope::{
    BondType, BoundaryPolicy, BraidPolicy, ChannelProtein, EnvelopeLayer, EnvelopeTopology,
};
pub use error::ConfigError;
pub use firewall::{FirewallRule, FirewallRuleset, NftablesConfig};
pub use identity::{GateMobility, MembraneIdentity};
pub use provider::{ProviderConfig, SubstrateProfile};
pub use service::{
    BinaryIntegrity, HashAlgorithm, MembraneService, ServerContract, ServiceCapability,
    binary_integrity_for,
};
pub use sync::{DivergencePolicy, GateTransport, PushTarget};
pub use topology::{
    AffinityTable, BackboneLink, NetworkSegment, PhysicalZone, ResolvedTopology, TopologyMap,
    TopologyMeta, ZoneStatus,
};
pub use transport::{ENV_TRANSPORT_ENDPOINT, TransportEndpoint};
pub use validation::{Report, ReportEntry, Severity};
pub use wireguard::{WgConfig, WgPeer};

/// Shared serde default for boolean fields that should be `true` when omitted.
pub(crate) const fn default_true() -> bool {
    true
}
