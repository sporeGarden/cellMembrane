// SPDX-License-Identifier: AGPL-3.0-or-later

//! impulsePotential — inter-gate coordination via membrane action potentials.
//!
//! Impulses are TOML files in `infra/wateringHole/impulses/active/` that
//! ride alongside code pushes. Gates fire impulses with `impulse.post` (rP),
//! sense pending potential with `potential.sense` (qS), acknowledge with
//! `impulse.ack` (rP+wF), and archive with `impulse.archive` (wF).
//!
//! Triad mapping:
//!   - `impulse.post`    → rootPulse (ACTION) — fire action potential
//!   - `impulse.ack`     → rootPulse + waterFall (ACTION + SYNC)
//!   - `impulse.archive` → waterFall (SYNC) — discharge spent impulses
//!   - `potential.sense`  → quorumSignal (SENSE) — measure membrane potential
//!   - `potential.check`  → quorumSignal (SENSE) — gradient health

use crate::error::{Result, ShadowError};
use crate::identity;
use chrono::{Local, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Impulse types following the IMPULSE_POTENTIAL_STANDARD.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ImpulseType {
    /// Amends a standing order — action required.
    Frago,
    /// Informational state update — no action required.
    Status,
    /// Asks for something from target gate(s).
    Request,
    /// Broadcast ecosystem-wide notice.
    Announce,
}

impl std::fmt::Display for ImpulseType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Frago => write!(f, "FRAGO"),
            Self::Status => write!(f, "STATUS"),
            Self::Request => write!(f, "REQUEST"),
            Self::Announce => write!(f, "ANNOUNCE"),
        }
    }
}

/// Priority levels for impulses.
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

/// Top-level impulse file structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpulseFile {
    /// Impulse metadata.
    pub impulse: ImpulseMeta,
    /// Origin information.
    pub from: ImpulseFrom,
    /// Target information.
    pub to: ImpulseTo,
    /// Message content.
    pub content: ImpulseContent,
    /// Operational metadata.
    pub meta: ImpulseOpMeta,
    /// Ed25519 signature (optional — present when bearDog is available).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<ImpulseSignature>,
    /// Acknowledgments from receiving gates.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub acks: Vec<ImpulseAck>,
}

/// The [impulse] table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpulseMeta {
    /// Unique impulse ID.
    pub id: String,
    /// Impulse type.
    #[serde(rename = "type")]
    pub impulse_type: ImpulseType,
    /// Priority level.
    pub priority: Priority,
    /// Wave number when created.
    pub wave: u32,
}

/// The [from] table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpulseFrom {
    /// Originating gate.
    pub gate: String,
    /// Team name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub team: String,
    /// Project path (e.g. "springs/hotSpring").
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub project: String,
    /// Commit ref — rootPulse DAG provenance.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    #[serde(rename = "ref")]
    pub git_ref: String,
}

/// The [to] table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpulseTo {
    /// Target gates (["*"] for broadcast).
    pub gates: Vec<String>,
    /// Target teams (informational).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub teams: Vec<String>,
}

/// The [content] table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpulseContent {
    /// Short subject line (max 80 chars).
    pub subject: String,
    /// Optional extended body.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body: String,
}

/// The [meta] table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpulseOpMeta {
    /// Creation timestamp.
    pub created: String,
    /// Expiration timestamp (optional).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub expires: String,
    /// Whether acknowledgment is required.
    #[serde(default)]
    pub ack_required: bool,
}

/// Optional Ed25519 signature (Phase 3 graduation — bearDog signing).
///
/// When bearDog is available, `impulse.post` signs the impulse payload
/// and stores the signature here. Gates can verify authenticity without
/// trusting git history alone.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpulseSignature {
    /// Signing algorithm.
    pub algorithm: String,
    /// Hex-encoded Ed25519 public key of the signing gate.
    pub public_key: String,
    /// Hex-encoded signature over the canonical impulse payload.
    pub value: String,
    /// When the signature was created.
    pub signed_at: String,
}

/// An acknowledgment entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpulseAck {
    /// Gate that acknowledged.
    pub gate: String,
    /// When it was acknowledged.
    pub timestamp: String,
    /// Optional note.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// Arguments for firing a new impulse (rootPulse ACTION).
pub struct PostArgs<'a> {
    /// Target gate names.
    pub to_gates: Vec<&'a str>,
    /// Impulse type (frago, status, request, announce).
    pub impulse_type: ImpulseType,
    /// Priority level.
    pub priority: Priority,
    /// Short subject line.
    pub subject: &'a str,
    /// Optional extended body text.
    pub body: &'a str,
    /// Project path (e.g. "springs/hotSpring").
    pub project: &'a str,
    /// Team name.
    pub team: &'a str,
}

