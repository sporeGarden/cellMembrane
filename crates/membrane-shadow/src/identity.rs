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

impl std::fmt::Display for GateIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.name, self.source)
    }
}

impl std::fmt::Display for IdentitySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Environment => f.write_str("env"),
            Self::GateFile => f.write_str(".gate file"),
        }
    }
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
        let contents = std::fs::read_to_string(&gate_file).map_err(ShadowError::Io)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn resolve_from_gate_file() {
        let dir = std::env::temp_dir().join("membrane-test-identity-gate");
        std::fs::create_dir_all(&dir).unwrap();
        let gate_file = dir.join(".gate");
        let mut f = std::fs::File::create(&gate_file).unwrap();
        writeln!(f, "eastGate").unwrap();
        drop(f);

        // Only test file path when GATE_NAME is not already set
        if std::env::var("GATE_NAME").is_err() {
            let result = resolve(&dir);
            let identity = result.unwrap();
            assert_eq!(identity.name, "eastGate");
            assert!(matches!(identity.source, IdentitySource::GateFile));
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_fails_without_identity() {
        if std::env::var("GATE_NAME").is_err() {
            let result = resolve(Path::new("/tmp/nonexistent-gate-identity-test"));
            assert!(result.is_err());
        }
    }

    #[test]
    fn identity_display_format() {
        let id = GateIdentity {
            name: "ironGate".into(),
            source: IdentitySource::Environment,
        };
        assert_eq!(format!("{id}"), "ironGate (env)");

        let id2 = GateIdentity {
            name: "eastGate".into(),
            source: IdentitySource::GateFile,
        };
        assert_eq!(format!("{id2}"), "eastGate (.gate file)");
    }

    #[test]
    fn identity_source_display() {
        assert_eq!(format!("{}", IdentitySource::Environment), "env");
        assert_eq!(format!("{}", IdentitySource::GateFile), ".gate file");
    }
}
