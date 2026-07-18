// SPDX-License-Identifier: AGPL-3.0-or-later

//! Context braids — ephemeral developer-state weaving across the gate mesh.
//!
//! The external analog of sweetGrass braids: sweetGrass weaves meaning into
//! data (provenance, attribution, lineage); context braids weave meaning into
//! developer state (focus, breadcrumbs, next actions, blockers).
//!
//! Context braids are TOML files in `infra/wateringHole/context/{gate}/` that
//! provide short-term memory for developers rotating across LAN and WAN gates.
//!
//! Commands:
//!   - `context.weave`  — create/update a context braid (last-writer-wins)
//!   - `context.sense`  — read context braids (observe mesh state)
//!   - `context.clear`  — decay expired braids or explicitly clear one

use crate::error::{Result, ShadowError};
use crate::identity;
use chrono::{Local, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::warn;

// ── Schema ────────────────────────────────────────────────────────────────

/// Top-level context braid file structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextBraid {
    /// Braid header metadata.
    pub braid: BraidHeader,
    /// Woven strands of developer context.
    pub strands: BraidStrands,
}

/// The `[braid]` table — metadata about this context braid.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BraidHeader {
    /// Gate that owns this braid.
    pub gate: String,
    /// Project path relative to workspace root.
    pub project: String,
    /// When this braid was last woven (ISO-8601).
    pub updated: String,
    /// Gate that last updated this braid.
    pub updated_by: String,
    /// Hours before this braid auto-decays.
    pub ttl_hours: u32,
    /// Ecosystem wave at time of weaving.
    pub wave: u32,
}

/// The `[strands]` table — multiple strands woven together.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BraidStrands {
    /// What is actively being worked on (required).
    pub focus: FocusStrand,
    /// File paths and entry points a developer would need.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub breadcrumbs: Option<BreadcrumbStrand>,
    /// Upcoming actions or handoff tasks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<NextStrand>,
    /// What's preventing progress.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blockers: Option<BlockerStrand>,
    /// Freeform context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<NotesStrand>,
}

/// Focus strand — what is being worked on right now.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FocusStrand {
    /// One-line description of current work.
    pub summary: String,
    /// Current status.
    pub status: FocusStatus,
}

/// Focus status values.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FocusStatus {
    /// Actively being worked on.
    Active,
    /// Paused but not blocked.
    Paused,
    /// Blocked by something.
    Blocked,
    /// Work is complete.
    Complete,
}

impl std::str::FromStr for FocusStatus {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "active" => Ok(Self::Active),
            "paused" => Ok(Self::Paused),
            "blocked" => Ok(Self::Blocked),
            "complete" => Ok(Self::Complete),
            _ => Err(format!(
                "unknown status: {s} (expected: active|paused|blocked|complete)"
            )),
        }
    }
}

impl std::fmt::Display for FocusStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Active => write!(f, "active"),
            Self::Paused => write!(f, "paused"),
            Self::Blocked => write!(f, "BLOCKED"),
            Self::Complete => write!(f, "complete"),
        }
    }
}

/// Breadcrumbs strand — code locations for orientation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BreadcrumbStrand {
    /// Ordered list of relevant code locations.
    pub trail: Vec<String>,
}

/// Next strand — upcoming actions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NextStrand {
    /// What should happen next.
    pub actions: Vec<String>,
}

/// Blocker strand — what's preventing progress.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockerStrand {
    /// Current blockers.
    pub items: Vec<String>,
}

/// Notes strand — freeform context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotesStrand {
    /// Multi-line freeform text.
    pub body: String,
}

// ── CLI Arguments ─────────────────────────────────────────────────────────

/// Arguments for weaving a context braid.
pub struct WeaveArgs<'a> {
    /// Project path (e.g. "springs/hotSpring").
    pub project: &'a str,
    /// Focus strand summary.
    pub summary: &'a str,
    /// Focus status (default: active).
    pub status: FocusStatus,
    /// Comma-separated breadcrumb trail entries.
    pub breadcrumbs: &'a str,
    /// Comma-separated next actions.
    pub next: &'a str,
    /// Comma-separated blockers.
    pub blockers: &'a str,
    /// Freeform notes body.
    pub notes: &'a str,
    /// TTL in hours.
    pub ttl_hours: u32,
}

// ── Path Helpers ──────────────────────────────────────────────────────────

