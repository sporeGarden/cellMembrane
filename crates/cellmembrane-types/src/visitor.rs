// SPDX-License-Identifier: AGPL-3.0-or-later

//! Visitor classification — behavioral + UA-based request taxonomy.
//!
//! Shared between membrane-shadow (receptor) and skunky-ingest (skunkBat
//! anomaly detection). Both crates need identical classification logic so
//! that "Human" means the same thing across the entire signal pipeline.
//!
//! ## Classification Layers
//!
//! 1. **Path-based** — probe paths (`/.env`, `/wp-admin`, etc.) →
//!    `ScraperBot` instantly, regardless of User-Agent.
//! 2. **UA-based** — keyword matching for known bot signatures.
//! 3. **Session-level** — velocity checks and behavioral pattern
//!    reclassification (implemented in membrane-shadow's receptor,
//!    using these types).
//!
//! ## Privacy
//!
//! No IP addresses or PII. UA strings are hashed (SHA-256) for session
//! grouping — never stored raw in signal logs.
//!
//! ## skunky-ingest Integration
//!
//! **Option A (recommended): Import from cellmembrane-types**
//!
//! ```toml
//! # In skunky-ingest Cargo.toml:
//! cellmembrane-types = { path = "../cellMembrane/crates/cellmembrane-types" }
//! ```
//!
//! ```rust,ignore
//! use cellmembrane_types::{classify_request, VisitorClass};
//!
//! let class = classify_request(&ua_string, &path);
//! if class.is_bot() { /* skip for anomaly baseline */ }
//! ```
//!
//! Phase 2 scanner fingerprinting in skunky-ingest maps directly to
//! [`classify_request`] — the composite classifier combines path-based
//! instant classification (credential probes, WordPress scans) with
//! UA-based bot detection, producing the same [`VisitorClass`] taxonomy
//! that the signal log uses.
//!
//! **Option B: Tail signal.jsonl**
//!
//! If cross-crate dependency is not viable, skunky-ingest can tail
//! `/var/log/membrane/signal.jsonl` for pre-classified data. Receptor
//! events contain per-host visitor counts already broken down by class.
//! See `membrane-shadow::signal::ReceptorSignal` for the schema.

use serde::{Deserialize, Serialize};

// ── Visitor Classification ──────────────────────────────────────────

/// Classification of a visitor based on User-Agent + behavioral analysis.
///
/// The taxonomy is ordered by signal reliability: UA-identified bots are
/// classified first, then behavioral signals refine the classification.
/// A "Human" classification means the visitor passed ALL checks — UA looks
/// like a real browser, path isn't a probe target, velocity is human-speed,
/// and session behavior is consistent with reading content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisitorClass {
    /// Search engine crawler (Googlebot, Bingbot, etc.)
    SearchBot,
    /// Social media preview bot (Twitterbot, facebookexternalhit, etc.)
    SocialBot,
    /// AI/LLM training crawler (GPTBot, ClaudeBot, etc.)
    AiBot,
    /// Monitoring/uptime checker (UptimeRobot, Pingdom, etc.)
    MonitorBot,
    /// Automated reconnaissance/scraping (credential scanners, vuln probes).
    ScraperBot,
    /// Generic bot (curl, wget, python-requests, scrapy, etc.)
    GenericBot,
    /// Stealth fleet — hides identity, rotates IPs, evades detection.
    /// Classified by population-level behavioral analysis, not individual UA.
    StealthFleet,
    /// Human visitor with a standard browser (provably human — passed all checks).
    Human,
    /// Human using automated tools (API clients, research scripts).
    AgenticHuman,
    /// Unclassifiable (empty or missing User-Agent).
    Unknown,
}

// ── Ecological Symbiosis ────────────────────────────────────────────

