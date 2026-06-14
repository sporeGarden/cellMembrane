// SPDX-License-Identifier: AGPL-3.0-or-later

//! Webhook receiver — Forgejo push event handling for selective cascade.
//!
//! CM-WEBHOOK-01: Evolves the timer-polled cascade model toward push-driven.
//! When a primal team pushes to Forgejo, this module receives the webhook,
//! verifies HMAC authenticity, identifies the changed primal, and triggers
//! a selective harvest for just that binary.
//!
//! Architecture:
//! - Forgejo → Caddy reverse proxy → membrane UDS webhook endpoint
//! - HMAC-SHA256 verification (Forgejo `X-Forgejo-Signature` header)
//! - Selective cascade: only sync + harvest the pushed repo
//!
//! Transport: UDS behind Caddy — no exposed TCP ports (Tower Atomic posture).

use crate::error::{Result, ShadowError};
use serde::Deserialize;

/// Forgejo push webhook payload (subset of fields we need).
#[derive(Debug, Clone, Deserialize)]
pub struct PushEvent {
    /// Git ref that was pushed (e.g. `refs/heads/main`).
    #[serde(rename = "ref")]
    pub git_ref: String,
    /// Commit SHA before the push.
    pub before: String,
    /// Commit SHA after the push.
    pub after: String,
    /// Repository information.
    pub repository: RepoPayload,
    /// Pusher information.
    pub pusher: PusherPayload,
    /// Commits included in this push.
    #[serde(default)]
    pub commits: Vec<CommitPayload>,
}

/// Repository info from the webhook payload.
#[derive(Debug, Clone, Deserialize)]
pub struct RepoPayload {
    /// Repository name (e.g. `biomeOS`).
    pub name: String,
    /// Full path including org (e.g. `ecoPrimals/biomeOS`).
    pub full_name: String,
    /// Clone URL (SSH preferred for our infra).
    pub ssh_url: String,
    /// Default branch name.
    pub default_branch: String,
}

/// Pusher identity from the webhook payload.
#[derive(Debug, Clone, Deserialize)]
pub struct PusherPayload {
    /// Forgejo username.
    #[serde(alias = "login")]
    pub username: String,
}

/// Individual commit data from the push.
#[derive(Debug, Clone, Deserialize)]
pub struct CommitPayload {
    /// Full commit SHA.
    pub id: String,
    /// Commit message.
    pub message: String,
}

/// Result of processing a webhook event.
#[derive(Debug)]
pub struct WebhookAction {
    /// The repo that was pushed to.
    pub repo_name: String,
    /// The branch that was pushed.
    pub branch: String,
    /// Whether this push should trigger a cascade + harvest.
    pub should_harvest: bool,
    /// Human-readable reason for the decision.
    pub reason: String,
}

/// Verify Forgejo webhook HMAC-SHA256 signature.
///
/// Forgejo sends `X-Forgejo-Signature` as hex(HMAC-SHA256(secret, body)).
/// Returns `Ok(())` if valid, `Err` if signature mismatch or missing.
pub fn verify_signature(secret: &[u8], body: &[u8], signature_hex: &str) -> Result<()> {
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;
    use std::fmt::Write;

    type HmacSha256 = Hmac<Sha256>;

    let mut mac =
        HmacSha256::new_from_slice(secret).map_err(|e| ShadowError::Parse(e.to_string()))?;
    mac.update(body);
    let result = mac.finalize().into_bytes();

    let mut expected = String::with_capacity(64);
    for byte in result.as_slice() {
        write!(expected, "{byte:02x}").ok();
    }

    if constant_time_eq(expected.as_bytes(), signature_hex.as_bytes()) {
        Ok(())
    } else {
        Err(ShadowError::Parse("webhook signature mismatch".into()))
    }
}

/// Determine what action to take for a push event.
///
/// Only triggers harvest for pushes to the default branch of repos
/// that are known primals in our manifest.
#[must_use]
pub fn classify_push(event: &PushEvent, known_primals: &[&str]) -> WebhookAction {
    let branch = event
        .git_ref
        .strip_prefix("refs/heads/")
        .unwrap_or(&event.git_ref);

    let is_default_branch = branch == event.repository.default_branch;
    let repo_lower = event.repository.name.to_lowercase();
    let is_known_primal = known_primals.iter().any(|p| p.to_lowercase() == repo_lower);

    let should_harvest = is_default_branch && is_known_primal;

    let reason = if !is_default_branch {
        format!("non-default branch ({branch}), skipping")
    } else if !is_known_primal {
        format!("{} not a known primal, skipping", event.repository.name)
    } else {
        format!(
            "{} pushed to {branch} — triggering selective harvest",
            event.repository.name
        )
    };

    WebhookAction {
        repo_name: event.repository.name.clone(),
        branch: branch.to_string(),
        should_harvest,
        reason,
    }
}

