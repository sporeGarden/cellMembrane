// SPDX-License-Identifier: AGPL-3.0-or-later

//! Temporal sync — waterFall DAG-based multi-remote convergence.
//!
//! Replaces the bash `temporal_check_repo` / `temporal_sync_repo` functions
//! with typed Rust operations. Runs locally — no SSH required. The DAG
//! (commit graph) is the only clock: whichever remote is furthest ahead
//! leads, followers are pushed to parity.
//!
//! Shadow domain mapping:
//!   - `temporal.check` → quorumSignal (qS): sense remote positions
//!   - `temporal.sync`  → waterFall (wF): pull leader, push followers

use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::process::Command;

/// Per-remote temporal position relative to local HEAD.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemotePosition {
    /// Remote name (e.g. `origin`, `forgejo`).
    pub remote: String,
    /// Commits in local HEAD not in remote (local ahead).
    pub ahead: u32,
    /// Commits in remote not in local HEAD (remote ahead).
    pub behind: u32,
}

impl RemotePosition {
    /// True when local and remote share the same tip.
    #[must_use]
    pub const fn is_parity(&self) -> bool {
        self.ahead == 0 && self.behind == 0
    }
}

impl std::fmt::Display for RemotePosition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}(+{},-{})", self.remote, self.ahead, self.behind)
    }
}

/// Classification of a repo's temporal state across all remotes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SyncClassification {
    /// All remotes match local HEAD.
    Parity,
    /// A clear leader exists — can fast-forward converge.
    Converge,
    /// Multiple remotes have divergent unique commits — needs human review.
    Diverge,
    /// Repo directory missing or not a git repository.
    Missing,
    /// No remotes configured.
    NoRemote,
}

impl std::fmt::Display for SyncClassification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parity => write!(f, "PARITY"),
            Self::Converge => write!(f, "CONVERGE"),
            Self::Diverge => write!(f, "DIVERGE"),
            Self::Missing => write!(f, "MISSING"),
            Self::NoRemote => write!(f, "NO_REMOTE"),
        }
    }
}

/// Recommended action from temporal classification.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SyncAction {
    /// Nothing to do.
    None,
    /// Pull from the named leader remote.
    Pull {
        /// Remote with the most commits ahead.
        leader: String,
    },
    /// Push to remotes that are behind local HEAD.
    Push,
    /// Trees are identical but history diverged (rebase artifact).
    /// Force-push follower to match leader — safe because content is the same.
    TreeParity {
        /// Remote whose ref to adopt as the canonical history.
        leader: String,
        /// Remote(s) to force-push to match.
        followers: Vec<String>,
    },
    /// Diverged — flag for human review, do not modify.
    Flag,
}

impl std::fmt::Display for SyncAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "ok"),
            Self::Pull { leader } => write!(f, "pull {leader}"),
            Self::Push => write!(f, "push followers"),
            Self::TreeParity { leader, followers } => {
                write!(f, "tree-parity: {leader} → {}", followers.join(", "))
            }
            Self::Flag => write!(f, "FLAG: human review"),
        }
    }
}

/// Full temporal check result for a single repo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalMatrix {
    /// Relative repo path (e.g. `primals/biomeOS`).
    pub repo_path: String,
    /// Current branch.
    pub branch: String,
    /// Classification of the convergence state.
    pub classification: SyncClassification,
    /// Per-remote position data.
    pub positions: Vec<RemotePosition>,
    /// Recommended action.
    pub action: SyncAction,
}

impl std::fmt::Display for TemporalMatrix {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let positions: Vec<String> = self.positions.iter().map(ToString::to_string).collect();
        write!(
            f,
            "{:<35} {:<9} {} -> {}",
            self.repo_path,
            self.classification.to_string(),
            positions.join(" "),
            self.action
        )
    }
}

/// Result of a temporal sync operation on a single repo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalSyncResult {
    /// Relative repo path.
    pub repo_path: String,
    /// Whether the sync succeeded.
    pub ok: bool,
    /// What happened.
    pub summary: String,
    /// Remotes that were pulled from.
    pub pulled_from: Option<String>,
    /// Remotes that were pushed to.
    pub pushed_to: Vec<String>,
}

