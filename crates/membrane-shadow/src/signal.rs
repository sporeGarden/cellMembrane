// SPDX-License-Identifier: AGPL-3.0-or-later

//! Signal/FRAGO system — git-mediated inter-gate messaging.
//!
//! Signals are TOML files in `infra/wateringHole/signals/active/` that
//! ride alongside code pushes. Teams create signals with `signal.post`,
//! discover them with `signal.list`, and acknowledge with `signal.ack`.

use crate::error::{Result, ShadowError};
use crate::identity;
use chrono::{Local, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Signal types following the SIGNAL_FRAGO_STANDARD.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SignalType {
    /// Amends a standing order — action required.
    Frago,
    /// Informational state update — no action required.
    Status,
    /// Asks for something from target gate(s).
    Request,
    /// Broadcast ecosystem-wide notice.
    Announce,
}

impl std::fmt::Display for SignalType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Frago => write!(f, "FRAGO"),
            Self::Status => write!(f, "STATUS"),
            Self::Request => write!(f, "REQUEST"),
            Self::Announce => write!(f, "ANNOUNCE"),
        }
    }
}

/// Priority levels for signals.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    /// Normal workflow coordination.
    Routine,
    /// Time-sensitive, blocking other work.
    Priority,
    /// Critical — requires immediate attention.
    Flash,
}

impl std::fmt::Display for Priority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Routine => write!(f, "routine"),
            Self::Priority => write!(f, "PRIORITY"),
            Self::Flash => write!(f, "FLASH"),
        }
    }
}

/// Top-level signal file structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalFile {
    /// Signal metadata.
    pub signal: SignalMeta,
    /// Origin information.
    pub from: SignalFrom,
    /// Target information.
    pub to: SignalTo,
    /// Message content.
    pub content: SignalContent,
    /// Operational metadata.
    pub meta: SignalOpMeta,
    /// Acknowledgments from receiving gates.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub acks: Vec<SignalAck>,
}

/// The [signal] table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalMeta {
    /// Unique signal ID.
    pub id: String,
    /// Signal type.
    #[serde(rename = "type")]
    pub signal_type: SignalType,
    /// Priority level.
    pub priority: Priority,
    /// Wave number when created.
    pub wave: u32,
}

/// The [from] table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalFrom {
    /// Originating gate.
    pub gate: String,
    /// Team name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub team: String,
    /// Project path (e.g. "springs/hotSpring").
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub project: String,
    /// Commit ref that prompted this signal.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    #[serde(rename = "ref")]
    pub git_ref: String,
}

/// The [to] table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalTo {
    /// Target gates (["*"] for broadcast).
    pub gates: Vec<String>,
    /// Target teams (informational).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub teams: Vec<String>,
}

/// The [content] table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalContent {
    /// Short subject line (max 80 chars).
    pub subject: String,
    /// Optional extended body.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body: String,
}

/// The [meta] table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalOpMeta {
    /// Creation timestamp.
    pub created: String,
    /// Expiration timestamp (optional).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub expires: String,
    /// Whether acknowledgment is required.
    #[serde(default)]
    pub ack_required: bool,
}

/// An acknowledgment entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalAck {
    /// Gate that acknowledged.
    pub gate: String,
    /// When it was acknowledged.
    pub timestamp: String,
    /// Optional note.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// Arguments for creating a new signal.
pub struct PostArgs<'a> {
    /// Target gate names.
    pub to_gates: Vec<&'a str>,
    /// Signal type (frago, status, request, announce).
    pub signal_type: SignalType,
    /// Priority level.
    pub priority: Priority,
    /// Short subject line.
    pub subject: &'a str,
    /// Optional extended body text.
    pub body: &'a str,
    /// Project path (e.g. "springs/hotSpring").
    pub project: &'a str,
    /// Git commit ref that prompted this signal.
    pub git_ref: &'a str,
    /// Team name.
    pub team: &'a str,
}

/// Resolve the signals directory within the workspace.
fn signals_dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join("infra/wateringHole/signals")
}

/// Resolve the active signals directory.
fn active_dir(workspace_root: &Path) -> PathBuf {
    signals_dir(workspace_root).join("active")
}

/// Get the current wave number from freshness.toml.
fn current_wave(workspace_root: &Path) -> u32 {
    let freshness = workspace_root.join("infra/wateringHole/freshness.toml");
    if let Ok(contents) = std::fs::read_to_string(&freshness) {
        if let Ok(val) = contents.parse::<toml::Table>() {
            if let Some(wave) = val.get("wave").and_then(|w| w.as_table()) {
                if let Some(id) = wave.get("id").and_then(|i| i.as_integer()) {
                    return id as u32;
                }
            }
        }
    }
    0
}

