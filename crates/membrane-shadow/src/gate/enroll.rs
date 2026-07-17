// SPDX-License-Identifier: AGPL-3.0-or-later

//! Gate enrollment — mesh onboarding automation.
//!
//! `gate.enroll` is the *pre-bootstrap* step that gets a new gate onto the
//! `WireGuard` mesh with Forgejo-first git remotes. It automates the manual
//! process documented in the northGate AAR (Wave 147a):
//!
//! 1. `wg.keygen` — Generate `WireGuard` keypair
//! 2. `wg.config` — Render wg-quick config from manifest
//! 3. `mesh.verify` — Verify tunnel connectivity to hub
//! 4. `forgejo.verify` — Verify Forgejo SSH via mesh
//! 5. `git.remotes` — Configure Forgejo-first remotes on local repos
//!
//! After enrollment, `gate.bootstrap` handles depot fetch + NUCLEUS deployment.

use super::bootstrap::BootstrapPhase;
use crate::error::Result;
use serde::{Deserialize, Serialize};

const ENROLL_PHASE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Result of a `gate.enroll` run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrollResult {
    /// Gate being enrolled.
    pub gate_name: String,
    /// Mesh IP assigned from manifest.
    pub mesh_ip: Option<String>,
    /// Per-phase results.
    pub phases: Vec<BootstrapPhase>,
    /// Whether all phases passed.
    pub all_pass: bool,
}

/// Orchestrate gate mesh enrollment.
///
/// Phases: resolve manifest profile → generate WG keys → render config →
/// verify mesh connectivity → verify Forgejo SSH → configure git remotes.
pub async fn enroll(gate_name: &str, dry_run: bool) -> Result<EnrollResult> {
    let mut phases = Vec::new();

    let profile = super::mesh::resolve_gate_profile(gate_name);
    let mesh_ip = resolve_mesh_ip(gate_name);

    phases.push(BootstrapPhase {
        name: "manifest.resolve".into(),
        ok: mesh_ip.is_some(),
        detail: mesh_ip.as_ref().map_or_else(
            || format!("{gate_name}: no wg_ip in manifest — add [gates.{gate_name}] with wg_ip"),
            |ip| format!("{gate_name}: mesh_ip={ip}, transport={}", profile.transport),
        ),
    });

    if mesh_ip.is_none() {
        return Ok(EnrollResult {
            gate_name: gate_name.into(),
            mesh_ip: None,
            phases,
            all_pass: false,
        });
    }

    phases.push(
        timed_phase_enroll("wg.keygen", wg_keygen_phase(dry_run)).await,
    );

    let ip = mesh_ip.clone().unwrap_or_default();
    phases.push(
        timed_phase_enroll(
            "wg.config",
            wg_config_phase(gate_name, &ip, dry_run),
        )
        .await,
    );

    phases.push(
        timed_phase_enroll(
            "mesh.verify",
            mesh_verify_phase(&ip, dry_run),
        )
        .await,
    );

    phases.push(
        timed_phase_enroll(
            "forgejo.verify",
            forgejo_verify_phase(dry_run),
        )
        .await,
    );

    phases.push(
        timed_phase_enroll(
            "git.remotes",
            git_remotes_phase(gate_name, dry_run),
        )
        .await,
    );

    let all_pass = phases.iter().all(|p| p.ok);

    Ok(EnrollResult {
        gate_name: gate_name.into(),
        mesh_ip,
        phases,
        all_pass,
    })
}

async fn timed_phase_enroll<F>(name: &str, fut: F) -> BootstrapPhase
where
    F: std::future::Future<Output = BootstrapPhase>,
{
    tokio::time::timeout(ENROLL_PHASE_TIMEOUT, fut)
        .await
        .unwrap_or_else(|_| BootstrapPhase {
            name: name.into(),
            ok: false,
            detail: format!("timeout after {}s", ENROLL_PHASE_TIMEOUT.as_secs()),
        })
}

// ── Phase implementations ──────────────────────────────────────────

fn resolve_mesh_ip(gate_name: &str) -> Option<String> {
    let root = crate::temporal::resolve_workspace_root().ok()?;
    let manifest = crate::manifest::load_from_workspace(&root).ok()?;
    manifest
        .gates
        .get(gate_name)
        .and_then(|p| p.wg_ip.clone())
}

/// Generate a `WireGuard` keypair. Returns the public key in the phase detail.
async fn wg_keygen_phase(dry_run: bool) -> BootstrapPhase {
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

fn wg_private_key_path() -> std::path::PathBuf {
    let config_dir = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_CONFIG_DIR,
        cellmembrane_types::service::DEFAULT_CONFIG_DIR,
    );
    std::path::PathBuf::from(config_dir).join("wg_private.key")
}

