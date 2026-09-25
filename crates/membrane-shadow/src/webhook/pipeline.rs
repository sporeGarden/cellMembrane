// SPDX-License-Identifier: AGPL-3.0-or-later

//! Webhook pipelines — harvest, publish, and cascade for push events.

use super::{PushEvent, WebhookAction};
use tracing::{error, info, warn};

pub(super) async fn run_harvest_pipeline(
    action: &WebhookAction,
    event: &PushEvent,
    config: &crate::ShadowConfig,
) -> crate::error::Result<crate::ShadowOutcome> {
    let harvest_args = crate::plasmid::HarvestArgs {
        primal: Some(action.repo_name.to_lowercase()),
        force: false,
        dry_run: false,
        depot_dir: None,
        target: None,
        local: false,
        push: false,
        with_restart: false,
    };

    let harvest_outcome = crate::plasmid::harvest(&harvest_args).await?;

    if !harvest_outcome.ok {
        return Ok(crate::ShadowOutcome {
            ok: false,
            message: format!(
                "webhook: {} harvest failed — {}",
                action.repo_name, harvest_outcome.message
            ),
            data: harvest_outcome.data,
        });
    }

    let primal_lower = action.repo_name.to_lowercase();
    if let Some(fail) = run_sandbox(&primal_lower, event, &action.repo_name).await? {
        return Ok(fail);
    }

    let refresh_args = crate::plasmid::RefreshArgs {
        primal: Some(primal_lower),
        dry_run: false,
        source_dir: None,
    };

    let refresh_outcome = crate::plasmid::refresh(config, &refresh_args).await?;

    Ok(crate::ShadowOutcome {
        ok: refresh_outcome.ok,
        message: format!(
            "webhook: {} -> harvest: {} | sandbox: PASS | refresh: {}",
            action.repo_name, harvest_outcome.message, refresh_outcome.message
        ),
        data: refresh_outcome.data,
    })
}

/// Run git cascade for an ecosystem repo push (non-primal).
///
/// Forgejo push: sync the repo locally (`temporal.sync`) so this gate stays current.
/// GitHub push: relay inward — pull from GitHub mirror, push to Forgejo.
pub(super) async fn run_cascade_pipeline(
    action: &WebhookAction,
    _config: &crate::ShadowConfig,
) -> crate::error::Result<crate::ShadowOutcome> {
    let root = crate::temporal::resolve_workspace_root()?;
    let manifest = crate::manifest::load_from_workspace_async(&root).await?;
    let push_target = manifest.sync.push_target;

    let repo_path = manifest
        .repos
        .iter()
        .find(|(_, entry)| {
            entry
                .local_path
                .to_lowercase()
                .contains(&action.repo_name.to_lowercase())
        })
        .map(|(_, entry)| entry.local_path.clone());

    let path = repo_path.unwrap_or_else(|| {
        format!(
            "{}/{}",
            cellmembrane_types::service::INFRA_WATERING_HOLE,
            action.repo_name
        )
    });

    match action.provider {
        super::WebhookProvider::Forgejo => {
            info!(
                repo = %action.repo_name,
                path = %path,
                "webhook cascade: syncing from Forgejo push"
            );
            let result = crate::temporal::sync_with_target(&root, &path, push_target).await?;
            Ok(crate::ShadowOutcome::ok(format!(
                "webhook: {} cascade sync — {}",
                action.repo_name,
                if result.ok { "parity" } else { &result.summary }
            )))
        }
        super::WebhookProvider::GitHub => {
            info!(
                repo = %action.repo_name,
                path = %path,
                "webhook cascade: relaying from GitHub push"
            );
            let relay_config = crate::relay::RelayConfig::from_env();
            let (pulled, failures) = crate::relay::mediate(&relay_config, &[path.as_str()]).await;
            let ok = failures.is_empty();
            Ok(crate::ShadowOutcome {
                ok,
                message: format!(
                    "webhook: {} GitHub relay — pulled={} failed={}",
                    action.repo_name,
                    pulled.len(),
                    failures.len()
                ),
                data: Some(serde_json::json!({
                    "pulled": pulled,
                    "failures": failures,
                })),
            })
        }
    }
}