/// Create a new signal file, commit it, and push to all remotes.
pub async fn post(workspace_root: &Path, args: &PostArgs<'_>) -> Result<SignalFile> {
    let gate_id = identity::resolve(workspace_root)?;
    let now = Local::now();
    let ts_file = now.format("%Y-%m-%dT%H-%M").to_string();
    let ts_iso = now.format("%Y-%m-%dT%H:%M:%S%:z").to_string();

    let slug = args
        .subject
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug = if slug.len() > 50 { &slug[..50] } else { &slug };

    let signal_id = format!("{}-{}-{}", ts_file, gate_id.name, slug);
    let filename = format!("{}_{}_{}. toml", ts_file, gate_id.name, slug)
        .replace(". toml", ".toml");

    let wave = current_wave(workspace_root);

    let signal = SignalFile {
        signal: SignalMeta {
            id: signal_id,
            signal_type: args.signal_type.clone(),
            priority: args.priority.clone(),
            wave,
        },
        from: SignalFrom {
            gate: gate_id.name.clone(),
            team: args.team.to_string(),
            project: args.project.to_string(),
            git_ref: args.git_ref.to_string(),
        },
        to: SignalTo {
            gates: args.to_gates.iter().map(|s| s.to_string()).collect(),
            teams: if args.team.is_empty() {
                vec![]
            } else {
                vec![args.team.to_string()]
            },
        },
        content: SignalContent {
            subject: args.subject.to_string(),
            body: args.body.to_string(),
        },
        meta: SignalOpMeta {
            created: ts_iso,
            expires: String::new(),
            ack_required: args.signal_type == SignalType::Frago
                || args.signal_type == SignalType::Request,
        },
        acks: vec![],
    };

    let active = active_dir(workspace_root);
    std::fs::create_dir_all(&active).map_err(ShadowError::Io)?;

    let filepath = active.join(&filename);
    let toml_str = toml::to_string_pretty(&signal)
        .map_err(|e| ShadowError::Parse(format!("serialize signal: {e}")))?;
    std::fs::write(&filepath, &toml_str).map_err(ShadowError::Io)?;

    let wh_dir = workspace_root.join("infra/wateringHole");
    git_add_commit_push(&wh_dir, &format!("signals/active/{filename}"), &format!(
        "signal: {} → {} — {}",
        gate_id.name,
        args.to_gates.join(","),
        args.subject,
    ))
    .await?;

    Ok(signal)
}

/// List active signals, optionally filtered to the local gate.
pub fn list(workspace_root: &Path, all: bool) -> Result<Vec<(String, SignalFile)>> {
    let active = active_dir(workspace_root);
    if !active.exists() {
        return Ok(vec![]);
    }

    let local_gate = if all {
        None
    } else {
        Some(identity::resolve(workspace_root)?.name)
    };

    let mut signals = Vec::new();
    let entries = std::fs::read_dir(&active).map_err(ShadowError::Io)?;

    for entry in entries {
        let entry = entry.map_err(ShadowError::Io)?;
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "toml") {
            let contents = std::fs::read_to_string(&path).map_err(ShadowError::Io)?;
            if let Ok(signal) = toml::from_str::<SignalFile>(&contents) {
                let dominated = if let Some(ref gate) = local_gate {
                    signal.to.gates.contains(&"*".to_string())
                        || signal.to.gates.iter().any(|g| g == gate)
                } else {
                    true
                };
                if dominated {
                    let fname = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    signals.push((fname, signal));
                }
            }
        }
    }

    signals.sort_by(|a, b| b.1.meta.created.cmp(&a.1.meta.created));
    Ok(signals)
}

/// Acknowledge a signal by ID.
pub async fn ack(workspace_root: &Path, signal_id: &str, note: &str) -> Result<SignalFile> {
    let gate_id = identity::resolve(workspace_root)?;
    let active = active_dir(workspace_root);

    let (filepath, mut signal) = find_signal_by_id(&active, signal_id)?;

    let ack_entry = SignalAck {
        gate: gate_id.name.clone(),
        timestamp: Utc::now().format("%Y-%m-%dT%H:%M:%S%:z").to_string(),
        note: note.to_string(),
    };
    signal.acks.push(ack_entry);

    let toml_str = toml::to_string_pretty(&signal)
        .map_err(|e| ShadowError::Parse(format!("serialize signal: {e}")))?;
    std::fs::write(&filepath, &toml_str).map_err(ShadowError::Io)?;

    let rel_path = filepath
        .strip_prefix(workspace_root.join("infra/wateringHole"))
        .unwrap_or(&filepath)
        .to_string_lossy()
        .to_string();

    let wh_dir = workspace_root.join("infra/wateringHole");
    git_add_commit_push(
        &wh_dir,
        &rel_path,
        &format!("signal ack: {} ← {}", signal.signal.id, gate_id.name),
    )
    .await?;

    Ok(signal)
}