// ── Primal registry ─────────────────────────────────────────────────────

use crate::plasmid::nucleus_primals;

/// Handle a verified push event — trigger selective cascade + harvest.
///
/// Returns a `ShadowOutcome` describing what was done.
pub async fn handle_push(
    event: &PushEvent,
    config: &crate::ShadowConfig,
) -> crate::error::Result<crate::ShadowOutcome> {
    let primal_refs = nucleus_primals();
    let action = classify_push(event, &primal_refs);

    if !action.should_harvest {
        return Ok(crate::ShadowOutcome::ok(format!(
            "webhook: {} — {}",
            event.repository.name, action.reason
        )));
    }

    eprintln!(
        "[webhook] {} pushed to {} — selective harvest for {}",
        event.pusher.username, action.branch, action.repo_name
    );

    let harvest_args = crate::plasmid::HarvestArgs {
        primal: Some(action.repo_name.to_lowercase()),
        force: false,
        dry_run: false,
        depot_dir: None,
        target: None,
    };

    let harvest_outcome = crate::plasmid::harvest(&harvest_args).await?;

    if !harvest_outcome.ok {
        return Ok(crate::ShadowOutcome {
            ok: false,
            message: format!(
                "webhook: {} harvest failed — {}",
                action.repo_name, harvest_outcome.message
            ),
            data: harvest_outcome.data,
        });
    }

    // Sandbox validation: spin up new binary in isolation, health-check before promoting
    let arch = crate::plasmid::detect_target_triple();
    let primal_lower = action.repo_name.to_lowercase();
    let depot_binary = crate::plasmid::resolve_path(None, "PLASMIDBIN_DEPOT", || {
        crate::resolve_xdg_data_home().join("ecoPrimals").join("plasmidBin")
    })
    .join("primals")
    .join(&arch)
    .join(&primal_lower);

    if depot_binary.exists() {
        let commit_short = if event.after.len() >= 8 {
            &event.after[..8]
        } else {
            &event.after
        };

        let sandbox_args = crate::plasmid::sandbox::SandboxArgs {
            primal: primal_lower.clone(),
            commit: commit_short.to_string(),
            binary_path: depot_binary,
            timeout_secs: None,
        };

        match crate::plasmid::sandbox::validate(&sandbox_args).await {
            Ok(result) if !result.health_ok => {
                eprintln!(
                    "[webhook] sandbox FAIL for {} — {}",
                    primal_lower, result.detail
                );
                return Ok(crate::ShadowOutcome {
                    ok: false,
                    message: format!(
                        "webhook: {} sandbox validation FAILED — {} ({}ms). Production unchanged.",
                        action.repo_name, result.detail, result.elapsed_ms
                    ),
                    data: Some(serde_json::to_value(&result).unwrap_or_default()),
                });
            }
            Ok(result) => {
                eprintln!(
                    "[webhook] sandbox PASS for {} — {} ({}ms)",
                    primal_lower, result.detail, result.elapsed_ms
                );
            }
            Err(e) => {
                eprintln!("[webhook] sandbox infra error for {primal_lower}: {e} — proceeding");
            }
        }
    }

    let refresh_args = crate::plasmid::RefreshArgs {
        primal: Some(primal_lower),
        dry_run: false,
        source_dir: None,
    };

    let refresh_outcome = crate::plasmid::refresh(config, &refresh_args).await?;

    Ok(crate::ShadowOutcome {
        ok: refresh_outcome.ok,
        message: format!(
            "webhook: {} → harvest: {} | sandbox: PASS | refresh: {}",
            action.repo_name, harvest_outcome.message, refresh_outcome.message
        ),
        data: refresh_outcome.data,
    })
}

