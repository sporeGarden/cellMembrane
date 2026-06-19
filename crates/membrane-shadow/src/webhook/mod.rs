// SPDX-License-Identifier: AGPL-3.0-or-later

//! Webhook receiver — Forgejo + GitHub push event handling for selective cascade.
//!
//! CM-WEBHOOK-01: Push-driven cascade (replaces timer-polled model).
//! Webhook events arrive from either Forgejo (sovereign) or GitHub (outer membrane),
//! are verified via HMAC-SHA256, classified, and dispatched to:
//! - Selective harvest (plasmid pipeline) for primal repos
//! - Git cascade (`temporal.sync` / `relay.run`) for ecosystem repos
//!
//! Provider abstraction: [`WebhookProvider`] distinguishes Forgejo vs GitHub
//! signature headers and payload shapes.
//!
//! Architecture:
//! - Forgejo/GitHub -> Caddy reverse proxy -> membrane UDS webhook endpoint
//! - HMAC-SHA256 verification (provider-specific header)
//! - Selective cascade: only sync + harvest the pushed repo
//!
//! Transport: UDS behind Caddy — no exposed TCP ports (Tower Atomic posture).

mod pipeline;

use crate::error::{Result, ShadowError};
use serde::{Deserialize, Serialize};
use tracing::info;

/// Webhook provider — determines signature header format and payload shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebhookProvider {
    /// Sovereign Forgejo instance. Header: `X-Forgejo-Signature` = hex(HMAC-SHA256).
    Forgejo,
    /// GitHub outer membrane. Header: `X-Hub-Signature-256` = `sha256=` + hex(HMAC-SHA256).
    GitHub,
}

impl WebhookProvider {
    /// Detect provider from HTTP headers.
    ///
    /// Checks for provider-specific signature headers and returns the
    /// provider + raw signature value.
    #[must_use]
    pub fn detect(headers: &[(String, String)]) -> Option<(Self, String)> {
        for (name, value) in headers {
            let lower = name.to_lowercase();
            if lower == "x-forgejo-signature" {
                return Some((Self::Forgejo, value.clone()));
            }
            if lower == "x-hub-signature-256" {
                return Some((Self::GitHub, value.clone()));
            }
        }
        None
    }

    /// Extract the hex signature from the raw header value.
    ///
    /// Forgejo sends bare hex; GitHub prefixes with `sha256=`.
    #[must_use]
    pub fn extract_signature(self, raw: &str) -> &str {
        match self {
            Self::Forgejo => raw,
            Self::GitHub => raw.strip_prefix("sha256=").unwrap_or(raw),
        }
    }
}

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
    /// Whether this push should trigger git cascade (relay/temporal sync).
    pub should_cascade: bool,
    /// Which provider sent the webhook.
    pub provider: WebhookProvider,
    /// Human-readable reason for the decision.
    pub reason: String,
}

/// Verify webhook HMAC-SHA256 signature (provider-aware).
///
/// Extracts the hex digest from the raw header value according to provider
/// conventions, then performs constant-time comparison.
pub fn verify_provider_signature(
    provider: WebhookProvider,
    secret: &[u8],
    body: &[u8],
    raw_signature: &str,
) -> Result<()> {
    let hex_sig = provider.extract_signature(raw_signature);
    verify_signature(secret, body, hex_sig)
}

/// Verify HMAC-SHA256 signature against bare hex digest.
///
/// Both Forgejo and GitHub use HMAC-SHA256 — only the header format differs.
/// Returns `Ok(())` if valid, `Err` if signature mismatch.
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

/// Bootstrap list of repos that trigger cascade when no manifest is loaded.
///
/// Once topology data is available, prefer [`cascade_repos_from_manifest`].
const BOOTSTRAP_CASCADE_REPOS: &[&str] =
    &["cellmembrane", "wateringhole", "whitepaper", "primalspring"];

