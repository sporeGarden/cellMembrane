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

/// Per-push outcome with failure classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushOutcome {
    /// Push accepted by the remote.
    Ok,
    /// Push rejected because the remote bare repo is shallow and cannot
    /// resolve delta bases for merge commits. Recovery: reshallow from mirror.
    ShallowRejected,
    /// Push failed for another reason (timeout, auth, network, etc.).
    Failed,
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

/// Attempt a single push to a remote with shallow-failure detection.
async fn try_push(repo_dir: &Path, remote: &str) -> bool {
    let outcome = git_push_classified(repo_dir, &["push", remote, "main", "--quiet"]).await;
    if outcome == PushOutcome::ShallowRejected {
        tracing::warn!(
            remote = %remote,
            repo = %repo_dir.display(),
            "post-sync push rejected: shallow bare repo — reshallow needed"
        );
    }
    outcome == PushOutcome::Ok
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
                if let Err(e) = tokio::process::Command::new("git")
                    .args(["rebase", "--abort"])
                    .current_dir(repo_dir)
                    .status()
                    .await
                {
                    tracing::warn!(repo = %repo_dir.display(), error = %e, "rebase --abort failed");
                }
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
pub async fn run_git(repo_dir: &Path, args: &[&str]) -> Result<()> {
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

/// Resolve the full HEAD commit SHA (sync). Returns `None` if not a git repo.
#[must_use]
pub fn resolve_head_full(project_path: &Path) -> Option<String> {
    if !project_path.join(".git").exists() {
        return None;
    }
    std::process::Command::new("git")
        .args(["-C", &project_path.to_string_lossy(), "rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
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

/// Run a git push command with stderr capture for failure classification.
///
/// Detects shallow-repo rejections (where the remote bare repo cannot resolve
/// delta bases for merge commits) and classifies them distinctly from general
/// push failures. This enables callers to trigger auto-reshallow recovery.
pub async fn git_push_classified(repo_path: &Path, args: &[&str]) -> PushOutcome {
    let child = git_command(repo_path, args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output();

    let Ok(Ok(output)) = tokio::time::timeout(GIT_OP_TIMEOUT, child).await else {
        return PushOutcome::Failed;
    };

    if output.status.success() {
        return PushOutcome::Ok;
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    if is_shallow_rejection(&stderr) {
        PushOutcome::ShallowRejected
    } else {
        let detail = stderr.lines().next().unwrap_or("unknown error");
        tracing::debug!(
            repo = %repo_path.display(),
            error = %detail,
            "git push failed"
        );
        PushOutcome::Failed
    }
}

/// Detect whether a push failure is caused by a shallow bare repo.
///
/// Git produces various error messages when pushing objects that reference
/// bases missing in a shallow clone: "shallow update not allowed",
/// "unresolved deltas", "missing tree", "bad object".
fn is_shallow_rejection(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("shallow update not allowed")
        || lower.contains("unresolved delta")
        || (lower.contains("missing") && lower.contains("tree"))
        || lower.contains("unable to read")
}

/// Run a git command, returning stdout as `Option` (returns `None` on failure/timeout).
pub async fn git_output_opt(repo_path: &Path, args: &[&str]) -> Option<String> {
    git_output(repo_path, args).await.ok()
}

/// Resolve HEAD as a short ref (8 chars). Returns `None` if not a git repo.
pub async fn head_short(repo_path: &Path) -> Option<String> {
    git_output_opt(repo_path, &["rev-parse", "--short=8", "HEAD"]).await
}

/// Clone a repo from `url` into `dest`. Fails if `dest` already exists.
pub async fn git_clone(url: &str, dest: &Path) -> Result<()> {
    let status = tokio::time::timeout(
        GIT_OP_TIMEOUT,
        tokio::process::Command::new("git")
            .args(["clone", url, &dest.to_string_lossy()])
            .env("GIT_SSH_COMMAND", SSH_CMD_WITH_TIMEOUT)
            .status(),
    )
    .await
    .map_err(|_| {
        ShadowError::Git(format!(
            "git clone timed out after {}s",
            GIT_OP_TIMEOUT.as_secs()
        ))
    })?
    .map_err(ShadowError::Io)?;

    if !status.success() {
        return Err(ShadowError::Git(format!("git clone {url} failed")));
    }
    Ok(())
}

/// Fast-forward pull from `remote` on branch `main`.
pub async fn pull_ff_only(repo_path: &Path, remote: &str) -> bool {
    git_success(repo_path, &["pull", "--ff-only", remote, "main", "--quiet"]).await
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

    #[test]
    fn resolve_head_full_returns_full_sha() {
        let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let workspace = crate_dir.parent().unwrap().parent().unwrap();
        let full = resolve_head_full(workspace);
        assert!(full.is_some());
        let sha = full.unwrap();
        assert_eq!(sha.len(), 40, "full SHA should be 40 hex chars");
        assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn resolve_head_full_returns_none_for_missing() {
        assert!(resolve_head_full(Path::new("/tmp/no-git-full-sha")).is_none());
    }

    #[tokio::test]
    async fn git_clone_fails_for_bogus_url() {
        let dest = std::env::temp_dir().join("git-clone-bogus-test");
        let _ = std::fs::remove_dir_all(&dest);
        let result = git_clone("https://invalid.test/no-repo.git", &dest).await;
        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[tokio::test]
    async fn pull_ff_only_returns_false_for_missing_dir() {
        let tmp = std::env::temp_dir().join("no-git-pull-ff");
        assert!(!pull_ff_only(&tmp, "origin").await);
    }

    #[tokio::test]
    async fn run_git_pub_works() {
        let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let workspace = crate_dir.parent().unwrap().parent().unwrap();
        assert!(run_git(workspace, &["status"]).await.is_ok());
    }

    #[tokio::test]
    async fn run_git_fails_on_bad_args() {
        let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let workspace = crate_dir.parent().unwrap().parent().unwrap();
        let result = run_git(workspace, &["checkout", "--this-flag-does-not-exist"]).await;
        assert!(result.is_err());
    }

    #[test]
    fn shallow_rejection_detected() {
        assert!(is_shallow_rejection(
            "error: shallow update not allowed"
        ));
        assert!(is_shallow_rejection(
            "fatal: pack has unresolved deltas"
        ));
        assert!(is_shallow_rejection(
            "error: missing tree 0000000000000000000000000000000000000000"
        ));
        assert!(is_shallow_rejection(
            "error: unable to read sha1 file of blob"
        ));
    }

    #[test]
    fn shallow_rejection_negative() {
        assert!(!is_shallow_rejection("Everything up-to-date"));
        assert!(!is_shallow_rejection(
            "error: failed to push some refs to 'forgejo'"
        ));
        assert!(!is_shallow_rejection("remote: Permission denied"));
    }

    #[test]
    fn push_outcome_variants() {
        assert_eq!(PushOutcome::Ok, PushOutcome::Ok);
        assert_ne!(PushOutcome::Ok, PushOutcome::ShallowRejected);
        assert_ne!(PushOutcome::ShallowRejected, PushOutcome::Failed);
    }

    #[tokio::test]
    async fn classified_push_returns_failed_for_bogus_remote() {
        let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let workspace = crate_dir.parent().unwrap().parent().unwrap();
        let outcome =
            git_push_classified(workspace, &["push", "nonexistent-remote", "main"]).await;
        assert_eq!(outcome, PushOutcome::Failed);
    }
}
