// SPDX-License-Identifier: AGPL-3.0-or-later

//! Tests for webhook signature verification, provider detection, and push classification.

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

#[test]
fn classify_push_non_default_branch_skips() {
    let event = sample_push_event("bearDog", "feature/x", "main");
    let primals = &["beardog"];
    let action = classify_push(&event, primals, WebhookProvider::Forgejo);
    assert!(!action.should_harvest);
    assert!(!action.should_cascade);
    assert!(action.reason.contains("non-default branch"));
}

#[test]
fn classify_push_unknown_repo_skips() {
    let event = sample_push_event("randomRepo", "main", "main");
    let primals = &["beardog", "songbird"];
    let action = classify_push(&event, primals, WebhookProvider::Forgejo);
    assert!(!action.should_harvest);
    assert!(
        !action.should_cascade,
        "unknown repo is neither primal nor cascade"
    );
}

#[test]
fn classify_push_case_insensitive_primal_match() {
    let event = sample_push_event("BearDog", "main", "main");
    let primals = &["beardog"];
    let action = classify_push(&event, primals, WebhookProvider::Forgejo);
    assert!(action.should_harvest, "case-insensitive primal match");
}

#[test]
fn classify_push_harvest_preserves_original_repo_name() {
    let event = sample_push_event("BiomeOS", "main", "main");
    let primals = &["biomeos"];
    let action = classify_push(&event, primals, WebhookProvider::Forgejo);
    assert_eq!(action.repo_name, "BiomeOS", "preserves original casing");
}

#[test]
fn classify_push_branch_extracts_from_ref() {
    let event = sample_push_event("bearDog", "main", "main");
    let primals = &["beardog"];
    let action = classify_push(&event, primals, WebhookProvider::Forgejo);
    assert_eq!(action.branch, "main");
}

#[test]
fn verify_signature_wrong_secret_fails() {
    let body = b"test payload";
    let sig = compute_hmac_hex(b"correct-secret", body);
    assert!(verify_signature(b"wrong-secret", body, &sig).is_err());
}

#[test]
fn verify_signature_correct_secret_passes() {
    let body = b"test payload";
    let sig = compute_hmac_hex(b"mysecret", body);
    assert!(verify_signature(b"mysecret", body, &sig).is_ok());
}

#[test]
fn provider_detect_case_insensitive() {
    let headers = vec![("X-FORGEJO-SIGNATURE".to_string(), "abc".to_string())];
    let (provider, _) = WebhookProvider::detect(&headers).expect("should detect");
    assert_eq!(provider, WebhookProvider::Forgejo);
}

#[test]
fn provider_extract_github_strips_prefix() {
    assert_eq!(
        WebhookProvider::GitHub.extract_signature("sha256=deadbeef"),
        "deadbeef"
    );
}

#[test]
fn provider_extract_github_no_prefix_returns_as_is() {
    assert_eq!(
        WebhookProvider::GitHub.extract_signature("noprefix"),
        "noprefix"
    );
}

#[test]
fn constant_time_eq_basic() {
    assert!(constant_time_eq(b"hello", b"hello"));
    assert!(!constant_time_eq(b"hello", b"world"));
    assert!(!constant_time_eq(b"short", b"longer"));
}
