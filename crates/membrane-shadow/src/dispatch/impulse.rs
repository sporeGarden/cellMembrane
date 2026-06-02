// SPDX-License-Identifier: AGPL-3.0-or-later

//! Impulse + potential domain dispatch — rootPulse ACTION + quorumSignal SENSE.

use crate::cli;
use crate::{ShadowOutcome, impulse, temporal};

/// Dispatch impulse.post, impulse.ack, impulse.archive.
pub(super) async fn dispatch_impulse(cmd: &str, args: &[&str]) -> crate::Result<ShadowOutcome> {
    let root = temporal::resolve_workspace_root()?;
    match cmd {
        "impulse.post" => {
            let post_args = cli::parse_impulse_post_args(args)?;
            let imp = impulse::post(&root, &post_args).await?;
            Ok(ShadowOutcome::ok_with(
                format!(
                    "FIRED [{}] {} → {}: {}",
                    imp.impulse.impulse_type,
                    imp.from.gate,
                    imp.to.gates.join(","),
                    imp.content.subject,
                ),
                serde_json::to_value(&imp)?,
            ))
        }
        "impulse.ack" => {
            let impulse_id = cli::require_arg(args, 0, "impulse-id")?;
            let note = cli::extract_flag_value(args, "--note").unwrap_or("");
            let imp = impulse::ack(&root, impulse_id, note).await?;
            Ok(ShadowOutcome::ok(format!(
                "ACKED: {} (note: {})",
                imp.impulse.id,
                if note.is_empty() { "-" } else { note },
            )))
        }
        "impulse.archive" => {
            let archived = impulse::archive(&root).await?;
            if archived.is_empty() {
                Ok(ShadowOutcome::ok("No impulses to discharge.".to_string()))
            } else {
                Ok(ShadowOutcome::ok(format!(
                    "Discharged {} impulse(s): {}",
                    archived.len(),
                    archived.join(", "),
                )))
            }
        }
        _ => Ok(ShadowOutcome::fail(format!(
            "unknown impulse command: {cmd}"
        ))),
    }
}

/// Dispatch potential.sense, potential.check (quorumSignal SENSE).
pub(super) fn dispatch_potential(cmd: &str, args: &[&str]) -> crate::Result<ShadowOutcome> {
    let root = temporal::resolve_workspace_root()?;
    match cmd {
        "potential.sense" => {
            let all = args.contains(&"--all");
            let count_only = args.contains(&"--count");
            let (impulses, count) = impulse::sense(&root, all, count_only)?;
            if count_only {
                Ok(ShadowOutcome::ok(count.to_string()))
            } else if impulses.is_empty() {
                Ok(ShadowOutcome::ok(
                    "Membrane potential: resting (no pending impulses).".to_string(),
                ))
            } else {
                let lines: Vec<String> = impulses
                    .iter()
                    .map(|(_, s)| {
                        let ack_mark = if s.meta.ack_required && s.acks.is_empty() {
                            " [NEEDS ACK]"
                        } else if !s.acks.is_empty() {
                            " [ACKED]"
                        } else {
                            ""
                        };
                        format!(
                            "  [{}] {}/{}: {}{}",
                            s.impulse.impulse_type,
                            s.from.gate,
                            s.from.team,
                            s.content.subject,
                            ack_mark,
                        )
                    })
                    .collect();
                Ok(ShadowOutcome::ok_with(
                    format!("{count} active impulse(s)\n{}", lines.join("\n")),
                    serde_json::to_value(impulses.iter().map(|(_, s)| s).collect::<Vec<_>>())?,
                ))
            }
        }
        "potential.check" => {
            let health = impulse::check(&root)?;
            let wave_lines: Vec<String> = health
                .by_wave
                .iter()
                .map(|(w, c)| format!("  wave {w}: {c} impulse(s)"))
                .collect();
            let msg = format!(
                "Membrane potential gradient:\n\
                 Total active:    {}\n\
                 Needs ack:       {}\n\
                 Expired:         {}\n\
                 Current wave:    {}\n\
                 {}",
                health.total,
                health.needs_ack,
                health.expired,
                health.current_wave,
                if wave_lines.is_empty() {
                    String::new()
                } else {
                    format!("Volume:\n{}", wave_lines.join("\n"))
                },
            );
            Ok(ShadowOutcome::ok_with(msg, serde_json::to_value(&health)?))
        }
        _ => Ok(ShadowOutcome::fail(format!(
            "unknown potential command: {cmd}"
        ))),
    }
}
