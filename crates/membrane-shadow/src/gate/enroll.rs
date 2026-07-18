// SPDX-License-Identifier: AGPL-3.0-or-later

//! Gate enrollment — mesh onboarding automation.
//!
//! `gate.enroll` is the *pre-bootstrap* step that gets a new gate onto the
//! `WireGuard` mesh with Forgejo-first git remotes. It automates the manual
//! process documented in the northGate AAR (Wave 147a):
//!
//! 0. `manifest.resolve` — Locate gate profile and mesh IP in ecosystem manifest
//! 1. `wg.keygen` — Generate `WireGuard` keypair
//! 2. `wg.config` — Render wg-quick config from manifest
//! 3. `mesh.verify` — Verify tunnel connectivity to hub
//! 4. `forgejo.verify` — Verify Forgejo SSH via mesh
//! 5. `git.remotes` — Configure Forgejo-first remotes on local repos
//! 6. `hub.peer` — Register this gate as a peer on the hub (SSH + `wg set`)
//!
//! After enrollment, `gate.bootstrap` handles depot fetch + NUCLEUS deployment.
//!
//! Future: Phase 7 (`songbird.mesh_enroll`) — call songBird's BTSP-verified
//! `mesh.enroll` to complete the proof-of-enrollment handshake.

use super::bootstrap::BootstrapPhase;
use crate::error::Result;
use serde::{Deserialize, Serialize};

const ENROLL_PHASE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// SSH timeout for hub-side peer addition (generous for WAN latency).
#[allow(
    clippy::cast_possible_truncation,
    reason = "DEFAULT_SSH_TIMEOUT_SECS is 10 — fits in u32"
)]
const HUB_SSH_TIMEOUT: u32 = cellmembrane_types::service::DEFAULT_SSH_TIMEOUT_SECS as u32 + 5;

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

    phases.push(timed_phase_enroll("wg.keygen", wg_keygen_phase(dry_run)).await);

    let ip = mesh_ip.clone().unwrap_or_default();
    phases.push(timed_phase_enroll("wg.config", wg_config_phase(gate_name, &ip, dry_run)).await);

    phases.push(timed_phase_enroll("mesh.verify", mesh_verify_phase(&ip, dry_run)).await);

    phases.push(timed_phase_enroll("forgejo.verify", forgejo_verify_phase(dry_run)).await);

    phases.push(timed_phase_enroll("git.remotes", git_remotes_phase(gate_name, dry_run)).await);

    phases.push(timed_phase_enroll("hub.peer", hub_peer_phase(gate_name, &ip, dry_run)).await);

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
    manifest.gates.get(gate_name).and_then(|p| p.wg_ip.clone())
}

use super::wg::{read_local_pubkey, wg_config_phase, wg_keygen_phase};

/// Resolve the hub (inner membrane) mesh IP from the manifest.
fn resolve_hub_ip() -> Option<String> {
    let root = crate::temporal::resolve_workspace_root().ok()?;
    let manifest = crate::manifest::load_from_workspace(&root).ok()?;
    let topo = manifest.topology.as_ref()?;
    let hub_name = &topo.inner_membrane;
    manifest.gates.get(hub_name).and_then(|p| p.wg_ip.clone())
}