// ── Publish Pipeline ────────────────────────────────────────────────

/// Publish a static site: git fetch → zola build → seo.notify.
///
/// Replaces the per-repo bash hooks (`10-deploy-detroit`, `50-zola-publish`,
/// `20-crawler-notify`, `60-seo-resubmit`) with a single Rust function
/// driven by the [`PublishSite`] registry.
pub(super) async fn run_publish_pipeline(
    action: &WebhookAction,
) -> crate::error::Result<crate::ShadowOutcome> {
    let site = crate::seo::find_publish_site(&action.repo_name).ok_or_else(|| {
        crate::error::ShadowError::config(format!(
            "no publish site config for repo: {}",
            action.repo_name
        ))
    })?;

    info!(
        repo = %action.repo_name,
        host = %site.host,
        worktree = %site.worktree,
        "publish pipeline: starting"
    );

    // Step 1: Git fetch + reset to latest
    let git_result = publish_git_update(site).await;
    if let Err(e) = &git_result {
        error!(repo = %action.repo_name, error = %e, "publish: git update failed");
        return Ok(crate::ShadowOutcome {
            ok: false,
            message: format!("webhook: {} publish failed — git: {e}", action.repo_name),
            data: None,
        });
    }

    // Step 2: Zola build
    let build_result = publish_zola_build(site).await;
    match &build_result {
        Ok(page_count) => {
            info!(
                repo = %action.repo_name,
                host = %site.host,
                pages = page_count,
                "publish: zola build succeeded"
            );
        }
        Err(e) => {
            error!(repo = %action.repo_name, error = %e, "publish: zola build failed");
            return Ok(crate::ShadowOutcome {
                ok: false,
                message: format!("webhook: {} publish failed — build: {e}", action.repo_name),
                data: None,
            });
        }
    }

    let page_count = build_result.unwrap_or(0);

    // Step 3: Notify crawlers (non-fatal — build succeeded, so we report success)
    let seo_result = crate::seo::notify_crawlers(Some(site.host)).await;
    let seo_msg = match seo_result {
        Ok(msg) => msg,
        Err(e) => {
            warn!(repo = %action.repo_name, error = %e, "publish: seo notify failed (non-fatal)");
            format!("  [seo] FAILED: {e}")
        }
    };

    Ok(crate::ShadowOutcome {
        ok: true,
        message: format!(
            "webhook: {} published to {} ({} pages)\n{}",
            action.repo_name, site.host, page_count, seo_msg
        ),
        data: Some(serde_json::json!({
            "host": site.host,
            "pages": page_count,
        })),
    })
}