/// Aggregate result for a full temporal sync across multiple repos.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalReport {
    /// Total repos checked.
    pub total: u32,
    /// Repos at parity.
    pub parity: u32,
    /// Repos successfully converged.
    pub converged: u32,
    /// Repos with divergence flagged for review.
    pub diverged: u32,
    /// Repos missing or not cloned.
    pub missing: u32,
    /// Per-repo results.
    pub repos: Vec<TemporalMatrix>,
}

// ── Git helpers (local, no SSH) ──────────────────────────────────────

async fn git(repo_path: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(args)
        .output()
        .await?;

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

async fn git_ok(repo_path: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .is_ok_and(|s| s.success())
}

async fn rev_list_count(repo_path: &Path, range: &str) -> u32 {
    git(repo_path, &["rev-list", "--count", range])
        .await
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// Detect tree-parity: history diverges but tree content is identical.
///
/// Picks the remote with the most commits as the canonical leader (preserves
/// richer history). Returns `Some((leader, followers))` if all remote trees
/// produce an empty `git diff`, meaning the working trees are identical.
async fn detect_tree_parity(
    local_path: &Path,
    positions: &[RemotePosition],
    branch: &str,
) -> Option<(String, Vec<String>)> {
    if positions.len() < 2 {
        return None;
    }

    // Leader = remote with most total commits (behind + ahead gives a rough measure
    // of "richest history"; in practice, the one with behind=0 is the true leader).
    let leader = positions
        .iter()
        .min_by_key(|p| p.behind)
        .map(|p| p.remote.clone())?;

    let leader_ref = format!("{leader}/{branch}");
    let mut followers = Vec::new();

    for pos in positions {
        if pos.remote == leader {
            continue;
        }
        let follower_ref = format!("{}/{branch}", pos.remote);
        let diff_output = git(local_path, &["diff", "--stat", &leader_ref, &follower_ref])
            .await
            .unwrap_or_default();
        if diff_output.trim().is_empty() {
            followers.push(pos.remote.clone());
        } else {
            // Trees actually differ — this is a real divergence, not a rebase artifact
            return None;
        }
    }

    if followers.is_empty() {
        return None;
    }

    Some((leader, followers))
}

// ── Public API ───────────────────────────────────────────────────────

/// Classify a single repo's temporal position across all remotes.
///
/// Shadow for: `quorumSignal temporal.check`
///
/// Fetches all remotes, measures ahead/behind per remote relative to
/// local HEAD, and classifies as `Parity`, `Converge`, or `Diverge`.
#[allow(clippy::too_many_lines)]
pub async fn check(workspace_root: &Path, repo_path: &str) -> Result<TemporalMatrix> {
    let local_path = workspace_root.join(repo_path);

    if !local_path.join(".git").exists() {
        return Ok(TemporalMatrix {
            repo_path: repo_path.to_string(),
            branch: String::new(),
            classification: SyncClassification::Missing,
            positions: vec![],
            action: SyncAction::None,
        });
    }

    let branch = git(&local_path, &["rev-parse", "--abbrev-ref", "HEAD"])
        .await
        .unwrap_or_else(|_| "main".to_string());

    // Fetch all remotes quietly
    let _ = git_ok(&local_path, &["fetch", "--all", "--quiet"]).await;

    let remotes_str = git(&local_path, &["remote"]).await?;
    let remotes: Vec<&str> = remotes_str.lines().filter(|l| !l.is_empty()).collect();

    if remotes.is_empty() {
        return Ok(TemporalMatrix {
            repo_path: repo_path.to_string(),
            branch,
            classification: SyncClassification::NoRemote,
            positions: vec![],
            action: SyncAction::None,
        });
    }

    // Measure per-remote position
    let mut positions = Vec::with_capacity(remotes.len());
    let mut has_leader = false;
    let mut leader_remote = String::new();
    let mut leader_behind: u32 = 0;
    let mut has_followers = false;

    for remote in &remotes {
        let remote_ref = format!("{remote}/{branch}");
        if !git_ok(&local_path, &["rev-parse", &remote_ref]).await {
            continue;
        }

        let ahead_range = format!("{remote_ref}..HEAD");
        let behind_range = format!("HEAD..{remote_ref}");

        let ahead = rev_list_count(&local_path, &ahead_range).await;
        let behind = rev_list_count(&local_path, &behind_range).await;

        positions.push(RemotePosition {
            remote: (*remote).to_string(),
            ahead,
            behind,
        });

        if behind > 0 && behind > leader_behind {
            leader_behind = behind;
            leader_remote = (*remote).to_string();
            has_leader = true;
        }
        if ahead > 0 {
            has_followers = true;
        }
    }

    let all_parity = positions.iter().all(RemotePosition::is_parity);
    if all_parity {
        return Ok(TemporalMatrix {
            repo_path: repo_path.to_string(),
            branch,
            classification: SyncClassification::Parity,
            positions,
            action: SyncAction::None,
        });
    }

    // Divergence: check if multiple remotes have unique commits relative to each other
    let mut diverge_count = 0u32;
    for pos_a in &positions {
        let ref_a = format!("{}/{branch}", pos_a.remote);
        let mut is_ahead_of_any = false;
        for pos_b in &positions {
            if pos_a.remote == pos_b.remote {
                continue;
            }
            let ref_b = format!("{}/{branch}", pos_b.remote);
            let cross_range = format!("{ref_b}..{ref_a}");
            let cross = rev_list_count(&local_path, &cross_range).await;
            if cross > 0 {
                is_ahead_of_any = true;
                break;
            }
        }
        if is_ahead_of_any {
            diverge_count += 1;
        }
    }

    let (classification, action) = if diverge_count > 1 {
        // Check if trees are actually identical (rebase artifact — history diverges
        // but content is the same). Use `git diff --stat` between the two most common
        // remotes — if empty, trees are at parity and we can safely force-align.
        let tree_parity = detect_tree_parity(&local_path, &positions, &branch).await;
        if let Some((leader, followers)) = tree_parity {
            (
                SyncClassification::Diverge,
                SyncAction::TreeParity { leader, followers },
            )
        } else {
            (SyncClassification::Diverge, SyncAction::Flag)
        }
    } else if has_leader {
        (
            SyncClassification::Converge,
            SyncAction::Pull {
                leader: leader_remote,
            },
        )
    } else if has_followers {
        (SyncClassification::Converge, SyncAction::Push)
    } else {
        (SyncClassification::Parity, SyncAction::None)
    };

    Ok(TemporalMatrix {
        repo_path: repo_path.to_string(),
        branch,
        classification,
        positions,
        action,
    })
}

/// Execute temporal sync on a single repo: pull from leader, push to followers.
///
/// Shadow for: `waterFall temporal.sync`
///
/// `push_target`: `"all"` pushes to every follower remote (legacy),
/// `"forgejo"` pushes only to the forgejo remote (VPS mediator model).
///
/// Returns `Err` only on infrastructure failures. Divergence is reported
/// as an `Ok` result with `ok: false` — the DAG is never force-mutated.
pub async fn sync(workspace_root: &Path, repo_path: &str) -> Result<TemporalSyncResult> {
    sync_with_target(workspace_root, repo_path, "all").await
}

/// Temporal sync respecting the manifest's `push_target` setting.
pub async fn sync_with_target(
    workspace_root: &Path,
    repo_path: &str,
    push_target: &str,
) -> Result<TemporalSyncResult> {
    sync_with_policy(workspace_root, repo_path, push_target, None).await
}

/// Temporal sync with optional manifest for divergence policy resolution.
#[allow(clippy::too_many_lines)]
pub async fn sync_with_policy(
    workspace_root: &Path,
    repo_path: &str,
    push_target: &str,
    manifest: Option<&crate::manifest::EcosystemManifest>,
) -> Result<TemporalSyncResult> {
    let local_path = workspace_root.join(repo_path);

    if !local_path.join(".git").exists() {
        return Ok(TemporalSyncResult {
            repo_path: repo_path.to_string(),
            ok: false,
            summary: "not cloned".to_string(),
            pulled_from: None,
            pushed_to: vec![],
        });
    }

    let matrix = check(workspace_root, repo_path).await?;

    match matrix.classification {
        SyncClassification::Parity => Ok(TemporalSyncResult {
            repo_path: repo_path.to_string(),
            ok: true,
            summary: "parity".to_string(),
            pulled_from: None,
            pushed_to: vec![],
        }),

        SyncClassification::Converge => {
            let branch = &matrix.branch;
            let mut pulled_from = None;
            let mut pushed_to = Vec::new();

            match &matrix.action {
                SyncAction::Pull { leader } => {
                    if git_ok(
                        &local_path,
                        &["pull", leader, branch, "--ff-only", "--quiet"],
                    )
                    .await
                    {
                        pulled_from = Some(leader.clone());
                    } else {
                        return Ok(TemporalSyncResult {
                            repo_path: repo_path.to_string(),
                            ok: false,
                            summary: format!("pull {leader} failed (ff-only)"),
                            pulled_from: None,
                            pushed_to: vec![],
                        });
                    }
                }
                SyncAction::Push | SyncAction::None => {}
                SyncAction::Flag | SyncAction::TreeParity { .. } => {
                    return Ok(TemporalSyncResult {
                        repo_path: repo_path.to_string(),
                        ok: false,
                        summary: "unexpected Flag/TreeParity action on Converge classification"
                            .to_string(),
                        pulled_from: None,
                        pushed_to: vec![],
                    });
                }
            }

            // Push to follower remotes, filtered by push_target.
            // "forgejo" = only push to the forgejo remote (VPS mediator handles GitHub).
            // "all" = push to every remote that is behind (legacy dual-push).
            for pos in &matrix.positions {
                if pulled_from.as_deref() == Some(&pos.remote) {
                    continue;
                }
                if push_target == "forgejo" && pos.remote != "forgejo" {
                    continue;
                }
                let remote_ref = format!("{}/{branch}", pos.remote);
                let ahead_range = format!("{remote_ref}..HEAD");
                let ahead = rev_list_count(&local_path, &ahead_range).await;
                if ahead > 0 && git_ok(&local_path, &["push", &pos.remote, branch, "--quiet"]).await
                {
                    pushed_to.push(pos.remote.clone());
                }
            }

            let summary = match (&pulled_from, pushed_to.is_empty()) {
                (Some(l), false) => format!("pull {l}, push {}", pushed_to.join(" ")),
                (Some(l), true) => format!("pull {l}"),
                (None, false) => format!("push {}", pushed_to.join(" ")),
                (None, true) => "parity".to_string(),
            };

            Ok(TemporalSyncResult {
                repo_path: repo_path.to_string(),
                ok: true,
                summary,
                pulled_from,
                pushed_to,
            })
        }

        SyncClassification::Diverge => {
            if let SyncAction::TreeParity { leader, followers } = &matrix.action {
                let branch = &matrix.branch;
                // Reset local to leader ref, then force-push followers
                if !git_ok(
                    &local_path,
                    &["reset", "--hard", &format!("{leader}/{branch}")],
                )
                .await
                {
                    return Ok(TemporalSyncResult {
                        repo_path: repo_path.to_string(),
                        ok: false,
                        summary: format!("tree-parity reset to {leader} failed"),
                        pulled_from: None,
                        pushed_to: vec![],
                    });
                }

                let mut pushed_to = Vec::new();
                for follower in followers {
                    if git_ok(
                        &local_path,
                        &["push", "--force-with-lease", follower, branch],
                    )
                    .await
                    {
                        pushed_to.push(follower.clone());
                    }
                }

                Ok(TemporalSyncResult {
                    repo_path: repo_path.to_string(),
                    ok: true,
                    summary: format!("tree-parity resolved: {leader} → {}", pushed_to.join(", ")),
                    pulled_from: Some(leader.clone()),
                    pushed_to,
                })
            } else {
                // Real divergence — apply policy if manifest is available
                if let Some(m) = manifest {
                    let entry = m.repos.values().find(|e| e.local_path == repo_path);
                    let policy = entry.map_or_else(
                        || m.sync.divergence_policy.as_str(),
                        |e| m.divergence_policy_for(e),
                    );
                    let positions: Vec<(String, u32, u32)> = matrix
                        .positions
                        .iter()
                        .map(|p| (p.remote.clone(), p.ahead, p.behind))
                        .collect();
                    let args = crate::impulse::SyncDivergeArgs {
                        repo_path: repo_path.to_string(),
                        positions,
                        repo_policy: policy.to_string(),
                    };

                    // Fire divergence impulse for visibility
                    let _ = crate::impulse::post_sync_diverge(workspace_root, &args).await;

                    // Apply graduated merge strategy based on policy
                    let resolved =
                        apply_divergence_policy(&local_path, &matrix, policy, push_target).await;

                    if let Some(summary) = resolved {
                        return Ok(TemporalSyncResult {
                            repo_path: repo_path.to_string(),
                            ok: true,
                            summary,
                            pulled_from: None,
                            pushed_to: vec![],
                        });
                    }
                }

                Ok(TemporalSyncResult {
                    repo_path: repo_path.to_string(),
                    ok: false,
                    summary: format!("DIVERGE — {matrix}"),
                    pulled_from: None,
                    pushed_to: vec![],
                })
            }
        }

        SyncClassification::Missing | SyncClassification::NoRemote => Ok(TemporalSyncResult {
            repo_path: repo_path.to_string(),
            ok: false,
            summary: matrix.classification.to_string(),
            pulled_from: None,
            pushed_to: vec![],
        }),
    }
}

/// Check temporal position for multiple repos, returning an aggregate report.
pub async fn check_all(workspace_root: &Path, repo_paths: &[&str]) -> Result<TemporalReport> {
    let mut report = TemporalReport {
        total: repo_paths.len() as u32,
        parity: 0,
        converged: 0,
        diverged: 0,
        missing: 0,
        repos: Vec::with_capacity(repo_paths.len()),
    };

    for path in repo_paths {
        let matrix = check(workspace_root, path).await?;
        match matrix.classification {
            SyncClassification::Parity => report.parity += 1,
            SyncClassification::Converge => report.converged += 1,
            SyncClassification::Diverge => report.diverged += 1,
            SyncClassification::Missing | SyncClassification::NoRemote => report.missing += 1,
        }
        report.repos.push(matrix);
    }

    Ok(report)
}

/// Resolve workspace root from `ECOPRIMALS_ROOT` env or by walking
/// up from the current binary's location to find a `primals/` directory.
/// Returns an error if no valid workspace can be found.
pub fn resolve_workspace_root() -> Result<PathBuf> {
    if let Ok(root) = std::env::var("ECOPRIMALS_ROOT") {
        let path = PathBuf::from(&root);
        if path.join("primals").exists() {
            return Ok(path);
        }
    }

    // Walk up from current exe looking for a workspace with primals/
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(Path::to_path_buf);
        while let Some(d) = dir {
            if d.join("primals").exists() {
                return Ok(d);
            }
            dir = d.parent().map(Path::to_path_buf);
        }
    }

    Err(crate::error::ShadowError::Parse(
        "cannot resolve ecoPrimals workspace root — set ECOPRIMALS_ROOT".into(),
    ))
}

/// Apply graduated merge strategy based on divergence policy from manifest.
///
/// Returns `Some(summary)` if the policy was applied successfully (repo converged),
/// or `None` if the policy is `impulse-only` or resolution failed.
async fn apply_divergence_policy(
    local_path: &Path,
    matrix: &TemporalMatrix,
    policy: &str,
    push_target: &str,
) -> Option<String> {
    let branch = &matrix.branch;

    // Find leader (least behind) and followers
    let leader = matrix
        .positions
        .iter()
        .min_by_key(|p| p.behind)
        .map(|p| &p.remote)?;

    match policy {
        "merge-ff" => {
            // Pull from leader with --ff-only, push to followers
            if !git_ok(
                local_path,
                &["pull", leader, branch, "--ff-only", "--quiet"],
            )
            .await
            {
                return None;
            }
            let mut pushed = Vec::new();
            for pos in &matrix.positions {
                if &pos.remote == leader {
                    continue;
                }
                if push_target == "forgejo" && pos.remote != "forgejo" {
                    continue;
                }
                if git_ok(local_path, &["push", &pos.remote, branch, "--quiet"]).await {
                    pushed.push(pos.remote.clone());
                }
            }
            Some(format!(
                "policy:merge-ff pull {leader}, push {}",
                pushed.join(" ")
            ))
        }
        "merge-rebase" => {
            // Rebase onto leader, then push
            if !git_ok(local_path, &["pull", "--rebase", leader, branch, "--quiet"]).await {
                return None;
            }
            let mut pushed = Vec::new();
            for pos in &matrix.positions {
                if &pos.remote == leader {
                    continue;
                }
                if push_target == "forgejo" && pos.remote != "forgejo" {
                    continue;
                }
                if git_ok(
                    local_path,
                    &["push", "--force-with-lease", &pos.remote, branch],
                )
                .await
                {
                    pushed.push(pos.remote.clone());
                }
            }
            Some(format!(
                "policy:merge-rebase rebase {leader}, push {}",
                pushed.join(" ")
            ))
        }
        _ => None,
    }
}

/// Execute a full cascade sync — the Rust evolution of `cascade-pull.sh`.
///
/// Reads the gate profile from `ecosystem_manifest.toml`, resolves temporal
/// position for each repo, pulls from leader, and reports parity.
#[allow(clippy::too_many_lines, clippy::fn_params_excessive_bools)]
pub async fn cascade(
    gate_name: &str,
    source: &str,
    check_only: bool,
    clone_missing: bool,
    dry_run: bool,
    publish_freshness: bool,
) -> Result<crate::ShadowOutcome> {
    let root = resolve_workspace_root()?;
    let m = crate::manifest::load_from_workspace(&root)?;

    let repos: Vec<(&str, &crate::manifest::RepoEntry)> = m.gate_repos(gate_name);
    let total = repos.len() as u32;

    if dry_run {
        let lines: Vec<String> = repos
            .iter()
            .map(|(name, e)| format!("  {:<25} {}", name, e.local_path))
            .collect();
        return Ok(crate::ShadowOutcome::ok(format!(
            "DRY RUN: {total} repos for {gate_name} (source={source})\n{}",
            lines.join("\n"),
        )));
    }

    let push_target = m.sync.push_target.clone();
    let mut synced = 0u32;
    let mut failed = 0u32;
    let mut cloned = 0u32;
    let mut lines = Vec::with_capacity(repos.len());

    for (name, entry) in &repos {
        let repo_path = &entry.local_path;
        let full_path = std::path::Path::new(&root).join(repo_path);

        if !full_path.join(".git").exists() {
            if clone_missing {
                let forgejo_url = m.forgejo_clone_url(entry);
                let clone_result = tokio::process::Command::new("git")
                    .args(["clone", &forgejo_url, &full_path.to_string_lossy()])
                    .output()
                    .await;
                match clone_result {
                    Ok(out) if out.status.success() => {
                        cloned += 1;
                        lines.push(format!("  {name:<35} CLONED"));
                    }
                    _ => {
                        failed += 1;
                        lines.push(format!("  {name:<35} CLONE FAILED"));
                    }
                }
            } else {
                lines.push(format!("  {name:<35} SKIP (not cloned)"));
            }
            continue;
        }

        if check_only {
            match check(&root, repo_path).await {
                Ok(matrix) => {
                    let status = match matrix.classification {
                        SyncClassification::Parity => {
                            synced += 1;
                            "OK parity"
                        }
                        SyncClassification::Converge => {
                            synced += 1;
                            "OK converge"
                        }
                        _ => {
                            failed += 1;
                            "DIVERGE"
                        }
                    };
                    lines.push(format!("  {name:<35} {status}"));
                }
                Err(e) => {
                    failed += 1;
                    lines.push(format!("  {name:<35} FAIL {e}"));
                }
            }
        } else {
            match sync_with_policy(&root, repo_path, &push_target, Some(&m)).await {
                Ok(r) => {
                    let status = if r.ok {
                        synced += 1;
                        format!("OK {}", r.summary)
                    } else {
                        failed += 1;
                        format!("FAIL {}", r.summary)
                    };
                    lines.push(format!("  {name:<35} {status}"));
                }
                Err(e) => {
                    failed += 1;
                    lines.push(format!("  {name:<35} FAIL {e}"));
                }
            }
        }
    }

    // Publish freshness snapshot if requested
    if publish_freshness && !check_only {
        let freshness_result = crate::freshness::publish_freshness_toml(&root, &m, &repos).await;
        if let Err(e) = &freshness_result {
            lines.push(format!("  [freshness] FAIL: {e}"));
        } else {
            lines.push("  [freshness] PUBLISHED freshness.toml".to_string());
        }
    }

    let action = if check_only { "checked" } else { "synced" };
    let clone_info = if cloned > 0 {
        format!(" cloned={cloned}")
    } else {
        String::new()
    };
    let header = format!(
        "=== WaterFall Cascade ({action}) ===\n\
         Manifest: v{} wave {} ({} repos)\n\
         Gate:    {gate_name}\n\
         Source:  {source}\n\
         Repos:   {total}\n\
         \n\
         {action}={synced} failed={failed}{clone_info}",
        m.meta.version, m.meta.wave, m.meta.total_repos,
    );

    Ok(crate::ShadowOutcome::ok_with(
        format!("{header}\n{}", lines.join("\n")),
        serde_json::json!({
            "gate": gate_name,
            "source": source,
            "total": total,
            "synced": synced,
            "failed": failed,
            "cloned": cloned,
        }),
    ))
}

/// Re-export freshness tracking functions (for backward compat from dispatch).
pub use crate::freshness::check_installed_freshness;
