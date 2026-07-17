// SPDX-License-Identifier: AGPL-3.0-or-later

//! `WireGuard` key management and config generation for gate enrollment.
//!
//! Pure WG concerns extracted from `enroll.rs` to keep both files under
//! the 800-line threshold and make these helpers reusable (e.g. key rotation,
//! hub-side config generation).

use super::bootstrap::BootstrapPhase;

/// Generate a `WireGuard` keypair. Returns the public key in the phase detail.
pub(super) async fn wg_keygen_phase(dry_run: bool) -> BootstrapPhase {
    if dry_run {
        return BootstrapPhase {
            name: "wg.keygen".into(),
            ok: true,
            detail: "dry-run: would generate WireGuard keypair".into(),
        };
    }

    let existing = wg_private_key_path();
    if existing.exists() {
        return match tokio::fs::read_to_string(&existing).await {
            Ok(key) => {
                let pubkey = derive_wg_pubkey(key.trim()).await;
                BootstrapPhase {
                    name: "wg.keygen".into(),
                    ok: true,
                    detail: format!(
                        "existing keypair at {} (pub: {})",
                        existing.display(),
                        pubkey.as_deref().unwrap_or("derive failed")
                    ),
                }
            }
            Err(e) => BootstrapPhase {
                name: "wg.keygen".into(),
                ok: false,
                detail: format!("cannot read {}: {e}", existing.display()),
            },
        };
    }

    let genkey = tokio::process::Command::new("wg")
        .arg("genkey")
        .output()
        .await;

    let Ok(output) = genkey else {
        return BootstrapPhase {
            name: "wg.keygen".into(),
            ok: false,
            detail: "`wg genkey` failed — is wireguard-tools installed?".into(),
        };
    };

    if !output.status.success() {
        return BootstrapPhase {
            name: "wg.keygen".into(),
            ok: false,
            detail: format!(
                "wg genkey exit {}: {}",
                output.status.code().unwrap_or(-1),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        };
    }

    let private_key = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if let Some(parent) = existing.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    if let Err(e) = tokio::fs::write(&existing, &private_key).await {
        return BootstrapPhase {
            name: "wg.keygen".into(),
            ok: false,
            detail: format!("cannot write private key to {}: {e}", existing.display()),
        };
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&existing, std::fs::Permissions::from_mode(0o600));
    }

    let pubkey = derive_wg_pubkey(&private_key).await;
    BootstrapPhase {
        name: "wg.keygen".into(),
        ok: true,
        detail: format!(
            "keypair generated → {} (pub: {})",
            existing.display(),
            pubkey.as_deref().unwrap_or("derive failed")
        ),
    }
}

pub(super) fn wg_private_key_path() -> std::path::PathBuf {
    let config_dir = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_CONFIG_DIR,
        cellmembrane_types::service::DEFAULT_CONFIG_DIR,
    );
    std::path::PathBuf::from(config_dir).join("wg_private.key")
}

pub(super) async fn derive_wg_pubkey(private_key: &str) -> Option<String> {
    use tokio::io::AsyncWriteExt;

    let mut child = tokio::process::Command::new("wg")
        .arg("pubkey")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(private_key.as_bytes()).await.ok()?;
        stdin.shutdown().await.ok()?;
    }

    let output = child.wait_with_output().await.ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

/// Generate wg-quick config from manifest and local keypair.
pub(super) async fn wg_config_phase(
    gate_name: &str,
    mesh_ip: &str,
    dry_run: bool,
) -> BootstrapPhase {
    let Ok(root) = crate::temporal::resolve_workspace_root() else {
        return BootstrapPhase {
            name: "wg.config".into(),
            ok: false,
            detail: "cannot resolve workspace root".into(),
        };
    };
    let Ok(manifest) = crate::manifest::load_from_workspace(&root) else {
        return BootstrapPhase {
            name: "wg.config".into(),
            ok: false,
            detail: "cannot load ecosystem manifest".into(),
        };
    };

    let config = manifest_to_wg_config(gate_name, mesh_ip, &manifest);
    let rendered = config.to_wg_quick();

    if dry_run {
        let peer_count = config.peers.len();
        return BootstrapPhase {
            name: "wg.config".into(),
            ok: true,
            detail: format!(
                "dry-run: would write wg0.conf ({peer_count} peers, address {mesh_ip}/24)"
            ),
        };
    }

    let wg_config_path = wg_config_file_path();
    if let Some(parent) = wg_config_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }

    match tokio::fs::write(&wg_config_path, &rendered).await {
        Ok(()) => BootstrapPhase {
            name: "wg.config".into(),
            ok: true,
            detail: format!(
                "wrote {} ({} peers)",
                wg_config_path.display(),
                config.peers.len()
            ),
        },
        Err(e) => BootstrapPhase {
            name: "wg.config".into(),
            ok: false,
            detail: format!("cannot write {}: {e}", wg_config_path.display()),
        },
    }
}

pub(super) fn wg_config_file_path() -> std::path::PathBuf {
    let config_dir = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_CONFIG_DIR,
        cellmembrane_types::service::DEFAULT_CONFIG_DIR,
    );
    std::path::PathBuf::from(config_dir).join("wg0.conf")
}

