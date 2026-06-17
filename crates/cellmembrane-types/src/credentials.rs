// SPDX-License-Identifier: AGPL-3.0-or-later

//! Credential management model.
//!
//! Represents the evolution path for membrane credential handling:
//! from age-encrypted files to BTSP vault to autonomous rotation.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Credential management strategy from `[membrane.credentials]` in `membrane.toml`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CredentialConfig {
    /// Active credential model.
    #[serde(default)]
    pub model: CredentialModel,

    /// Age recipient public keys for encrypted credential sharing.
    #[serde(default)]
    pub age_recipients: Vec<String>,

    /// Forward-compatible extension fields.
    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}

impl Default for CredentialConfig {
    fn default() -> Self {
        Self {
            model: CredentialModel::Age,
            age_recipients: Vec::new(),
            extra: BTreeMap::new(),
        }
    }
}

/// How credentials are stored and shared between operators and membranes.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialModel {
    /// Credentials encrypted with `age` using SSH ed25519 recipient keys.
    /// Current production model via `share_credentials.sh`.
    #[default]
    Age,

    /// Credentials stored in `BearDog`'s BTSP-encrypted secrets store.
    /// Mid-term target — requires Tower composition.
    BtspVault,

    /// Credentials managed manually by the operator.
    /// Fallback for minimal deployments.
    Manual,
}

impl std::fmt::Display for CredentialModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Age => write!(f, "age"),
            Self::BtspVault => write!(f, "btsp_vault"),
            Self::Manual => write!(f, "manual"),
        }
    }
}

/// A file on the membrane host that must have specific permissions.
///
/// Maps to MEM-08 (credential perms) and MEM-12 (`RustDesk` key) in
/// `darkforest_membrane.sh`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialFile {
    /// Resolved path on the membrane host.
    pub path: String,
    /// Expected octal mode (e.g. "600").
    pub expected_mode: &'static str,
    /// Expected file owner.
    pub expected_owner: &'static str,
    /// What this file contains.
    pub description: &'static str,
}

/// Runtime credential path resolver.
///
/// Derives credential file locations from a configurable base path,
/// eliminating hardcoded `/opt/membrane/` and `/etc/songbird/` assumptions.
#[derive(Debug, Clone)]
pub struct CredentialPaths {
    /// Base path for membrane credentials (default: `/opt/membrane`).
    pub membrane_base: String,
    /// Base path for songbird config (default: `/etc/songbird`).
    pub songbird_config: String,
}

impl CredentialPaths {
    /// Resolve from environment or use defaults.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            membrane_base: std::env::var(crate::service::ENV_INSTALL_BASE)
                .unwrap_or_else(|_| crate::service::DEFAULT_INSTALL_BASE.to_string()),
            songbird_config: std::env::var(crate::service::ENV_SONGBIRD_CONFIG)
                .unwrap_or_else(|_| crate::service::DEFAULT_RELAY_CONFIG_DIR.to_string()),
        }
    }
}

impl Default for CredentialPaths {
    fn default() -> Self {
        Self::from_env()
    }
}

/// Credential files required for a given composition (using default paths).
#[must_use]
pub fn credential_files_for(
    composition: crate::composition::MembraneComposition,
) -> Vec<CredentialFile> {
    credential_files_for_paths(composition, &CredentialPaths::from_env())
}

/// Credential files required for a given composition with configurable paths.
///
/// These are the files `darkforest_membrane.sh` MEM-08/MEM-12 audit.
#[must_use]
pub fn credential_files_for_paths(
    composition: crate::composition::MembraneComposition,
    paths: &CredentialPaths,
) -> Vec<CredentialFile> {
    use crate::composition::MembraneComposition;

    let mut files = vec![];

    let relay_binary =
        crate::service::MembraneService::binary_for(crate::service::ServiceCapability::TurnServer);
    files.push(CredentialFile {
        path: format!("{}/relay-credentials", paths.songbird_config),
        expected_mode: "600",
        expected_owner: "root",
        description: "TURN relay shared secret",
    });
    files.push(CredentialFile {
        path: format!("{}/{relay_binary}/turn-credentials", paths.membrane_base),
        expected_mode: "600",
        expected_owner: "root",
        description: "TURN relay credentials (legacy path)",
    });

    if composition >= MembraneComposition::RustDesk {
        files.push(CredentialFile {
            path: format!("{}/rustdesk/id_ed25519", paths.membrane_base),
            expected_mode: "600",
            expected_owner: "root",
            description: "RustDesk private key",
        });
        files.push(CredentialFile {
            path: format!("{}/rustdesk/id_ed25519.pub", paths.membrane_base),
            expected_mode: "644",
            expected_owner: "root",
            description: "RustDesk public key",
        });
    }

    if composition >= MembraneComposition::Tower {
        files.push(CredentialFile {
            path: format!("{}/tower.env", paths.membrane_base),
            expected_mode: "600",
            expected_owner: "root",
            description: "BTSP family seed and membrane identity",
        });
    }

    files
}
