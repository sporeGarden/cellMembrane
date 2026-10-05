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

// ── Defense Posture ─────────────────────────────────────────────────

/// Tit-for-tat escalation posture for fleet defense.
///
/// Ordered from least to most aggressive. The immune system starts at
/// `Observe` and escalates on repeat defection (same behavioral shape
/// returning after rejection). De-escalates one level per `forgive_window`
/// of silence.
///
/// The ordering is significant: `PartialOrd`/`Ord` derive means
/// `Observe < WarnRoute < SlowDegrade < Scatter < Vanish`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DefensePosture {
    /// P0: Normal traffic, antibodies watch but do not intervene.
    Observe,
    /// P1: 403 + warning page + redirect to external membrane (GitHub).
    WarnRoute,
    /// P2: Tarpit — artificial delays, partial content, randomized timing.
    /// Wastes attacker compute without revealing they are detected.
    SlowDegrade,
    /// P3: Plausible-but-wrong content. Poisons training data.
    /// Anti-AI defense: the shape looks correct, the content is garbage.
    Scatter,
    /// P4: Connection reset / abort. The membrane vanishes.
    /// Real humans still see everything — only antibody-matched shapes disappear.
    Vanish,
}

impl DefensePosture {
    /// Escalate one level (capped at Vanish).
    #[must_use]
    pub const fn escalate(self) -> Self {
        match self {
            Self::Observe => Self::WarnRoute,
            Self::WarnRoute => Self::SlowDegrade,
            Self::SlowDegrade => Self::Scatter,
            Self::Scatter => Self::Vanish,
            Self::Vanish => Self::Vanish,
        }
    }

    /// De-escalate one level (capped at Observe).
    #[must_use]
    pub const fn deescalate(self) -> Self {
        match self {
            Self::Observe => Self::Observe,
            Self::WarnRoute => Self::Observe,
            Self::SlowDegrade => Self::WarnRoute,
            Self::Scatter => Self::SlowDegrade,
            Self::Vanish => Self::Scatter,
        }
    }

    /// Numeric level (0-4) for threshold comparisons.
    #[must_use]
    pub const fn level(self) -> u8 {
        match self {
            Self::Observe => 0,
            Self::WarnRoute => 1,
            Self::SlowDegrade => 2,
            Self::Scatter => 3,
            Self::Vanish => 4,
        }
    }
}

impl Default for DefensePosture {
    fn default() -> Self {
        Self::Observe
    }
}

/// Trust descriptor for bearDog ecosystem mapping.
///
/// Provides the semantic bridge between defense postures and the bearDog
/// trust spectrum. bearDog's ecosystem evolution engine consumes these
/// values to construct its own `EcosystemMembership` variants.
#[derive(Debug, Clone, PartialEq)]
pub struct TrustDescriptor {
    /// Trust level on bearDog's 0.0–1.0 spectrum.
    pub trust_level: f64,
    /// Which `EcosystemMembership` variant this maps to.
    pub membership_hint: &'static str,
    /// Which `SymbiosisType` this implies.
    pub symbiosis_hint: &'static str,
    /// Whether healing/restoration is available.
    pub restoration_available: bool,
}

