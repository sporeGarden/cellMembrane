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
//! 7. `mesh.enroll` — BTSP-verified genetic enrollment via songBird
//!
//! After enrollment, `gate.bootstrap` handles depot fetch + NUCLEUS deployment.

use super::bootstrap::BootstrapPhase;
use crate::error::Result;
use serde::{Deserialize, Serialize};

const ENROLL_PHASE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(
    cellmembrane_types::service::DEFAULT_ENROLL_PHASE_TIMEOUT_SECS,
);

const DEFAULT_SSH_PORT: &str = "22";

/// SSH timeout for hub-side peer addition (generous for WAN latency).
///
/// Compile-time assertion below guarantees the `as u32` truncation is safe.
#[allow(
    clippy::cast_possible_truncation,
    reason = "guarded by const assertion below"
)]
const HUB_SSH_TIMEOUT: u32 = cellmembrane_types::service::DEFAULT_SSH_TIMEOUT_SECS as u32 + 5;
const _: () = assert!(
    cellmembrane_types::service::DEFAULT_SSH_TIMEOUT_SECS <= u32::MAX as u64,
    "SSH timeout must fit in u32"
);

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

    let ip = mesh_ip.as_deref().unwrap_or_default();
    phases.push(timed_phase_enroll("wg.config", wg_config_phase(gate_name, ip, dry_run)).await);

    phases.push(timed_phase_enroll("mesh.verify", mesh_verify_phase(ip, dry_run)).await);

    phases.push(timed_phase_enroll("forgejo.verify", forgejo_verify_phase(dry_run)).await);

    phases.push(timed_phase_enroll("git.remotes", git_remotes_phase(gate_name, dry_run)).await);

    phases.push(timed_phase_enroll("hub.peer", hub_peer_phase(gate_name, ip, dry_run)).await);

    phases
        .push(timed_phase_enroll("mesh.enroll", mesh_enroll_phase(gate_name, ip, dry_run)).await);

    phases.push(timed_phase_enroll("ssh_cert", ssh_cert_phase(gate_name, dry_run)).await);

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
    let hub_ip = resolve_hub_ip()
        .or_else(|| cellmembrane_types::mesh_address("golgi").map(Into::into))
        .unwrap_or_else(|| cellmembrane_types::service::DEFAULT_HUB_MESH_IP.into());

    if dry_run {
        return BootstrapPhase {
            name: "mesh.verify".into(),
            ok: true,
            detail: format!("dry-run: would ping hub {hub_ip} from {mesh_ip}"),
        };
    }

    let ping_result = tokio::process::Command::new("ping")
        .args([
            "-c",
            "3",
            "-W",
            &cellmembrane_types::service::DEFAULT_PROBE_TIMEOUT_SECS.to_string(),
            &hub_ip,
        ])
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

    let (host, port) = git_addr.split_once(':').unwrap_or((&git_addr, DEFAULT_SSH_PORT));

    let ssh_result = tokio::process::Command::new("ssh")
        .args([
            "-o",
            "StrictHostKeyChecking=accept-new",
            "-o",
            &format!(
                "ConnectTimeout={}",
                cellmembrane_types::service::DEFAULT_SSH_TIMEOUT_SECS
            ),
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

// ── Phase 7: mesh.enroll — BTSP-verified genetic enrollment ──────

/// BTSP-verified genetic enrollment via songBird's `mesh.enroll` endpoint.
///
/// Computes an HMAC-SHA256 enrollment proof from `FAMILY_SEED` (the same
/// algorithm bearDog's `enrollment.verify` checks) and calls songBird's
/// `mesh.enroll` JSON-RPC to complete cryptographic enrollment into the mesh.
///
/// Requires: `FAMILY_SEED` or `BEARDOG_FAMILY_SEED` env var set.
/// Requires: songBird running locally with its UDS socket reachable.
async fn mesh_enroll_phase(gate_name: &str, mesh_ip: &str, dry_run: bool) -> BootstrapPhase {
    let family_seed = load_enrollment_family_seed();

    if family_seed.is_none() {
        return BootstrapPhase {
            name: "mesh.enroll".into(),
            ok: false,
            detail: "FAMILY_SEED or BEARDOG_FAMILY_SEED not set — cannot compute enrollment proof"
                .into(),
        };
    }
    let family_seed = family_seed.unwrap_or_default();

    let Some(pubkey) = read_local_pubkey().await else {
        return BootstrapPhase {
            name: "mesh.enroll".into(),
            ok: false,
            detail: "cannot read local WG public key — run wg.keygen first".into(),
        };
    };

    let seed_generation = load_enrollment_seed_generation();
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());

    let proof =
        compute_enrollment_proof(&family_seed, gate_name, &pubkey, timestamp, seed_generation);

    if dry_run {
        return BootstrapPhase {
            name: "mesh.enroll".into(),
            ok: true,
            detail: format!(
                "dry-run: would call mesh.enroll for {gate_name} (gen={seed_generation}, proof={}...)",
                &proof[..8.min(proof.len())]
            ),
        };
    }

    let songbird_socket = resolve_relay_socket();
    if !songbird_socket.exists() {
        return BootstrapPhase {
            name: "mesh.enroll".into(),
            ok: false,
            detail: format!(
                "songBird socket not found at {} — is songBird running?",
                songbird_socket.display()
            ),
        };
    }

    let params = serde_json::json!({
        "node_id": gate_name,
        "public_key": pubkey,
        "timestamp": timestamp,
        "proof": proof,
        "address": format!("{mesh_ip}:{}", cellmembrane_types::service::DEFAULT_FEDERATION_PORT),
    });
    let request = crate::jsonrpc::request_with_params("mesh.enroll", &params, 1);

    match crate::jsonrpc::call(&songbird_socket, &request).await {
        Ok(response) => {
            let enrolled = serde_json::from_str::<serde_json::Value>(&response)
                .ok()
                .and_then(|j| j.get("result").cloned().or(Some(j)))
                .and_then(|r| r.get("enrolled").and_then(serde_json::Value::as_bool))
                .unwrap_or(false);

            if enrolled {
                BootstrapPhase {
                    name: "mesh.enroll".into(),
                    ok: true,
                    detail: format!("{gate_name} enrolled into mesh (gen={seed_generation})"),
                }
            } else {
                let reason = serde_json::from_str::<serde_json::Value>(&response)
                    .ok()
                    .and_then(|j| j.get("result").cloned().or(Some(j)))
                    .and_then(|r| {
                        r.get("reason")
                            .and_then(serde_json::Value::as_str)
                            .map(String::from)
                    })
                    .unwrap_or_else(|| "unknown".into());

                BootstrapPhase {
                    name: "mesh.enroll".into(),
                    ok: false,
                    detail: format!("mesh.enroll rejected for {gate_name}: {reason}"),
                }
            }
        }
        Err(e) => BootstrapPhase {
            name: "mesh.enroll".into(),
            ok: false,
            detail: format!("mesh.enroll RPC failed: {e}"),
        },
    }
}

/// Load `FAMILY_SEED` from environment (same precedence as bearDog).
fn load_enrollment_family_seed() -> Option<Vec<u8>> {
    for var in ["BEARDOG_FAMILY_SEED", "FAMILY_SEED"] {
        if let Ok(val) = std::env::var(var) {
            if !val.is_empty() {
                return Some(val.into_bytes());
            }
        }
    }
    None
}

/// Load enrollment seed generation from environment (default 0).
fn load_enrollment_seed_generation() -> u32 {
    std::env::var("BEARDOG_ENROLLMENT_SEED_GENERATION")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

/// Compute the HMAC-SHA256 enrollment proof.
///
/// Mirrors bearDog's enrollment crypto exactly:
/// ```text
/// key = HKDF-SHA256(family_seed, salt=FAMILY_ID, info="enrollment-v{gen}")
/// message = "node_id|public_key|timestamp|generation"
/// proof = base64(HMAC-SHA256(key, message))
/// ```
fn compute_enrollment_proof(
    family_seed: &[u8],
    node_id: &str,
    public_key: &str,
    timestamp: u64,
    generation: u32,
) -> String {
    use base64::Engine;
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;

    let family_id = std::env::var("FAMILY_ID").unwrap_or_else(|_| "default".into());

    // HKDF-SHA256 extract + expand (mirrors bearDog derive_enrollment_key)
    let info = format!("enrollment-v{generation}");
    let mut extract_mac = HmacSha256::new_from_slice(family_id.as_bytes()).expect("HMAC key init");
    extract_mac.update(family_seed);
    let prk = extract_mac.finalize().into_bytes();

    let mut expand_mac = HmacSha256::new_from_slice(&prk).expect("HMAC key init");
    expand_mac.update(info.as_bytes());
    expand_mac.update(&[1u8]);
    let enrollment_key: [u8; 32] = expand_mac.finalize().into_bytes().into();

    // HMAC-SHA256 proof over the enrollment message
    let message = format!("{node_id}|{public_key}|{timestamp}|{generation}");
    let mut proof_mac = HmacSha256::new_from_slice(&enrollment_key).expect("HMAC key init");
    proof_mac.update(message.as_bytes());
    let proof_bytes = proof_mac.finalize().into_bytes();

    base64::engine::general_purpose::STANDARD.encode(proof_bytes)
}

/// Resolve the mesh relay UDS socket path via capability discovery.
fn resolve_relay_socket() -> std::path::PathBuf {
    let relay = cellmembrane_types::MembraneService::binary_for(
        cellmembrane_types::ServiceCapability::MeshRelay,
    );
    let paths = super::health::resolve_primal_socket_paths(relay);
    std::path::PathBuf::from(&paths[0])
}

/// Phase 8: Request an SSH certificate from the sovereign step-ca instance.
///
/// If step-ca is reachable and `STEP_CA_FINGERPRINT` is set, obtains a
/// short-lived SSH user certificate. Non-fatal: enrollment succeeds even
/// if step-ca is unavailable (the gate can enroll SSH certs later via
/// `gate.keys`).
async fn ssh_cert_phase(gate_name: &str, dry_run: bool) -> BootstrapPhase {
    match super::key_portal::request_ssh_certificate(gate_name, dry_run).await {
        Ok(cert) => BootstrapPhase {
            name: "ssh_cert".into(),
            ok: true,
            detail: format!(
                "SSH cert issued for {} (expires {})",
                cert.principals.join(", "),
                if cert.valid_before == 0 {
                    "N/A (dry-run)".into()
                } else {
                    format!("epoch {}", cert.valid_before)
                }
            ),
        },
        Err(e) => {
            let msg = e.to_string();
            let is_missing_dep = msg.contains("not found") || msg.contains("STEP_CA_FINGERPRINT");
            BootstrapPhase {
                name: "ssh_cert".into(),
                ok: is_missing_dep,
                detail: if is_missing_dep {
                    format!("skipped (step-ca not configured): {msg}")
                } else {
                    format!("SSH cert request failed: {msg}")
                },
            }
        }
    }
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

    #[test]
    fn enrollment_proof_is_deterministic() {
        let seed = b"test-family-seed";
        let p1 = compute_enrollment_proof(seed, "gate1", "pubkey123", 1000, 0);
        let p2 = compute_enrollment_proof(seed, "gate1", "pubkey123", 1000, 0);
        assert_eq!(p1, p2);
    }

    #[test]
    fn enrollment_proof_changes_with_inputs() {
        let seed = b"test-family-seed";
        let p1 = compute_enrollment_proof(seed, "gate1", "pubkey1", 1000, 0);
        let p2 = compute_enrollment_proof(seed, "gate2", "pubkey1", 1000, 0);
        let p3 = compute_enrollment_proof(seed, "gate1", "pubkey2", 1000, 0);
        let p4 = compute_enrollment_proof(seed, "gate1", "pubkey1", 2000, 0);
        let p5 = compute_enrollment_proof(seed, "gate1", "pubkey1", 1000, 1);
        assert_ne!(p1, p2);
        assert_ne!(p1, p3);
        assert_ne!(p1, p4);
        assert_ne!(p1, p5);
    }

    #[test]
    fn enrollment_proof_is_valid_base64() {
        use base64::Engine;
        let proof = compute_enrollment_proof(b"seed", "node", "key", 1, 0);
        let decoded = base64::engine::general_purpose::STANDARD.decode(&proof);
        assert!(decoded.is_ok(), "proof should be valid base64");
        assert_eq!(decoded.unwrap().len(), 32, "HMAC-SHA256 output is 32 bytes");
    }

    #[tokio::test]
    async fn enroll_dry_run_includes_mesh_enroll_phase() {
        let result = enroll("testGate", true).await;
        assert!(result.is_ok());
        let r = result.unwrap();
        // If the gate isn't in the manifest, enroll returns early after
        // manifest.resolve — mesh.enroll only appears when IP is resolved.
        if r.mesh_ip.is_some() {
            let phase = r.phases.iter().find(|p| p.name == "mesh.enroll");
            assert!(
                phase.is_some(),
                "mesh.enroll phase should be present when IP resolved"
            );
            if let Some(p) = phase {
                assert!(
                    p.detail.contains("dry-run") || p.detail.contains("FAMILY_SEED"),
                    "mesh.enroll should be dry-run or report missing seed: {}",
                    p.detail
                );
            }
        }
    }

    #[test]
    fn load_enrollment_seed_generation_parses_or_defaults() {
        let result = load_enrollment_seed_generation();
        assert!(result < 1000, "generation should be reasonable: {result}");
    }

    #[test]
    fn relay_socket_path_ends_with_sock() {
        let path = resolve_relay_socket();
        assert!(
            path.extension().is_some_and(|e| e == "sock"),
            "socket path should end with .sock: {path:?}"
        );
    }

    #[tokio::test]
    async fn ssh_cert_phase_dry_run() {
        let phase = ssh_cert_phase("testGate", true).await;
        assert_eq!(phase.name, "ssh_cert");
        assert!(phase.ok, "dry-run ssh_cert should pass: {}", phase.detail);
        assert!(
            phase.detail.contains("testGate"),
            "detail should contain gate name: {}",
            phase.detail
        );
    }

    #[tokio::test]
    async fn enroll_dry_run_includes_ssh_cert_phase() {
        let result = enroll("testGate", true).await;
        assert!(result.is_ok());
        let r = result.unwrap();
        if r.mesh_ip.is_some() {
            let phase = r.phases.iter().find(|p| p.name == "ssh_cert");
            assert!(
                phase.is_some(),
                "ssh_cert phase should be present when IP resolved"
            );
        }
    }
}