/// Ecological relationship between a visitor and the host membrane.
///
/// Guides the membrane response — both inner and outer thymus use this:
///
/// - **Mutualist** — humans. Full access, privacy respected.
/// - **Commensal** — honest bots, metadata scrapers. Welcome, guided to
///   external membrane for efficiency. We want the signal (SEO,
///   discoverability, lysogeny propagation). We just don't want the
///   resource loss. May become mutualists if they register/authenticate.
/// - **Parasitic** — vulnerability scanners, credential probers. These
///   probe for disease — weaknesses in the membrane itself. Not targeted
///   adversaries, but opportunistic infections testing every surface.
///   Response: instant 403, no content, no signal, don't engage.
/// - **Pathogenic** — stealth fleets. Deceptive, resource-draining,
///   deliberately evading detection. The adaptive immune system targets
///   these with antibodies, opsonization, and scatter poison.
///
/// The distinction between Parasitic and Pathogenic matters:
/// - A scanner is **disease**: probing for general vulnerabilities, not
///   targeting *us* specifically. It will move on.
/// - A stealth fleet is **predation**: deliberately targeting *our* data,
///   adapting to *our* defenses. It persists until defeated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Symbiosis {
    /// Human — full access, privacy respected.
    Mutualist,
    /// Honest bot — welcome, guided to external membrane for efficiency.
    /// Lysogeny travels with the data either way.
    Commensal,
    /// Vulnerability scanner / credential prober — opportunistic infection.
    /// Instant rejection, no engagement, no signal. Disease, not predation.
    Parasitic,
    /// Stealth fleet — deceptive, resource-draining, deliberately targeting.
    /// Antibodies, opsonization, and scatter poison. Predation, not disease.
    Pathogenic,
}

impl VisitorClass {
    /// Whether this class counts as a bot (not a real human reader).
    #[must_use]
    pub fn is_bot(self) -> bool {
        !matches!(self, Self::Human | Self::AgenticHuman)
    }

    /// Whether this class counts as a human (real reader or agentic tool user).
    #[must_use]
    pub fn is_human(self) -> bool {
        matches!(self, Self::Human | Self::AgenticHuman)
    }

    /// Ecological relationship with the host membrane.
    ///
    /// Commensals identify honestly and we want their signal — they're
    /// guided to efficient paths (external membrane), not blocked.
    /// Pathogens hide and drain resources — antibodies target them.
    #[must_use]
    pub fn symbiosis(self) -> Symbiosis {
        match self {
            Self::Human | Self::AgenticHuman => Symbiosis::Mutualist,
            Self::SearchBot | Self::SocialBot | Self::AiBot
            | Self::MonitorBot | Self::GenericBot => Symbiosis::Commensal,
            Self::ScraperBot | Self::Unknown => Symbiosis::Parasitic,
            Self::StealthFleet => Symbiosis::Pathogenic,
        }
    }
}

