// SPDX-License-Identifier: AGPL-3.0-or-later

//! K-Derm relay chain — structural relay sync (bash → Rust evolution).
//!
//! Implements the diderm relay chain (Peptidoglycan composition layer):
//!
//! ```text
//! ext (trans) ──absorb──→ golgi (cis) ──metallic──→ relay node ──ionic──→ ext (trans)
//! ```
//!
//! Four stages:
//! 0. **`absorb_extracellular()`** — Pull from GitHub when ahead of Forgejo
//!    (reverse sync, catches gates that pushed to GitHub directly).
//! 1. **`mediate()`** — Pull from Forgejo (metallic bond inward), run impulse
//!    cascade. Evolved from legacy `pepti-sync-relay.sh`.
//! 2. **`ship_extracellular()`** — Push to GitHub via ext node (ionic →
//!    weak bond outward). Evolved from legacy `ext-github-push.sh`.
//! 3. **`run()`** — Full relay chain: absorb + mediate + impulse sense + ship.
//!    Exposed as `membrane relay.run <repo_path>`.
//!
//! The bash `golgi-post-receive-relay.sh` remains — Forgejo server-side hook
//! constraint. It calls `membrane relay.run` instead of the bash chain.

use crate::error::Result;
use crate::impulse;
use serde::Serialize;
use std::borrow::Cow;
use std::path::{Path, PathBuf};

use tracing::{debug, error, info, warn};

/// Result of a full relay run.
#[derive(Debug, Serialize)]
pub struct RelayResult {
    /// Repos absorbed from GitHub → Forgejo (reverse sync, stage 0).
    pub absorbed: Vec<String>,
    /// Repos successfully pulled from Forgejo (metallic bond).
    pub pulled: Vec<String>,
    /// Repos that failed to pull.
    pub pull_failures: Vec<String>,
    /// Number of pending impulses detected.
    pub impulses_sensed: usize,
    /// Repos successfully pushed to GitHub (weak bond outward).
    pub pushed: Vec<String>,
    /// Repos that were skipped (already up to date or not cloned on ext).
    pub push_skipped: Vec<String>,
    /// Repos that failed to push extracellularly.
    pub push_failures: Vec<String>,
}

/// Configuration for the relay chain, resolved from environment + config.
#[derive(Debug, Clone)]
pub struct RelayConfig {
    /// Root of the ecoPrimals workspace on this node.
    pub ecoprimals_root: PathBuf,
    /// Forgejo remote name for pull operations.
    pub forgejo_remote: Cow<'static, str>,
    /// GitHub/origin remote name for extracellular sync.
    pub github_remote: Cow<'static, str>,
    /// SSH host alias for golgiBody-ext (outer membrane).
    pub golgi_ext_host: Cow<'static, str>,
}

impl RelayConfig {
    /// Resolve configuration from membrane.toml, environment variables, then defaults.
    ///
    /// Priority: membrane.toml `[relay]` > environment > built-in defaults.
    #[must_use]
    pub fn from_env() -> Self {
        let membrane_config = load_relay_from_membrane_toml();

        let ecoprimals_root = std::env::var(cellmembrane_types::service::ENV_ECOPRIMALS_ROOT)
            .ok()
            .or_else(|| {
                membrane_config
                    .as_ref()
                    .and_then(|c| c.ecoprimals_root.clone())
            })
            .unwrap_or_else(|| cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT.to_string());

        let forgejo_remote: Cow<'static, str> =
            std::env::var(cellmembrane_types::service::ENV_RELAY_FORGEJO_REMOTE)
                .ok()
                .or_else(|| {
                    membrane_config
                        .as_ref()
                        .and_then(|c| c.forgejo_remote.clone())
                })
                .map_or(Cow::Borrowed("forgejo"), Cow::Owned);

        let github_remote: Cow<'static, str> =
            std::env::var(cellmembrane_types::service::ENV_RELAY_GITHUB_REMOTE)
                .ok()
                .or_else(|| {
                    membrane_config
                        .as_ref()
                        .and_then(|c| c.github_remote.clone())
                })
                .map_or(Cow::Borrowed("origin"), Cow::Owned);

        let golgi_ext_host: Cow<'static, str> =
            std::env::var(cellmembrane_types::service::ENV_GOLGI_EXT_HOST)
                .ok()
                .or_else(|| {
                    membrane_config
                        .as_ref()
                        .and_then(|c| c.golgi_ext_host.clone())
                })
                .map_or(
                    Cow::Borrowed(cellmembrane_types::service::DEFAULT_SSH_ALIAS_EXT),
                    Cow::Owned,
                );

        Self {
            ecoprimals_root: PathBuf::from(ecoprimals_root),
            forgejo_remote,
            github_remote,
            golgi_ext_host,
        }
    }
}

