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
/// `Observe < WarnRoute < SlowDegrade < Scatter < Vanish < Disperse`.
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
    /// P5: Active confusion — skunk spray. Maximally-wrong responses that
    /// waste attacker compute and poison any downstream processing.
    /// Wrong MIME types, garbled structure, fake auth flows, random redirects.
    /// Friendly systems (WireGuard, authenticated) bypass via recognition.
    Disperse,
}

impl DefensePosture {
    /// Escalate one level (capped at Disperse).
    #[must_use]
    pub const fn escalate(self) -> Self {
        match self {
            Self::Observe => Self::WarnRoute,
            Self::WarnRoute => Self::SlowDegrade,
            Self::SlowDegrade => Self::Scatter,
            Self::Scatter => Self::Vanish,
            Self::Vanish => Self::Disperse,
            Self::Disperse => Self::Disperse,
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
            Self::Disperse => Self::Vanish,
        }
    }

    /// Numeric level (0-5) for threshold comparisons.
    #[must_use]
    pub const fn level(self) -> u8 {
        match self {
            Self::Observe => 0,
            Self::WarnRoute => 1,
            Self::SlowDegrade => 2,
            Self::Scatter => 3,
            Self::Vanish => 4,
            Self::Disperse => 5,
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
            Self::Disperse => TrustDescriptor {
                trust_level: 0.0,
                membership_hint: "EcosystemProtection",
                symbiosis_hint: "ActiveDefense",
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
            Self::Disperse => write!(f, "disperse"),
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
/// Cumulative defection count to escalate from P4 to P5 (skunk spray).
/// With ignores_rejection=true, reaching Vanish takes ~11 defections,
/// so 20 means ~9 more windows of persistence at Vanish before Disperse.
pub const ESCALATE_TO_DISPERSE: u32 = 20; // P4→P5: persists despite vanish

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
///
/// ## Header Poverty Signal (Wave 165f)
///
/// Real Chrome 145+ sends 11+ headers per request (Sec-Ch-Ua, Sec-Fetch-*,
/// Priority, Accept-Language, etc.). The fleet sends only 3 (Accept,
/// Accept-Encoding, User-Agent). `header_poverty = true` when >80% of
/// a Chrome-UA population is missing mandatory browser headers.
///
/// ## Chrome Impersonation Signal
///
/// `chrome_impersonation = true` when the population claims Chrome UA
/// but is missing Sec-Fetch-Mode and/or Sec-Ch-Ua — headers that Chrome
/// has sent since versions 76 and 89 respectively. This is the definitive
/// proof that the HTTP client is not Chrome.
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
    /// Population with Chrome UAs missing mandatory Sec-Fetch-Mode / Sec-Ch-Ua
    /// headers — definitive proof of HTTP client impersonating Chrome.
    #[serde(default)]
    pub chrome_impersonation: bool,
    /// >80% of requests have fewer than 6 distinct header types — the
    /// population is using a bare HTTP library, not a browser.
    #[serde(default)]
    pub header_poverty: bool,
    /// >80% of Chrome-UA population uses a version 5+ behind current
    /// stable. Real Chrome auto-updates — hardcoded UA string.
    #[serde(default)]
    pub stale_chrome: bool,
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
                DefensePosture::Vanish => {
                    if self.defection_count >= ESCALATE_TO_DISPERSE {
                        DefensePosture::Disperse
                    } else {
                        DefensePosture::Vanish
                    }
                }
                DefensePosture::Disperse => DefensePosture::Disperse,
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
    if a.chrome_impersonation && b.chrome_impersonation {
        overlap += 1;
    }
    if a.header_poverty && b.header_poverty {
        overlap += 1;
    }
    if a.stale_chrome && b.stale_chrome {
        overlap += 1;
    }
    overlap
}

// ── Opsonize Tag ────────────────────────────────────────────────────

/// Gossip-propagated opsonization tag — marks a behavioral pattern for
/// cross-gate immune response.
///
/// When a gate detects fleet behavior, it creates an `OpsonizeTag` and
/// gossips it via swarmVine's `defense` topic as `defense.opsonize:<hash>`.
/// Other gates receive the tag and activate their own immune response
/// (content_gate + scatter) for matching behavioral patterns.
///
/// The tag accumulates **detectors** — each sub-antibody that independently
/// confirms the same behavioral hash adds its name. More detectors = higher
/// confidence = richer poison response.
///
/// ## No IPs stored
///
/// Like antibodies, opsonize tags contain ZERO IP addresses. The
/// behavioral hash is computed from invariant population-level features
/// (the conserved epitope). The fleet can rotate IPs infinitely but
/// cannot change the behavioral hash without changing their purpose.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpsonizeTag {
    /// Stable behavioral hash — computed from invariant features.
    /// Same fleet behavior → same hash, regardless of IP rotation.
    pub behavioral_hash: String,
    /// Sub-antibodies that independently detected this pattern.
    /// Each string is a detector name (e.g. "content_gate", "stale_chrome").
    /// More detectors = higher confidence.
    pub detectors: Vec<String>,
    /// Combined confidence across all detectors (0.0–1.0).
    pub confidence: f64,
    /// Gate that first created this tag.
    pub origin_gate: String,
    /// Behavioral invariants that define the hash.
    pub invariants: BehavioralInvariants,
    /// Recommended defense response.
    pub response: OpsonizeResponse,
    /// When this tag was first created.
    pub created_epoch: u64,
    /// When this tag was last confirmed by a new observation.
    pub last_confirmed_epoch: u64,
    /// Total observation windows that matched this hash.
    pub match_count: u64,
}

/// Quantized behavioral invariants — the conserved epitope.
///
/// These values are bucketed into bands so that minor observation-to-
/// observation variation doesn't change the hash. The fleet can't change
/// these without changing their purpose.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehavioralInvariants {
    /// Whether deep content paths dominate (commit, src, raw, etc.).
    pub deep_content_dominant: bool,
    /// Whether session cookies are absent from the population.
    pub cookieless: bool,
    /// Single-page session ratio band: "high" (>0.8), "medium" (0.4-0.8), "low" (<0.4).
    pub session_depth: String,
    /// UA diversity band: "narrow" (1-5), "moderate" (6-20), "diverse" (21+).
    pub ua_diversity: String,
    /// IP rotation pattern: "rotating" (>0.8 single-use), "sticky" (<0.3), "mixed".
    pub ip_pattern: String,
    /// Primary deception signals active.
    pub deception_flags: Vec<String>,
}

/// Recommended response for opsonized requests.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpsonizeResponse {
    /// Serve scatter (poison) content at the given ratio.
    Scatter { ratio: f32 },
    /// Abort the connection (zero signal).
    Abort,
    /// Full vanish — connection reset, no trace.
    Vanish,
}

