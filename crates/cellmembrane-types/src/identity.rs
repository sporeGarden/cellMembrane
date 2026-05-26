// SPDX-License-Identifier: AGPL-3.0-or-later

//! Membrane identity types.
//!
//! A membrane's identity is its persistent state across redeploys:
//! family ID, gate ID, domain, and host address.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Persistent membrane identity from `[membrane.identity]` in `membrane.toml`.
///
/// The family ID ties this membrane to its ecosystem. The gate ID distinguishes
/// it from other membranes in the same family.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MembraneIdentity {
    /// Family identifier — shared across all membranes and gates in one ecosystem.
    /// Maps to `FAMILY_ID` in `tower.env`.
    pub family_id: String,

    /// Unique gate identifier for this membrane instance.
    /// Auto-generated from hostname if not specified.
    #[serde(default)]
    pub gate_id: Option<String>,

    /// Forward-compatible extension fields.
    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}

impl MembraneIdentity {
    /// Gate ID, falling back to a default derived from the family ID.
    pub fn gate_id_or_default(&self) -> String {
        self.gate_id
            .clone()
            .unwrap_or_else(|| format!("{}-membrane", self.family_id))
    }
}