/// Result of `potential.check` — membrane gradient health.
#[derive(Debug, Serialize)]
pub struct PotentialHealth {
    /// Total active impulses.
    pub total: usize,
    /// Impulses needing ack.
    pub needs_ack: usize,
    /// Expired but unarchived.
    pub expired: usize,
    /// Impulses per wave.
    pub by_wave: std::collections::BTreeMap<u32, usize>,
    /// Current wave.
    pub current_wave: u32,
}

fn impulses_dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join("infra/wateringHole/impulses")
}

fn active_dir(workspace_root: &Path) -> PathBuf {
    impulses_dir(workspace_root).join("active")
}

fn current_wave(workspace_root: &Path) -> u32 {
    let freshness = workspace_root.join("infra/wateringHole/freshness.toml");
    if let Ok(contents) = std::fs::read_to_string(&freshness) {
        if let Ok(val) = contents.parse::<toml::Table>() {
            if let Some(wave) = val.get("wave").and_then(|w| w.as_table()) {
                if let Some(id) = wave.get("id").and_then(toml::Value::as_integer) {
                    return id as u32;
                }
            }
        }
    }
    0
}

/// Resolve the HEAD commit SHA for a project repo (rootPulse provenance).
fn resolve_head_ref(workspace_root: &Path, project: &str) -> String {
    if project.is_empty() {
        return String::new();
    }
    crate::git_ops::resolve_head_ref(&workspace_root.join(project))
}

/// Fire a new impulse — rootPulse ACTION.
///
/// Creates TOML file in `impulses/active/`, auto-populates `[from].ref`
/// from project HEAD, commits, and pushes to all remotes.
pub async fn post(workspace_root: &Path, args: &PostArgs<'_>) -> Result<ImpulseFile> {
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
    git_add_commit_push(
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
///
/// Reads active impulses, optionally filtered to the local gate.
/// With `count_only`, returns just the count (for cascade-pull integration).
pub fn sense(workspace_root: &Path, all: bool, count_only: bool) -> Result<(Vec<(String, ImpulseFile)>, usize)> {
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
///
/// Reports gradient across the mesh: expired unacked, TTL violations,
/// volume per wave.
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
            if !path.extension().is_some_and(|e| e == "toml") {
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
    git_add_commit_push(
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
        if !path.extension().is_some_and(|e| e == "toml") {
            continue;
        }

        let contents = std::fs::read_to_string(&path).map_err(ShadowError::Io)?;
        let impulse: ImpulseFile = match parse_impulse_or_signal(&contents) {
            Ok(i) => i,
            Err(_) => continue,
        };

        let should_archive = is_expired(&impulse.meta.expires, &now) || is_fully_acked(&impulse);

        if should_archive {
            let fname = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            let dest = archive_dir.join(&fname);
            std::fs::rename(&path, &dest).map_err(ShadowError::Io)?;
            archived.push(fname);
        }
    }

    if !archived.is_empty() {
        let wh_dir = workspace_root.join("infra/wateringHole");
        let msg = format!("impulse archive: {} discharged → wave{wave}", archived.len());
        git_add_all_commit_push(&wh_dir, &msg).await?;
    }

    Ok(archived)
}

/// Parse a TOML file that may use either `[impulse]` or `[signal]` table name.
/// Backward compatible: reads old `[signal]` format and maps to `ImpulseFile`.
fn parse_impulse_or_signal(contents: &str) -> std::result::Result<ImpulseFile, toml::de::Error> {
    toml::from_str::<ImpulseFile>(contents).or_else(|_| {
        #[derive(Deserialize)]
        struct LegacySignalFile {
            signal: ImpulseMeta,
            from: ImpulseFrom,
            to: ImpulseTo,
            content: ImpulseContent,
            meta: ImpulseOpMeta,
            #[serde(default)]
            acks: Vec<ImpulseAck>,
        }
        let legacy: LegacySignalFile = toml::from_str(contents)?;
        Ok(ImpulseFile {
            impulse: legacy.signal,
            from: legacy.from,
            to: legacy.to,
            content: legacy.content,
            meta: legacy.meta,
            signature: None,
            acks: legacy.acks,
        })
    })
}

/// Attempt to sign an impulse via bearDog's UDS socket.
///
/// Discovers bearDog at the standard socket path. If unavailable,
/// returns None — unsigned impulses are valid (Phase 3 graceful
/// degradation). When bearDog is present, signs the impulse ID
/// with Ed25519 and returns the signature.
/// Attempt near-realtime impulse relay via songbird mesh.publish.
///
/// Discovers songbird at the standard socket path. If unavailable,
/// silently falls through — git push is the reliable baseline.
/// When songbird is present, publishes a lightweight notification
/// to the mesh topic `impulse/{gate}` so subscribing gates can
/// trigger an immediate cascade-pull.
fn try_relay_impulse(impulse: &ImpulseFile) {
    let xdg = std::env::var("XDG_RUNTIME_DIR").unwrap_or_default();
    let socket_candidates = [
        format!("{xdg}/biomeos/songbird-default.sock"),
        "/tmp/biomeos/songbird-default.sock".to_string(),
    ];

    let socket_path = match socket_candidates
        .iter()
        .find(|p| std::path::Path::new(p).exists())
    {
        Some(p) => p.clone(),
        None => return,
    };

    let notification = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "mesh.publish",
        "params": {
            "topic": format!("impulse/{}", impulse.from.gate),
            "payload": {
                "id": impulse.impulse.id,
                "type": impulse.impulse.impulse_type,
                "from": impulse.from.gate,
                "to": impulse.to.gates,
                "subject": impulse.content.subject,
                "priority": impulse.impulse.priority,
            }
        }
    });

    let Ok(request_str) = serde_json::to_string(&notification) else {
        return;
    };

    let mut child = match std::process::Command::new("socat")
        .args(["-", &format!("UNIX-CONNECT:{socket_path}")])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return,
    };

    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        let _ = writeln!(stdin, "{request_str}");
    }

    let _ = child.wait();
}