/// Compute a stable behavioral hash from a fleet observation.
///
/// The hash is computed from **quantized** invariant features so that
/// minor variations between observation windows don't change it. This
/// is the conserved epitope — the behavioral constant that persists
/// across IP rotation, UA changes, and timing drift.
///
/// No IPs are included in the hash computation.
#[must_use]
pub fn behavioral_hash(obs: &FleetObservation) -> String {
    use std::hash::{Hash, Hasher};

    // Quantize continuous values into discrete bands
    let commit_band = quantize_pct(obs.path_pattern.commit_url_pct);
    let single_page_band = quantize_pct(obs.path_pattern.single_page_pct);
    let referrer_band = quantize_pct(obs.path_pattern.has_referrer_pct);
    let ua_band = quantize_ua_count(obs.ua_fingerprint.ua_count);
    let top_ua_band = quantize_pct(obs.ua_fingerprint.top_ua_pct);
    let cv_band = quantize_cv(obs.timing.interval_cv);

    // Hash the quantized invariants
    let mut hasher = std::hash::DefaultHasher::new();
    commit_band.hash(&mut hasher);
    single_page_band.hash(&mut hasher);
    referrer_band.hash(&mut hasher);
    ua_band.hash(&mut hasher);
    top_ua_band.hash(&mut hasher);
    cv_band.hash(&mut hasher);
    obs.deception.hides_identity.hash(&mut hasher);
    obs.deception.rotates_ips.hash(&mut hasher);
    obs.deception.ignores_rejection.hash(&mut hasher);
    obs.deception.encoding_uniform.hash(&mut hasher);
    obs.deception.chrome_impersonation.hash(&mut hasher);
    obs.deception.header_poverty.hash(&mut hasher);
    obs.deception.stale_chrome.hash(&mut hasher);

    format!("{:016x}", hasher.finish())
}

