// SPDX-License-Identifier: AGPL-3.0-or-later

//! K-Derm relay chain — peptidoglycan sync relay (bash → Rust evolution).
//!
//! Implements the golgiBody diderm relay chain:
//!
//! ```text
//! golgiBody-inner (cis) ──metallic──→ peptidoglycan ──ionic──→ golgiBody-ext (trans)
//! ```
//!
//! Three stages:
//! 1. **`mediate()`** — Pull from Forgejo (metallic bond inward), run impulse
//!    cascade. Replaces `pepti-sync-relay.sh`.
//! 2. **`ship_extracellular()`** — Push to GitHub via golgiBody-ext (ionic →
//!    weak bond outward). Replaces `ext-github-push.sh`.
//! 3. **`run()`** — Full relay chain: mediate + impulse sense + ship.
//!    Exposed as `membrane relay.run <repo_path>`.
//!
//! The bash `golgi-post-receive-relay.sh` remains — Forgejo server-side hook
//! constraint. It calls `membrane relay.run` instead of the bash chain.

use crate::error::Result;
use crate::impulse;
use serde::Serialize;
use std::path::PathBuf;
use tokio::process::Command;

/// Default ecoPrimals root path on relay nodes.
const DEFAULT_ECOPRIMALS_ROOT: &str = "/opt/ecoPrimals";

/// Default Forgejo remote name (sovereign primary).
const REMOTE_FORGEJO: &str = "forgejo";

/// Default SSH host alias for golgiBody-ext (trans face / outer membrane).
const DEFAULT_GOLGI_EXT_HOST: &str = "golgi-ext";

