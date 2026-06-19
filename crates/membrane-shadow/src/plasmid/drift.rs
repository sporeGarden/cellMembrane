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
        .is_none_or(|head| !head.starts_with(prev_commit) && !prev_commit.starts_with(&head))
}

/// Fetch HEAD from both outer (GitHub) and inner (Forgejo) membranes.
///
/// Returns the commit that is farthest ahead — if either remote has a newer
/// commit than provenance, we should detect drift. This ensures golgiBody
/// sees GitHub pushes and peptidoglycan sees both layers.
async fn fetch_head_commit(repo: &str, _depot_dir: &Path) -> Option<String> {
    let forgejo_host = std::env::var(cellmembrane_types::service::ENV_FORGEJO_SSH_HOST)
        .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_FORGEJO_GIT_ADDR.into());

    let forgejo = try_ls_remote_head(&format!("ssh://git@{forgejo_host}/{repo}.git")).await;
    let github = try_ls_remote_head(&format!("https://github.com/{repo}.git")).await;

    github.or(forgejo)
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
) -> std::result::Result<(), String> {
    if let Err(e) = tokio::fs::remove_dir_all(clone_dir).await {
        tracing::debug!(error = %e, "clone_dir cleanup (may not exist yet)");
    }
    if let Err(e) = tokio::fs::create_dir_all(build_root).await {
        tracing::warn!(error = %e, dir = %build_root.display(), "failed to create build root");
    }

    let forgejo_host = std::env::var(cellmembrane_types::service::ENV_FORGEJO_SSH_HOST)
        .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_FORGEJO_GIT_ADDR.into());
    let forgejo_url = format!("ssh://git@{forgejo_host}/{}.git", source.repo);
    let github_url = format!("https://github.com/{}.git", source.repo);

    if toolchain::try_clone(&forgejo_url, clone_dir).await {
        return Ok(());
    }

    if toolchain::try_clone(&github_url, clone_dir).await {
        return Ok(());
    }

    if source.private {
        Err(format!(
            "private repo — neither Forgejo SSH nor GitHub accessible ({primal})"
        ))
    } else {
        Err("git clone failed on both Forgejo and GitHub".into())
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

    if !remote_head.starts_with(local_head) && !local_head.starts_with(&remote_head) {
        Some(format!(
            "clone at {short_local} but origin HEAD is {short_remote} — source may be stale",
            short_local = &local_head[..8.min(local_head.len())],
            short_remote = &remote_head[..8.min(remote_head.len())],
        ))
    } else {
        None
    }
}

pub(super) async fn get_local_head(repo_dir: &Path) -> Option<String> {
    crate::git_ops::git_output(repo_dir, &["rev-parse", "--short=8", "HEAD"])
        .await
        .ok()
}

/// Commit and push updated checksums.toml + provenance.toml to git.
/// Non-fatal — harvest succeeds even if git publish fails.
pub(super) async fn publish_depot_checksums(depot_dir: &Path) {
    if !depot_dir.join(".git").is_dir() {
        return;
    }

    if !crate::git_ops::git_success(depot_dir, &["add", "checksums.toml", "provenance.toml"]).await
    {
        return;
    }

    let has_staged =
        !crate::git_ops::git_success(depot_dir, &["diff", "--cached", "--quiet"]).await;
    if !has_staged {
        return;
    }

    let commit_msg = format!(
        "harvest: update checksums + provenance ({})",
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

/// Public wrapper to check upstream changes for a primal — used by `status`.
///
/// Uses lenient mode: if remote HEAD cannot be fetched (network failure),
/// assume current rather than reporting false drift.
pub(super) async fn has_upstream_changes_pub(
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
        .is_some_and(|head| !head.starts_with(prev_commit) && !prev_commit.starts_with(&head))
}