/// Git fetch + hard reset a publish site's worktree to latest.
async fn publish_git_update(
    site: &crate::seo::PublishSite,
) -> crate::error::Result<()> {
    let worktree = site.worktree;

    // Check if worktree exists; if not, clone
    if !std::path::Path::new(worktree).join(".git").exists()
        && !std::path::Path::new(worktree).join("HEAD").exists()
    {
        warn!(
            worktree = %worktree,
            "publish: worktree missing, skipping git update (initial clone needed)"
        );
        return Err(crate::error::ShadowError::config(format!(
            "worktree {worktree} does not exist — initial clone required"
        )));
    }

    // Unset GIT_DIR — when called from a post-receive hook, Forgejo sets
    // GIT_DIR to the bare repo, which breaks operations on the worktree.
    let output = tokio::process::Command::new("git")
        .args(["fetch", "origin", "main"])
        .current_dir(worktree)
        .env_remove("GIT_DIR")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .map_err(|e| crate::error::ShadowError::Io(e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(crate::error::ShadowError::config(format!(
            "git fetch failed: {}",
            stderr.lines().next().unwrap_or("unknown")
        )));
    }

    let output = tokio::process::Command::new("git")
        .args(["reset", "--hard", "origin/main"])
        .current_dir(worktree)
        .env_remove("GIT_DIR")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .map_err(|e| crate::error::ShadowError::Io(e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(crate::error::ShadowError::config(format!(
            "git reset failed: {}",
            stderr.lines().next().unwrap_or("unknown")
        )));
    }

    Ok(())
}

/// Run `zola build` for a publish site, returning the page count.
async fn publish_zola_build(
    site: &crate::seo::PublishSite,
) -> crate::error::Result<usize> {
    let build_dir = match site.build_subdir {
        Some(sub) => format!("{}/{}", site.worktree, sub),
        None => site.worktree.to_string(),
    };

    let output = tokio::process::Command::new("zola")
        .args(["build", "--force", "--output-dir", site.public_dir])
        .current_dir(&build_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .map_err(|e| crate::error::ShadowError::Io(e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if stderr.is_empty() {
            stdout.to_string()
        } else {
            stderr.to_string()
        };
        return Err(crate::error::ShadowError::config(format!(
            "zola build failed (exit {}): {}",
            output.status.code().unwrap_or(-1),
            detail.lines().last().unwrap_or("unknown")
        )));
    }

    // Count built HTML files
    let page_count = count_html_files(site.public_dir).await;
    Ok(page_count)
}

/// Count HTML files in a directory (recursive).
async fn count_html_files(dir: &str) -> usize {
    let mut count = 0;
    let Ok(entries) = tokio::fs::read_dir(dir).await else {
        return 0;
    };
    // Use a stack-based walk to avoid recursion depth issues
    let mut dirs = vec![dir.to_string()];
    while let Some(d) = dirs.pop() {
        let Ok(mut rd) = tokio::fs::read_dir(&d).await else {
            continue;
        };
        while let Ok(Some(entry)) = rd.next_entry().await {
            let ft = entry.file_type().await;
            if let Ok(ft) = ft {
                if ft.is_dir() {
                    dirs.push(entry.path().to_string_lossy().to_string());
                } else if ft.is_file() {
                    if let Some(name) = entry.file_name().to_str() {
                        if name.ends_with(".html") {
                            count += 1;
                        }
                    }
                }
            }
        }
    }
    // Suppress unused variable warning — entries was used to validate dir access
    drop(entries);
    count
}

// ── Sandbox ─────────────────────────────────────────────────────────

pub(super) async fn run_sandbox(
    primal_lower: &str,
    event: &PushEvent,
    repo_name: &str,
) -> crate::error::Result<Option<crate::ShadowOutcome>> {
    let arch = crate::plasmid::detect_target_triple();
    let depot_binary = crate::plasmid::resolve_path(
        None,
        cellmembrane_types::service::ENV_PLASMIDBIN_DEPOT,
        || {
            crate::resolve_xdg_data_home()
                .join("ecoPrimals")
                .join(cellmembrane_types::service::PLASMID_BIN_DIR)
        },
    )
    .join("primals")
    .join(arch)
    .join(primal_lower);

    if !depot_binary.exists() {
        return Ok(None);
    }

    let commit_short = if event.after.len() >= 8 {
        &event.after[..8]
    } else {
        &event.after
    };

    let sandbox_args = crate::plasmid::sandbox::SandboxArgs {
        primal: primal_lower.to_string(),
        commit: commit_short.to_string(),
        binary_path: depot_binary,
        timeout_secs: None,
    };

    match crate::plasmid::sandbox::validate_with_deps(&sandbox_args).await {
        Ok(result) if !result.health_ok => {
            error!(
                primal = %primal_lower,
                detail = %result.detail,
                "sandbox validation failed"
            );
            Ok(Some(crate::ShadowOutcome {
                ok: false,
                message: format!(
                    "webhook: {repo_name} sandbox validation FAILED — {} ({}ms). Production unchanged.",
                    result.detail, result.elapsed_ms
                ),
                data: Some(serde_json::to_value(&result).unwrap_or_default()),
            }))
        }
        Ok(result) => {
            info!(
                primal = %primal_lower,
                detail = %result.detail,
                elapsed_ms = result.elapsed_ms,
                "sandbox validation passed"
            );
            Ok(None)
        }
        Err(e) => {
            warn!(primal = %primal_lower, error = %e, "sandbox infra error — proceeding");
            Ok(None)
        }
    }
}