/// Result of a full relay run.
#[derive(Debug, Serialize)]
pub struct RelayResult {
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

/// Configuration for the relay chain, resolved from environment.
#[derive(Debug, Clone)]
pub struct RelayConfig {
    /// Root of the ecoPrimals workspace on this node.
    pub ecoprimals_root: PathBuf,
    /// SSH host alias for golgiBody-ext (outer membrane).
    pub golgi_ext_host: String,
}

impl RelayConfig {
    /// Resolve configuration from environment variables, with defaults.
    #[must_use]
    pub fn from_env() -> Self {
        let ecoprimals_root = std::env::var("ECOPRIMALS_ROOT")
            .unwrap_or_else(|_| DEFAULT_ECOPRIMALS_ROOT.to_string());

        let golgi_ext_host =
            std::env::var("GOLGI_EXT_HOST").unwrap_or_else(|_| DEFAULT_GOLGI_EXT_HOST.to_string());

        Self {
            ecoprimals_root: PathBuf::from(ecoprimals_root),
            golgi_ext_host,
        }
    }
}

/// Full relay chain: pull → impulse sense → ship extracellular.
///
/// This is the Rust replacement for calling `pepti-sync-relay.sh` followed
/// by `ext-github-push.sh`. Exposed as `membrane relay.run <repo_path>`.
///
/// # Errors
///
/// Returns `Err` only for infrastructure failures (IO, serialization).
pub async fn run(config: &RelayConfig, repo_paths: &[&str]) -> Result<RelayResult> {
    let paths: Vec<&str> = if repo_paths.is_empty() {
        vec!["infra/wateringHole"]
    } else {
        repo_paths.to_vec()
    };

    eprintln!(
        "[relay] K-Derm relay chain triggered for {} repo(s)",
        paths.len()
    );

    let (pulled, pull_failures) = mediate(config, &paths).await;
    let impulses_sensed = sense_impulses(config);
    let (pushed, push_skipped, push_failures) = ship_extracellular(config, &paths).await;

    let result = RelayResult {
        pulled,
        pull_failures,
        impulses_sensed,
        pushed,
        push_skipped,
        push_failures,
    };

    eprintln!(
        "[relay] chain complete: pulled={} pushed={} impulses={} failures={}",
        result.pulled.len(),
        result.pushed.len(),
        result.impulses_sensed,
        result.pull_failures.len() + result.push_failures.len(),
    );

    Ok(result)
}

/// Stage 1: Pull from Forgejo on the local node (metallic bond inward).
///
/// Replaces the pull loop in `pepti-sync-relay.sh`.
/// For each repo path, does a `git pull --ff-only forgejo main`.
pub async fn mediate(config: &RelayConfig, repo_paths: &[&str]) -> (Vec<String>, Vec<String>) {
    let mut pulled = Vec::new();
    let mut failures = Vec::new();

    for &repo_path in repo_paths {
        let local_path = config.ecoprimals_root.join(repo_path);

        if !local_path.join(".git").exists() {
            eprintln!("[relay] SKIP {repo_path} (not cloned on peptidoglycan)");
            continue;
        }

        let status = Command::new("git")
            .args(["pull", "--ff-only", REMOTE_FORGEJO, "main", "--quiet"])
            .current_dir(&local_path)
            .status()
            .await;

        match status {
            Ok(s) if s.success() => {
                eprintln!("[relay] pulled {repo_path} (metallic bond)");
                pulled.push(repo_path.to_string());
            }
            _ => {
                eprintln!("[relay] WARN: {repo_path} pull failed — may be up to date");
                failures.push(repo_path.to_string());
            }
        }
    }

    (pulled, failures)
}

/// Stage 2: Sense pending impulses in the wateringHole.
///
/// Replaces the impulse detection in `pepti-sync-relay.sh` and
/// `impulse-relay-hook.sh`. Uses the `impulse` module directly
/// instead of shelling out to a membrane binary.
fn sense_impulses(config: &RelayConfig) -> usize {
    match impulse::sense(&config.ecoprimals_root, true, true) {
        Ok((_, count)) => {
            if count > 0 {
                eprintln!("[relay] detected {count} pending impulse(s)");
            } else {
                eprintln!("[relay] resting potential (no pending impulses)");
            }
            count
        }
        Err(e) => {
            eprintln!("[relay] WARN: impulse sense failed ({e}) — continuing");
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

/// Ship a single repo via SSH to golgiBody-ext.
///
/// The command on ext:
///   1. Pull from Forgejo (keep ext in sync)
///   2. Determine if ahead of GitHub remote
///   3. If ahead, push to GitHub
async fn ship_one_repo(config: &RelayConfig, repo_path: &str) -> ShipResult {
    let remote_script = format!(
        r#"set -euo pipefail
d="/opt/ecoPrimals/{repo_path}"
[ -d "$d/.git" ] || exit 2
cd "$d"
git pull --ff-only forgejo main --quiet 2>/dev/null || true
REMOTE=$(git remote get-url github 2>/dev/null && echo github || echo origin)
git fetch "$REMOTE" --quiet 2>/dev/null || exit 3
branch=$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo main)
ahead=$(git rev-list --count "$REMOTE/$branch..HEAD" 2>/dev/null || echo 0)
[ "$ahead" -eq 0 ] && exit 4
git push "$REMOTE" "$branch" --quiet 2>/dev/null || exit 5
echo "+$ahead"
"#
    );

    let output = Command::new("ssh")
        .args([
            "-o",
            "ConnectTimeout=5",
            "-o",
            "BatchMode=yes",
            &config.golgi_ext_host,
            &remote_script,
        ])
        .output()
        .await;

    match output {
        Ok(o) => match o.status.code() {
            Some(0) => {
                let commits = String::from_utf8_lossy(&o.stdout).trim().to_string();
                eprintln!("[relay] PUSHED {repo_path} ({commits} commits → GitHub)");
                ShipResult::Pushed
            }
            Some(2) => {
                eprintln!("[relay] SKIP {repo_path} (not cloned on outer membrane)");
                ShipResult::Skipped
            }
            Some(4) => ShipResult::Skipped,
            _ => {
                eprintln!("[relay] FAIL {repo_path} (push to GitHub failed)");
                ShipResult::Failed
            }
        },
        Err(e) => {
            eprintln!("[relay] FAIL SSH to golgiBody-ext ({e})");
            ShipResult::Failed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_config_respects_defaults() {
        let config = RelayConfig {
            ecoprimals_root: PathBuf::from(DEFAULT_ECOPRIMALS_ROOT),
            golgi_ext_host: DEFAULT_GOLGI_EXT_HOST.to_string(),
        };
        assert_eq!(config.ecoprimals_root, PathBuf::from("/opt/ecoPrimals"));
        assert_eq!(config.golgi_ext_host, "golgi-ext");
    }

    #[test]
    fn relay_config_custom_values() {
        let config = RelayConfig {
            ecoprimals_root: PathBuf::from("/tmp/test-eco"),
            golgi_ext_host: "custom-ext".to_string(),
        };
        assert_eq!(config.ecoprimals_root, PathBuf::from("/tmp/test-eco"));
        assert_eq!(config.golgi_ext_host, "custom-ext");
    }

    #[test]
    fn default_constants() {
        assert_eq!(DEFAULT_ECOPRIMALS_ROOT, "/opt/ecoPrimals");
        assert_eq!(REMOTE_FORGEJO, "forgejo");
        assert_eq!(DEFAULT_GOLGI_EXT_HOST, "golgi-ext");
    }

    #[tokio::test]
    async fn mediate_skips_nonexistent_repos() {
        let config = RelayConfig {
            ecoprimals_root: PathBuf::from("/tmp/nonexistent-relay-test"),
            golgi_ext_host: "test".to_string(),
        };
        let (pulled, failures) = mediate(&config, &["no/such/repo"]).await;
        assert!(pulled.is_empty());
        assert!(failures.is_empty());
    }
}
