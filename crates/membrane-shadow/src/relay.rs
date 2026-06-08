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

/// Configuration for the relay chain, resolved from environment + config.
#[derive(Debug, Clone)]
pub struct RelayConfig {
    /// Root of the ecoPrimals workspace on this node.
    pub ecoprimals_root: PathBuf,
    /// Forgejo remote name for pull operations.
    pub forgejo_remote: String,
    /// SSH host alias for golgiBody-ext (outer membrane).
    pub golgi_ext_host: String,
}

impl RelayConfig {
    /// Resolve configuration from membrane.toml, environment variables, then defaults.
    ///
    /// Priority: membrane.toml [relay] > environment > built-in defaults.
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

        let forgejo_remote = std::env::var("RELAY_FORGEJO_REMOTE")
            .ok()
            .or_else(|| {
                membrane_config
                    .as_ref()
                    .and_then(|c| c.forgejo_remote.clone())
            })
            .unwrap_or_else(|| "forgejo".to_string());

        let golgi_ext_host = std::env::var("GOLGI_EXT_HOST")
            .ok()
            .or_else(|| {
                membrane_config
                    .as_ref()
                    .and_then(|c| c.golgi_ext_host.clone())
            })
            .unwrap_or_else(|| "golgi-ext".to_string());

        Self {
            ecoprimals_root: PathBuf::from(ecoprimals_root),
            forgejo_remote,
            golgi_ext_host,
        }
    }
}

/// Optional [relay] section from membrane.toml.
#[derive(Debug, serde::Deserialize)]
struct MembraneRelayConfig {
    ecoprimals_root: Option<String>,
    forgejo_remote: Option<String>,
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
    let config_home = std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        format!("{home}/.config")
    });
    let path = PathBuf::from(config_home).join("ecoPrimals/membrane.toml");
    if path.exists() { Some(path) } else { None }
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
            .args([
                "pull",
                "--ff-only",
                &config.forgejo_remote,
                "main",
                "--quiet",
            ])
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
    let eco_root = config.ecoprimals_root.to_string_lossy();
    let remote_script = format!(
        r#"set -euo pipefail
d="{eco_root}/{repo_path}"
[ -d "$d/.git" ] || exit 2
cd "$d"
git pull --ff-only forgejo main --quiet 2>/dev/null || true
REMOTE=$(git remote get-url github >/dev/null 2>&1 && echo github || echo origin)
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
            ecoprimals_root: PathBuf::from(cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT),
            forgejo_remote: "forgejo".to_string(),
            golgi_ext_host: "golgi-ext".to_string(),
        };
        assert_eq!(
            config.ecoprimals_root,
            PathBuf::from(cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT)
        );
        assert_eq!(config.forgejo_remote, "forgejo");
        assert_eq!(config.golgi_ext_host, "golgi-ext");
    }

    #[test]
    fn relay_config_custom_values() {
        let config = RelayConfig {
            ecoprimals_root: PathBuf::from("/tmp/test-eco"),
            forgejo_remote: "forgejo".to_string(),
            golgi_ext_host: "custom-ext".to_string(),
        };
        assert_eq!(config.ecoprimals_root, PathBuf::from("/tmp/test-eco"));
        assert_eq!(config.forgejo_remote, "forgejo");
        assert_eq!(config.golgi_ext_host, "custom-ext");
    }

    #[tokio::test]
    async fn mediate_skips_nonexistent_repos() {
        let config = RelayConfig {
            ecoprimals_root: PathBuf::from("/tmp/nonexistent-relay-test"),
            forgejo_remote: "forgejo".to_string(),
            golgi_ext_host: "test".to_string(),
        };
        let (pulled, failures) = mediate(&config, &["no/such/repo"]).await;
        assert!(pulled.is_empty());
        assert!(failures.is_empty());
    }

    #[test]
    fn relay_result_serializes() {
        let result = RelayResult {
            pulled: vec!["bearDog".into()],
            pull_failures: vec![],
            impulses_sensed: 2,
            pushed: vec!["bearDog".into()],
            push_skipped: vec![],
            push_failures: vec!["songBird".into()],
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["pulled"][0], "bearDog");
        assert_eq!(json["impulses_sensed"], 2);
        assert_eq!(json["push_failures"][0], "songBird");
    }

    #[test]
    fn ship_result_variants() {
        assert!(matches!(ShipResult::Pushed, ShipResult::Pushed));
        assert!(matches!(ShipResult::Skipped, ShipResult::Skipped));
        assert!(matches!(ShipResult::Failed, ShipResult::Failed));
    }
}
