// SPDX-License-Identifier: AGPL-3.0-or-later

//! Impulse lifecycle operations: post, sense, check, ack, archive.

use chrono::{Local, Utc};
use std::path::Path;

use super::parse::{find_impulse_by_id, parse_impulse_or_signal};
use super::policy::{is_expired, is_fully_acked_with_externals, load_external_acks};
use super::primal::{try_relay_impulse, try_sign_impulse};
use super::types::{
    ImpulseAck, ImpulseContent, ImpulseFile, ImpulseFrom, ImpulseMeta, ImpulseOpMeta, ImpulseTo,
    ImpulseType, PostArgs, PotentialHealth, active_dir, current_wave, impulses_dir,
    resolve_head_ref,
};
use crate::error::{Result, ShadowError};
use crate::identity;
use tracing::warn;

/// Fire a new impulse — rootPulse ACTION.
pub async fn post(workspace_root: &Path, args: &PostArgs<'_>) -> Result<ImpulseFile> {
    let gate_id = identity::resolve(workspace_root)?;
    let now = Local::now();
    let ts_file = now.format("%Y-%m-%dT%H-%M").to_string();
    let ts_iso = now.format("%Y-%m-%dT%H:%M:%S%:z").to_string();

    let slug = args
        .subject
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug = if slug.len() > 50 { &slug[..50] } else { &slug };

    let impulse_id = format!("{ts_file}-{}-{slug}", gate_id.name);
    let filename = format!("{ts_file}_{}__{slug}.toml", gate_id.name);

    let wave = current_wave(workspace_root);
    let git_ref = resolve_head_ref(workspace_root, args.project);

    let impulse = ImpulseFile {
        impulse: ImpulseMeta {
            id: impulse_id.clone(),
            impulse_type: args.impulse_type.clone(),
            priority: args.priority.clone(),
            wave,
        },
        from: ImpulseFrom {
            gate: gate_id.name.clone(),
            team: args.team.to_string(),
            project: args.project.to_string(),
            git_ref,
        },
        to: ImpulseTo {
            gates: args.to_gates.iter().map(ToString::to_string).collect(),
            teams: if args.team.is_empty() {
                vec![]
            } else {
                vec![args.team.to_string()]
            },
        },
        content: ImpulseContent {
            subject: args.subject.to_string(),
            body: args.body.to_string(),
        },
        meta: ImpulseOpMeta {
            created: ts_iso,
            expires: String::new(),
            ack_required: args.impulse_type == ImpulseType::Frago
                || args.impulse_type == ImpulseType::Request,
        },
        signature: try_sign_impulse(workspace_root, &impulse_id),
        acks: vec![],
    };

    let active = active_dir(workspace_root);
    tokio::fs::create_dir_all(&active)
        .await
        .map_err(ShadowError::Io)?;

    let filepath = active.join(&filename);
    let toml_str = toml::to_string_pretty(&impulse).map_err(ShadowError::Serialize)?;
    crate::atomic_write_async(&filepath, toml_str.as_bytes())
        .await
        .map_err(ShadowError::Io)?;

    let wh_dir = workspace_root.join("infra/wateringHole");
    let push = crate::git_ops::add_commit_push(
        &wh_dir,
        &format!("impulses/active/{filename}"),
        &format!(
            "impulse: {} → {} — {}",
            gate_id.name,
            args.to_gates.join(","),
            args.subject,
        ),
    )
    .await?;
    if !push.failed.is_empty() {
        warn!(failed = ?push.failed, "impulse push partial failure");
    }

    try_relay_impulse(&impulse);

    Ok(impulse)
}

/// Sense pending impulses — quorumSignal SENSE.
pub fn sense(
    workspace_root: &Path,
    all: bool,
    count_only: bool,
) -> Result<(Vec<(String, ImpulseFile)>, usize)> {
    let active = active_dir(workspace_root);
    if !active.exists() {
        return Ok((vec![], 0));
    }

    let local_gate = if all {
        None
    } else {
        Some(identity::resolve(workspace_root)?.name)
    };

    let mut impulses = Vec::new();
    let entries = std::fs::read_dir(&active).map_err(ShadowError::Io)?;

    for entry in entries {
        let entry = entry.map_err(ShadowError::Io)?;
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "toml") {
            let contents = std::fs::read_to_string(&path).map_err(ShadowError::Io)?;
            if let Ok(mut imp) = parse_impulse_or_signal(&contents) {
                // Merge external acks from separate ack files
                let ext_acks = load_external_acks(workspace_root, &imp.impulse.id);
                for ack in ext_acks {
                    if !imp.acks.iter().any(|a| a.gate == ack.gate) {
                        imp.acks.push(ack);
                    }
                }

                let dominated = if let Some(ref gate) = local_gate {
                    imp.to.gates.iter().any(|g| g == "*") || imp.to.gates.iter().any(|g| g == gate)
                } else {
                    true
                };
                if dominated {
                    let fname = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    impulses.push((fname, imp));
                }
            }
        }
    }

    impulses.sort_by(|a, b| b.1.meta.created.cmp(&a.1.meta.created));
    let count = impulses.len();

    if count_only {
        Ok((vec![], count))
    } else {
        Ok((impulses, count))
    }
}