/// Extract behavioral invariants from a fleet observation.
#[must_use]
pub fn extract_invariants(obs: &FleetObservation) -> BehavioralInvariants {
    let mut deception_flags = Vec::new();
    if obs.deception.hides_identity { deception_flags.push("hides_identity".to_string()); }
    if obs.deception.rotates_ips { deception_flags.push("rotates_ips".to_string()); }
    if obs.deception.ignores_rejection { deception_flags.push("ignores_rejection".to_string()); }
    if obs.deception.encoding_uniform { deception_flags.push("encoding_uniform".to_string()); }
    if obs.deception.chrome_impersonation { deception_flags.push("chrome_impersonation".to_string()); }
    if obs.deception.header_poverty { deception_flags.push("header_poverty".to_string()); }
    if obs.deception.stale_chrome { deception_flags.push("stale_chrome".to_string()); }

    BehavioralInvariants {
        deep_content_dominant: obs.path_pattern.commit_url_pct > 0.5,
        cookieless: obs.path_pattern.has_referrer_pct < 0.1,
        session_depth: if obs.path_pattern.single_page_pct > 0.8 {
            "high".to_string()
        } else if obs.path_pattern.single_page_pct > 0.4 {
            "medium".to_string()
        } else {
            "low".to_string()
        },
        ua_diversity: if obs.ua_fingerprint.ua_count <= 5 {
            "narrow".to_string()
        } else if obs.ua_fingerprint.ua_count <= 20 {
            "moderate".to_string()
        } else {
            "diverse".to_string()
        },
        ip_pattern: if obs.path_pattern.single_page_pct > 0.8 && obs.unique_ips > 10 {
            "rotating".to_string()
        } else if obs.path_pattern.single_page_pct < 0.3 {
            "sticky".to_string()
        } else {
            "mixed".to_string()
        },
        deception_flags,
    }
}

/// Quantize a percentage (0.0-1.0) into 5 bands.
fn quantize_pct(v: f32) -> u8 {
    match v {
        x if x < 0.1 => 0,
        x if x < 0.3 => 1,
        x if x < 0.6 => 2,
        x if x < 0.8 => 3,
        _ => 4,
    }
}

/// Quantize UA count into 3 bands.
fn quantize_ua_count(count: u8) -> u8 {
    match count {
        0..=5 => 0,
        6..=20 => 1,
        _ => 2,
    }
}