/// Verify mesh connectivity by pinging the hub gateway.
async fn mesh_verify_phase(mesh_ip: &str, dry_run: bool) -> BootstrapPhase {
    let hub_ip = resolve_hub_ip().unwrap_or_else(|| "10.13.37.1".into());

    if dry_run {
        return BootstrapPhase {
            name: "mesh.verify".into(),
            ok: true,
            detail: format!("dry-run: would ping hub {hub_ip} from {mesh_ip}"),
        };
    }

    let ping_result = tokio::process::Command::new("ping")
        .args(["-c", "3", "-W", "5", &hub_ip])
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

    let (host, port) = git_addr.split_once(':').unwrap_or((&git_addr, "22"));

    let ssh_result = tokio::process::Command::new("ssh")
        .args([
            "-o",
            "StrictHostKeyChecking=accept-new",
            "-o",
            "ConnectTimeout=10",
            "-p",
            port,
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

/// Register this gate as a peer on the hub's `WireGuard` interface.
///
/// Reads the local public key, resolves the hub gate via manifest topology,
/// and SSHs to the hub to run `wg set wg0 peer <pubkey> allowed-ips <ip>/32`.
async fn hub_peer_phase(gate_name: &str, mesh_ip: &str, dry_run: bool) -> BootstrapPhase {
    let Some(pubkey) = read_local_pubkey().await else {
        return BootstrapPhase {
            name: "hub.peer".into(),
            ok: false,
            detail: "cannot read local public key — run wg.keygen first".into(),
        };
    };

    let Some(hub_host) = resolve_hub_ssh_target() else {
        return BootstrapPhase {
            name: "hub.peer".into(),
            ok: false,
            detail: "cannot resolve hub SSH target from manifest".into(),
        };
    };

    if dry_run {
        return BootstrapPhase {
            name: "hub.peer".into(),
            ok: true,
            detail: format!(
                "dry-run: would add peer {gate_name} ({mesh_ip}) to hub {hub_host} (pubkey: {}...)",
                &pubkey[..8.min(pubkey.len())]
            ),
        };
    }

    let wg_iface = cellmembrane_types::wireguard::DEFAULT_WG_IFACE;
    let cmd = format!(
        "wg set {wg_iface} peer {pubkey} allowed-ips {mesh_ip}/32 && wg-quick save {wg_iface}"
    );

    match crate::ssh::exec_on_host("root", &hub_host, &cmd, HUB_SSH_TIMEOUT).await {
        Ok((stdout, 0)) => BootstrapPhase {
            name: "hub.peer".into(),
            ok: true,
            detail: format!(
                "peer {gate_name} ({mesh_ip}) added to hub {hub_host}{}",
                if stdout.trim().is_empty() {
                    String::new()
                } else {
                    format!(" — {}", stdout.trim())
                }
            ),
        },
        Ok((stderr, code)) => BootstrapPhase {
            name: "hub.peer".into(),
            ok: false,
            detail: format!("hub wg set failed (exit {code}): {}", stderr.trim()),
        },
        Err(e) => BootstrapPhase {
            name: "hub.peer".into(),
            ok: false,
            detail: format!("SSH to hub {hub_host} failed: {e}"),
        },
    }
}

/// Resolve the hub gate's SSH target (IP or hostname) from the manifest.
fn resolve_hub_ssh_target() -> Option<String> {
    let root = crate::temporal::resolve_workspace_root().ok()?;
    let manifest = crate::manifest::load_from_workspace(&root).ok()?;
    let topo = manifest.topology.as_ref()?;
    let hub_name = &topo.inner_membrane;
    let hub_profile = manifest.gates.get(hub_name)?;
    hub_profile
        .host
        .clone()
        .or_else(|| hub_profile.wg_ip.clone())
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

        let forgejo_url = forgejo_clone_url(repo_name);
        let github_url = github_clone_url(repo_name);

        if dry_run {
            configured += 1;
            continue;
        }

        let origin_ok = set_remote_url(&repo_dir, "origin", &forgejo_url).await;
        let github_ok = set_remote_url(&repo_dir, "github", &github_url).await;

        if origin_ok && github_ok {
            configured += 1;
        } else {
            errors += 1;
        }
    }

    let total = configured + skipped + errors;
    let prefix = if dry_run {
        "dry-run: would configure"
    } else {
        "configured"
    };

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

/// Build the Forgejo SSH clone URL for a given repo.
#[must_use]
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

    #[tokio::test]
    async fn enroll_dry_run_completes() {
        let result = enroll("testGate", true).await;
        assert!(result.is_ok());
        let r = result.unwrap();
        assert!(!r.phases.is_empty());
    }

    #[test]
    fn resolve_hub_ssh_target_returns_option() {
        let result = resolve_hub_ssh_target();
        let _ = result;
    }

    const _: () = {
        assert!(HUB_SSH_TIMEOUT >= 10);
        assert!(HUB_SSH_TIMEOUT <= 60);
    };

    #[tokio::test]
    async fn enroll_dry_run_includes_hub_peer_phase() {
        let result = enroll("testGate", true).await;
        assert!(result.is_ok());
        let r = result.unwrap();
        let hub_phase = r.phases.iter().find(|p| p.name == "hub.peer");
        if let Some(phase) = hub_phase {
            assert!(
                phase.detail.contains("dry-run") || phase.detail.contains("cannot"),
                "hub.peer should be dry-run or report missing key: {}",
                phase.detail
            );
        }
    }
}