fn context_dir(workspace_root: &Path) -> PathBuf {
    workspace_root
        .join(cellmembrane_types::service::INFRA_WATERING_HOLE)
        .join("context")
}

fn gate_context_dir(workspace_root: &Path, gate: &str) -> PathBuf {
    context_dir(workspace_root).join(gate)
}

fn project_slug(project: &str) -> String {
    project
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn braid_filepath(workspace_root: &Path, gate: &str, project: &str) -> PathBuf {
    let slug = project_slug(project);
    gate_context_dir(workspace_root, gate).join(format!("{slug}.toml"))
}

fn current_wave(workspace_root: &Path) -> u32 {
    crate::freshness::current_wave(workspace_root)
}

// ── Operations ────────────────────────────────────────────────────────────

/// Weave a context braid — create or overwrite for this gate+project.
pub async fn weave(workspace_root: &Path, args: &WeaveArgs<'_>) -> Result<ContextBraid> {
    let gate_id = identity::resolve_async(workspace_root).await?;
    let now = Local::now();
    let ts_iso = now
        .format(cellmembrane_types::service::ISO8601_TZ)
        .to_string();
    let wave = current_wave(workspace_root);

    let breadcrumbs = if args.breadcrumbs.is_empty() {
        None
    } else {
        Some(BreadcrumbStrand {
            trail: args
                .breadcrumbs
                .split(',')
                .map(|s| s.trim().to_string())
                .collect(),
        })
    };

    let next = if args.next.is_empty() {
        None
    } else {
        Some(NextStrand {
            actions: args.next.split(',').map(|s| s.trim().to_string()).collect(),
        })
    };

    let blockers = if args.blockers.is_empty() {
        None
    } else {
        let items: Vec<String> = args
            .blockers
            .split(',')
            .map(|s| s.trim().to_string())
            .collect();
        if items.iter().all(String::is_empty) {
            None
        } else {
            Some(BlockerStrand { items })
        }
    };

    let notes = if args.notes.is_empty() {
        None
    } else {
        Some(NotesStrand {
            body: args.notes.to_string(),
        })
    };

    let braid = ContextBraid {
        braid: BraidHeader {
            gate: gate_id.name.clone(),
            project: args.project.to_string(),
            updated: ts_iso,
            updated_by: gate_id.name.clone(),
            ttl_hours: args.ttl_hours,
            wave,
        },
        strands: BraidStrands {
            focus: FocusStrand {
                summary: args.summary.to_string(),
                status: args.status.clone(),
            },
            breadcrumbs,
            next,
            blockers,
            notes,
        },
    };

    let gate_dir = gate_context_dir(workspace_root, &gate_id.name);
    tokio::fs::create_dir_all(&gate_dir)
        .await
        .map_err(ShadowError::Io)?;

    let filepath = braid_filepath(workspace_root, &gate_id.name, args.project);
    let toml_str = toml::to_string_pretty(&braid).map_err(ShadowError::Serialize)?;
    crate::atomic_write_async(&filepath, toml_str.as_bytes())
        .await
        .map_err(ShadowError::Io)?;

    let slug = project_slug(args.project);
    let rel_path = format!("context/{}/{slug}.toml", gate_id.name);
    let wh_dir = workspace_root.join(cellmembrane_types::service::INFRA_WATERING_HOLE);
    git_add_commit_push(
        &wh_dir,
        &rel_path,
        &format!("[context] weave {}/{slug}", gate_id.name),
    )
    .await?;

    Ok(braid)
}

/// Sense context braids — read current mesh state.
///
/// Filters by gate and/or project. Returns all matching braids sorted by
/// update time (most recent first).
pub(crate) fn sense(
    workspace_root: &Path,
    filter_gate: Option<&str>,
    filter_project: Option<&str>,
    all: bool,
) -> Result<Vec<ContextBraid>> {
    let ctx_dir = context_dir(workspace_root);
    if !ctx_dir.exists() {
        return Ok(vec![]);
    }

    let local_gate = if !all && filter_gate.is_none() {
        Some(identity::resolve(workspace_root)?.name)
    } else {
        None
    };

    let target_gate = filter_gate.map(ToString::to_string).or(local_gate);

    let mut braids = Vec::new();

    let gate_dirs = if let Some(ref gate) = target_gate {
        let gd = gate_context_dir(workspace_root, gate);
        if gd.exists() { vec![gd] } else { vec![] }
    } else {
        std::fs::read_dir(&ctx_dir)
            .map_err(ShadowError::Io)?
            .filter_map(std::result::Result::ok)
            .map(|e| e.path())
            .filter(|p: &PathBuf| p.is_dir())
            .collect()
    };

    let project_slug_filter = filter_project.map(project_slug);

    for gate_dir in gate_dirs {
        let Ok(entries) = std::fs::read_dir(&gate_dir) else {
            continue;
        };

        for entry in entries {
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "toml") {
                continue;
            }

            if let Some(ref slug_filter) = project_slug_filter {
                let file_slug = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                if &file_slug != slug_filter {
                    continue;
                }
            }

            let Ok(contents) = std::fs::read_to_string(&path) else {
                continue;
            };

            match toml::from_str::<ContextBraid>(&contents) {
                Ok(braid) => braids.push(braid),
                Err(e) => tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "skipping corrupt context braid"
                ),
            }
        }
    }

    braids.sort_by(|a, b| b.braid.updated.cmp(&a.braid.updated));
    Ok(braids)
}

