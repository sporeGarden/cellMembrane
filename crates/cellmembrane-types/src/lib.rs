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

pub mod channels;
pub mod composition;
pub mod config;
pub mod credentials;
pub mod envelope;
pub mod error;
pub mod firewall;
pub mod identity;
pub mod provider;
pub mod service;
pub mod validation;

pub use channels::{ChannelConfig, CryptoLayer, MembraneChannel, TrustLevel};
pub use composition::{CompositionSpec, MembraneComposition};
pub use config::{DeployPaths, MembraneConfig, ShadowMode};
pub use credentials::{CredentialFile, CredentialModel, credential_files_for};
pub use envelope::{
    BondType, BoundaryPolicy, BraidPolicy, ChannelProtein, EnvelopeLayer, EnvelopeTopology,
};
pub use error::ConfigError;
pub use firewall::{FirewallRule, FirewallRuleset};
pub use identity::MembraneIdentity;
pub use provider::{ProviderConfig, SubstrateProfile};
pub use service::{BinaryIntegrity, HashAlgorithm, MembraneService, binary_integrity_for};
pub use validation::{Report, ReportEntry, Severity};

/// Shared serde default for boolean fields that should be `true` when omitted.
pub(crate) const fn default_true() -> bool {
    true
}