/// Optional [relay] section from membrane.toml.
#[derive(Debug, serde::Deserialize)]
struct MembraneRelayConfig {
    ecoprimals_root: Option<String>,
    forgejo_remote: Option<String>,
    github_remote: Option<String>,
    golgi_ext_host: Option<String>,
}

/// Top-level membrane.toml structure (for relay config extraction).
#[derive(serde::Deserialize)]
struct MembraneToml {
    relay: Option<MembraneRelayConfig>,
}

/// Attempt to load [relay] config from membrane.toml (XDG config path).
fn load_relay_from_membrane_toml() -> Option<MembraneRelayConfig> {
    let config_path = resolve_membrane_toml_path()?;
    let contents = std::fs::read_to_string(config_path).ok()?;
    let parsed: MembraneToml = toml::from_str(&contents).ok()?;
    parsed.relay
}

/// Resolve `membrane.toml` location: `XDG_CONFIG_HOME`/ecoPrimals/membrane.toml.
fn resolve_membrane_toml_path() -> Option<PathBuf> {
    use cellmembrane_types::service::{ENV_XDG_CONFIG_HOME, env_or};
    let config_home = std::env::var(ENV_XDG_CONFIG_HOME).unwrap_or_else(|_| {
        format!(
            "{}/.config",
            env_or(cellmembrane_types::service::ENV_HOME, "/tmp")
        )
    });
    let path = PathBuf::from(config_home).join("ecoPrimals/membrane.toml");
    if path.exists() { Some(path) } else { None }
}

/// Full relay chain: absorb → pull → impulse sense → ship extracellular.
///
/// Evolved from the legacy shell scripts (`pepti-sync-relay.sh` +
/// `ext-github-push.sh`). Exposed as `membrane relay.run <repo_path>`.
///
/// Stage 0 (`absorb_extracellular`) was added in Wave 132e to close the
/// bidirectional relay gap — when a gate pushes to GitHub directly, this
/// stage detects GitHub-ahead repos and syncs them back into Forgejo.
///
/// # Errors
///
/// Returns `Err` only for infrastructure failures (IO, serialization).
pub async fn run(config: &RelayConfig, repo_paths: &[&str]) -> Result<RelayResult> {
    let paths: Vec<&str> = if repo_paths.is_empty() {
        vec![cellmembrane_types::service::INFRA_WATERING_HOLE]
    } else {
        repo_paths.to_vec()
    };

    info!(count = paths.len(), "K-Derm relay chain triggered");

    let absorbed = absorb_extracellular(config, &paths).await;
    if !absorbed.is_empty() {
        info!(
            count = absorbed.len(),
            repos = ?absorbed,
            "absorbed extracellular → sovereign"
        );
    }

    let (pulled, pull_failures) = mediate(config, &paths).await;
    let sense_root = config.ecoprimals_root.clone();
    let impulses_sensed = tokio::task::spawn_blocking(move || sense_impulses_blocking(&sense_root))
        .await
        .unwrap_or(0);
    let (pushed, push_skipped, push_failures) = ship_extracellular(config, &paths).await;

    let result = RelayResult {
        absorbed,
        pulled,
        pull_failures,
        impulses_sensed,
        pushed,
        push_skipped,
        push_failures,
    };

    info!(
        absorbed = result.absorbed.len(),
        pulled = result.pulled.len(),
        pushed = result.pushed.len(),
        impulses = result.impulses_sensed,
        failures = result.pull_failures.len() + result.push_failures.len(),
        "chain complete"
    );

    Ok(result)
}