/// Clear context braids — decay expired or remove specific project braid.
///
/// Returns list of cleared braid descriptions (gate/slug).
pub async fn clear(
    workspace_root: &Path,
    project: Option<&str>,
    expired_only: bool,
) -> Result<Vec<String>> {
    let ctx_dir = context_dir(workspace_root);
    if !ctx_dir.exists() {
        return Ok(vec![]);
    }

    let mut cleared = Vec::new();

    if let Some(proj) = project {
        let gate_id = identity::resolve_async(workspace_root).await?;
        let filepath = braid_filepath(workspace_root, &gate_id.name, proj);
        if filepath.exists() {
            tokio::fs::remove_file(&filepath)
                .await
                .map_err(ShadowError::Io)?;
            let slug = project_slug(proj);
            cleared.push(format!("{}/{slug}", gate_id.name));
        }
    } else if expired_only {
        let ctx_owned = ctx_dir.clone();
        cleared = tokio::task::spawn_blocking(move || clear_expired_braids(&ctx_owned))
            .await
            .map_err(|_| ShadowError::Io(std::io::Error::other("clear task panicked")))??;
    }

    if !cleared.is_empty() {
        let wh_dir = workspace_root.join(cellmembrane_types::service::INFRA_WATERING_HOLE);
        let msg = format!("[context] clear {} expired braid(s)", cleared.len());
        git_add_all_commit_push(&wh_dir, &msg).await?;
    }

    Ok(cleared)
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn clear_expired_braids(ctx_dir: &Path) -> Result<Vec<String>> {
    let now = Utc::now();
    let mut cleared = Vec::new();

    let gate_dirs: Vec<PathBuf> = std::fs::read_dir(ctx_dir)
        .map_err(ShadowError::Io)?
        .filter_map(std::result::Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();

    for gate_dir in gate_dirs {
        let gate_name = gate_dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let Ok(entries) = std::fs::read_dir(&gate_dir) else {
            continue;
        };

        for entry in entries {
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "toml") {
                continue;
            }

            let Ok(contents) = std::fs::read_to_string(&path) else {
                continue;
            };

            if let Ok(braid) = toml::from_str::<ContextBraid>(&contents) {
                if is_expired(&braid.braid.updated, braid.braid.ttl_hours, &now) {
                    let slug = path
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    std::fs::remove_file(&path).map_err(ShadowError::Io)?;
                    cleared.push(format!("{gate_name}/{slug}"));
                }
            }
        }
    }
    Ok(cleared)
}

fn is_expired(updated: &str, ttl_hours: u32, now: &chrono::DateTime<Utc>) -> bool {
    chrono::DateTime::parse_from_str(updated, cellmembrane_types::service::ISO8601_TZ).is_ok_and(
        |updated_dt| {
            let expires_at = updated_dt + chrono::Duration::hours(i64::from(ttl_hours));
            now > &expires_at
        },
    )
}

async fn git_add_commit_push(repo_dir: &Path, file_path: &str, message: &str) -> Result<()> {
    let push = crate::git_ops::add_commit_push(repo_dir, file_path, message).await?;
    if !push.failed.is_empty() {
        warn!(
            succeeded = push.succeeded,
            total = push.succeeded + u32::try_from(push.failed.len()).unwrap_or(u32::MAX),
            failed = ?push.failed,
            "context push partial failure"
        );
    }
    Ok(())
}

async fn git_add_all_commit_push(repo_dir: &Path, message: &str) -> Result<()> {
    let push = crate::git_ops::add_all_commit_push(repo_dir, "context/", message).await?;
    if !push.failed.is_empty() {
        warn!(
            succeeded = push.succeeded,
            total = push.succeeded + u32::try_from(push.failed.len()).unwrap_or(u32::MAX),
            failed = ?push.failed,
            "context push partial failure"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_slug_normalizes() {
        assert_eq!(project_slug("springs/hotSpring"), "springs-hotspring");
        assert_eq!(project_slug("primals/bearDog"), "primals-beardog");
        assert_eq!(project_slug("cellMembrane"), "cellmembrane");
    }

    #[test]
    fn project_slug_collapses_separators() {
        assert_eq!(project_slug("a//b--c"), "a-b-c");
        assert_eq!(project_slug("---leading---"), "leading");
    }

    #[test]
    fn braid_filepath_structure() {
        let root = Path::new("/opt/eco");
        let path = braid_filepath(root, "eastGate", "springs/hotSpring");
        assert_eq!(
            path,
            PathBuf::from("/opt/eco/infra/wateringHole/context/eastGate/springs-hotspring.toml")
        );
    }

    #[test]
    fn focus_status_display() {
        assert_eq!(FocusStatus::Active.to_string(), "active");
        assert_eq!(FocusStatus::Paused.to_string(), "paused");
        assert_eq!(FocusStatus::Blocked.to_string(), "BLOCKED");
        assert_eq!(FocusStatus::Complete.to_string(), "complete");
    }

    #[test]
    fn focus_status_from_str() {
        assert_eq!(
            "active".parse::<FocusStatus>().unwrap(),
            FocusStatus::Active
        );
        assert_eq!(
            "paused".parse::<FocusStatus>().unwrap(),
            FocusStatus::Paused
        );
        assert_eq!(
            "blocked".parse::<FocusStatus>().unwrap(),
            FocusStatus::Blocked
        );
        assert_eq!(
            "complete".parse::<FocusStatus>().unwrap(),
            FocusStatus::Complete
        );
        assert!("invalid".parse::<FocusStatus>().is_err());
    }

    #[test]
    fn is_expired_within_ttl() {
        let now = Utc::now();
        let updated = (now - chrono::Duration::hours(1))
            .format(cellmembrane_types::service::ISO8601_TZ)
            .to_string();
        assert!(!is_expired(&updated, 24, &now));
    }

    #[test]
    fn is_expired_past_ttl() {
        let now = Utc::now();
        let updated = (now - chrono::Duration::hours(25))
            .format(cellmembrane_types::service::ISO8601_TZ)
            .to_string();
        assert!(is_expired(&updated, 24, &now));
    }

    #[test]
    fn is_expired_invalid_timestamp() {
        let now = Utc::now();
        assert!(!is_expired("not-a-date", 24, &now));
    }

    #[test]
    fn context_braid_roundtrip() {
        let braid = ContextBraid {
            braid: BraidHeader {
                gate: "eastGate".into(),
                project: "cellMembrane".into(),
                updated: "2026-06-07T12:00:00-04:00".into(),
                updated_by: "eastGate".into(),
                ttl_hours: 72,
                wave: 88,
            },
            strands: BraidStrands {
                focus: FocusStrand {
                    summary: "Pipeline evolution".into(),
                    status: FocusStatus::Active,
                },
                breadcrumbs: Some(BreadcrumbStrand {
                    trail: vec!["src/plasmid/mod.rs".into()],
                }),
                next: None,
                blockers: None,
                notes: None,
            },
        };

        let serialized = toml::to_string_pretty(&braid).unwrap();
        let deserialized: ContextBraid = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.braid.gate, "eastGate");
        assert_eq!(deserialized.strands.focus.status, FocusStatus::Active);
        assert!(deserialized.strands.breadcrumbs.is_some());
        assert!(deserialized.strands.next.is_none());
    }

    #[test]
    fn focus_status_serde_lowercase() {
        let json = serde_json::to_string(&FocusStatus::Blocked).unwrap();
        assert_eq!(json, "\"blocked\"");
        let parsed: FocusStatus = serde_json::from_str("\"complete\"").unwrap();
        assert_eq!(parsed, FocusStatus::Complete);
    }
}