impl std::fmt::Display for VisitorClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SearchBot => write!(f, "search_bot"),
            Self::SocialBot => write!(f, "social_bot"),
            Self::AiBot => write!(f, "ai_bot"),
            Self::MonitorBot => write!(f, "monitor_bot"),
            Self::ScraperBot => write!(f, "scraper_bot"),
            Self::GenericBot => write!(f, "generic_bot"),
            Self::StealthFleet => write!(f, "stealth_fleet"),
            Self::Human => write!(f, "human"),
            Self::AgenticHuman => write!(f, "agentic_human"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

// ── Path-Based Classification ───────────────────────────────────────

/// Known probe/recon path prefixes that indicate automated scanning.
///
/// Any hit to these patterns is immediately classified as `ScraperBot`
/// regardless of User-Agent. Real humans never request `/.env`.
pub const PROBE_PREFIXES: &[&str] = &[
    "/.env",
    "/.aws",
    "/.git/",
    "/.svn/",
    "/.DS_Store",
    "/wp-admin",
    "/wp-login",
    "/wp-content",
    "/wp-includes",
    "/xmlrpc.php",
    "/admin",
    "/phpmyadmin",
    "/actuator",
    "/api/v1",
    "/cgi-bin",
    "/config.",
    "/backup",
    "/debug",
    "/.htaccess",
    "/.htpasswd",
    "/server-status",
    "/telescope",
    "/vendor/",
    "/node_modules/",
    "/composer.",
    "/package.json",
    "/yarn.lock",
    "/Dockerfile",
    "/docker-compose",
];

/// Classify a request path as a probe/recon target.
///
/// Returns `Some(ScraperBot)` if the path matches known probe patterns.
/// Returns `None` if the path looks like legitimate content.
#[must_use]
pub fn classify_path(path: &str) -> Option<VisitorClass> {
    let lower = path.to_lowercase();

    // Dotfile probes (/.env, /.env.backup, /.env.prod, etc.)
    if lower.starts_with("/.") && !lower.starts_with("/.well-known/acme-challenge/") {
        return Some(VisitorClass::ScraperBot);
    }

    for prefix in PROBE_PREFIXES {
        if lower.starts_with(prefix) {
            return Some(VisitorClass::ScraperBot);
        }
    }

    // PHP/ASP/JSP probes on a static site
    if lower.ends_with(".php")
        || lower.ends_with(".asp")
        || lower.ends_with(".aspx")
        || lower.ends_with(".jsp")
    {
        return Some(VisitorClass::ScraperBot);
    }

    None
}

// ── UA-Based Classification ─────────────────────────────────────────

/// Classify a visitor from their User-Agent string alone.
///
/// Search bots are identified first (highest signal value for SEO),
/// then social/AI/monitor bots, then generic bots. Remaining traffic
/// with standard browser signatures is classified as human.
#[must_use]
pub fn classify_visitor(user_agent: &str) -> VisitorClass {
    if user_agent.is_empty() {
        return VisitorClass::Unknown;
    }

    let ua = user_agent.to_lowercase();

    // Search engine crawlers — high SEO signal
    if ua.contains("googlebot")
        || ua.contains("bingbot")
        || ua.contains("yandexbot")
        || ua.contains("duckduckbot")
        || ua.contains("baiduspider")
        || ua.contains("slurp")
        || ua.contains("seznambot")
        || ua.contains("applebot")
        || ua.contains("google-inspectiontool")
        || ua.contains("adsbot-google")
        || ua.contains("mediapartners-google")
    {
        return VisitorClass::SearchBot;
    }

    // Social media preview bots
    if ua.contains("twitterbot")
        || ua.contains("facebookexternalhit")
        || ua.contains("linkedinbot")
        || ua.contains("slackbot")
        || ua.contains("discordbot")
        || ua.contains("telegrambot")
        || ua.contains("whatsapp")
        || ua.contains("mastodon")
    {
        return VisitorClass::SocialBot;
    }

    // AI/LLM training crawlers
    if ua.contains("gptbot")
        || ua.contains("claudebot")
        || ua.contains("anthropic")
        || ua.contains("chatgpt")
        || ua.contains("cohere-ai")
        || ua.contains("perplexitybot")
        || ua.contains("bytespider")
        || ua.contains("ccbot")
    {
        return VisitorClass::AiBot;
    }

    // Monitoring/uptime bots
    if ua.contains("uptimerobot")
        || ua.contains("pingdom")
        || ua.contains("statuscake")
        || ua.contains("site24x7")
        || ua.contains("datadog")
        || ua.contains("newrelic")
        || ua.contains("prometheus")
    {
        return VisitorClass::MonitorBot;
    }

    // Generic bots (tools, libraries, scrapers)
    if ua.contains("curl/")
        || ua.contains("wget/")
        || ua.contains("python-requests")
        || ua.contains("python-urllib")
        || ua.contains("httpx")
        || ua.contains("scrapy")
        || ua.contains("go-http-client")
        || ua.contains("java/")
        || ua.contains("libwww")
        || ua.contains("ahrefs")
        || ua.contains("semrush")
        || ua.contains("mj12bot")
        || ua.contains("dotbot")
        || ua.contains("petalbot")
        || ua.starts_with("mozilla/5.0 (compatible;")
        || !ua.contains("mozilla")
    {
        return VisitorClass::GenericBot;
    }

    // Standard browser User-Agents contain "Mozilla/5.0" with a rendering engine
    if (ua.contains("chrome/") || ua.contains("firefox/") || ua.contains("safari/"))
        && ua.contains("mozilla/5.0")
    {
        return VisitorClass::Human;
    }

    VisitorClass::Unknown
}

// ── Composite Classification ────────────────────────────────────────

/// Composite classification: combine UA-based and path-based signals.
///
/// Path classification overrides UA classification when the path is a
/// known probe target — a clean Chrome UA requesting `/.env` is a scanner.
#[must_use]
pub fn classify_request(user_agent: &str, path: &str) -> VisitorClass {
    if let Some(path_class) = classify_path(path) {
        return path_class;
    }
    classify_visitor(user_agent)
}

// ── Session Behavioral Constants ────────────────────────────────────

/// Session window — hits from the same UA within this many seconds are grouped.
pub const SESSION_WINDOW_SECS: f64 = 1800.0; // 30 minutes

/// Velocity threshold: more than this many hits in [`VELOCITY_WINDOW_SECS`] → not human.
pub const VELOCITY_THRESHOLD: usize = 5;

/// Velocity window in seconds.
pub const VELOCITY_WINDOW_SECS: f64 = 10.0;

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_googlebot() {
        assert_eq!(
            classify_visitor(
                "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)"
            ),
            VisitorClass::SearchBot
        );
    }

    #[test]
    fn classify_chrome_human() {
        assert_eq!(
            classify_visitor(
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/120.0.0.0 Safari/537.36"
            ),
            VisitorClass::Human
        );
    }

    #[test]
    fn classify_empty_unknown() {
        assert_eq!(classify_visitor(""), VisitorClass::Unknown);
    }

    #[test]
    fn classify_curl_generic() {
        assert_eq!(classify_visitor("curl/7.88.1"), VisitorClass::GenericBot);
    }

    #[test]
    fn classify_gptbot_ai() {
        assert_eq!(
            classify_visitor("GPTBot/1.0 (+https://openai.com/gptbot)"),
            VisitorClass::AiBot
        );
    }

    #[test]
    fn classify_path_dotenv() {
        assert_eq!(classify_path("/.env"), Some(VisitorClass::ScraperBot));
        assert_eq!(
            classify_path("/.env.backup"),
            Some(VisitorClass::ScraperBot)
        );
    }

    #[test]
    fn classify_path_wordpress() {
        assert_eq!(classify_path("/wp-admin"), Some(VisitorClass::ScraperBot));
        assert_eq!(classify_path("/xmlrpc.php"), Some(VisitorClass::ScraperBot));
    }

    #[test]
    fn classify_path_php_on_static() {
        assert_eq!(classify_path("/index.php"), Some(VisitorClass::ScraperBot));
    }

    #[test]
    fn classify_path_normal_content() {
        assert_eq!(classify_path("/"), None);
        assert_eq!(classify_path("/evidence/"), None);
        assert_eq!(classify_path("/network/dpsc/"), None);
    }

    #[test]
    fn classify_request_probe_overrides_ua() {
        assert_eq!(
            classify_request("Mozilla/5.0 Chrome/120.0.0.0 Safari/537.36", "/.env"),
            VisitorClass::ScraperBot
        );
    }

    #[test]
    fn classify_request_normal_uses_ua() {
        assert_eq!(
            classify_request(
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/120.0.0.0 Safari/537.36",
                "/evidence/"
            ),
            VisitorClass::Human
        );
    }

    #[test]
    fn visitor_class_display() {
        assert_eq!(VisitorClass::SearchBot.to_string(), "search_bot");
        assert_eq!(VisitorClass::Human.to_string(), "human");
        assert_eq!(VisitorClass::ScraperBot.to_string(), "scraper_bot");
        assert_eq!(VisitorClass::AgenticHuman.to_string(), "agentic_human");
    }

    #[test]
    fn is_bot_is_human_partition() {
        for class in [
            VisitorClass::SearchBot,
            VisitorClass::SocialBot,
            VisitorClass::AiBot,
            VisitorClass::MonitorBot,
            VisitorClass::ScraperBot,
            VisitorClass::GenericBot,
            VisitorClass::StealthFleet,
            VisitorClass::Unknown,
        ] {
            assert!(class.is_bot(), "{class} should be bot");
            assert!(!class.is_human(), "{class} should not be human");
        }
        for class in [VisitorClass::Human, VisitorClass::AgenticHuman] {
            assert!(!class.is_bot(), "{class} should not be bot");
            assert!(class.is_human(), "{class} should be human");
        }
    }

    #[test]
    fn symbiosis_mapping() {
        // Mutualists — humans
        assert_eq!(VisitorClass::Human.symbiosis(), Symbiosis::Mutualist);
        assert_eq!(VisitorClass::AgenticHuman.symbiosis(), Symbiosis::Mutualist);
        // Commensals — honest bots
        assert_eq!(VisitorClass::SearchBot.symbiosis(), Symbiosis::Commensal);
        assert_eq!(VisitorClass::AiBot.symbiosis(), Symbiosis::Commensal);
        assert_eq!(VisitorClass::SocialBot.symbiosis(), Symbiosis::Commensal);
        assert_eq!(VisitorClass::GenericBot.symbiosis(), Symbiosis::Commensal);
        assert_eq!(VisitorClass::MonitorBot.symbiosis(), Symbiosis::Commensal);
        // Parasitic — disease (scanners, probers)
        assert_eq!(VisitorClass::ScraperBot.symbiosis(), Symbiosis::Parasitic);
        assert_eq!(VisitorClass::Unknown.symbiosis(), Symbiosis::Parasitic);
        // Pathogenic — predation (stealth fleets)
        assert_eq!(VisitorClass::StealthFleet.symbiosis(), Symbiosis::Pathogenic);
    }

    #[test]
    fn serde_roundtrip() {
        for class in [
            VisitorClass::SearchBot,
            VisitorClass::ScraperBot,
            VisitorClass::StealthFleet,
            VisitorClass::Human,
            VisitorClass::AgenticHuman,
        ] {
            let json = serde_json::to_string(&class).unwrap();
            let parsed: VisitorClass = serde_json::from_str(&json).unwrap();
            assert_eq!(class, parsed);
        }
    }
}
