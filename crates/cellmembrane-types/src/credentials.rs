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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialModel {
    /// Credentials encrypted with `age` using SSH ed25519 recipient keys.
    /// Current production model via `share_credentials.sh`.
    Age,

    /// Credentials stored in BearDog's BTSP-encrypted secrets store.
    /// Mid-term target — requires Tower composition.
    BtspVault,

    /// Credentials managed manually by the operator.
    /// Fallback for minimal deployments.
    Manual,
}

impl Default for CredentialModel {
    fn default() -> Self {
        Self::Age
    }
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
/// Maps to MEM-08 (credential perms) and MEM-12 (RustDesk key) in
/// `darkforest_membrane.sh`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialFile {
    /// Absolute path on the membrane host.
    pub path: &'static str,
    /// Expected octal mode (e.g. "600").
    pub expected_mode: &'static str,
    /// Expected file owner.
    pub expected_owner: &'static str,
    /// What this file contains.
    pub description: &'static str,
}

/// Credential files required for a given composition.
///
/// These are the files `darkforest_membrane.sh` MEM-08/MEM-12 audit.
pub fn credential_files_for(
    composition: crate::composition::MembraneComposition,
) -> Vec<CredentialFile> {
    use crate::composition::MembraneComposition;

    let mut files = vec![];

    // TURN credentials — all compositions
    files.push(CredentialFile {
        path: "/etc/songbird/relay-credentials",
        expected_mode: "600",
        expected_owner: "root",
        description: "Songbird TURN shared secret",
    });
    // Legacy path checked by darkforest_membrane.sh
    files.push(CredentialFile {
        path: "/opt/membrane/songbird/turn-credentials",
        expected_mode: "600",
        expected_owner: "root",
        description: "Songbird TURN credentials (legacy path)",
    });

    // RustDesk key — RustDesk+ compositions
    if composition >= MembraneComposition::RustDesk {
        files.push(CredentialFile {
            path: "/opt/membrane/rustdesk/id_ed25519",
            expected_mode: "600",
            expected_owner: "root",
            description: "RustDesk private key",
        });
        files.push(CredentialFile {
            path: "/opt/membrane/rustdesk/id_ed25519.pub",
            expected_mode: "644",
            expected_owner: "root",
            description: "RustDesk public key",
        });
    }

    // tower.env — Tower+ compositions
    if composition >= MembraneComposition::Tower {
        files.push(CredentialFile {
            path: "/opt/membrane/tower.env",
            expected_mode: "600",
            expected_owner: "root",
            description: "BTSP family seed and membrane identity",
        });
    }

    files
}
