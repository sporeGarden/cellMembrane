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

    /// SSH certificates issued by a sovereign step-ca instance.
    /// Short-lived certs replace static `authorized_keys`.
    StepCa,

    /// Credentials managed manually by the operator.
    /// Fallback for minimal deployments.
    Manual,
}

impl std::fmt::Display for CredentialModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Age => write!(f, "age"),
            Self::BtspVault => write!(f, "btsp_vault"),
            Self::StepCa => write!(f, "step_ca"),
            Self::Manual => write!(f, "manual"),
        }
    }
}

/// An SSH certificate issued by a step-ca certificate authority.
///
/// Represents a short-lived SSH user or host certificate. These replace
/// static `authorized_keys` entries with cryptographically verifiable,
/// time-bounded access tokens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshCertificate {
    /// Certificate type: "user" or "host".
    pub cert_type: SshCertType,
    /// Principal(s) the certificate is valid for (e.g. "root", gate name).
    pub principals: Vec<String>,
    /// Serial number assigned by the CA.
    pub serial: u64,
    /// Path to the certificate file on disk.
    pub cert_path: String,
    /// Path to the corresponding private key.
    pub key_path: String,
    /// Unix timestamp when the certificate expires.
    pub valid_before: u64,
    /// CA fingerprint (SHA256) for verification.
    pub ca_fingerprint: String,
}

/// SSH certificate type — user or host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SshCertType {
    /// User certificate — authenticates a person/gate to a host.
    User,
    /// Host certificate — authenticates a host to connecting clients.
    Host,
}

impl std::fmt::Display for SshCertType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::User => write!(f, "user"),
            Self::Host => write!(f, "host"),
        }
    }
}

impl SshCertificate {
    /// Check whether this certificate has expired.
    #[must_use]
    pub fn is_expired(&self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        now >= self.valid_before
    }

    /// Seconds remaining until expiry, or 0 if already expired.
    #[must_use]
    pub fn seconds_remaining(&self) -> u64 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        self.valid_before.saturating_sub(now)
    }

    /// Whether the certificate should be renewed (less than 25% lifetime remaining).
    #[must_use]
    pub fn needs_renewal(&self, lifetime_secs: u64) -> bool {
        self.seconds_remaining() < lifetime_secs / 4
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::composition::MembraneComposition;

    #[test]
    fn credential_model_display() {
        assert_eq!(CredentialModel::Age.to_string(), "age");
        assert_eq!(CredentialModel::BtspVault.to_string(), "btsp_vault");
        assert_eq!(CredentialModel::StepCa.to_string(), "step_ca");
        assert_eq!(CredentialModel::Manual.to_string(), "manual");
    }

    #[test]
    fn ssh_cert_type_display() {
        assert_eq!(SshCertType::User.to_string(), "user");
        assert_eq!(SshCertType::Host.to_string(), "host");
    }

    #[test]
    fn ssh_certificate_expired() {
        let cert = SshCertificate {
            cert_type: SshCertType::User,
            principals: vec!["root".into()],
            serial: 1,
            cert_path: "/tmp/test-cert.pub".into(),
            key_path: "/tmp/test-key".into(),
            valid_before: 0,
            ca_fingerprint: "SHA256:test".into(),
        };
        assert!(cert.is_expired());
        assert_eq!(cert.seconds_remaining(), 0);
    }

    #[test]
    fn ssh_certificate_not_expired() {
        let far_future = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 86400;
        let cert = SshCertificate {
            cert_type: SshCertType::Host,
            principals: vec!["golgi.primals.eco".into()],
            serial: 42,
            cert_path: "/etc/ssh/host-cert.pub".into(),
            key_path: "/etc/ssh/host-key".into(),
            valid_before: far_future,
            ca_fingerprint: "SHA256:abc".into(),
        };
        assert!(!cert.is_expired());
        assert!(cert.seconds_remaining() > 0);
    }

    #[test]
    fn ssh_certificate_needs_renewal() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let cert = SshCertificate {
            cert_type: SshCertType::User,
            principals: vec!["sporegate".into()],
            serial: 100,
            cert_path: "/tmp/cert.pub".into(),
            key_path: "/tmp/key".into(),
            valid_before: now + 600,
            ca_fingerprint: "SHA256:xyz".into(),
        };
        assert!(
            cert.needs_renewal(28800),
            "10min left of 8h → needs renewal"
        );
        assert!(
            !cert.needs_renewal(1200),
            "10min left of 20min → no renewal"
        );
    }

    #[test]
    fn credential_config_default_is_age() {
        let config = CredentialConfig::default();
        assert_eq!(config.model, CredentialModel::Age);
        assert!(config.age_recipients.is_empty());
    }

    #[test]
    fn relay_composition_has_relay_credentials() {
        let paths = CredentialPaths {
            membrane_base: "/test/membrane".into(),
            songbird_config: "/test/songbird".into(),
        };
        let files = credential_files_for_paths(MembraneComposition::Relay, &paths);
        assert!(
            files.iter().any(|f| f.path.contains("relay-credentials")),
            "Relay must include relay credentials"
        );
    }

    #[test]
    fn rustdesk_composition_adds_keys() {
        let paths = CredentialPaths {
            membrane_base: "/test/membrane".into(),
            songbird_config: "/test/songbird".into(),
        };
        let files = credential_files_for_paths(MembraneComposition::RustDesk, &paths);
        assert!(
            files.iter().any(|f| f.path.contains("id_ed25519")),
            "RustDesk must include private key"
        );
        assert!(
            files.iter().any(|f| f.path.contains("id_ed25519.pub")),
            "RustDesk must include public key"
        );
    }

    #[test]
    fn tower_composition_adds_tower_env() {
        let paths = CredentialPaths {
            membrane_base: "/test/membrane".into(),
            songbird_config: "/test/songbird".into(),
        };
        let files = credential_files_for_paths(MembraneComposition::Tower, &paths);
        assert!(
            files.iter().any(|f| f.path.contains("tower.env")),
            "Tower must include tower.env"
        );
    }

    #[test]
    fn credential_file_permissions_are_restrictive() {
        let paths = CredentialPaths {
            membrane_base: "/test".into(),
            songbird_config: "/test".into(),
        };
        let files = credential_files_for_paths(MembraneComposition::Tower, &paths);
        for file in &files {
            assert!(
                file.expected_mode == "600" || file.expected_mode == "644",
                "credential file {} has unexpected mode: {}",
                file.path,
                file.expected_mode
            );
        }
    }

    #[test]
    fn higher_composition_has_more_credentials() {
        let paths = CredentialPaths {
            membrane_base: "/test".into(),
            songbird_config: "/test".into(),
        };
        let relay = credential_files_for_paths(MembraneComposition::Relay, &paths);
        let tower = credential_files_for_paths(MembraneComposition::Tower, &paths);
        assert!(
            tower.len() > relay.len(),
            "Tower ({}) should have more cred files than Relay ({})",
            tower.len(),
            relay.len()
        );
    }

    #[test]
    fn paths_use_configured_base() {
        let paths = CredentialPaths {
            membrane_base: "/custom/base".into(),
            songbird_config: "/custom/songbird".into(),
        };
        let files = credential_files_for_paths(MembraneComposition::Relay, &paths);
        assert!(
            files.iter().any(|f| f.path.starts_with("/custom/")),
            "paths should use the configured base"
        );
    }
}
