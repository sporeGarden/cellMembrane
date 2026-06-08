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
    use std::fmt::Write;

    let key = hmac_sha256_key(secret);
    let mac = hmac_sha256(&key, body);
    let mut expected = String::with_capacity(64);
    for byte in &mac {
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

// ── HMAC-SHA256 (pure Rust, no external crate) ─────────────────────────

/// Nucleus primals known to the plasmidBin pipeline.
fn nucleus_primals() -> Vec<&'static str> {
    vec![
        "beardog",
        "songbird",
        "biomeos",
        "nestgate",
        "skunkbat",
        "squirrel",
        "rhizocrypt",
        "loamspine",
        "sweetgrass",
        "toadstool",
        "coralreef",
        "barracuda",
        "petaltongue",
    ]
}

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

    let refresh_args = crate::plasmid::RefreshArgs {
        primal: Some(action.repo_name.to_lowercase()),
        dry_run: false,
        source_dir: None,
    };

    let refresh_outcome = crate::plasmid::refresh(config, &refresh_args).await?;

    Ok(crate::ShadowOutcome {
        ok: refresh_outcome.ok,
        message: format!(
            "webhook: {} → harvest: {} | refresh: {}",
            action.repo_name, harvest_outcome.message, refresh_outcome.message
        ),
        data: refresh_outcome.data,
    })
}

// ── HMAC-SHA256 (pure Rust, no external crate) ─────────────────────────

const BLOCK_SIZE: usize = 64;
const HASH_SIZE: usize = 32;

fn hmac_sha256_key(secret: &[u8]) -> [u8; BLOCK_SIZE] {
    let mut key = [0u8; BLOCK_SIZE];
    if secret.len() > BLOCK_SIZE {
        let hash = sha256(secret);
        key[..HASH_SIZE].copy_from_slice(&hash);
    } else {
        key[..secret.len()].copy_from_slice(secret);
    }
    key
}

fn hmac_sha256(key: &[u8; BLOCK_SIZE], message: &[u8]) -> [u8; HASH_SIZE] {
    let mut ipad = [0x36u8; BLOCK_SIZE];
    let mut opad = [0x5cu8; BLOCK_SIZE];
    for i in 0..BLOCK_SIZE {
        ipad[i] ^= key[i];
        opad[i] ^= key[i];
    }

    let mut inner_input = Vec::with_capacity(BLOCK_SIZE + message.len());
    inner_input.extend_from_slice(&ipad);
    inner_input.extend_from_slice(message);
    let inner_hash = sha256(&inner_input);

    let mut outer_input = Vec::with_capacity(BLOCK_SIZE + HASH_SIZE);
    outer_input.extend_from_slice(&opad);
    outer_input.extend_from_slice(&inner_hash);
    sha256(&outer_input)
}

fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h: [u32; 8] = [
        0x6a09_e667,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];

    let k: [u32; 64] = [
        0x428a_2f98,
        0x7137_4491,
        0xb5c0_fbcf,
        0xe9b5_dba5,
        0x3956_c25b,
        0x59f1_11f1,
        0x923f_82a4,
        0xab1c_5ed5,
        0xd807_aa98,
        0x1283_5b01,
        0x2431_85be,
        0x550c_7dc3,
        0x72be_5d74,
        0x80de_b1fe,
        0x9bdc_06a7,
        0xc19b_f174,
        0xe49b_69c1,
        0xefbe_4786,
        0x0fc1_9dc6,
        0x240c_a1cc,
        0x2de9_2c6f,
        0x4a74_84aa,
        0x5cb0_a9dc,
        0x76f9_88da,
        0x983e_5152,
        0xa831_c66d,
        0xb003_27c8,
        0xbf59_7fc7,
        0xc6e0_0bf3,
        0xd5a7_9147,
        0x06ca_6351,
        0x1429_2967,
        0x27b7_0a85,
        0x2e1b_2138,
        0x4d2c_6dfc,
        0x5338_0d13,
        0x650a_7354,
        0x766a_0abb,
        0x81c2_c92e,
        0x9272_2c85,
        0xa2bf_e8a1,
        0xa81a_664b,
        0xc24b_8b70,
        0xc76c_51a3,
        0xd192_e819,
        0xd699_0624,
        0xf40e_3585,
        0x106a_a070,
        0x19a4_c116,
        0x1e37_6c08,
        0x2748_774c,
        0x34b0_bcb5,
        0x391c_0cb3,
        0x4ed8_aa4a,
        0x5b9c_ca4f,
        0x682e_6ff3,
        0x748f_82ee,
        0x78a5_636f,
        0x84c8_7814,
        0x8cc7_0208,
        0x90be_fffa,
        0xa450_6ceb,
        0xbef9_a3f7,
        0xc671_78f2,
    ];

    let bit_len = (data.len() as u64) * 8;
    let mut padded = data.to_vec();
    padded.push(0x80);
    while (padded.len() % 64) != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in padded.chunks_exact(64) {
        sha256_compress(&mut h, chunk, &k);
    }

    let mut result = [0u8; 32];
    for (i, &val) in h.iter().enumerate() {
        result[i * 4..i * 4 + 4].copy_from_slice(&val.to_be_bytes());
    }
    result
}

#[allow(clippy::many_single_char_names)]
fn sha256_compress(h: &mut [u32; 8], chunk: &[u8], k: &[u32; 64]) {
    let mut w = [0u32; 64];
    for i in 0..16 {
        w[i] = u32::from_be_bytes([
            chunk[i * 4],
            chunk[i * 4 + 1],
            chunk[i * 4 + 2],
            chunk[i * 4 + 3],
        ]);
    }
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16]
            .wrapping_add(s0)
            .wrapping_add(w[i - 7])
            .wrapping_add(s1);
    }

    let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
        (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);

    for i in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ ((!e) & g);
        let temp1 = hh
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(k[i])
            .wrapping_add(w[i]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = s0.wrapping_add(maj);

        hh = g;
        g = f;
        f = e;
        e = d.wrapping_add(temp1);
        d = c;
        c = b;
        b = a;
        a = temp1.wrapping_add(temp2);
    }

    h[0] = h[0].wrapping_add(a);
    h[1] = h[1].wrapping_add(b);
    h[2] = h[2].wrapping_add(c);
    h[3] = h[3].wrapping_add(d);
    h[4] = h[4].wrapping_add(e);
    h[5] = h[5].wrapping_add(f);
    h[6] = h[6].wrapping_add(g);
    h[7] = h[7].wrapping_add(hh);
}

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

    fn to_hex(bytes: &[u8]) -> String {
        let mut s = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            write!(s, "{b:02x}").unwrap();
        }
        s
    }

    #[test]
    fn sha256_empty() {
        let hash = sha256(b"");
        let hex = to_hex(&hash);
        assert_eq!(
            hex,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_hello() {
        let hash = sha256(b"hello");
        let hex = to_hex(&hash);
        assert_eq!(
            hex,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn hmac_sha256_rfc4231_test1() {
        // RFC 4231 Test Case 1
        let key = [0x0bu8; 20];
        let data = b"Hi There";
        let hmac_key = hmac_sha256_key(&key);
        let mac = hmac_sha256(&hmac_key, data);
        let hex = to_hex(&mac);
        assert_eq!(
            hex,
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    #[test]
    fn verify_signature_valid() {
        let secret = b"webhook-secret-123";
        let body = b"{\"ref\":\"refs/heads/main\"}";
        let key = hmac_sha256_key(secret);
        let mac = hmac_sha256(&key, body);
        let sig = to_hex(&mac);
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
        let key = hmac_sha256_key(wrong_secret);
        let mac = hmac_sha256(&key, body);
        let sig = to_hex(&mac);
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
