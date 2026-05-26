// SPDX-License-Identifier: AGPL-3.0-or-later

//! Typed domain models for cellMembrane sovereign infrastructure deployment.
//!
//! This crate defines the membrane architecture as Rust types: channels,
//! compositions, firewall rules, provider configs, and validation. It parses
//! `membrane.toml` configuration files and validates them against the
//! cellMembrane specification.
//!
//! No async runtime, no network I/O — pure data types, serde, and validation.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod channels;
pub mod composition;
pub mod config;
pub mod credentials;
pub mod firewall;
pub mod identity;
pub mod provider;
pub mod service;
pub mod validation;

pub use channels::{ChannelConfig, CryptoLayer, MembraneChannel, TrustLevel};
pub use composition::{CompositionSpec, MembraneComposition};
pub use config::MembraneConfig;
pub use credentials::{CredentialFile, CredentialModel, credential_files_for};
pub use firewall::{FirewallRule, FirewallRuleset};
pub use identity::MembraneIdentity;
pub use provider::{ProviderConfig, SubstrateProfile};
pub use service::{BinaryIntegrity, HashAlgorithm, MembraneService, binary_integrity_for};
pub use validation::{Report, ReportEntry, Severity};
