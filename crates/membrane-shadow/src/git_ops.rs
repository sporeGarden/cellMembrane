// SPDX-License-Identifier: AGPL-3.0-or-later

//! Shared git operations used across membrane-shadow commands.
//!
//! Consolidates `git add`, `commit`, and multi-remote `push` logic
//! that was duplicated in `impulse.rs`, `context.rs`, and elsewhere.

use crate::error::{Result, ShadowError};
use std::path::Path;

/// Default remote push order: sovereign first, then extracellular shadow.
const PUSH_REMOTES: &[&str] = &["forgejo", "origin"];

/// Stage a specific file, commit, and push to all remotes.
pub async fn add_commit_push(repo_dir: &Path, file_path: &str, message: &str) -> Result<()> {
    run_git(repo_dir, &["add", file_path]).await?;
    run_git(repo_dir, &["commit", "-m", message]).await?;
    push_all_remotes(repo_dir).await;
    Ok(())
}

/// Stage all changes in a subdirectory, commit, and push.
pub async fn add_all_commit_push(repo_dir: &Path, subdir: &str, message: &str) -> Result<()> {
    run_git(repo_dir, &["add", "-A", subdir]).await?;
    run_git(repo_dir, &["commit", "-m", message]).await?;
    push_all_remotes(repo_dir).await;
    Ok(())
}

/// Push to all configured remotes (best-effort, non-fatal).
pub async fn push_all_remotes(repo_dir: &Path) {
    for remote in PUSH_REMOTES {
        let _ = tokio::process::Command::new("git")
            .args(["push", remote, "main", "--quiet"])
            .current_dir(repo_dir)
            .status()
            .await;
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_remotes_order() {
        assert_eq!(PUSH_REMOTES[0], "forgejo");
        assert_eq!(PUSH_REMOTES[1], "origin");
    }

    #[test]
    fn head_ref_returns_empty_for_missing_git() {
        let tmp = std::env::temp_dir().join("no-git-here");
        assert!(resolve_head_ref(&tmp).is_empty());
    }
}