/// Check membrane potential health — quorumSignal SENSE.
pub fn check(workspace_root: &Path) -> Result<PotentialHealth> {
    let active = active_dir(workspace_root);
    let now = Utc::now();
    let wave = current_wave(workspace_root);

    let mut total = 0usize;
    let mut needs_ack = 0usize;
    let mut expired = 0usize;
    let mut by_wave = std::collections::BTreeMap::new();

    if active.exists() {
        let entries = std::fs::read_dir(&active).map_err(ShadowError::Io)?;
        for entry in entries {
            let entry = entry.map_err(ShadowError::Io)?;
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "toml") {
                continue;
            }
            let contents = std::fs::read_to_string(&path).map_err(ShadowError::Io)?;
            if let Ok(imp) = parse_impulse_or_signal(&contents) {
                total += 1;
                *by_wave.entry(imp.impulse.wave).or_insert(0) += 1;
                let ext_acks = load_external_acks(workspace_root, &imp.impulse.id);
                let all_acks_empty = imp.acks.is_empty() && ext_acks.is_empty();
                if imp.meta.ack_required && all_acks_empty {
                    needs_ack += 1;
                }
                if is_expired(&imp.meta.expires, &now) {
                    expired += 1;
                }
            }
        }
    }

    Ok(PotentialHealth {
        total,
        needs_ack,
        expired,
        by_wave,
        current_wave: wave,
    })
}

/// Acknowledge an impulse — rootPulse ACTION + waterFall SYNC.
///
/// Writes a separate ack file to `impulses/acks/{impulse-id}_{gate}.toml`
/// instead of appending to the original impulse TOML. This prevents ack loss
/// during rebase operations (Wave 67 safety evolution).
pub async fn ack(workspace_root: &Path, impulse_id: &str, note: &str) -> Result<ImpulseFile> {
    let gate_id = identity::resolve(workspace_root)?;
    let active = active_dir(workspace_root);

    let (_filepath, mut impulse) = find_impulse_by_id(&active, impulse_id)?;

    let ack_entry = ImpulseAck {
        gate: gate_id.name.clone(),
        timestamp: Utc::now().format("%Y-%m-%dT%H:%M:%S%:z").to_string(),
        note: note.to_string(),
    };

    let acks_dir = impulses_dir(workspace_root).join("acks");
    tokio::fs::create_dir_all(&acks_dir)
        .await
        .map_err(ShadowError::Io)?;

    let ack_filename = format!("{}_{}.toml", impulse_id, gate_id.name);
    let ack_path = acks_dir.join(&ack_filename);

    let ack_toml = toml::to_string_pretty(&ack_entry).map_err(ShadowError::Serialize)?;
    crate::atomic_write_async(&ack_path, ack_toml.as_bytes())
        .await
        .map_err(ShadowError::Io)?;

    // Also append to in-memory representation for return value
    impulse.acks.push(ack_entry);

    let wh_dir = workspace_root.join("infra/wateringHole");
    let push = crate::git_ops::add_commit_push(
        &wh_dir,
        &format!("impulses/acks/{ack_filename}"),
        &format!("impulse ack: {} ← {}", impulse.impulse.id, gate_id.name),
    )
    .await?;
    if !push.failed.is_empty() {
        warn!(failed = ?push.failed, "ack push partial failure");
    }

    Ok(impulse)
}

/// Archive discharged impulses — waterFall SYNC.
fn archive_expired_impulses(
    active: &Path,
    archive_dir: &Path,
    workspace_root: &Path,
    now: &chrono::DateTime<Utc>,
) -> Result<Vec<String>> {
    std::fs::create_dir_all(archive_dir).map_err(ShadowError::Io)?;
    let mut archived = Vec::new();
    let entries = std::fs::read_dir(active).map_err(ShadowError::Io)?;

    for entry in entries {
        let entry = entry.map_err(ShadowError::Io)?;
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "toml") {
            continue;
        }

        let contents = std::fs::read_to_string(&path).map_err(ShadowError::Io)?;
        let impulse: ImpulseFile = match parse_impulse_or_signal(&contents) {
            Ok(i) => i,
            Err(_) => continue,
        };

        let ext_acks = load_external_acks(workspace_root, &impulse.impulse.id);
        let should_archive = is_expired(&impulse.meta.expires, now)
            || is_fully_acked_with_externals(&impulse, &ext_acks);

        if should_archive {
            let fname = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let dest = archive_dir.join(&fname);
            std::fs::rename(&path, &dest).map_err(ShadowError::Io)?;
            archived.push(fname);
        }
    }
    Ok(archived)
}

/// Archive discharged impulses — moves expired or fully-acked impulses to `archive/waveN/`.
pub async fn archive(workspace_root: &Path) -> Result<Vec<String>> {
    let active = active_dir(workspace_root);
    if !active.exists() {
        return Ok(vec![]);
    }

    let now = Utc::now();
    let wave = current_wave(workspace_root);
    let archive_dir = impulses_dir(workspace_root)
        .join("archive")
        .join(format!("wave{wave}"));

    let ws_root = workspace_root.to_path_buf();
    let archived = tokio::task::spawn_blocking(move || {
        archive_expired_impulses(&active, &archive_dir, &ws_root, &now)
    })
    .await
    .map_err(|_| ShadowError::Parse("archive task panicked".into()))??;

    if !archived.is_empty() {
        let wh_dir = workspace_root.join("infra/wateringHole");
        let msg = format!(
            "impulse archive: {} discharged → wave{wave}",
            archived.len()
        );
        let push = crate::git_ops::add_all_commit_push(&wh_dir, "impulses/", &msg).await?;
        if !push.failed.is_empty() {
            warn!(failed = ?push.failed, "archive push partial failure");
        }
    }

    Ok(archived)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sense_returns_empty_for_missing_dir() {
        let result = sense(
            std::path::Path::new("/tmp/nonexistent-impulse-test"),
            true,
            true,
        );
        assert!(result.is_ok());
        let (impulses, count) = result.unwrap();
        assert!(impulses.is_empty());
        assert_eq!(count, 0);
    }
}