fn try_sign_impulse(_workspace_root: &Path, impulse_id: &str) -> Option<ImpulseSignature> {
    let xdg = std::env::var("XDG_RUNTIME_DIR").unwrap_or_default();
    let socket_candidates = [
        format!("{xdg}/biomeos/beardog-default.sock"),
        "/tmp/biomeos/beardog-default.sock".to_string(),
    ];

    let socket_path = socket_candidates
        .iter()
        .find(|p| std::path::Path::new(p).exists())?;

    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "crypto.sign_ed25519",
        "params": { "data": impulse_id }
    });
    let request_str = serde_json::to_string(&request).ok()?;

    let mut child = std::process::Command::new("socat")
        .args(["-", &format!("UNIX-CONNECT:{socket_path}")])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        let _ = writeln!(stdin, "{request_str}");
    }

    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }

    let response: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    let result = response.get("result")?;

    Some(ImpulseSignature {
        algorithm: "ed25519".to_string(),
        public_key: result.get("public_key")?.as_str()?.to_string(),
        value: result.get("signature")?.as_str()?.to_string(),
        signed_at: Local::now().format("%Y-%m-%dT%H:%M:%S%:z").to_string(),
    })
}

fn is_expired(expires: &str, now: &chrono::DateTime<Utc>) -> bool {
    if expires.is_empty() {
        return false;
    }
    chrono::DateTime::parse_from_str(expires, "%Y-%m-%dT%H:%M:%S%:z")
        .map(|exp| now > &exp)
        .unwrap_or(false)
}

fn is_fully_acked(impulse: &ImpulseFile) -> bool {
    if !impulse.meta.ack_required || impulse.to.gates.is_empty() {
        return false;
    }
    if impulse.to.gates.contains(&"*".to_string()) {
        return false;
    }
    impulse
        .to
        .gates
        .iter()
        .all(|g| impulse.acks.iter().any(|a| &a.gate == g))
}

fn find_impulse_by_id(active_dir: &Path, impulse_id: &str) -> Result<(PathBuf, ImpulseFile)> {
    let entries = std::fs::read_dir(active_dir).map_err(ShadowError::Io)?;
    for entry in entries {
        let entry = entry.map_err(ShadowError::Io)?;
        let path = entry.path();
        if !path.extension().is_some_and(|e| e == "toml") {
            continue;
        }
        let contents = std::fs::read_to_string(&path).map_err(ShadowError::Io)?;
        if let Ok(impulse) = parse_impulse_or_signal(&contents) {
            if impulse.impulse.id == impulse_id
                || path.file_stem().is_some_and(|s| s.to_string_lossy().contains(impulse_id))
            {
                return Ok((path, impulse));
            }
        }
    }
    Err(ShadowError::Parse(format!(
        "impulse not found: {impulse_id}"
    )))
}

async fn git_add_commit_push(repo_dir: &Path, file_path: &str, message: &str) -> Result<()> {
    crate::git_ops::add_commit_push(repo_dir, file_path, message).await
}

async fn git_add_all_commit_push(repo_dir: &Path, message: &str) -> Result<()> {
    crate::git_ops::add_all_commit_push(repo_dir, "impulses/", message).await
}
