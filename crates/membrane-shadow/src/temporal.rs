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
    /// Diverged — flag for human review, do not modify.
    Flag,
}

impl std::fmt::Display for SyncAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "ok"),
            Self::Pull { leader } => write!(f, "pull {leader}"),
            Self::Push => write!(f, "push followers"),
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

// ── Public API ───────────────────────────────────────────────────────

/// Classify a single repo's temporal position across all remotes.
///
/// Shadow for: `quorumSignal temporal.check`
///
/// Fetches all remotes, measures ahead/behind per remote relative to
/// local HEAD, and classifies as `Parity`, `Converge`, or `Diverge`.
/// Remotes listed in `exclude_remotes` are filtered out of the matrix.
pub async fn check(workspace_root: &Path, repo_path: &str) -> Result<TemporalMatrix> {
    check_with_excludes(workspace_root, repo_path, &[]).await
}

/// `check` with explicit remote exclusion list (from manifest `exclude_remotes`).
pub async fn check_with_excludes(
    workspace_root: &Path,
    repo_path: &str,
    exclude_remotes: &[String],
) -> Result<TemporalMatrix> {
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

    let _ = git_ok(&local_path, &["fetch", "--all", "--quiet"]).await;

    let remotes_str = git(&local_path, &["remote"]).await?;
    let remotes: Vec<&str> = remotes_str
        .lines()
        .filter(|l| !l.is_empty())
        .filter(|r| !exclude_remotes.iter().any(|ex| ex == r))
        .collect();

    if remotes.is_empty() {
        return Ok(TemporalMatrix {
            repo_path: repo_path.to_string(),
            branch,
            classification: SyncClassification::NoRemote,
            positions: vec![],
            action: SyncAction::None,
        });
    }

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
        (SyncClassification::Diverge, SyncAction::Flag)
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
    sync_with_policy(workspace_root, repo_path, push_target, "flag", &[]).await
}

/// Temporal sync with full policy engine — per-repo `divergence_policy`
/// and `exclude_remotes` from the manifest.
///
/// Policy engine:
/// - `flag` — report diverge, no mutation (default)
/// - `merge-ff` — auto-resolve if one side is a strict ancestor
/// - `merge-rebase` — auto-rebase if no content conflicts
/// - `impulse-only` — fire SYNC impulse, never auto-resolve
/// - `agentic` — full pipeline (Phase 4 future)
pub async fn sync_with_policy(
    workspace_root: &Path,
    repo_path: &str,
    push_target: &str,
    divergence_policy: &str,
    exclude_remotes: &[String],
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

    let matrix = check_with_excludes(workspace_root, repo_path, exclude_remotes).await?;

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
                SyncAction::Flag => {
                    return Ok(TemporalSyncResult {
                        repo_path: repo_path.to_string(),
                        ok: false,
                        summary: "unexpected Flag action on Converge classification".to_string(),
                        pulled_from: None,
                        pushed_to: vec![],
                    });
                }
            }

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
            try_policy_resolve(workspace_root, &matrix, push_target, divergence_policy).await
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