/// Stage 0: Absorb extracellular → inner membrane (reverse sync).
///
/// Detects when GitHub/origin is ahead of Forgejo for any repo in the relay
/// set. When divergence is found, fetches from GitHub then pushes to Forgejo,
/// closing the gap that occurs when a gate pushes to GitHub directly.
///
/// This is a defensive stage — it should be a no-op in normal operation when
/// all gates push to Forgejo first. Returns the list of repos that were absorbed.
pub async fn absorb_extracellular(config: &RelayConfig, repo_paths: &[&str]) -> Vec<String> {
    let mut absorbed = Vec::new();

    for &repo_path in repo_paths {
        let local_path = config.ecoprimals_root.join(repo_path);

        if !local_path.join(".git").exists() {
            debug!(repo = repo_path, "not cloned — skipping absorb");
            continue;
        }

        match absorb_one_repo(config, &local_path, repo_path).await {
            AbsorbOutcome::Absorbed(count) => {
                info!(
                    repo = repo_path,
                    commits = count,
                    "absorbed from GitHub → Forgejo"
                );
                absorbed.push(repo_path.to_string());
            }
            AbsorbOutcome::AtParity => {
                debug!(repo = repo_path, "GitHub ↔ Forgejo at parity");
            }
            AbsorbOutcome::FetchFailed => {
                warn!(repo = repo_path, "GitHub fetch failed — skipping absorb");
            }
            AbsorbOutcome::PushFailed => {
                error!(
                    repo = repo_path,
                    "absorbed from GitHub but push to Forgejo failed"
                );
            }
            AbsorbOutcome::NoGitHubRemote => {
                debug!(repo = repo_path, "no GitHub remote configured — skipping");
            }
        }
    }

    absorbed
}

enum AbsorbOutcome {
    Absorbed(u32),
    AtParity,
    FetchFailed,
    PushFailed,
    NoGitHubRemote,
}

/// Check if a remote exists for a given repo.
async fn has_remote(repo_path: &Path, remote: &str) -> bool {
    crate::git_ops::git_success(repo_path, &["remote", "get-url", remote]).await
}

/// Absorb one repo: fetch GitHub, check if ahead of Forgejo, merge + push.
async fn absorb_one_repo(
    config: &RelayConfig,
    local_path: &Path,
    repo_name: &str,
) -> AbsorbOutcome {
    let github = &config.github_remote;
    let forgejo = &config.forgejo_remote;

    if !has_remote(local_path, github).await {
        return AbsorbOutcome::NoGitHubRemote;
    }

    if !crate::git_ops::git_success(local_path, &["fetch", github, "main", "--quiet"]).await {
        debug!(repo = repo_name, remote = %github, "fetch failed");
        return AbsorbOutcome::FetchFailed;
    }

    if !crate::git_ops::git_success(local_path, &["fetch", forgejo, "main", "--quiet"]).await {
        debug!(repo = repo_name, remote = %forgejo, "fetch failed");
        return AbsorbOutcome::FetchFailed;
    }

    let range = format!("{forgejo}/main..{github}/main");
    let ahead = crate::git_ops::rev_list_count(local_path, &range).await;

    if ahead == 0 {
        return AbsorbOutcome::AtParity;
    }

    info!(
        repo = repo_name,
        commits = ahead,
        "GitHub is {ahead} commit(s) ahead of Forgejo — absorbing"
    );

    let merge_ok = crate::git_ops::git_success(
        local_path,
        &["merge", &format!("{github}/main"), "--ff-only"],
    )
    .await;

    if !merge_ok {
        warn!(
            repo = repo_name,
            "ff-only merge from {github}/main failed — non-fast-forward divergence"
        );
        return AbsorbOutcome::PushFailed;
    }

    let push_ok =
        crate::git_ops::git_success(local_path, &["push", forgejo, "main", "--quiet"]).await;

    if push_ok {
        AbsorbOutcome::Absorbed(ahead)
    } else {
        AbsorbOutcome::PushFailed
    }
}