/// Quantize coefficient of variation into 3 bands.
fn quantize_cv(cv: f32) -> u8 {
    match cv {
        x if x < 0.3 => 0,  // metronomic
        x if x < 1.0 => 1,  // moderate
        _ => 2,              // organic
    }
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
                chrome_impersonation: true,
                header_poverty: true,
                stale_chrome: true,
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
                chrome_impersonation: true,
                header_poverty: true,
                stale_chrome: true,
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
                chrome_impersonation: false,
                header_poverty: false,
                stale_chrome: false,
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
        // New deception fields default to false for old data
        assert!(!parsed.deception.chrome_impersonation);
        assert!(!parsed.deception.header_poverty);
    }

    #[test]
    fn posture_escalation_ladder() {
        assert_eq!(DefensePosture::Observe.escalate(), DefensePosture::WarnRoute);
        assert_eq!(DefensePosture::WarnRoute.escalate(), DefensePosture::SlowDegrade);
        assert_eq!(DefensePosture::SlowDegrade.escalate(), DefensePosture::Scatter);
        assert_eq!(DefensePosture::Scatter.escalate(), DefensePosture::Vanish);
        assert_eq!(DefensePosture::Vanish.escalate(), DefensePosture::Disperse);
        assert_eq!(DefensePosture::Disperse.escalate(), DefensePosture::Disperse);
    }

    #[test]
    fn posture_deescalation_ladder() {
        assert_eq!(DefensePosture::Disperse.deescalate(), DefensePosture::Vanish);
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

        // P4 → P5 (need ESCALATE_TO_DISPERSE cumulative defections)
        while ab.escalation != DefensePosture::Disperse {
            t += 60;
            ab.tick_escalation(true, t);
        }
        assert_eq!(ab.escalation, DefensePosture::Disperse);
        assert!(ab.defection_count >= ESCALATE_TO_DISPERSE);
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
        assert!(DefensePosture::Vanish < DefensePosture::Disperse);
    }

    #[test]
    fn trust_descriptor_monotonically_decreasing() {
        let postures = [
            DefensePosture::Observe,
            DefensePosture::WarnRoute,
            DefensePosture::SlowDegrade,
            DefensePosture::Scatter,
            DefensePosture::Vanish,
            DefensePosture::Disperse,
        ];
        for i in 0..postures.len() - 1 {
            let a = postures[i].trust_descriptor();
            let b = postures[i + 1].trust_descriptor();
            assert!(
                a.trust_level >= b.trust_level,
                "{:?} trust ({}) should be >= {:?} trust ({})",
                postures[i], a.trust_level, postures[i + 1], b.trust_level,
            );
        }
        // First and last should be strictly different
        let first = postures[0].trust_descriptor();
        let last = postures[postures.len() - 1].trust_descriptor();
        assert!(first.trust_level > last.trust_level);
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

        // Disperse → active defense, no restoration
        let d = DefensePosture::Disperse.trust_descriptor();
        assert_eq!(d.membership_hint, "EcosystemProtection");
        assert_eq!(d.symbiosis_hint, "ActiveDefense");
        assert_eq!(d.trust_level, 0.0);
        assert!(!d.restoration_available);
    }

    #[test]
    fn behavioral_hash_stable_across_minor_variation() {
        let obs1 = similar_observation();
        let mut obs2 = obs1.clone();
        // Minor variations that should NOT change the hash
        obs2.total_requests += 50;
        obs2.unique_ips += 20;
        obs2.timing.mean_interval_ms += 500;

        assert_eq!(
            behavioral_hash(&obs1),
            behavioral_hash(&obs2),
            "minor variations should not change behavioral hash"
        );
    }

    #[test]
    fn behavioral_hash_changes_on_major_shift() {
        let obs = similar_observation();
        let mut shifted = obs.clone();
        // Major shift: from commit-heavy to diverse browsing
        shifted.path_pattern.commit_url_pct = 0.05;
        shifted.path_pattern.single_page_pct = 0.20;
        shifted.deception.hides_identity = false;
        shifted.deception.rotates_ips = false;

        assert_ne!(
            behavioral_hash(&obs),
            behavioral_hash(&shifted),
            "major behavioral shift should change hash"
        );
    }

    #[test]
    fn behavioral_hash_ignores_ip_count() {
        let obs1 = similar_observation();
        let mut obs2 = obs1.clone();
        obs2.unique_ips = 5000; // wildly different IP count

        assert_eq!(
            behavioral_hash(&obs1),
            behavioral_hash(&obs2),
            "hash must not depend on IP count"
        );
    }

    #[test]
    fn extract_invariants_fleet_pattern() {
        let obs = similar_observation();
        let inv = extract_invariants(&obs);
        assert!(inv.deep_content_dominant);
        assert!(inv.cookieless);
        assert_eq!(inv.session_depth, "high");
        assert_eq!(inv.ua_diversity, "narrow");
        assert_eq!(inv.ip_pattern, "rotating");
        assert!(inv.deception_flags.contains(&"hides_identity".to_string()));
        assert!(inv.deception_flags.contains(&"rotates_ips".to_string()));
    }

    #[test]
    fn opsonize_tag_serde_roundtrip() {
        let tag = OpsonizeTag {
            behavioral_hash: "a3f71234deadbeef".to_string(),
            detectors: vec!["content_gate".to_string(), "stale_chrome".to_string()],
            confidence: 0.95,
            origin_gate: "golgiBody".to_string(),
            invariants: extract_invariants(&similar_observation()),
            response: OpsonizeResponse::Scatter { ratio: 0.3 },
            created_epoch: 1000,
            last_confirmed_epoch: 2000,
            match_count: 50,
        };
        let json = serde_json::to_string(&tag).unwrap();
        let parsed: OpsonizeTag = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.behavioral_hash, tag.behavioral_hash);
        assert_eq!(parsed.detectors.len(), 2);
        assert_eq!(parsed.match_count, 50);
    }
}