impl DefensePosture {
    /// Map this posture to a bearDog trust descriptor.
    ///
    /// The descriptor tells bearDog's ecosystem evolution engine what
    /// trust level and membership category to assign to traffic matching
    /// this posture's antibody.
    ///
    /// # Mapping
    ///
    /// | Posture      | Trust  | Membership              | Symbiosis     |
    /// |--------------|--------|-------------------------|---------------|
    /// | Observe      | 0.7    | `ActiveContributor`     | Commensal     |
    /// | WarnRoute    | 0.45   | `CautiousInteraction`   | Commensal     |
    /// | SlowDegrade  | 0.25   | `CautiousInteraction`   | Competitive   |
    /// | Scatter      | 0.10   | `EcosystemProtection`   | Protective    |
    /// | Vanish       | 0.0    | `EcosystemProtection`   | Protective    |
    #[must_use]
    pub const fn trust_descriptor(&self) -> TrustDescriptor {
        match self {
            Self::Observe => TrustDescriptor {
                trust_level: 0.7,
                membership_hint: "ActiveContributor",
                symbiosis_hint: "Commensal",
                restoration_available: true,
            },
            Self::WarnRoute => TrustDescriptor {
                trust_level: 0.45,
                membership_hint: "CautiousInteraction",
                symbiosis_hint: "Commensal",
                restoration_available: true,
            },
            Self::SlowDegrade => TrustDescriptor {
                trust_level: 0.25,
                membership_hint: "CautiousInteraction",
                symbiosis_hint: "Competitive",
                restoration_available: true,
            },
            Self::Scatter => TrustDescriptor {
                trust_level: 0.10,
                membership_hint: "EcosystemProtection",
                symbiosis_hint: "Protective",
                restoration_available: true,
            },
            Self::Vanish => TrustDescriptor {
                trust_level: 0.0,
                membership_hint: "EcosystemProtection",
                symbiosis_hint: "Protective",
                restoration_available: false,
            },
        }
    }
}

impl std::fmt::Display for DefensePosture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Observe => write!(f, "observe"),
            Self::WarnRoute => write!(f, "warn_route"),
            Self::SlowDegrade => write!(f, "slow_degrade"),
            Self::Scatter => write!(f, "scatter"),
            Self::Vanish => write!(f, "vanish"),
        }
    }
}

/// Default forgive window: 4 hours (14400 seconds).
pub const DEFAULT_FORGIVE_WINDOW_SECS: u64 = 14_400;

/// Escalation thresholds: consecutive defection windows to trigger each level.
pub const ESCALATE_TO_SLOW: u32 = 1;    // P1→P2: came back after 403
/// Consecutive defection windows to escalate from P2 to P3.
pub const ESCALATE_TO_SCATTER: u32 = 3; // P2→P3: persists despite tarpit
/// Consecutive defection windows to escalate from P3 to P4.
pub const ESCALATE_TO_VANISH: u32 = 6;  // P3→P4: persists despite scatter

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

    // ── Escalation state (tit-for-tat + forgive-1) ──────────────

    /// Current defense posture for this antibody's behavioral shape.
    #[serde(default)]
    pub escalation: DefensePosture,
    /// When the last defection (matched observation) occurred.
    #[serde(default)]
    pub last_defection_epoch: u64,
    /// Consecutive observation windows with a match (resets on forgive).
    #[serde(default)]
    pub defection_count: u32,
    /// Seconds of silence before de-escalating one level.
    #[serde(default = "default_forgive_window")]
    pub forgive_window_secs: u64,
}

