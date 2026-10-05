// SPDX-License-Identifier: AGPL-3.0-or-later

//! Fleet antibody types — adaptive immune pattern memory.
//!
//! An antibody stores the **shape** that recognizes a pathogenic fleet,
//! not the fleet itself. No IPs are stored — IPs are ephemeral routing
//! decisions made by the Caddy layer. The antibody persists the behavioral
//! fingerprint so the same fleet is recognized even from new IPs.
//!
//! ## Commensal vs Pathogenic
//!
//! Antibodies only encode **pathogenic** patterns. A commensal bot that
//! identifies itself honestly (Googlebot, `ClaudeBot`, `AhrefsBot`) can never
//! match an antibody because antibodies key on **deception signals**: UA
//! uniformity across a population, identity hiding, and rule-ignoring
//! persistence.
//!
//! ## Usage
//!
//! Shared between:
//! - `skunky-ingest` — generates `FleetObservation` from log analysis
//! - `skunk-bat-core` — `FleetDetector` creates antibodies from observations
//! - `membrane-shadow` — matches incoming requests against stored antibodies

use serde::{Deserialize, Serialize};

// ── Fleet Antibody ──────────────────────────────────────────────────

/// A behavioral fingerprint that recognizes a pathogenic fleet pattern.
///
/// The antibody does not store IPs — it stores the **shape** of the
/// fleet's behavior. Same UA distribution + same timing + same path
/// pattern = same fleet, even from entirely new IPs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FleetAntibody {
    /// Deterministic hash of the pattern components.
    pub id: String,
    /// UA distribution signature across the fleet population.
    pub ua_fingerprint: UaFingerprint,
    /// Timing regularity of requests.
    pub timing: TimingSignature,
    /// Path access pattern.
    pub path_pattern: PathPattern,
    /// What makes this pathogenic, not commensal.
    pub deception: DeceptionSignals,
    /// Detection confidence (0.0–1.0). Decays over time without matches.
    pub confidence: f64,
    /// When this antibody was first created.
    pub first_seen_epoch: u64,
    /// When this antibody last matched a request.
    pub last_matched_epoch: u64,
    /// Total number of times this antibody has matched.
    pub match_count: u64,
}

/// What distinguishes a pathogen from a commensal.
///
/// A commensal identifies itself honestly. A pathogen hides.
/// These signals are the basis for antibody generation — if none
/// are true, the system is commensal and should be guided, not blocked.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct DeceptionSignals {
    /// Uses human-mimicking UA strings (no bot identifier in UA).
    pub hides_identity: bool,
    /// Population shows many IPs each hitting 1 page (rotating pool).
    pub rotates_ips: bool,
    /// Continues hitting after receiving 403/429 responses.
    pub ignores_rejection: bool,
    /// Entire population has near-identical Accept-Encoding/Language.
    pub encoding_uniform: bool,
}

/// UA distribution signature across a fleet population.
///
/// Real human traffic has diverse UAs. Fleets use 2-4 UAs split
/// nearly evenly. The fingerprint captures this population shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UaFingerprint {
    /// Number of distinct UA strings in the population.
    pub ua_count: u8,
    /// Fraction of requests using the most common UA (0.0–1.0).
    pub top_ua_pct: f32,
    /// Platform distribution: `[mac_pct, windows_pct, linux_pct]`.
    pub platform_split: [f32; 3],
}

/// Timing regularity of fleet requests.
///
/// Human browsing has high variance in inter-arrival times.
/// Fleets have metronomic regularity (low coefficient of variation).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimingSignature {
    /// Mean interval between requests in milliseconds.
    pub mean_interval_ms: u32,
    /// Coefficient of variation of inter-arrival times.
    /// < 0.3 is suspiciously regular (metronomic).
    pub interval_cv: f32,
}

/// Path access pattern of a fleet.
///
/// Fleets systematically walk commit-level URLs. Humans browse
/// current-branch content with referrers from navigation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathPattern {
    /// Fraction of requests targeting commit-hash URLs (0.0–1.0).
    pub commit_url_pct: f32,
    /// Fraction of sessions that visit exactly one page (0.0–1.0).
    pub single_page_pct: f32,
    /// Fraction of requests carrying a Referer header (0.0–1.0).
    pub has_referrer_pct: f32,
}