/// Attempt policy-based auto-resolution of diverged repos.
///
/// Returns `ok: true` if the policy successfully resolved the divergence,
/// `ok: false` if human/agentic review is needed.
async fn try_policy_resolve(
    workspace_root: &Path,
    matrix: &TemporalMatrix,
    push_target: &str,
    policy: &str,
) -> Result<TemporalSyncResult> {
    let local_path = workspace_root.join(&matrix.repo_path);
    let branch = &matrix.branch;

    match policy {
        "merge-ff" => {
            // Auto-resolve if exactly one remote is strictly ahead (ancestor relationship).
            // This handles the plasmidBin case: origin(+2) forgejo(+0).
            let leaders: Vec<_> = matrix
                .positions
                .iter()
                .filter(|p| p.behind > 0 && p.ahead == 0)
                .collect();

            if leaders.len() == 1 {
                let leader = &leaders[0].remote;
                if git_ok(
                    &local_path,
                    &["pull", leader, branch, "--ff-only", "--quiet"],
                )
                .await
                {
                    let mut pushed_to = Vec::new();
                    for pos in &matrix.positions {
                        if pos.remote == *leader {
                            continue;
                        }
                        if push_target == "forgejo" && pos.remote != "forgejo" {
                            continue;
                        }
                        if git_ok(&local_path, &["push", &pos.remote, branch, "--quiet"]).await {
                            pushed_to.push(pos.remote.clone());
                        }
                    }
                    return Ok(TemporalSyncResult {
                        repo_path: matrix.repo_path.clone(),
                        ok: true,
                        summary: format!("merge-ff: pull {leader}, push {}", pushed_to.join(" ")),
                        pulled_from: Some(leader.clone()),
                        pushed_to,
                    });
                }
            }

            Ok(TemporalSyncResult {
                repo_path: matrix.repo_path.clone(),
                ok: false,
                summary: format!("DIVERGE (merge-ff failed) — {matrix}"),
                pulled_from: None,
                pushed_to: vec![],
            })
        }

        "merge-rebase" => {
            // Auto-rebase local on the leader, then push. Fails gracefully
            // on conflicts (falls through to flag behavior).
            let leaders: Vec<_> = matrix.positions.iter().filter(|p| p.behind > 0).collect();

            if let Some(leader) = leaders.first() {
                let remote_ref = format!("{}/{branch}", leader.remote);
                if git_ok(&local_path, &["rebase", &remote_ref]).await {
                    let mut pushed_to = Vec::new();
                    for pos in &matrix.positions {
                        if push_target == "forgejo" && pos.remote != "forgejo" {
                            continue;
                        }
                        if git_ok(&local_path, &["push", &pos.remote, branch, "--quiet"]).await {
                            pushed_to.push(pos.remote.clone());
                        }
                    }
                    return Ok(TemporalSyncResult {
                        repo_path: matrix.repo_path.clone(),
                        ok: true,
                        summary: format!(
                            "merge-rebase: rebase on {}, push {}",
                            leader.remote,
                            pushed_to.join(" ")
                        ),
                        pulled_from: Some(leader.remote.clone()),
                        pushed_to,
                    });
                }
                let _ = git_ok(&local_path, &["rebase", "--abort"]).await;
            }

            Ok(TemporalSyncResult {
                repo_path: matrix.repo_path.clone(),
                ok: false,
                summary: format!("DIVERGE (merge-rebase conflict) — {matrix}"),
                pulled_from: None,
                pushed_to: vec![],
            })
        }

        // impulse-only, agentic, flag — all fall through to flag behavior.
        // Impulse firing is handled at the cascade level, not here.
        _ => Ok(TemporalSyncResult {
            repo_path: matrix.repo_path.clone(),
            ok: false,
            summary: format!("DIVERGE — {matrix}"),
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

/// Execute a full cascade sync — the Rust evolution of `cascade-pull.sh`.
///
/// Reads the gate profile from `ecosystem_manifest.toml`, resolves temporal
/// position for each repo, pulls from leader, and reports parity.
///
/// Honors per-repo `divergence_policy` and `exclude_remotes` from the manifest.
/// When `diverge_impulse = true` in `[sync]`, auto-fires SYNC impulses on diverge.
pub async fn cascade(
    gate_name: &str,
    source: &str,
    check_only: bool,
    clone_missing: bool,
    dry_run: bool,
) -> Result<crate::ShadowOutcome> {
    let root = resolve_workspace_root()?;
    let m = crate::manifest::load_from_workspace(&root)?;

    let repos: Vec<(&str, &crate::manifest::RepoEntry)> = m.gate_repos(gate_name);
    let total = repos.len() as u32;

    if dry_run {
        let lines: Vec<String> = repos
            .iter()
            .map(|(name, e)| {
                let policy = m.divergence_policy_for(e);
                let excl = if e.exclude_remotes.is_empty() {
                    String::new()
                } else {
                    format!(" (exclude: {})", e.exclude_remotes.join(","))
                };
                format!("  {:<25} {} [{}]{}", name, e.local_path, policy, excl)
            })
            .collect();
        return Ok(crate::ShadowOutcome::ok(format!(
            "DRY RUN: {total} repos for {gate_name} (source={source})\n{}",
            lines.join("\n"),
        )));
    }

    let push_target = m.sync.push_target.clone();
    let diverge_impulse = m.sync.diverge_impulse;
    let mut synced = 0u32;
    let mut failed = 0u32;
    let mut cloned = 0u32;
    let mut impulse_count = 0u32;
    let mut lines = Vec::with_capacity(repos.len());

    for (name, entry) in &repos {
        let repo_path = &entry.local_path;
        let full_path = std::path::Path::new(&root).join(repo_path);
        let repo_policy = m.divergence_policy_for(entry).to_string();

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
                        lines.push(format!("  {:<35} CLONED", name));
                    }
                    _ => {
                        failed += 1;
                        lines.push(format!("  {:<35} CLONE FAILED", name));
                    }
                }
            } else {
                lines.push(format!("  {:<35} SKIP (not cloned)", name));
            }
            continue;
        }

        if check_only {
            match check_with_excludes(&root, repo_path, &entry.exclude_remotes).await {
                Ok(matrix) => {
                    let status = match matrix.classification {
                        SyncClassification::Parity => {
                            synced += 1;
                            "OK parity".to_string()
                        }
                        SyncClassification::Converge => {
                            synced += 1;
                            "OK converge".to_string()
                        }
                        SyncClassification::Diverge => {
                            failed += 1;
                            format!("DIVERGE [{}]", repo_policy)
                        }
                        _ => {
                            failed += 1;
                            "MISSING".to_string()
                        }
                    };
                    lines.push(format!("  {:<35} {status}", name));
                }
                Err(e) => {
                    failed += 1;
                    lines.push(format!("  {:<35} FAIL {e}", name));
                }
            }
        } else {
            match sync_with_policy(
                &root,
                repo_path,
                &push_target,
                &repo_policy,
                &entry.exclude_remotes,
            )
            .await
            {
                Ok(r) => {
                    if r.ok {
                        synced += 1;
                        lines.push(format!("  {:<35} OK {}", name, r.summary));
                    } else {
                        failed += 1;
                        lines.push(format!("  {:<35} FAIL {}", name, r.summary));

                        // Auto-fire SYNC impulse on diverge when enabled
                        if diverge_impulse
                            && r.summary.starts_with("DIVERGE")
                            && repo_policy != "merge-ff"
                        {
                            if let Ok(matrix) =
                                check_with_excludes(&root, repo_path, &entry.exclude_remotes).await
                            {
                                let positions: Vec<(String, u32, u32)> = matrix
                                    .positions
                                    .iter()
                                    .map(|p| (p.remote.clone(), p.ahead, p.behind))
                                    .collect();
                                let impulse_args = crate::impulse::SyncDivergeArgs {
                                    repo_path: repo_path.to_string(),
                                    positions,
                                    repo_policy: repo_policy.clone(),
                                };
                                match crate::impulse::post_sync_diverge(&root, &impulse_args).await
                                {
                                    Ok(_) => {
                                        impulse_count += 1;
                                        lines.push(format!("  {:<35}   -> SYNC impulse fired", ""));
                                    }
                                    Err(e) => {
                                        lines
                                            .push(format!("  {:<35}   -> impulse failed: {e}", ""));
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    failed += 1;
                    lines.push(format!("  {:<35} FAIL {e}", name));
                }
            }
        }
    }

    let action = if check_only { "checked" } else { "synced" };
    let clone_info = if cloned > 0 {
        format!(" cloned={cloned}")
    } else {
        String::new()
    };
    let impulse_info = if impulse_count > 0 {
        format!(" impulses={impulse_count}")
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
         {action}={synced} failed={failed}{clone_info}{impulse_info}",
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
            "impulses": impulse_count,
        }),
    ))
}