fn default_forgive_window() -> u64 {
    DEFAULT_FORGIVE_WINDOW_SECS
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

    /// Tick the escalation state machine.
    ///
    /// Call once per observation window with `matched = true` if this
    /// antibody matched any observation in the window, `false` otherwise.
    ///
    /// Returns the previous posture if it changed (for audit logging).
    pub fn tick_escalation(&mut self, matched: bool, now_epoch: u64) -> Option<DefensePosture> {
        let prev = self.escalation;

        if matched {
            self.last_defection_epoch = now_epoch;
            self.defection_count = self.defection_count.saturating_add(1);

            let new = match self.escalation {
                DefensePosture::Observe => DefensePosture::WarnRoute,
                DefensePosture::WarnRoute => {
                    if self.deception.ignores_rejection
                        || self.defection_count >= ESCALATE_TO_SLOW
                    {
                        DefensePosture::SlowDegrade
                    } else {
                        DefensePosture::WarnRoute
                    }
                }
                DefensePosture::SlowDegrade => {
                    if self.defection_count >= ESCALATE_TO_SCATTER {
                        DefensePosture::Scatter
                    } else {
                        DefensePosture::SlowDegrade
                    }
                }
                DefensePosture::Scatter => {
                    if self.defection_count >= ESCALATE_TO_VANISH {
                        DefensePosture::Vanish
                    } else {
                        DefensePosture::Scatter
                    }
                }
                DefensePosture::Vanish => DefensePosture::Vanish,
            };

            self.escalation = new;
        } else {
            // Forgive-1: de-escalate if silent for forgive_window
            let silence = now_epoch.saturating_sub(self.last_defection_epoch);
            if silence >= self.forgive_window_secs && self.escalation != DefensePosture::Observe {
                self.escalation = self.escalation.deescalate();
                self.defection_count = 0;
                // Reset the timer so next forgive needs another full window
                self.last_defection_epoch = now_epoch;
            }
        }

        if self.escalation != prev {
            Some(prev)
        } else {
            None
        }
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
            escalation: DefensePosture::Observe,
            last_defection_epoch: 0,
            defection_count: 0,
            forgive_window_secs: DEFAULT_FORGIVE_WINDOW_SECS,
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
        assert_eq!(parsed.escalation, DefensePosture::Observe);
    }

    #[test]
    fn serde_backwards_compat() {
        // Old antibodies without escalation fields should deserialize with defaults
        let json = r#"{"id":"old","ua_fingerprint":{"ua_count":2,"top_ua_pct":0.5,"platform_split":[0.5,0.5,0.0]},"timing":{"mean_interval_ms":6000,"interval_cv":0.15},"path_pattern":{"commit_url_pct":0.9,"single_page_pct":0.9,"has_referrer_pct":0.0},"deception":{"hides_identity":true,"rotates_ips":true,"ignores_rejection":true,"encoding_uniform":true},"confidence":0.8,"first_seen_epoch":1000,"last_matched_epoch":2000,"match_count":10}"#;
        let parsed: FleetAntibody = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.escalation, DefensePosture::Observe);
        assert_eq!(parsed.defection_count, 0);
        assert_eq!(parsed.forgive_window_secs, DEFAULT_FORGIVE_WINDOW_SECS);
    }

    #[test]
    fn posture_escalation_ladder() {
        assert_eq!(DefensePosture::Observe.escalate(), DefensePosture::WarnRoute);
        assert_eq!(DefensePosture::WarnRoute.escalate(), DefensePosture::SlowDegrade);
        assert_eq!(DefensePosture::SlowDegrade.escalate(), DefensePosture::Scatter);
        assert_eq!(DefensePosture::Scatter.escalate(), DefensePosture::Vanish);
        assert_eq!(DefensePosture::Vanish.escalate(), DefensePosture::Vanish);
    }

    #[test]
    fn posture_deescalation_ladder() {
        assert_eq!(DefensePosture::Vanish.deescalate(), DefensePosture::Scatter);
        assert_eq!(DefensePosture::Scatter.deescalate(), DefensePosture::SlowDegrade);
        assert_eq!(DefensePosture::SlowDegrade.deescalate(), DefensePosture::WarnRoute);
        assert_eq!(DefensePosture::WarnRoute.deescalate(), DefensePosture::Observe);
        assert_eq!(DefensePosture::Observe.deescalate(), DefensePosture::Observe);
    }

    #[test]
    fn tick_escalation_first_match_goes_to_warn() {
        let mut ab = sample_antibody();
        let prev = ab.tick_escalation(true, 5000);
        assert_eq!(prev, Some(DefensePosture::Observe));
        assert_eq!(ab.escalation, DefensePosture::WarnRoute);
    }

    #[test]
    fn tick_escalation_ignores_rejection_fast_tracks() {
        let mut ab = sample_antibody();
        ab.escalation = DefensePosture::WarnRoute;
        // ignores_rejection is true on sample_antibody
        let prev = ab.tick_escalation(true, 5000);
        assert_eq!(prev, Some(DefensePosture::WarnRoute));
        assert_eq!(ab.escalation, DefensePosture::SlowDegrade);
    }

    #[test]
    fn tick_escalation_full_ladder() {
        let mut ab = sample_antibody();
        let mut t = 1000u64;

        // P0 → P1
        ab.tick_escalation(true, t);
        assert_eq!(ab.escalation, DefensePosture::WarnRoute);

        // P1 → P2 (ignores_rejection = true)
        t += 60;
        ab.tick_escalation(true, t);
        assert_eq!(ab.escalation, DefensePosture::SlowDegrade);

        // P2 → P3 (need ESCALATE_TO_SCATTER consecutive)
        for _ in 0..ESCALATE_TO_SCATTER {
            t += 60;
            ab.tick_escalation(true, t);
        }
        assert_eq!(ab.escalation, DefensePosture::Scatter);

        // P3 → P4 (need ESCALATE_TO_VANISH consecutive)
        for _ in 0..ESCALATE_TO_VANISH {
            t += 60;
            ab.tick_escalation(true, t);
        }
        assert_eq!(ab.escalation, DefensePosture::Vanish);
    }

    #[test]
    fn tick_forgive_deescalates() {
        let mut ab = sample_antibody();
        ab.escalation = DefensePosture::SlowDegrade;
        ab.last_defection_epoch = 1000;

        // Not enough silence
        let prev = ab.tick_escalation(false, 1000 + ab.forgive_window_secs - 1);
        assert!(prev.is_none());
        assert_eq!(ab.escalation, DefensePosture::SlowDegrade);

        // Enough silence → forgive one level
        let prev = ab.tick_escalation(false, 1000 + ab.forgive_window_secs);
        assert_eq!(prev, Some(DefensePosture::SlowDegrade));
        assert_eq!(ab.escalation, DefensePosture::WarnRoute);
        assert_eq!(ab.defection_count, 0);
    }

    #[test]
    fn posture_ordering() {
        assert!(DefensePosture::Observe < DefensePosture::WarnRoute);
        assert!(DefensePosture::WarnRoute < DefensePosture::SlowDegrade);
        assert!(DefensePosture::SlowDegrade < DefensePosture::Scatter);
        assert!(DefensePosture::Scatter < DefensePosture::Vanish);
    }

    #[test]
    fn trust_descriptor_monotonically_decreasing() {
        let postures = [
            DefensePosture::Observe,
            DefensePosture::WarnRoute,
            DefensePosture::SlowDegrade,
            DefensePosture::Scatter,
            DefensePosture::Vanish,
        ];
        for i in 0..postures.len() - 1 {
            let a = postures[i].trust_descriptor();
            let b = postures[i + 1].trust_descriptor();
            assert!(
                a.trust_level > b.trust_level,
                "{:?} trust ({}) should exceed {:?} trust ({})",
                postures[i], a.trust_level, postures[i + 1], b.trust_level,
            );
        }
    }

    #[test]
    fn trust_descriptor_beardog_mapping() {
        // Observe → still trusted, active contributor
        let d = DefensePosture::Observe.trust_descriptor();
        assert_eq!(d.membership_hint, "ActiveContributor");
        assert!(d.trust_level >= 0.6);
        assert!(d.restoration_available);

        // WarnRoute → cautious, first signal
        let d = DefensePosture::WarnRoute.trust_descriptor();
        assert_eq!(d.membership_hint, "CautiousInteraction");
        assert!(d.trust_level >= 0.3 && d.trust_level < 0.6);
        assert!(d.restoration_available);

        // Scatter → ecosystem protection
        let d = DefensePosture::Scatter.trust_descriptor();
        assert_eq!(d.membership_hint, "EcosystemProtection");
        assert!(d.trust_level < 0.2);
        assert!(d.restoration_available);

        // Vanish → full protection, no restoration
        let d = DefensePosture::Vanish.trust_descriptor();
        assert_eq!(d.membership_hint, "EcosystemProtection");
        assert_eq!(d.trust_level, 0.0);
        assert!(!d.restoration_available);
    }
}
