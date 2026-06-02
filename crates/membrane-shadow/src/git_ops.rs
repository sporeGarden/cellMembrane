// SPDX-License-Identifier: AGPL-3.0-or-later

//! Shared git operations used across membrane-shadow commands.
//!
//! Consolidates `git add`, `commit`, and multi-remote `push` logic
//! that was duplicated in `impulse.rs`, `context.rs`, and elsewhere.

use crate::error::{Result, ShadowError};
use std::path::Path;

/// Fallback remote push order when manifest is unavailable.
const DEFAULT_PUSH_REMOTES: &[&str] = &["forgejo", "origin"];

/// Resolve push remotes from manifest `[sync]` config, falling back to defaults.
fn resolve_push_remotes() -> Vec<String> {
    if let Ok(root) = crate::temporal::resolve_workspace_root() {
        if let Ok(m) = crate::manifest::load_from_workspace(&root) {
            if !m.sync.push_remotes.is_empty() {
                return m.sync.push_remotes;
            }
        }
    }
    DEFAULT_PUSH_REMOTES
        .iter()
        .map(|s| (*s).to_string())
        .collect()
}

/// Outcome of a multi-remote push operation.
pub struct PushResult {
    /// Number of remotes that accepted the push.
    pub succeeded: u32,
    /// Remote names that rejected or failed.
    pub failed: Vec<String>,
}

/// Stage a specific file, commit, and push to all remotes.
///
/// Returns the push result so callers can surface partial failures.
pub async fn add_commit_push(
    repo_dir: &Path,
    file_path: &str,
    message: &str,
) -> Result<PushResult> {
    run_git(repo_dir, &["add", file_path]).await?;
    run_git(repo_dir, &["commit", "-m", message]).await?;
    Ok(push_all_remotes(repo_dir).await)
}

/// Stage all changes in a subdirectory, commit, and push.
pub async fn add_all_commit_push(
    repo_dir: &Path,
    subdir: &str,
    message: &str,
) -> Result<PushResult> {
    run_git(repo_dir, &["add", "-A", subdir]).await?;
    run_git(repo_dir, &["commit", "-m", message]).await?;
    Ok(push_all_remotes(repo_dir).await)
}

/// Push to all configured remotes, returning per-remote results.
pub async fn push_all_remotes(repo_dir: &Path) -> PushResult {
    let remotes = resolve_push_remotes();
    let mut result = PushResult {
        succeeded: 0,
        failed: Vec::new(),
    };
    for remote in &remotes {
        let ok = tokio::process::Command::new("git")
            .args(["push", remote, "main", "--quiet"])
            .current_dir(repo_dir)
            .status()
            .await
            .is_ok_and(|s| s.success());
        if ok {
            result.succeeded += 1;
        } else {
            result.failed.push(remote.clone());
        }
    }
    result
}

/// Run a git command, returning an error on non-zero exit.
async fn run_git(repo_dir: &Path, args: &[&str]) -> Result<()> {
    let status = tokio::process::Command::new("git")
        .args(args)
        .current_dir(repo_dir)
        .status()
        .await
        .map_err(ShadowError::Io)?;
    if !status.success() {
        return Err(ShadowError::Parse(format!(
            "git {} failed in {}",
            args.join(" "),
            repo_dir.display()
        )));
    }
    Ok(())
}

/// Resolve the HEAD commit short SHA for a path containing a git repo.
#[must_use]
pub fn resolve_head_ref(project_path: &Path) -> String {
    if !project_path.join(".git").exists() {
        return String::new();
    }
    std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(project_path)
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_default()
}

// ── Async git utilities (used by temporal, dispatch, etc.) ───────────

/// Run a git command in a repo directory, returning stdout as a trimmed string.
pub async fn git_output(repo_path: &Path, args: &[&str]) -> Result<String> {
    let output = tokio::process::Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(args)
        .output()
        .await
        .map_err(ShadowError::Io)?;

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Run a git command, returning true if it exits successfully.
pub async fn git_success(repo_path: &Path, args: &[&str]) -> bool {
    tokio::process::Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .is_ok_and(|s| s.success())
}

/// Count commits in a rev-list range (e.g. `"origin/main..HEAD"`).
pub async fn rev_list_count(repo_path: &Path, range: &str) -> u32 {
    git_output(repo_path, &["rev-list", "--count", range])
        .await
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_push_remotes_order() {
        assert_eq!(DEFAULT_PUSH_REMOTES[0], "forgejo");
        assert_eq!(DEFAULT_PUSH_REMOTES[1], "origin");
    }

    #[test]
    fn head_ref_returns_empty_for_missing_git() {
        let tmp = std::env::temp_dir().join("no-git-here");
        assert!(resolve_head_ref(&tmp).is_empty());
    }
}
