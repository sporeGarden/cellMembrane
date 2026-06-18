// SPDX-License-Identifier: AGPL-3.0-or-later

//! Shared git operations used across membrane-shadow commands.
//!
//! Consolidates `git add`, `commit`, and multi-remote `push` logic
//! that was duplicated in `impulse.rs`, `context.rs`, and elsewhere.

use crate::error::{Result, ShadowError};
use std::path::Path;

use cellmembrane_types::service::DEFAULT_PUSH_REMOTES;

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

/// Push to all configured remotes with auto-reconciliation on non-fast-forward.
///
/// For each remote: attempt push. If rejected (non-ff), fetch + rebase + retry.
/// This eliminates the chronic diderm divergence where parallel gate pushes
/// create non-ff rejections requiring manual `--force-with-lease`.
pub async fn push_all_remotes(repo_dir: &Path) -> PushResult {
    let remotes = resolve_push_remotes();
    let mut result = PushResult {
        succeeded: 0,
        failed: Vec::new(),
    };
    for remote in &remotes {
        let ok = try_push(repo_dir, remote).await || reconcile_and_push(repo_dir, remote).await;
        if ok {
            result.succeeded += 1;
        } else {
            result.failed.push(remote.clone());
        }
    }
    result
}

/// Attempt a single push to a remote.
async fn try_push(repo_dir: &Path, remote: &str) -> bool {
    tokio::process::Command::new("git")
        .args(["push", remote, "main", "--quiet"])
        .current_dir(repo_dir)
        .env("GIT_SSH_COMMAND", SSH_CMD_WITH_TIMEOUT)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .is_ok_and(|s| s.success())
}

/// Auto-reconcile a non-ff rejection: fetch, try ff-merge, fallback to rebase.
///
/// Strategy (SHA-preserving when possible):
/// 1. Fetch the remote
/// 2. Try `merge --ff-only` — preserves SHA identity if local is simply behind
/// 3. If ff-merge fails (diverged), fall back to rebase
/// 4. Push (retry up to 2 times)
async fn reconcile_and_push(repo_dir: &Path, remote: &str) -> bool {
    for _attempt in 0..2 {
        let fetch_ok = tokio::process::Command::new("git")
            .args(["fetch", remote, "main"])
            .current_dir(repo_dir)
            .env("GIT_SSH_COMMAND", SSH_CMD_WITH_TIMEOUT)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .is_ok_and(|s| s.success());

        if !fetch_ok {
            return false;
        }

        let merge_ref = format!("{remote}/main");

        // Try fast-forward merge first — preserves SHA identity
        let ff_ok = tokio::process::Command::new("git")
            .args(["merge", "--ff-only", &merge_ref])
            .current_dir(repo_dir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .is_ok_and(|s| s.success());

        if !ff_ok {
            // Diverged — fall back to rebase
            let rebase_ok = tokio::process::Command::new("git")
                .args(["rebase", &merge_ref])
                .current_dir(repo_dir)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .await
                .is_ok_and(|s| s.success());

            if !rebase_ok {
                let _ = tokio::process::Command::new("git")
                    .args(["rebase", "--abort"])
                    .current_dir(repo_dir)
                    .status()
                    .await;
                return false;
            }
        }

        if try_push(repo_dir, remote).await {
            return true;
        }
    }
    false
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
        return Err(ShadowError::Git(format!(
            "{} failed in {}",
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

const GIT_OP_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(cellmembrane_types::service::DEFAULT_GIT_OP_TIMEOUT_SECS);

const SSH_CMD_WITH_TIMEOUT: &str =
    "ssh -o ConnectTimeout=10 -o ServerAliveInterval=5 -o ServerAliveCountMax=3 -o BatchMode=yes";

fn git_command(repo_path: &Path, args: &[&str]) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("git");
    cmd.arg("-C")
        .arg(repo_path)
        .env("GIT_SSH_COMMAND", SSH_CMD_WITH_TIMEOUT)
        .args(args);
    cmd
}

/// Run a git command in a repo directory, returning stdout as a trimmed string.
///
/// Enforces a 60-second timeout to prevent cascade hangs on unreachable remotes.
pub async fn git_output(repo_path: &Path, args: &[&str]) -> Result<String> {
    let child = git_command(repo_path, args).output();

    let output = tokio::time::timeout(GIT_OP_TIMEOUT, child)
        .await
        .map_err(|_| {
            ShadowError::Parse(format!(
                "git {:?} timed out after {}s",
                args.first().unwrap_or(&"?"),
                GIT_OP_TIMEOUT.as_secs(),
            ))
        })?
        .map_err(ShadowError::Io)?;

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Run a git command, returning true if it exits successfully.
///
/// Enforces a 60-second timeout — returns `false` on timeout.
pub async fn git_success(repo_path: &Path, args: &[&str]) -> bool {
    let child = git_command(repo_path, args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    tokio::time::timeout(GIT_OP_TIMEOUT, child)
        .await
        .is_ok_and(|r| r.is_ok_and(|s| s.success()))
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

    #[test]
    fn head_ref_returns_value_for_real_repo() {
        let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let workspace_root = crate_dir.parent().unwrap().parent().unwrap();
        let head = resolve_head_ref(workspace_root);
        assert!(!head.is_empty(), "should return a commit SHA");
        assert!(head.len() >= 7 && head.len() <= 12);
    }

    #[test]
    fn git_op_timeout_is_60s() {
        assert_eq!(GIT_OP_TIMEOUT.as_secs(), 60);
    }

    #[test]
    fn push_result_fields() {
        let r = PushResult {
            succeeded: 2,
            failed: vec!["upstream".into()],
        };
        assert_eq!(r.succeeded, 2);
        assert_eq!(r.failed.len(), 1);
    }

    #[tokio::test]
    async fn git_success_on_real_repo() {
        let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let workspace = crate_dir.parent().unwrap().parent().unwrap();
        assert!(git_success(workspace, &["status"]).await);
    }

    #[tokio::test]
    async fn git_output_status() {
        let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let workspace = crate_dir.parent().unwrap().parent().unwrap();
        let output = git_output(workspace, &["rev-parse", "--short", "HEAD"]).await;
        assert!(output.is_ok());
        assert!(!output.unwrap().is_empty());
    }

    #[tokio::test]
    async fn rev_list_count_returns_zero_for_bogus() {
        let tmp = std::env::temp_dir().join("no-git-revlist");
        let count = rev_list_count(&tmp, "HEAD").await;
        assert_eq!(count, 0);
    }
}