/// Archive expired or fully-acknowledged signals.
pub async fn archive(workspace_root: &Path) -> Result<Vec<String>> {
    let active = active_dir(workspace_root);
    if !active.exists() {
        return Ok(vec![]);
    }

    let now = Utc::now();
    let wave = current_wave(workspace_root);
    let archive_dir = signals_dir(workspace_root)
        .join("archive")
        .join(format!("wave{wave}"));
    std::fs::create_dir_all(&archive_dir).map_err(ShadowError::Io)?;

    let mut archived = Vec::new();
    let entries = std::fs::read_dir(&active).map_err(ShadowError::Io)?;

    for entry in entries {
        let entry = entry.map_err(ShadowError::Io)?;
        let path = entry.path();
        if !path.extension().is_some_and(|e| e == "toml") {
            continue;
        }

        let contents = std::fs::read_to_string(&path).map_err(ShadowError::Io)?;
        let signal: SignalFile = match toml::from_str(&contents) {
            Ok(s) => s,
            Err(_) => continue,
        };

        let should_archive = is_expired(&signal.meta.expires, &now) || is_fully_acked(&signal);

        if should_archive {
            let fname = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            let dest = archive_dir.join(&fname);
            std::fs::rename(&path, &dest).map_err(ShadowError::Io)?;
            archived.push(fname);
        }
    }

    if !archived.is_empty() {
        let wh_dir = workspace_root.join("infra/wateringHole");
        let msg = format!("signal archive: {} signals → wave{wave}", archived.len());
        git_add_all_commit_push(&wh_dir, &msg).await?;
    }

    Ok(archived)
}

fn is_expired(expires: &str, now: &chrono::DateTime<Utc>) -> bool {
    if expires.is_empty() {
        return false;
    }
    chrono::DateTime::parse_from_str(expires, "%Y-%m-%dT%H:%M:%S%:z")
        .map(|exp| now > &exp)
        .unwrap_or(false)
}

fn is_fully_acked(signal: &SignalFile) -> bool {
    if !signal.meta.ack_required || signal.to.gates.is_empty() {
        return false;
    }
    if signal.to.gates.contains(&"*".to_string()) {
        return false;
    }
    signal
        .to
        .gates
        .iter()
        .all(|g| signal.acks.iter().any(|a| &a.gate == g))
}

fn find_signal_by_id(active_dir: &Path, signal_id: &str) -> Result<(PathBuf, SignalFile)> {
    let entries = std::fs::read_dir(active_dir).map_err(ShadowError::Io)?;
    for entry in entries {
        let entry = entry.map_err(ShadowError::Io)?;
        let path = entry.path();
        if !path.extension().is_some_and(|e| e == "toml") {
            continue;
        }
        let contents = std::fs::read_to_string(&path).map_err(ShadowError::Io)?;
        if let Ok(signal) = toml::from_str::<SignalFile>(&contents) {
            if signal.signal.id == signal_id || path.file_stem().is_some_and(|s| s.to_string_lossy().contains(signal_id))
            {
                return Ok((path, signal));
            }
        }
    }
    Err(ShadowError::Parse(format!(
        "signal not found: {signal_id}"
    )))
}

async fn git_add_commit_push(repo_dir: &Path, file_path: &str, message: &str) -> Result<()> {
    let status = tokio::process::Command::new("git")
        .args(["add", file_path])
        .current_dir(repo_dir)
        .status()
        .await
        .map_err(ShadowError::Io)?;
    if !status.success() {
        return Err(ShadowError::Parse(format!("git add failed for {file_path}")));
    }

    let status = tokio::process::Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(repo_dir)
        .status()
        .await
        .map_err(ShadowError::Io)?;
    if !status.success() {
        return Err(ShadowError::Parse("git commit failed".into()));
    }

    for remote in ["origin", "forgejo"] {
        let _ = tokio::process::Command::new("git")
            .args(["push", remote, "main", "--quiet"])
            .current_dir(repo_dir)
            .status()
            .await;
    }

    Ok(())
}

async fn git_add_all_commit_push(repo_dir: &Path, message: &str) -> Result<()> {
    let status = tokio::process::Command::new("git")
        .args(["add", "-A", "signals/"])
        .current_dir(repo_dir)
        .status()
        .await
        .map_err(ShadowError::Io)?;
    if !status.success() {
        return Err(ShadowError::Parse("git add -A signals/ failed".into()));
    }

    let status = tokio::process::Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(repo_dir)
        .status()
        .await
        .map_err(ShadowError::Io)?;
    if !status.success() {
        return Err(ShadowError::Parse("git commit failed".into()));
    }

    for remote in ["origin", "forgejo"] {
        let _ = tokio::process::Command::new("git")
            .args(["push", remote, "main", "--quiet"])
            .current_dir(repo_dir)
            .status()
            .await;
    }

    Ok(())
}
