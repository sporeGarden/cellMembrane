// SPDX-License-Identifier: AGPL-3.0-or-later

//! Drift detection, clone management, and depot publishing for harvest.
//!
//! Handles upstream change detection (GitHub/Forgejo HEAD comparison),
//! source cloning, clone freshness verification, and depot checksum
//! git publishing.

use std::path::Path;

use super::harvest::{ProvenanceFile, SourceEntry};
use super::toolchain;

pub(super) async fn has_upstream_changes(
    primal: &str,
    source: &SourceEntry,
    provenance: Option<&ProvenanceFile>,
    depot_dir: &Path,
) -> bool {
    let Some(prov) = provenance else {
        return true;
    };
    let Some(entry) = prov.entries.get(primal) else {
        return true;
    };
    let Some(prev_commit) = entry.commit.as_deref() else {
        return true;
    };

    fetch_head_commit(&source.repo, depot_dir)
        .await
        .is_none_or(|head| !commits_match(&head, prev_commit))
}

/// Fetch HEAD from both outer (GitHub) and inner (Forgejo) membranes.
///
/// Prefers sovereign (Forgejo) as the authoritative source. Falls back
/// to GitHub when Forgejo is unreachable. If both respond, returns
/// Forgejo HEAD since sovereign-inner pushes are the primary flow.
async fn fetch_head_commit(repo: &str, _depot_dir: &Path) -> Option<String> {
    let forgejo_host = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_FORGEJO_SSH_HOST,
        cellmembrane_types::service::DEFAULT_FORGEJO_GIT_ADDR,
    );

    let forgejo = try_ls_remote_head(&format!("ssh://git@{forgejo_host}/{repo}.git")).await;
    let github = try_ls_remote_head(&format!("https://github.com/{repo}.git")).await;

    forgejo.or(github)
}

async fn try_ls_remote_head(url: &str) -> Option<String> {
    let output =
        crate::git_ops::git_output_opt(std::path::Path::new("."), &["ls-remote", url, "HEAD"])
            .await?;
    output.split_whitespace().next().map(|s| s[..8].to_string())
}

pub(super) async fn clone_source(
    primal: &str,
    source: &SourceEntry,
    build_root: &Path,
    clone_dir: &Path,
) -> crate::Result<()> {
    if let Err(e) = tokio::fs::remove_dir_all(clone_dir).await {
        tracing::debug!(error = %e, "clone_dir cleanup (may not exist yet)");
    }
    if let Err(e) = tokio::fs::create_dir_all(build_root).await {
        tracing::warn!(error = %e, dir = %build_root.display(), "failed to create build root");
    }

    let forgejo_host = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_FORGEJO_SSH_HOST,
        cellmembrane_types::service::DEFAULT_FORGEJO_GIT_ADDR,
    );
    let forgejo_url = format!("ssh://git@{forgejo_host}/{}.git", source.repo);
    let github_url = format!("https://github.com/{}.git", source.repo);

    if toolchain::try_clone(&forgejo_url, clone_dir).await {
        return Ok(());
    }

    if toolchain::try_clone(&github_url, clone_dir).await {
        return Ok(());
    }

    if source.private {
        Err(crate::error::ShadowError::Git(format!(
            "private repo — neither Forgejo SSH nor GitHub accessible ({primal})"
        )))
    } else {
        Err(crate::error::ShadowError::Git(
            "git clone failed on both Forgejo and GitHub".into(),
        ))
    }
}

/// Verify the clone is at the same HEAD as the upstream origin.
/// Returns `Some(warning)` if the clone appears stale, `None` if fresh or unverifiable.
pub(super) async fn check_clone_freshness(
    _primal: &str,
    source: &SourceEntry,
    clone_dir: &Path,
    local_head: &str,
) -> Option<String> {
    if local_head.is_empty() {
        return Some("could not determine local HEAD".into());
    }

    let github_url = format!("https://github.com/{}.git", source.repo);
    let output =
        crate::git_ops::git_output_opt(clone_dir, &["ls-remote", &github_url, "HEAD"]).await?;

    let remote_head = output.split_whitespace().next().unwrap_or("").to_string();

    if remote_head.is_empty() {
        return None;
    }

    if commits_match(&remote_head, local_head) {
        None
    } else {
        Some(format!(
            "clone at {short_local} but origin HEAD is {short_remote} — source may be stale",
            short_local = &local_head[..8.min(local_head.len())],
            short_remote = &remote_head[..8.min(remote_head.len())],
        ))
    }
}

/// Commit and push updated checksums.toml + provenance.toml to git.
/// Non-fatal — harvest succeeds even if git publish fails.
pub(super) async fn publish_depot_checksums(depot_dir: &Path) {
    if !depot_dir.join(".git").is_dir() {
        return;
    }

    let mut add_args = vec!["add", "checksums.toml", "provenance.toml"];
    if depot_dir.join("signatures.toml").exists() {
        add_args.push("signatures.toml");
    }
    if !crate::git_ops::git_success(depot_dir, &add_args).await {
        return;
    }

    let has_staged =
        !crate::git_ops::git_success(depot_dir, &["diff", "--cached", "--quiet"]).await;
    if !has_staged {
        return;
    }

    let signed_label = if depot_dir.join("signatures.toml").exists() {
        " + signatures"
    } else {
        ""
    };
    let commit_msg = format!(
        "harvest: update checksums + provenance{signed_label} ({})",
        chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ")
    );

    if let Err(e) = crate::git_ops::git_output(depot_dir, &["commit", "-m", &commit_msg]).await {
        tracing::warn!(error = %e, "depot checksum commit failed");
    }
    let push = crate::git_ops::push_all_remotes(depot_dir).await;
    if !push.failed.is_empty() {
        tracing::warn!(
            failed = ?push.failed,
            succeeded = push.succeeded,
            "depot checksum push had failures"
        );
    }
}

/// Check upstream changes for a primal — used by `status`.
///
/// Uses lenient mode: if remote HEAD cannot be fetched (network failure),
/// assume current rather than reporting false drift.
pub(super) async fn has_upstream_changes_lenient(
    primal: &str,
    source: &SourceEntry,
    provenance: Option<&ProvenanceFile>,
    depot_dir: &Path,
) -> bool {
    let Some(prov) = provenance else {
        return true;
    };
    let Some(entry) = prov.entries.get(primal) else {
        return true;
    };
    let Some(prev_commit) = entry.commit.as_deref() else {
        return true;
    };

    fetch_head_commit(&source.repo, depot_dir)
        .await
        .is_some_and(|head| !commits_match(&head, prev_commit))
}

/// Two (possibly truncated) commit SHAs match if one is a prefix of the other.
fn commits_match(a: &str, b: &str) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commits_match_identical() {
        assert!(commits_match("abc123", "abc123"));
    }

    #[test]
    fn commits_match_prefix_left() {
        assert!(commits_match("abc12345", "abc123"));
    }

    #[test]
    fn commits_match_prefix_right() {
        assert!(commits_match("abc123", "abc12345"));
    }

    #[test]
    fn commits_match_different() {
        assert!(!commits_match("abc123", "def456"));
    }

    #[test]
    fn commits_match_empty() {
        assert!(commits_match("", ""));
        assert!(commits_match("abc", ""));
        assert!(commits_match("", "abc"));
    }

    #[test]
    fn commits_match_partial_overlap_not_prefix() {
        assert!(!commits_match("abc123", "abc124"));
    }
}
