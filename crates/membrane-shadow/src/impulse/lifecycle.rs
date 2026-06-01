// SPDX-License-Identifier: AGPL-3.0-or-later

//! Impulse lifecycle operations: post, sense, check, ack, archive.

use chrono::{Local, Utc};
use std::path::Path;

use super::parse::{find_impulse_by_id, parse_impulse_or_signal};
use super::policy::{is_expired, is_fully_acked};
use super::primal::{try_relay_impulse, try_sign_impulse};
use super::types::{
    ImpulseAck, ImpulseContent, ImpulseFile, ImpulseFrom, ImpulseMeta, ImpulseOpMeta, ImpulseTo,
    ImpulseType, PostArgs, PotentialHealth, active_dir, current_wave, impulses_dir,
    resolve_head_ref,
};
use crate::error::{Result, ShadowError};
use crate::identity;

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
    std::fs::create_dir_all(&active).map_err(ShadowError::Io)?;

    let filepath = active.join(&filename);
    let toml_str = toml::to_string_pretty(&impulse)
        .map_err(|e| ShadowError::Parse(format!("serialize impulse: {e}")))?;
    std::fs::write(&filepath, &toml_str).map_err(ShadowError::Io)?;

    let wh_dir = workspace_root.join("infra/wateringHole");
    crate::git_ops::add_commit_push(
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
            if let Ok(imp) = parse_impulse_or_signal(&contents) {
                let dominated = if let Some(ref gate) = local_gate {
                    imp.to.gates.contains(&"*".to_string())
                        || imp.to.gates.iter().any(|g| g == gate)
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
                if imp.meta.ack_required && imp.acks.is_empty() {
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
pub async fn ack(workspace_root: &Path, impulse_id: &str, note: &str) -> Result<ImpulseFile> {
    let gate_id = identity::resolve(workspace_root)?;
    let active = active_dir(workspace_root);

    let (filepath, mut impulse) = find_impulse_by_id(&active, impulse_id)?;

    let ack_entry = ImpulseAck {
        gate: gate_id.name.clone(),
        timestamp: Utc::now().format("%Y-%m-%dT%H:%M:%S%:z").to_string(),
        note: note.to_string(),
    };
    impulse.acks.push(ack_entry);

    let toml_str = toml::to_string_pretty(&impulse)
        .map_err(|e| ShadowError::Parse(format!("serialize impulse: {e}")))?;
    std::fs::write(&filepath, &toml_str).map_err(ShadowError::Io)?;

    let rel_path = filepath
        .strip_prefix(workspace_root.join("infra/wateringHole"))
        .unwrap_or(&filepath)
        .to_string_lossy()
        .to_string();

    let wh_dir = workspace_root.join("infra/wateringHole");
    crate::git_ops::add_commit_push(
        &wh_dir,
        &rel_path,
        &format!("impulse ack: {} ← {}", impulse.impulse.id, gate_id.name),
    )
    .await?;

    Ok(impulse)
}

/// Archive discharged impulses — waterFall SYNC.
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
    std::fs::create_dir_all(&archive_dir).map_err(ShadowError::Io)?;

    let mut archived = Vec::new();
    let entries = std::fs::read_dir(&active).map_err(ShadowError::Io)?;

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

        let should_archive = is_expired(&impulse.meta.expires, &now) || is_fully_acked(&impulse);

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

    if !archived.is_empty() {
        let wh_dir = workspace_root.join("infra/wateringHole");
        let msg = format!(
            "impulse archive: {} discharged → wave{wave}",
            archived.len()
        );
        crate::git_ops::add_all_commit_push(&wh_dir, "impulses/", &msg).await?;
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
