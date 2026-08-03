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

pub(crate) mod listener;
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

impl std::str::FromStr for WebhookProvider {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "github" => Ok(Self::GitHub),
            "forgejo" => Ok(Self::Forgejo),
            _ => Err(format!(
                "unknown webhook provider: {s} (expected: forgejo|github)"
            )),
        }
    }
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
    #[allow(dead_code, reason = "serde-populated — needed for delta analysis")]
    pub before: String,
    /// Commit SHA after the push.
    pub after: String,
    /// Repository information.
    pub repository: RepoPayload,
    /// Pusher information.
    pub pusher: PusherPayload,
    /// Commits included in this push.
    #[serde(default)]
    #[allow(
        dead_code,
        reason = "serde-populated — needed for commit-level cascade"
    )]
    pub commits: Vec<CommitPayload>,
}

/// Repository info from the webhook payload.
#[derive(Debug, Clone, Deserialize)]
pub struct RepoPayload {
    /// Repository name (e.g. `biomeOS`).
    pub name: String,
    /// Full path including org (e.g. `ecoPrimals/biomeOS`).
    #[allow(dead_code, reason = "serde-populated — needed for cross-org routing")]
    pub full_name: String,
    /// Clone URL (SSH preferred for our infra).
    #[allow(dead_code, reason = "serde-populated — needed for clone-based harvest")]
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
#[allow(
    dead_code,
    reason = "serde-populated — parsed for commit-level cascade analysis"
)]
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
    let expected = crate::crypto::hmac_sha256_hex(secret, body);

    if constant_time_eq(expected.as_bytes(), signature_hex.as_bytes()) {
        Ok(())
    } else {
        Err(ShadowError::Config("webhook signature mismatch".into()))
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
    let should_cascade = is_default_branch && is_cascade_repo && !is_known_primal;

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

    let has_signal = crate::plasmid::scheduler::has_harvest_signal(&event.commits);

    if has_signal {
        info!(
            pusher = %event.pusher.username,
            branch = %action.branch,
            repo = %action.repo_name,
            "immediate harvest triggered by [harvest] signal"
        );
        pipeline::run_harvest_pipeline(&action, event, config).await
    } else {
        info!(
            pusher = %event.pusher.username,
            branch = %action.branch,
            repo = %action.repo_name,
            "push queued for scheduled harvest (no [harvest] signal)"
        );
        let commit_short = if event.after.len() >= 12 {
            &event.after[..12]
        } else {
            &event.after
        };
        crate::plasmid::scheduler::ingest(
            &action.repo_name,
            commit_short,
            &event.pusher.username,
        )?;
        Ok(crate::ShadowOutcome::ok(format!(
            "webhook: {} push queued for batch harvest (commit {})",
            action.repo_name, commit_short
        )))
    }
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
mod tests;
