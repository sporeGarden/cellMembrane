// SPDX-License-Identifier: AGPL-3.0-or-later

//! Gate identity resolution — capability-based, no hostname heuristics.
//!
//! Resolution priority:
//! 1. `GATE_NAME` environment variable
//! 2. `$ECOPRIMALS_ROOT/.gate` file (one line, trimmed)
//! 3. Error — the gate must declare itself, not be guessed

use crate::error::{Result, ShadowError};
use std::path::Path;

/// Detected gate identity.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GateIdentity {
    /// Gate name (e.g. `eastGate`, `golgiBody`).
    pub name: String,
    /// How the identity was resolved.
    pub source: IdentitySource,
}

/// How the gate identity was determined.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentitySource {
    /// From `GATE_NAME` environment variable.
    Environment,
    /// From `.gate` file in workspace root.
    GateFile,
}

/// Resolve the gate identity for a workspace.
///
/// # Errors
/// Returns `ShadowError::Parse` if no identity can be resolved — the gate
/// must declare itself via `GATE_NAME` or a `.gate` file.
pub fn resolve(workspace_root: &Path) -> Result<GateIdentity> {
    if let Ok(name) = std::env::var("GATE_NAME") {
        let name = name.trim().to_string();
        if !name.is_empty() {
            return Ok(GateIdentity {
                name,
                source: IdentitySource::Environment,
            });
        }
    }

    let gate_file = workspace_root.join(".gate");
    if gate_file.exists() {
        let contents = std::fs::read_to_string(&gate_file)
            .map_err(ShadowError::Io)?;
        let name = contents.trim().to_string();
        if !name.is_empty() {
            return Ok(GateIdentity {
                name,
                source: IdentitySource::GateFile,
            });
        }
    }

    Err(ShadowError::Parse(
        "cannot resolve gate identity — set GATE_NAME or create .gate file".into(),
    ))
}
