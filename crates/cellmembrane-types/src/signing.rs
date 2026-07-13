// SPDX-License-Identifier: AGPL-3.0-or-later

//! Depot signing types — cryptographic provenance for cascade artifacts.
//!
//! Defines the `DepotSignature` structure that attests to the integrity and
//! origin of `checksums.toml`. A signature binds a BLAKE3 digest of the
//! checksums file to an Ed25519 public key, proving that a trusted gate
//! produced or endorsed the depot contents.
//!
//! The actual signing and verification operations live in `membrane-shadow`
//! (this crate is pure data types with no crypto dependencies).

use serde::{Deserialize, Serialize};

/// Cryptographic signature over depot metadata.
///
/// Stored in `signatures.toml` alongside `checksums.toml` in the depot.
/// The signed payload is the BLAKE3 digest of the canonical `checksums.toml`
/// at the time of signing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DepotSignature {
    /// Signing algorithm (currently always `ed25519`).
    pub algorithm: SignatureAlgorithm,
    /// Hex-encoded Ed25519 public key of the signing gate.
    pub public_key: String,
    /// BLAKE3 hash of `checksums.toml` that was signed.
    pub checksums_blake3: String,
    /// Hex-encoded ed25519 signature over `checksums_blake3`.
    pub signature: String,
    /// Gate identity of the signer (e.g. "eastGate").
    pub signer_gate: String,
    /// ISO 8601 timestamp of when the signature was created.
    pub signed_at: String,
}

/// Signature algorithm for depot attestation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SignatureAlgorithm {
    /// Ed25519 (RFC 8032) — default for all ecoPrimals signing.
    Ed25519,
}

impl std::fmt::Display for SignatureAlgorithm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ed25519 => write!(f, "ed25519"),
        }
    }
}

/// Trust policy governing how fetch treats depot signatures.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DepotTrustPolicy {
    /// BLAKE3 integrity only — no signature verification (current default).
    IntegrityOnly,
    /// Verify signature if present, warn if missing.
    #[default]
    VerifyIfPresent,
    /// Require valid signature — reject unsigned or mis-signed artifacts.
    RequireSigned,
}

impl std::str::FromStr for DepotTrustPolicy {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "integrity-only" | "integrity_only" => Ok(Self::IntegrityOnly),
            "verify-if-present" | "verify_if_present" => Ok(Self::VerifyIfPresent),
            "require-signed" | "require_signed" => Ok(Self::RequireSigned),
            _ => Err(format!(
                "unknown trust policy: {s} (expected: integrity-only|verify-if-present|require-signed)"
            )),
        }
    }
}

impl std::fmt::Display for DepotTrustPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IntegrityOnly => write!(f, "integrity-only"),
            Self::VerifyIfPresent => write!(f, "verify-if-present"),
            Self::RequireSigned => write!(f, "require-signed"),
        }
    }
}

/// Wrapper for `signatures.toml` — may contain multiple signatures
/// (e.g. from different gates or re-signing events).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SignaturesFile {
    /// Ordered list of signatures (most recent first).
    #[serde(default)]
    pub signatures: Vec<DepotSignature>,
}

impl SignaturesFile {
    /// Find the most recent signature matching a given public key.
    #[must_use]
    pub fn latest_for_key(&self, public_key: &str) -> Option<&DepotSignature> {
        self.signatures.iter().find(|s| s.public_key == public_key)
    }

    /// Find the most recent signature (any key).
    #[must_use]
    pub fn latest(&self) -> Option<&DepotSignature> {
        self.signatures.first()
    }

    /// Check whether any signature covers the given `checksums_blake3` digest.
    #[must_use]
    pub fn has_matching_digest(&self, checksums_blake3: &str) -> bool {
        self.signatures
            .iter()
            .any(|s| s.checksums_blake3 == checksums_blake3)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_signature() -> DepotSignature {
        DepotSignature {
            algorithm: SignatureAlgorithm::Ed25519,
            public_key: "abc123".into(),
            checksums_blake3: "deadbeef".into(),
            signature: "sig_hex".into(),
            signer_gate: "eastGate".into(),
            signed_at: "2026-07-10T21:38:00Z".into(),
        }
    }

    #[test]
    fn roundtrip_toml() {
        let file = SignaturesFile {
            signatures: vec![sample_signature()],
        };
        let toml_str = toml::to_string(&file).unwrap();
        let parsed: SignaturesFile = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.signatures.len(), 1);
        assert_eq!(parsed.signatures[0].signer_gate, "eastGate");
        assert_eq!(parsed.signatures[0].algorithm, SignatureAlgorithm::Ed25519);
    }

    #[test]
    fn roundtrip_json() {
        let sig = sample_signature();
        let json = serde_json::to_string(&sig).unwrap();
        let parsed: DepotSignature = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.public_key, "abc123");
        assert_eq!(parsed.checksums_blake3, "deadbeef");
    }

    #[test]
    fn latest_for_key_finds_match() {
        let file = SignaturesFile {
            signatures: vec![sample_signature()],
        };
        assert!(file.latest_for_key("abc123").is_some());
        assert!(file.latest_for_key("other_key").is_none());
    }

    #[test]
    fn has_matching_digest() {
        let file = SignaturesFile {
            signatures: vec![sample_signature()],
        };
        assert!(file.has_matching_digest("deadbeef"));
        assert!(!file.has_matching_digest("cafebabe"));
    }

    #[test]
    fn default_trust_policy_is_verify_if_present() {
        assert_eq!(DepotTrustPolicy::default(), DepotTrustPolicy::VerifyIfPresent);
    }

    #[test]
    fn empty_signatures_file() {
        let file = SignaturesFile::default();
        assert!(file.latest().is_none());
        assert!(!file.has_matching_digest("anything"));
    }

    #[test]
    fn algorithm_display() {
        assert_eq!(format!("{}", SignatureAlgorithm::Ed25519), "ed25519");
    }

    #[test]
    fn trust_policy_from_str_kebab_case() {
        assert_eq!(
            "integrity-only".parse::<DepotTrustPolicy>().unwrap(),
            DepotTrustPolicy::IntegrityOnly
        );
        assert_eq!(
            "verify-if-present".parse::<DepotTrustPolicy>().unwrap(),
            DepotTrustPolicy::VerifyIfPresent
        );
        assert_eq!(
            "require-signed".parse::<DepotTrustPolicy>().unwrap(),
            DepotTrustPolicy::RequireSigned
        );
    }

    #[test]
    fn trust_policy_from_str_snake_case() {
        assert_eq!(
            "integrity_only".parse::<DepotTrustPolicy>().unwrap(),
            DepotTrustPolicy::IntegrityOnly
        );
        assert_eq!(
            "verify_if_present".parse::<DepotTrustPolicy>().unwrap(),
            DepotTrustPolicy::VerifyIfPresent
        );
        assert_eq!(
            "require_signed".parse::<DepotTrustPolicy>().unwrap(),
            DepotTrustPolicy::RequireSigned
        );
    }

    #[test]
    fn trust_policy_from_str_invalid() {
        assert!("bogus".parse::<DepotTrustPolicy>().is_err());
        assert!("".parse::<DepotTrustPolicy>().is_err());
    }

    #[test]
    fn trust_policy_display_roundtrip() {
        for policy in [
            DepotTrustPolicy::IntegrityOnly,
            DepotTrustPolicy::VerifyIfPresent,
            DepotTrustPolicy::RequireSigned,
        ] {
            let s = policy.to_string();
            assert_eq!(s.parse::<DepotTrustPolicy>().unwrap(), policy);
        }
    }
}