// ── Fleet Observation ───────────────────────────────────────────────

/// Population-level fleet observation emitted by skunky-ingest.
///
/// Unlike `Observation` (per-IP), this captures the statistical
/// properties of the entire traffic population in a time window.
/// The `FleetDetector` in skunkBat uses these to create antibodies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FleetObservation {
    /// Timestamp of the observation window end.
    pub timestamp_epoch: u64,
    /// Total requests in the window (across all IPs).
    pub total_requests: u64,
    /// Number of unique source IPs.
    pub unique_ips: u32,
    /// UA distribution across all requests.
    pub ua_fingerprint: UaFingerprint,
    /// Timing regularity across the population.
    pub timing: TimingSignature,
    /// Path access pattern across all requests.
    pub path_pattern: PathPattern,
    /// Deception signals observed in the population.
    pub deception: DeceptionSignals,
    /// Per-IP request count distribution: `[1-page, 2-3, 4-10, 11+]`.
    pub depth_distribution: [u32; 4],
    /// Number of IPs that received only 403/429 responses.
    pub rejected_ips: u32,
}

impl FleetAntibody {
    /// Check whether this antibody matches a fleet observation.
    ///
    /// Matching is fuzzy — the observation's fingerprint must be
    /// "close enough" to the antibody's stored pattern. Each dimension
    /// contributes a similarity score; the overall match requires
    /// all dimensions above threshold.
    #[must_use]
    pub fn matches_observation(&self, obs: &FleetObservation) -> bool {
        if self.confidence < 0.1 {
            return false;
        }

        let ua_match = ua_similarity(&self.ua_fingerprint, &obs.ua_fingerprint) > 0.6;
        let timing_match = timing_similarity(&self.timing, &obs.timing) > 0.5;
        let path_match = path_similarity(&self.path_pattern, &obs.path_pattern) > 0.6;
        let deception_match = deception_overlap(&self.deception, &obs.deception) >= 2;

        ua_match && path_match && (timing_match || deception_match)
    }

    /// Apply confidence decay. Call periodically (e.g. daily).
    /// Antibodies that haven't matched recently lose confidence
    /// and eventually become inert.
    pub fn decay(&mut self, decay_factor: f64) {
        self.confidence *= decay_factor;
    }
}

/// Cosine-like similarity between two UA fingerprints.
fn ua_similarity(a: &UaFingerprint, b: &UaFingerprint) -> f32 {
    let count_sim = 1.0 - (f32::from(a.ua_count).max(1.0) - f32::from(b.ua_count).max(1.0)).abs()
        / f32::from(a.ua_count).max(f32::from(b.ua_count)).max(1.0);
    let pct_sim = 1.0 - (a.top_ua_pct - b.top_ua_pct).abs();
    let platform_sim: f32 = a
        .platform_split
        .iter()
        .zip(b.platform_split.iter())
        .map(|(pa, pb)| 1.0 - (pa - pb).abs())
        .sum::<f32>()
        / 3.0;
    (count_sim + pct_sim + platform_sim) / 3.0
}

/// Similarity between timing signatures.
fn timing_similarity(a: &TimingSignature, b: &TimingSignature) -> f32 {
    let interval_sim = 1.0
        - (a.mean_interval_ms as f32 - b.mean_interval_ms as f32).abs()
            / a.mean_interval_ms.max(b.mean_interval_ms).max(1) as f32;
    let cv_sim = 1.0 - (a.interval_cv - b.interval_cv).abs();
    (interval_sim + cv_sim) / 2.0
}

/// Similarity between path patterns.
fn path_similarity(a: &PathPattern, b: &PathPattern) -> f32 {
    let commit_sim = 1.0 - (a.commit_url_pct - b.commit_url_pct).abs();
    let single_sim = 1.0 - (a.single_page_pct - b.single_page_pct).abs();
    let ref_sim = 1.0 - (a.has_referrer_pct - b.has_referrer_pct).abs();
    (commit_sim + single_sim + ref_sim) / 3.0
}