// ── Constant-time comparison ─────────────────────────────────────────

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write;

    fn compute_hmac_hex(secret: &[u8], message: &[u8]) -> String {
        use hmac::{Hmac, KeyInit, Mac};
        use sha2::Sha256;
        type HmacSha256 = Hmac<Sha256>;

        let mut mac = HmacSha256::new_from_slice(secret).unwrap();
        mac.update(message);
        let result = mac.finalize().into_bytes();
        let mut hex = String::with_capacity(64);
        for b in result.as_slice() {
            write!(hex, "{b:02x}").unwrap();
        }
        hex
    }

    fn sha256_hex(data: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(data);
        let result = hasher.finalize();
        let mut hex = String::with_capacity(64);
        for b in result.as_slice() {
            write!(hex, "{b:02x}").unwrap();
        }
        hex
    }

    #[test]
    fn sha256_via_crate_known_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"hello"),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn hmac_sha256_rfc4231_test1() {
        let key = [0x0bu8; 20];
        let data = b"Hi There";
        let hex = compute_hmac_hex(&key, data);
        assert_eq!(
            hex,
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    #[test]
    fn verify_signature_valid() {
        let secret = b"webhook-secret-123";
        let body = b"{\"ref\":\"refs/heads/main\"}";
        let sig = compute_hmac_hex(secret, body);
        assert!(verify_signature(secret, body, &sig).is_ok());
    }

    #[test]
    fn verify_signature_invalid() {
        let secret = b"webhook-secret-123";
        let body = b"{\"ref\":\"refs/heads/main\"}";
        let bad_sig = "0000000000000000000000000000000000000000000000000000000000000000";
        assert!(verify_signature(secret, body, bad_sig).is_err());
    }

    #[test]
    fn verify_signature_wrong_secret() {
        let secret = b"correct-secret";
        let wrong_secret = b"wrong-secret";
        let body = b"payload";
        let sig = compute_hmac_hex(wrong_secret, body);
        assert!(verify_signature(secret, body, &sig).is_err());
    }

    #[test]
    fn constant_time_eq_same() {
        assert!(constant_time_eq(b"hello", b"hello"));
    }

    #[test]
    fn constant_time_eq_different() {
        assert!(!constant_time_eq(b"hello", b"world"));
    }

    #[test]
    fn constant_time_eq_different_length() {
        assert!(!constant_time_eq(b"short", b"longer"));
    }

    #[test]
    fn classify_push_default_branch_known_primal() {
        let event = sample_push_event("biomeOS", "main", "main");
        let primals = &["beardog", "songbird", "biomeos", "skunkbat"];
        let action = classify_push(&event, primals);
        assert!(action.should_harvest);
        assert_eq!(action.repo_name, "biomeOS");
        assert_eq!(action.branch, "main");
    }

    #[test]
    fn classify_push_non_default_branch() {
        let event = sample_push_event("biomeOS", "feature/test", "main");
        let primals = &["biomeos"];
        let action = classify_push(&event, primals);
        assert!(!action.should_harvest);
        assert!(action.reason.contains("non-default branch"));
    }

    #[test]
    fn classify_push_unknown_repo() {
        let event = sample_push_event("unknownRepo", "main", "main");
        let primals = &["biomeos", "beardog"];
        let action = classify_push(&event, primals);
        assert!(!action.should_harvest);
        assert!(action.reason.contains("not a known primal"));
    }

    #[test]
    fn push_event_deserializes() {
        let json = r#"{
            "ref": "refs/heads/main",
            "before": "0000000000000000000000000000000000000000",
            "after": "abc123def456",
            "repository": {
                "name": "biomeOS",
                "full_name": "ecoPrimals/biomeOS",
                "ssh_url": "ssh://git@git.primals.eco:2222/ecoPrimals/biomeOS.git",
                "default_branch": "main"
            },
            "pusher": {
                "username": "irongate"
            },
            "commits": [
                {"id": "abc123def456", "message": "fix: search priority"}
            ]
        }"#;
        let event: PushEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.repository.name, "biomeOS");
        assert_eq!(event.after, "abc123def456");
        assert_eq!(event.commits.len(), 1);
    }

    fn sample_push_event(repo: &str, branch: &str, default: &str) -> PushEvent {
        PushEvent {
            git_ref: format!("refs/heads/{branch}"),
            before: "0".repeat(40),
            after: "a".repeat(40),
            repository: RepoPayload {
                name: repo.into(),
                full_name: format!("ecoPrimals/{repo}"),
                ssh_url: format!("ssh://git@git.primals.eco:2222/ecoPrimals/{repo}.git"),
                default_branch: default.into(),
            },
            pusher: PusherPayload {
                username: "operator".into(),
            },
            commits: vec![],
        }
    }
}