pub(super) fn manifest_to_wg_config(
    gate_name: &str,
    mesh_ip: &str,
    manifest: &crate::manifest::EcosystemManifest,
) -> cellmembrane_types::wireguard::WgConfig {
    let hub_endpoint = manifest
        .topology
        .as_ref()
        .and_then(|t| t.hosts.get(&t.inner_membrane))
        .map(String::as_str);

    let peers: Vec<cellmembrane_types::wireguard::WgPeer> = manifest
        .gates
        .iter()
        .filter(|(name, _)| *name != gate_name)
        .filter_map(|(name, profile)| {
            let peer_ip = profile.wg_ip.as_deref()?;
            let endpoint = if profile.roles.iter().any(|r| {
                matches!(
                    r,
                    cellmembrane_types::GateRole::WgHub | cellmembrane_types::GateRole::Relay
                )
            }) {
                profile.host.clone().or_else(|| hub_endpoint.map(String::from))
            } else {
                None
            };

            let keepalive = if endpoint.is_some() { 25 } else { 0 };
            Some(cellmembrane_types::wireguard::WgPeer {
                name: name.clone(),
                mesh_ip: peer_ip.to_string(),
                public_key: None,
                endpoint,
                allowed_ips: vec![format!("{peer_ip}/32")],
                keepalive,
            })
        })
        .collect();

    cellmembrane_types::wireguard::WgConfig {
        gate_name: gate_name.into(),
        address: mesh_ip.into(),
        listen_port: cellmembrane_types::wireguard::DEFAULT_WG_PORT,
        subnet: cellmembrane_types::service::DEFAULT_WG_MESH_SUBNET.into(),
        peers,
    }
}

/// Read the local `WireGuard` public key by deriving it from the stored private key.
pub(super) async fn read_local_pubkey() -> Option<String> {
    let key_path = wg_private_key_path();
    let private_key = tokio::fs::read_to_string(&key_path).await.ok()?;
    derive_wg_pubkey(private_key.trim()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wg_private_key_path_is_under_config_dir() {
        let path = wg_private_key_path();
        assert!(path.ends_with("wg_private.key"));
    }

    #[test]
    fn wg_config_path_is_under_config_dir() {
        let path = wg_config_file_path();
        assert!(path.ends_with("wg0.conf"));
    }

    fn test_manifest(gates: &str) -> crate::manifest::EcosystemManifest {
        let toml_str = format!(
            r#"
[meta]
family = "ecoPrimals"
version = "1"
wave = 147

[sync]
divergence_policy = "flag"
push_targets = ["origin"]

[repos.cellMembrane]
org = "ecoPrimals"
local_path = "gardens/cellMembrane"

{gates}
"#
        );
        toml::from_str(&toml_str).unwrap()
    }

    #[test]
    fn manifest_to_wg_config_generates_peers() {
        let manifest = test_manifest(
            r#"
[gates.golgiBody]
target = "x86_64-unknown-linux-musl"
wg_ip = "10.13.37.1"
host = "157.230.3.183"
roles = ["wg_hub", "relay"]

[gates.sporeGate]
target = "x86_64-unknown-linux-musl"
wg_ip = "10.13.37.2"
roles = ["builder"]

[gates.testGate]
target = "x86_64-unknown-linux-musl"
wg_ip = "10.13.37.99"
roles = []
"#,
        );
        let config = manifest_to_wg_config("testGate", "10.13.37.99", &manifest);

        assert_eq!(config.gate_name, "testGate");
        assert_eq!(config.address, "10.13.37.99");
        assert_eq!(config.peers.len(), 2);

        let golgi = config.peers.iter().find(|p| p.name == "golgiBody").unwrap();
        assert_eq!(golgi.mesh_ip, "10.13.37.1");
        assert!(golgi.endpoint.is_some());
        assert_eq!(golgi.keepalive, 25);

        let spore = config.peers.iter().find(|p| p.name == "sporeGate").unwrap();
        assert_eq!(spore.mesh_ip, "10.13.37.2");
        assert!(spore.endpoint.is_none());
        assert_eq!(spore.keepalive, 0);
    }

    #[test]
    fn manifest_to_wg_config_excludes_self() {
        let manifest = test_manifest(
            r#"
[gates.golgiBody]
target = "x86_64-unknown-linux-musl"
wg_ip = "10.13.37.1"
roles = ["wg_hub"]

[gates.myGate]
target = "x86_64-unknown-linux-musl"
wg_ip = "10.13.37.50"
roles = []
"#,
        );
        let config = manifest_to_wg_config("myGate", "10.13.37.50", &manifest);
        assert!(!config.peers.iter().any(|p| p.name == "myGate"));
        assert_eq!(config.peers.len(), 1);
    }

    #[test]
    fn wg_config_renders_valid_output() {
        let manifest = test_manifest(
            r#"
[gates.golgiBody]
target = "x86_64-unknown-linux-musl"
wg_ip = "10.13.37.1"
host = "157.230.3.183"
roles = ["wg_hub"]
"#,
        );
        let config = manifest_to_wg_config("newGate", "10.13.37.99", &manifest);
        let rendered = config.to_wg_quick();

        assert!(rendered.contains("[Interface]"));
        assert!(rendered.contains("Address = 10.13.37.99/24"));
        assert!(rendered.contains("# golgiBody"));
        assert!(rendered.contains("AllowedIPs = 10.13.37.1/32"));
        assert!(rendered.contains("PersistentKeepalive = 25"));
    }
}