async fn derive_wg_pubkey(private_key: &str) -> Option<String> {
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
async fn wg_config_phase(gate_name: &str, mesh_ip: &str, dry_run: bool) -> BootstrapPhase {
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

fn wg_config_file_path() -> std::path::PathBuf {
    let config_dir = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_CONFIG_DIR,
        cellmembrane_types::service::DEFAULT_CONFIG_DIR,
    );
    std::path::PathBuf::from(config_dir).join("wg0.conf")
}

fn manifest_to_wg_config(
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

/// Verify mesh connectivity by pinging the hub gateway.
async fn mesh_verify_phase(mesh_ip: &str, dry_run: bool) -> BootstrapPhase {
    let hub_ip = "10.13.37.1";

    if dry_run {
        return BootstrapPhase {
            name: "mesh.verify".into(),
            ok: true,
            detail: format!("dry-run: would ping hub {hub_ip} from {mesh_ip}"),
        };
    }

    let ping_result = tokio::process::Command::new("ping")
        .args(["-c", "3", "-W", "5", hub_ip])
        .output()
        .await;

    match ping_result {
        Ok(output) if output.status.success() => BootstrapPhase {
            name: "mesh.verify".into(),
            ok: true,
            detail: format!("hub {hub_ip} reachable from mesh ({mesh_ip})"),
        },
        Ok(output) => BootstrapPhase {
            name: "mesh.verify".into(),
            ok: false,
            detail: format!(
                "hub {hub_ip} unreachable — is wg0 up? ({})",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        },
        Err(e) => BootstrapPhase {
            name: "mesh.verify".into(),
            ok: false,
            detail: format!("ping failed: {e}"),
        },
    }
}

/// Verify Forgejo SSH connectivity via mesh.
async fn forgejo_verify_phase(dry_run: bool) -> BootstrapPhase {
    let git_addr = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_FORGEJO_GIT_ADDR,
        cellmembrane_types::service::DEFAULT_FORGEJO_GIT_ADDR,
    );

    if dry_run {
        return BootstrapPhase {
            name: "forgejo.verify".into(),
            ok: true,
            detail: format!("dry-run: would verify SSH to {git_addr}"),
        };
    }

    let (host, port) = git_addr
        .split_once(':')
        .unwrap_or((&git_addr, "22"));

    let ssh_result = tokio::process::Command::new("ssh")
        .args([
            "-o", "StrictHostKeyChecking=accept-new",
            "-o", "ConnectTimeout=10",
            "-p", port,
            &format!("git@{host}"),
            "help",
        ])
        .output()
        .await;

    match ssh_result {
        Ok(output) => {
            let combined = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
            let forgejo_ok = combined.contains("Forgejo")
                || combined.contains("forgejo")
                || combined.contains("Hi there")
                || output.status.success();

            BootstrapPhase {
                name: "forgejo.verify".into(),
                ok: forgejo_ok,
                detail: if forgejo_ok {
                    format!("Forgejo SSH verified at {git_addr}")
                } else {
                    format!(
                        "SSH to {git_addr} failed: {}",
                        combined.lines().next().unwrap_or("(no output)")
                    )
                },
            }
        }
        Err(e) => BootstrapPhase {
            name: "forgejo.verify".into(),
            ok: false,
            detail: format!("SSH to {git_addr} failed: {e}"),
        },
    }
}

/// Configure Forgejo-first git remotes on local repos.
///
/// The enrollment standard (Wave 147a): `origin` = Forgejo (sovereign), `github` = GitHub (mirror).
async fn git_remotes_phase(gate_name: &str, dry_run: bool) -> BootstrapPhase {
    let Ok(root) = crate::temporal::resolve_workspace_root() else {
        return BootstrapPhase {
            name: "git.remotes".into(),
            ok: false,
            detail: "cannot resolve workspace root".into(),
        };
    };
    let Ok(manifest) = crate::manifest::load_from_workspace_async(&root).await else {
        return BootstrapPhase {
            name: "git.remotes".into(),
            ok: false,
            detail: "cannot load ecosystem manifest".into(),
        };
    };

    let git_addr = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_FORGEJO_GIT_ADDR,
        cellmembrane_types::service::DEFAULT_FORGEJO_GIT_ADDR,
    );
    let forgejo_org = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_FORGEJO_ORG,
        cellmembrane_types::service::DEFAULT_FORGEJO_ORG,
    );

    let github_org = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_GITHUB_ORG,
        cellmembrane_types::service::DEFAULT_GITHUB_ORG,
    );

    let _ = gate_name;

    let mut configured = 0u32;
    let mut skipped = 0u32;
    let mut errors = 0u32;

    for (repo_name, entry) in &manifest.repos {
        let repo_dir = root.join(&entry.local_path);
        if !repo_dir.join(".git").exists() {
            skipped += 1;
            continue;
        }

        let forgejo_url = format!("ssh://git@{git_addr}/{forgejo_org}/{repo_name}.git");
        let github_url = format!("git@github.com:{github_org}/{repo_name}.git");

        if dry_run {
            configured += 1;
            continue;
        }

        let origin_ok =
            set_remote_url(&repo_dir, "origin", &forgejo_url).await;
        let github_ok =
            ensure_remote(&repo_dir, "github", &github_url).await;

        if origin_ok && github_ok {
            configured += 1;
        } else {
            errors += 1;
        }
    }

    let total = configured + skipped + errors;
    let prefix = if dry_run { "dry-run: would configure" } else { "configured" };

    BootstrapPhase {
        name: "git.remotes".into(),
        ok: errors == 0,
        detail: format!(
            "{prefix} {configured}/{total} repos Forgejo-first (origin=forgejo, github=mirror){}",
            if skipped > 0 {
                format!(", {skipped} not cloned")
            } else {
                String::new()
            }
        ),
    }
}