/// Derive cascade repo list from manifest (non-primal ecosystem repos).
///
/// Any repo in the manifest that isn't a known primal binary triggers
/// cascade instead of harvest.
fn cascade_repos_from_manifest(known_primals: &[&str]) -> Vec<String> {
    let Ok(root) = crate::temporal::resolve_workspace_root() else {
        return BOOTSTRAP_CASCADE_REPOS
            .iter()
            .map(|s| (*s).to_string())
            .collect();
    };
    let Ok(manifest) = crate::manifest::load_from_workspace(&root) else {
        return BOOTSTRAP_CASCADE_REPOS
            .iter()
            .map(|s| (*s).to_string())
            .collect();
    };
    manifest
        .repos
        .keys()
        .filter(|name| {
            let lower = name.to_lowercase();
            !known_primals.iter().any(|p| p.to_lowercase() == lower)
        })
        .map(|name| name.to_lowercase())
        .collect()
}

/// Determine what action to take for a push event.
///
/// Triggers harvest for known primal repos on default branch.
/// Triggers git cascade for ecosystem infrastructure repos (manifest-driven).
#[must_use]
pub fn classify_push(
    event: &PushEvent,
    known_primals: &[&str],
    provider: WebhookProvider,
) -> WebhookAction {
    let branch = event
        .git_ref
        .strip_prefix("refs/heads/")
        .unwrap_or(&event.git_ref);

    let is_default_branch = branch == event.repository.default_branch;
    let repo_lower = event.repository.name.to_lowercase();
    let is_known_primal = known_primals.iter().any(|p| p.to_lowercase() == repo_lower);
    let cascade_repos = cascade_repos_from_manifest(known_primals);
    let is_cascade_repo = cascade_repos.contains(&repo_lower);

    let should_harvest = is_default_branch && is_known_primal;
    let should_cascade = is_default_branch && is_cascade_repo;

    let reason = if !is_default_branch {
        format!("non-default branch ({branch}), skipping")
    } else if should_harvest {
        format!(
            "{} pushed to {branch} — triggering selective harvest",
            event.repository.name
        )
    } else if should_cascade {
        format!(
            "{} pushed to {branch} — triggering git cascade",
            event.repository.name
        )
    } else {
        format!(
            "{} not a known primal or cascade repo, skipping",
            event.repository.name
        )
    };

    WebhookAction {
        repo_name: event.repository.name.clone(),
        branch: branch.to_string(),
        should_harvest,
        should_cascade,
        provider,
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
    provider: WebhookProvider,
) -> crate::error::Result<crate::ShadowOutcome> {
    let primal_refs = nucleus_primals();
    let action = classify_push(event, &primal_refs, provider);

    if !action.should_harvest && !action.should_cascade {
        return Ok(crate::ShadowOutcome::ok(format!(
            "webhook: {} — {}",
            event.repository.name, action.reason
        )));
    }

    if action.should_cascade && !action.should_harvest {
        info!(
            provider = ?action.provider,
            repo = %action.repo_name,
            branch = %action.branch,
            "git cascade triggered by webhook"
        );
        return pipeline::run_cascade_pipeline(&action, config).await;
    }

    info!(
        pusher = %event.pusher.username,
        branch = %action.branch,
        repo = %action.repo_name,
        "selective harvest triggered"
    );

    pipeline::run_harvest_pipeline(&action, event, config).await
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
        let action = classify_push(&event, primals, WebhookProvider::Forgejo);
        assert!(action.should_harvest);
        assert_eq!(action.repo_name, "biomeOS");
        assert_eq!(action.branch, "main");
        assert_eq!(action.provider, WebhookProvider::Forgejo);
    }

    #[test]
    fn classify_push_non_default_branch() {
        let event = sample_push_event("biomeOS", "feature/test", "main");
        let primals = &["biomeos"];
        let action = classify_push(&event, primals, WebhookProvider::Forgejo);
        assert!(!action.should_harvest);
        assert!(action.reason.contains("non-default branch"));
    }

    #[test]
    fn classify_push_unknown_repo() {
        let event = sample_push_event("unknownRepo", "main", "main");
        let primals = &["biomeos", "beardog"];
        let action = classify_push(&event, primals, WebhookProvider::Forgejo);
        assert!(!action.should_harvest);
        assert!(!action.should_cascade);
        assert!(action.reason.contains("not a known primal"));
    }

    #[test]
    fn classify_push_cascade_repo() {
        let event = sample_push_event("wateringHole", "main", "main");
        let primals = &["biomeos"];
        let action = classify_push(&event, primals, WebhookProvider::Forgejo);
        assert!(!action.should_harvest);
        assert!(action.should_cascade);
        assert!(action.reason.contains("git cascade"));
    }

    #[test]
    fn provider_detect_forgejo() {
        let headers = vec![("X-Forgejo-Signature".to_string(), "abc123".to_string())];
        let (provider, sig) = WebhookProvider::detect(&headers).unwrap();
        assert_eq!(provider, WebhookProvider::Forgejo);
        assert_eq!(sig, "abc123");
    }

    #[test]
    fn provider_detect_github() {
        let headers = vec![(
            "X-Hub-Signature-256".to_string(),
            "sha256=def456".to_string(),
        )];
        let (provider, sig) = WebhookProvider::detect(&headers).unwrap();
        assert_eq!(provider, WebhookProvider::GitHub);
        assert_eq!(sig, "sha256=def456");
    }

    #[test]
    fn provider_extract_signature_github() {
        assert_eq!(
            WebhookProvider::GitHub.extract_signature("sha256=abc123"),
            "abc123"
        );
    }

    #[test]
    fn provider_extract_signature_forgejo() {
        assert_eq!(
            WebhookProvider::Forgejo.extract_signature("abc123"),
            "abc123"
        );
    }

    #[test]
    fn verify_github_signature() {
        let secret = b"gh-secret";
        let body = b"payload";
        let sig = compute_hmac_hex(secret, body);
        let raw = format!("sha256={sig}");
        assert!(verify_provider_signature(WebhookProvider::GitHub, secret, body, &raw).is_ok());
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

    #[test]
    fn classify_push_github_cascade_repo() {
        let event = sample_push_event("cellMembrane", "main", "main");
        let primals = &["biomeos", "beardog"];
        let action = classify_push(&event, primals, WebhookProvider::GitHub);
        assert!(!action.should_harvest);
        assert!(action.should_cascade);
        assert_eq!(action.provider, WebhookProvider::GitHub);
    }

    #[test]
    fn classify_push_primal_overrides_cascade() {
        let event = sample_push_event("cellMembrane", "main", "main");
        let primals = &["cellmembrane"];
        let action = classify_push(&event, primals, WebhookProvider::Forgejo);
        assert!(action.should_harvest, "primal match triggers harvest");
        assert!(
            !action.should_cascade,
            "manifest-driven cascade excludes known primals"
        );
    }

    #[test]
    fn bootstrap_cascade_repos_are_not_primals() {
        let primals = &["biomeos", "beardog", "songbird", "skunkbat"];
        for repo in BOOTSTRAP_CASCADE_REPOS {
            assert!(
                !primals.iter().any(|p| p.to_lowercase() == *repo),
                "bootstrap cascade repo '{repo}' should not be a primal"
            );
        }
    }

    #[test]
    fn provider_no_headers_returns_none() {
        let headers: Vec<(String, String)> = vec![];
        assert!(WebhookProvider::detect(&headers).is_none());
    }

    #[test]
    fn provider_irrelevant_headers_returns_none() {
        let headers = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("X-Request-Id".to_string(), "123".to_string()),
        ];
        assert!(WebhookProvider::detect(&headers).is_none());
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