/// Stage 1: Pull from Forgejo on the local node (metallic bond inward).
///
/// Evolved from the pull loop in `pepti-sync-relay.sh`.
/// For each repo path, does a `git pull --ff-only forgejo main`.
pub async fn mediate(config: &RelayConfig, repo_paths: &[&str]) -> (Vec<String>, Vec<String>) {
    let mut pulled = Vec::new();
    let mut failures = Vec::new();

    for &repo_path in repo_paths {
        let local_path = config.ecoprimals_root.join(repo_path);

        if !local_path.join(".git").exists() {
            debug!(repo = repo_path, "not cloned on relay node — skipping");
            continue;
        }

        if crate::git_ops::pull_ff_only(&local_path, &config.forgejo_remote).await {
            info!(repo = repo_path, "pulled (metallic bond)");
            pulled.push(repo_path.to_string());
        } else {
            warn!(repo = repo_path, "pull failed — may be up to date");
            failures.push(repo_path.to_string());
        }
    }

    (pulled, failures)
}

/// Stage 2: Sense pending impulses in the wateringHole.
///
/// Evolved from `pepti-sync-relay.sh` / `impulse-relay-hook.sh`.
/// Uses the `impulse` module directly instead of shelling out.
fn sense_impulses_blocking(ecoprimals_root: &Path) -> usize {
    match impulse::sense(ecoprimals_root, true, true) {
        Ok((_, count)) => {
            if count > 0 {
                info!(count, "detected pending impulse(s)");
            } else {
                debug!("resting potential (no pending impulses)");
            }
            count
        }
        Err(e) => {
            warn!(error = %e, "impulse sense failed — continuing");
            0
        }
    }
}

/// Stage 3: Ship repos to extracellular via golgiBody-ext (ionic → weak bond).
///
/// SSHs to golgiBody-ext and for each repo: pulls from Forgejo, then
/// pushes to GitHub. Replaces `ext-github-push.sh`.
///
/// Returns (pushed, skipped, failures).
pub async fn ship_extracellular(
    config: &RelayConfig,
    repo_paths: &[&str],
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut pushed = Vec::new();
    let mut skipped = Vec::new();
    let mut failures = Vec::new();

    for &repo_path in repo_paths {
        let result = ship_one_repo(config, repo_path).await;
        match result {
            ShipResult::Pushed => pushed.push(repo_path.to_string()),
            ShipResult::Skipped => skipped.push(repo_path.to_string()),
            ShipResult::Failed => failures.push(repo_path.to_string()),
        }
    }

    (pushed, skipped, failures)
}

enum ShipResult {
    Pushed,
    Skipped,
    Failed,
}

/// Ship a single repo via SSH to `golgiBody-ext`.
///
/// The command on ext:
///   1. Fetch from Forgejo and hard-reset (sovereign authority)
///   2. Determine if ahead of GitHub mirror remote
///   3. If ahead, push with `--force-with-lease` (agentic divergence policy)
async fn ship_one_repo(config: &RelayConfig, repo_path: &str) -> ShipResult {
    let eco_root = config.ecoprimals_root.to_string_lossy();
    let remote_script = format!(
        r#"set -euo pipefail
d="{eco_root}/{repo_path}"
[ -d "$d/.git" ] || exit 2
cd "$d"
branch=$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo main)
git fetch forgejo "$branch" --quiet 2>/dev/null && git reset --hard "forgejo/$branch" --quiet 2>/dev/null || true
REMOTE=$(git remote get-url github >/dev/null 2>&1 && echo github || echo origin)
git fetch "$REMOTE" --quiet 2>/dev/null || exit 3
ahead=$(git rev-list --count "$REMOTE/$branch..HEAD" 2>/dev/null || echo 0)
[ "$ahead" -eq 0 ] && exit 4
git push --force-with-lease "$REMOTE" "$branch" --quiet 2>/dev/null || exit 5
echo "+$ahead"
"#
    );

    match crate::ssh::exec_raw_on(&config.golgi_ext_host, 5, &remote_script).await {
        Ok((stdout, 0)) => {
            let commits = stdout.trim().to_string();
            info!(repo = repo_path, commits, "pushed to GitHub");
            ShipResult::Pushed
        }
        Ok((_, 2)) => {
            debug!(repo = repo_path, "not cloned on outer membrane — skipping");
            ShipResult::Skipped
        }
        Ok((_, 4)) => ShipResult::Skipped,
        Ok((_, _)) => {
            error!(repo = repo_path, "push to GitHub failed");
            ShipResult::Failed
        }
        Err(e) => {
            error!(error = %e, "SSH to golgiBody-ext failed");
            ShipResult::Failed
        }
    }
}