/// Set or create a git remote URL.
async fn set_remote_url(repo_dir: &std::path::Path, remote: &str, url: &str) -> bool {
    let existing = crate::git_ops::git_output_opt(repo_dir, &["remote", "get-url", remote]).await;
    if existing.as_deref() == Some(url) {
        return true;
    }

    if existing.is_some() {
        crate::git_ops::git_success(repo_dir, &["remote", "set-url", remote, url]).await
    } else {
        crate::git_ops::git_success(repo_dir, &["remote", "add", remote, url]).await
    }
}

/// Ensure a remote exists. If it exists with a different URL, update it.
async fn ensure_remote(repo_dir: &std::path::Path, remote: &str, url: &str) -> bool {
    set_remote_url(repo_dir, remote, url).await
}

/// Build the Forgejo SSH clone URL for a given repo.
#[must_use]
#[allow(dead_code, reason = "enrollment API — wired by tests, consumed by git.remotes phase")]
pub fn forgejo_clone_url(repo_name: &str) -> String {
    let git_addr = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_FORGEJO_GIT_ADDR,
        cellmembrane_types::service::DEFAULT_FORGEJO_GIT_ADDR,
    );
    let org = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_FORGEJO_ORG,
        cellmembrane_types::service::DEFAULT_FORGEJO_ORG,
    );
    format!("ssh://git@{git_addr}/{org}/{repo_name}.git")
}

/// Build the GitHub SSH clone URL for a given repo.
#[must_use]
#[allow(dead_code, reason = "enrollment API — wired by tests, consumed by git.remotes phase")]
pub fn github_clone_url(repo_name: &str) -> String {
    let org = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_GITHUB_ORG,
        cellmembrane_types::service::DEFAULT_GITHUB_ORG,
    );
    format!("git@github.com:{org}/{repo_name}.git")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forgejo_url_format() {
        let url = forgejo_clone_url("cellMembrane");
        assert!(url.starts_with("ssh://git@"));
        assert!(url.contains("cellMembrane.git"));
        assert!(url.contains(cellmembrane_types::service::DEFAULT_FORGEJO_GIT_ADDR));
    }

    #[test]
    fn github_url_format() {
        let url = github_clone_url("cellMembrane");
        assert!(url.starts_with("git@github.com:"));
        assert!(url.contains("cellMembrane.git"));
    }

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

    #[test]
    fn manifest_to_wg_config_generates_peers() {
        use crate::manifest::EcosystemManifest;
        let toml_str = r#"
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
"#;
        let manifest: EcosystemManifest = toml::from_str(toml_str).unwrap();
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
        use crate::manifest::EcosystemManifest;
        let toml_str = r#"
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

[gates.golgiBody]
target = "x86_64-unknown-linux-musl"
wg_ip = "10.13.37.1"
roles = ["wg_hub"]

[gates.myGate]
target = "x86_64-unknown-linux-musl"
wg_ip = "10.13.37.50"
roles = []
"#;
        let manifest: EcosystemManifest = toml::from_str(toml_str).unwrap();
        let config = manifest_to_wg_config("myGate", "10.13.37.50", &manifest);
        assert!(!config.peers.iter().any(|p| p.name == "myGate"));
        assert_eq!(config.peers.len(), 1);
    }

    #[test]
    fn wg_config_renders_valid_output() {
        use crate::manifest::EcosystemManifest;
        let toml_str = r#"
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

[gates.golgiBody]
target = "x86_64-unknown-linux-musl"
wg_ip = "10.13.37.1"
host = "157.230.3.183"
roles = ["wg_hub"]
"#;
        let manifest: EcosystemManifest = toml::from_str(toml_str).unwrap();
        let config = manifest_to_wg_config("newGate", "10.13.37.99", &manifest);
        let rendered = config.to_wg_quick();

        assert!(rendered.contains("[Interface]"));
        assert!(rendered.contains("Address = 10.13.37.99/24"));
        assert!(rendered.contains("# golgiBody"));
        assert!(rendered.contains("AllowedIPs = 10.13.37.1/32"));
        assert!(rendered.contains("PersistentKeepalive = 25"));
    }

    #[tokio::test]
    async fn enroll_dry_run_completes() {
        let result = enroll("testGate", true).await;
        assert!(result.is_ok());
        let r = result.unwrap();
        assert!(!r.phases.is_empty());
    }
}
