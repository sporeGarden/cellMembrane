// SPDX-License-Identifier: AGPL-3.0-or-later

//! SYNC divergence impulse — auto-fired by `temporal.cascade` when
//! non-fast-forward divergence is detected.

use chrono::Local;
use std::path::Path;

use super::policy::{classify_diverge_type, suggest_action};
use super::primal::try_sign_impulse;
use super::types::{
    ImpulseContent, ImpulseFrom, ImpulseMeta, ImpulseOpMeta, ImpulseTo, ImpulseType, Priority,
    SyncDivergeArgs, SyncImpulseFile, SyncPayload, active_dir, current_wave, resolve_head_ref,
};
use crate::error::{Result, ShadowError};
use crate::identity;
use tracing::warn;

/// Fire a SYNC divergence impulse — auto-called by `temporal.cascade`.
pub async fn post_sync_diverge(
    workspace_root: &Path,
    args: &SyncDivergeArgs,
) -> Result<SyncImpulseFile> {
    let gate_id = identity::resolve(workspace_root)?;
    let now = Local::now();
    let ts_file = now.format("%Y-%m-%dT%H-%M").to_string();
    let ts_iso = now.format("%Y-%m-%dT%H:%M:%S%:z").to_string();

    let repo_name = args.repo_path.rsplit('/').next().unwrap_or(&args.repo_path);

    let mut remotes_map = std::collections::BTreeMap::new();
    let mut ahead_map = std::collections::BTreeMap::new();
    let mut diverge_summary_parts = Vec::new();

    for (remote, ahead, behind) in &args.positions {
        let sha = resolve_remote_head(workspace_root, &args.repo_path, remote).await;
        remotes_map.insert(remote.clone(), sha);
        ahead_map.insert(remote.clone(), *ahead);
        if *ahead > 0 || *behind > 0 {
            diverge_summary_parts.push(format!("{remote}(+{ahead},-{behind})"));
        }
    }

    let diverge_type = classify_diverge_type(&args.positions);
    let suggested = suggest_action(&diverge_type, &args.repo_policy);
    let subject = format!(
        "DIVERGE: {repo_name} - {}",
        diverge_summary_parts.join(" vs ")
    );

    let slug = format!("diverge-{repo_name}");
    let impulse_id = format!("{ts_file}-{}-{slug}", gate_id.name);
    let filename = format!("{ts_file}_{}__{slug}.toml", gate_id.name);

    let wave = current_wave(workspace_root);
    let git_ref = resolve_head_ref(workspace_root, &args.repo_path);

    let impulse = SyncImpulseFile {
        impulse: ImpulseMeta {
            id: impulse_id.clone(),
            impulse_type: ImpulseType::Sync,
            priority: Priority::Priority,
            wave,
        },
        from: ImpulseFrom {
            gate: gate_id.name.clone(),
            team: String::new(),
            project: args.repo_path.clone(),
            git_ref,
        },
        to: ImpulseTo {
            gates: vec!["*".to_string()],
            teams: vec![],
        },
        content: ImpulseContent {
            subject: subject.clone(),
            body: format!(
                "Cascade detected non-ff divergence in {}. Policy: {}. See payload for resolution context.",
                args.repo_path, args.repo_policy
            ),
        },
        payload: SyncPayload {
            repo: args.repo_path.clone(),
            diverge_type,
            merge_base: String::new(),
            remotes: remotes_map,
            ahead: ahead_map,
            repo_policy: args.repo_policy.clone(),
            suggested_action: suggested,
        },
        meta: ImpulseOpMeta {
            created: ts_iso,
            expires: String::new(),
            ack_required: true,
        },
        signature: try_sign_impulse(workspace_root, &impulse_id),
        acks: vec![],
    };

    let active = active_dir(workspace_root);
    std::fs::create_dir_all(&active).map_err(ShadowError::Io)?;

    let filepath = active.join(&filename);
    let toml_str = toml::to_string_pretty(&impulse).map_err(ShadowError::Serialize)?;
    crate::atomic_write(&filepath, toml_str.as_bytes()).map_err(ShadowError::Io)?;

    let wh_dir = workspace_root.join("infra/wateringHole");
    let push = crate::git_ops::add_commit_push(
        &wh_dir,
        &format!("impulses/active/{filename}"),
        &format!("impulse sync: {subject}"),
    )
    .await?;
    if !push.failed.is_empty() {
        warn!(failed = ?push.failed, "sync impulse push partial failure");
    }

    Ok(impulse)
}

async fn resolve_remote_head(workspace_root: &Path, repo_path: &str, remote: &str) -> String {
    let local_path = workspace_root.join(repo_path);
    crate::git_ops::git_output(
        &local_path,
        &["rev-parse", "--short", &format!("{remote}/main")],
    )
    .await
    .unwrap_or_default()
}
