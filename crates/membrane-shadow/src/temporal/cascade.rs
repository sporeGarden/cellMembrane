// SPDX-License-Identifier: AGPL-3.0-or-later
//! Full cascade sync — reads manifest, syncs all gate repos, reports parity.

use crate::error::Result;

use super::types::SyncClassification;
use super::{check, resolve_workspace_root, sync_with_policy};

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
                        SyncClassification::Parity | SyncClassification::Converge => {
                            synced += 1;
                            if matrix.classification == SyncClassification::Parity {
                                "OK parity"
                            } else {
                                "OK converge"
                            }
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