/// Result of checking parity between GitHub and Forgejo for a single repo.
#[derive(Debug, Serialize)]
pub struct ParityReport {
    /// Repo path.
    pub repo: String,
    /// Whether the two remotes are at the same commit.
    pub at_parity: bool,
    /// Human-readable detail.
    pub detail: String,
}

/// Check parity between GitHub and Forgejo for each repo.
///
/// For each repo, fetches both remotes and compares their `main` branch HEADs.
/// Reports which repos are diverged and by how many commits in each direction.
/// This is a read-only operation — no merges or pushes.
pub async fn check_parity(config: &RelayConfig, repo_paths: &[&str]) -> Vec<ParityReport> {
    let mut reports = Vec::new();

    for &repo_path in repo_paths {
        let local_path = config.ecoprimals_root.join(repo_path);

        if !local_path.join(".git").exists() {
            reports.push(ParityReport {
                repo: repo_path.into(),
                at_parity: true,
                detail: "not cloned — skipped".into(),
            });
            continue;
        }

        let github = &config.github_remote;
        let forgejo = &config.forgejo_remote;

        if !has_remote(&local_path, github).await {
            reports.push(ParityReport {
                repo: repo_path.into(),
                at_parity: true,
                detail: format!("no {github} remote — sovereign only"),
            });
            continue;
        }

        if !crate::git_ops::git_success(&local_path, &["fetch", github, "main", "--quiet"]).await {
            tracing::warn!(repo = %local_path.display(), remote = %github, "fetch failed");
        }
        if !crate::git_ops::git_success(&local_path, &["fetch", forgejo, "main", "--quiet"]).await {
            tracing::warn!(repo = %local_path.display(), remote = %forgejo, "fetch failed");
        }

        let gh_ahead_range = format!("{forgejo}/main..{github}/main");
        let fg_ahead_range = format!("{github}/main..{forgejo}/main");

        let gh_ahead = crate::git_ops::rev_list_count(&local_path, &gh_ahead_range).await;
        let fg_ahead = crate::git_ops::rev_list_count(&local_path, &fg_ahead_range).await;

        let at_parity = gh_ahead == 0 && fg_ahead == 0;
        let detail = if at_parity {
            "at parity".into()
        } else if gh_ahead > 0 && fg_ahead == 0 {
            format!("GitHub {gh_ahead} ahead of Forgejo")
        } else if fg_ahead > 0 && gh_ahead == 0 {
            format!("Forgejo {fg_ahead} ahead of GitHub")
        } else {
            format!("DIVERGED: GitHub +{gh_ahead}, Forgejo +{fg_ahead}")
        };

        reports.push(ParityReport {
            repo: repo_path.into(),
            at_parity,
            detail,
        });
    }

    reports
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_config_respects_defaults() {
        let config = RelayConfig {
            ecoprimals_root: PathBuf::from(cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT),
            forgejo_remote: Cow::Borrowed("forgejo"),
            github_remote: Cow::Borrowed("origin"),
            golgi_ext_host: Cow::Borrowed("golgi-ext"),
        };
        assert_eq!(
            config.ecoprimals_root,
            PathBuf::from(cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT)
        );
        assert_eq!(&*config.forgejo_remote, "forgejo");
        assert_eq!(&*config.github_remote, "origin");
        assert_eq!(&*config.golgi_ext_host, "golgi-ext");
    }

    #[test]
    fn relay_config_custom_values() {
        let config = RelayConfig {
            ecoprimals_root: PathBuf::from("/tmp/test-eco"),
            forgejo_remote: Cow::Borrowed("forgejo"),
            github_remote: Cow::Owned("github".to_string()),
            golgi_ext_host: Cow::Owned("custom-ext".to_string()),
        };
        assert_eq!(config.ecoprimals_root, PathBuf::from("/tmp/test-eco"));
        assert_eq!(&*config.forgejo_remote, "forgejo");
        assert_eq!(&*config.github_remote, "github");
        assert_eq!(&*config.golgi_ext_host, "custom-ext");
    }

    #[tokio::test]
    async fn mediate_skips_nonexistent_repos() {
        let config = RelayConfig {
            ecoprimals_root: PathBuf::from("/tmp/nonexistent-relay-test"),
            forgejo_remote: Cow::Borrowed("forgejo"),
            github_remote: Cow::Borrowed("origin"),
            golgi_ext_host: Cow::Borrowed("test"),
        };
        let (pulled, failures) = mediate(&config, &["no/such/repo"]).await;
        assert!(pulled.is_empty());
        assert!(failures.is_empty());
    }

    #[tokio::test]
    async fn absorb_skips_nonexistent_repos() {
        let config = RelayConfig {
            ecoprimals_root: PathBuf::from("/tmp/nonexistent-absorb-test"),
            forgejo_remote: Cow::Borrowed("forgejo"),
            github_remote: Cow::Borrowed("origin"),
            golgi_ext_host: Cow::Borrowed("test"),
        };
        let absorbed = absorb_extracellular(&config, &["no/such/repo"]).await;
        assert!(absorbed.is_empty());
    }

    #[tokio::test]
    async fn parity_skips_nonexistent_repos() {
        let config = RelayConfig {
            ecoprimals_root: PathBuf::from("/tmp/nonexistent-parity-test"),
            forgejo_remote: Cow::Borrowed("forgejo"),
            github_remote: Cow::Borrowed("origin"),
            golgi_ext_host: Cow::Borrowed("test"),
        };
        let reports = check_parity(&config, &["no/such/repo"]).await;
        assert_eq!(reports.len(), 1);
        assert!(
            reports[0].at_parity,
            "non-existent repos should count as parity"
        );
        assert!(reports[0].detail.contains("not cloned"));
    }

    #[test]
    fn relay_result_serializes() {
        let result = RelayResult {
            absorbed: vec!["songBird".into()],
            pulled: vec!["bearDog".into()],
            pull_failures: vec![],
            impulses_sensed: 2,
            pushed: vec!["bearDog".into()],
            push_skipped: vec![],
            push_failures: vec!["songBird".into()],
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["absorbed"][0], "songBird");
        assert_eq!(json["pulled"][0], "bearDog");
        assert_eq!(json["impulses_sensed"], 2);
        assert_eq!(json["push_failures"][0], "songBird");
    }

    #[test]
    fn parity_report_serializes() {
        let report = ParityReport {
            repo: "songBird".into(),
            at_parity: false,
            detail: "GitHub 3 ahead of Forgejo".into(),
        };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["repo"], "songBird");
        assert_eq!(json["at_parity"], false);
        assert!(json["detail"].as_str().unwrap().contains("GitHub"));
    }

    #[test]
    fn ship_result_variants() {
        assert!(matches!(ShipResult::Pushed, ShipResult::Pushed));
        assert!(matches!(ShipResult::Skipped, ShipResult::Skipped));
        assert!(matches!(ShipResult::Failed, ShipResult::Failed));
    }

    #[test]
    fn absorb_outcome_variants() {
        assert!(matches!(
            AbsorbOutcome::Absorbed(3),
            AbsorbOutcome::Absorbed(3)
        ));
        assert!(matches!(AbsorbOutcome::AtParity, AbsorbOutcome::AtParity));
        assert!(matches!(
            AbsorbOutcome::FetchFailed,
            AbsorbOutcome::FetchFailed
        ));
        assert!(matches!(
            AbsorbOutcome::PushFailed,
            AbsorbOutcome::PushFailed
        ));
        assert!(matches!(
            AbsorbOutcome::NoGitHubRemote,
            AbsorbOutcome::NoGitHubRemote
        ));
    }
}