/// Count of overlapping deception signals.
fn deception_overlap(a: &DeceptionSignals, b: &DeceptionSignals) -> u8 {
    let mut overlap = 0u8;
    if a.hides_identity && b.hides_identity {
        overlap += 1;
    }
    if a.rotates_ips && b.rotates_ips {
        overlap += 1;
    }
    if a.ignores_rejection && b.ignores_rejection {
        overlap += 1;
    }
    if a.encoding_uniform && b.encoding_uniform {
        overlap += 1;
    }
    overlap
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_antibody() -> FleetAntibody {
        FleetAntibody {
            id: "test-antibody-001".to_string(),
            ua_fingerprint: UaFingerprint {
                ua_count: 2,
                top_ua_pct: 0.50,
                platform_split: [0.50, 0.50, 0.0],
            },
            timing: TimingSignature {
                mean_interval_ms: 6000,
                interval_cv: 0.15,
            },
            path_pattern: PathPattern {
                commit_url_pct: 0.92,
                single_page_pct: 0.95,
                has_referrer_pct: 0.0,
            },
            deception: DeceptionSignals {
                hides_identity: true,
                rotates_ips: true,
                ignores_rejection: true,
                encoding_uniform: true,
            },
            confidence: 0.9,
            first_seen_epoch: 1000,
            last_matched_epoch: 2000,
            match_count: 50,
        }
    }

    fn similar_observation() -> FleetObservation {
        FleetObservation {
            timestamp_epoch: 3000,
            total_requests: 200,
            unique_ips: 180,
            ua_fingerprint: UaFingerprint {
                ua_count: 3,
                top_ua_pct: 0.45,
                platform_split: [0.45, 0.50, 0.05],
            },
            timing: TimingSignature {
                mean_interval_ms: 5500,
                interval_cv: 0.18,
            },
            path_pattern: PathPattern {
                commit_url_pct: 0.88,
                single_page_pct: 0.90,
                has_referrer_pct: 0.01,
            },
            deception: DeceptionSignals {
                hides_identity: true,
                rotates_ips: true,
                ignores_rejection: true,
                encoding_uniform: false,
            },
            depth_distribution: [170, 8, 2, 0],
            rejected_ips: 160,
        }
    }

    #[test]
    fn antibody_matches_similar_fleet() {
        let ab = sample_antibody();
        let obs = similar_observation();
        assert!(ab.matches_observation(&obs));
    }

    #[test]
    fn antibody_rejects_human_traffic() {
        let ab = sample_antibody();
        let human_obs = FleetObservation {
            timestamp_epoch: 3000,
            total_requests: 50,
            unique_ips: 30,
            ua_fingerprint: UaFingerprint {
                ua_count: 25,
                top_ua_pct: 0.08,
                platform_split: [0.30, 0.55, 0.15],
            },
            timing: TimingSignature {
                mean_interval_ms: 45_000,
                interval_cv: 1.8,
            },
            path_pattern: PathPattern {
                commit_url_pct: 0.02,
                single_page_pct: 0.20,
                has_referrer_pct: 0.70,
            },
            deception: DeceptionSignals {
                hides_identity: false,
                rotates_ips: false,
                ignores_rejection: false,
                encoding_uniform: false,
            },
            depth_distribution: [10, 12, 6, 2],
            rejected_ips: 0,
        };
        assert!(!ab.matches_observation(&human_obs));
    }

    #[test]
    fn decayed_antibody_does_not_match() {
        let mut ab = sample_antibody();
        for _ in 0..50 {
            ab.decay(0.9);
        }
        assert!(ab.confidence < 0.1);
        assert!(!ab.matches_observation(&similar_observation()));
    }

    #[test]
    fn serde_roundtrip() {
        let ab = sample_antibody();
        let json = serde_json::to_string(&ab).unwrap();
        let parsed: FleetAntibody = serde_json::from_str(&json).unwrap();
        assert_eq!(ab.id, parsed.id);
        assert_eq!(ab.match_count, parsed.match_count);
    }
}
