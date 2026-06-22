// SPDX-License-Identifier: AGPL-3.0-or-later

//! CLI argument parsing and helper types for the membrane dispatcher.
//!
//! Extracted from `main.rs` — these are pure parsing functions with no
//! side effects, making them testable and reusable by other entry points.

use crate::{ShadowError, ShadowOutcome, context, impulse};

/// Extract a positional argument or return a parse error.
pub(crate) fn require_arg<'a>(args: &[&'a str], idx: usize, name: &str) -> crate::Result<&'a str> {
    args.get(idx)
        .copied()
        .ok_or_else(|| ShadowError::Config(format!("{name} required")))
}

/// Split `"org/name"` into `("org", "name")`.
pub(crate) fn split_repo_path(path: &str) -> crate::Result<(&str, &str)> {
    path.split_once('/')
        .ok_or_else(|| ShadowError::Config(format!("expected org/name format, got: {path}")))
}

/// Extract `--flag value` from a flat args slice.
#[must_use]
pub(crate) fn extract_flag_value<'a>(args: &[&'a str], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| *a == flag)
        .and_then(|i| args.get(i + 1).copied())
}

/// Parse `impulse.post` CLI arguments into typed `PostArgs`.
pub(crate) fn parse_impulse_post_args<'a>(
    args: &[&'a str],
) -> crate::Result<impulse::PostArgs<'a>> {
    let to_str = extract_flag_value(args, "--to")
        .ok_or_else(|| ShadowError::Config("--to <gate> required".into()))?;
    let subject = extract_flag_value(args, "--subject")
        .ok_or_else(|| ShadowError::Config("--subject required".into()))?;

    let type_str = extract_flag_value(args, "--type").unwrap_or("status");
    let impulse_type = match type_str {
        "frago" => impulse::ImpulseType::Frago,
        "status" => impulse::ImpulseType::Status,
        "request" => impulse::ImpulseType::Request,
        "announce" => impulse::ImpulseType::Announce,
        _ => {
            return Err(ShadowError::Config(format!(
                "unknown impulse type: {type_str} (expected: frago|status|request|announce)"
            )));
        }
    };

    let priority_str = extract_flag_value(args, "--priority").unwrap_or("routine");
    let priority = match priority_str {
        "priority" => impulse::Priority::Priority,
        "flash" => impulse::Priority::Flash,
        _ => impulse::Priority::Routine,
    };

    let to_gates: Vec<&str> = to_str.split(',').collect();

    Ok(impulse::PostArgs {
        to_gates,
        impulse_type,
        priority,
        subject,
        body: extract_flag_value(args, "--body").unwrap_or(""),
        project: extract_flag_value(args, "--project").unwrap_or(""),
        team: extract_flag_value(args, "--team").unwrap_or(""),
    })
}

/// Parse `context.weave` CLI arguments into typed `WeaveArgs`.
pub(crate) fn parse_context_weave_args<'a>(
    args: &[&'a str],
) -> crate::Result<context::WeaveArgs<'a>> {
    let project = extract_flag_value(args, "--project")
        .ok_or_else(|| ShadowError::Config("--project <path> required".into()))?;
    let summary = extract_flag_value(args, "--summary")
        .ok_or_else(|| ShadowError::Config("--summary required".into()))?;

    let status_str = extract_flag_value(args, "--status").unwrap_or("active");
    let status = match status_str {
        "active" => context::FocusStatus::Active,
        "paused" => context::FocusStatus::Paused,
        "blocked" => context::FocusStatus::Blocked,
        "complete" => context::FocusStatus::Complete,
        _ => {
            return Err(ShadowError::Config(format!(
                "unknown status: {status_str} (expected: active|paused|blocked|complete)"
            )));
        }
    };

    let ttl_str = extract_flag_value(args, "--ttl").unwrap_or("48");
    let ttl_hours: u32 = ttl_str.parse().unwrap_or(48);

    Ok(context::WeaveArgs {
        project,
        summary,
        status,
        breadcrumbs: extract_flag_value(args, "--breadcrumbs").unwrap_or(""),
        next: extract_flag_value(args, "--next").unwrap_or(""),
        blockers: extract_flag_value(args, "--blockers").unwrap_or(""),
        notes: extract_flag_value(args, "--notes").unwrap_or(""),
        ttl_hours,
    })
}

/// Slugify a project path for display (e.g. `gardens/cellMembrane` → `gardens-cellmembrane`).
#[must_use]
pub(crate) fn context_slug(project: &str) -> String {
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

/// Extension trait for composing human-readable output.
pub(crate) trait TapMessage {
    /// Transform the message field while preserving the rest.
    #[must_use]
    fn tap_message(self, f: impl FnOnce(&str) -> String) -> Self;
}

impl TapMessage for ShadowOutcome {
    fn tap_message(mut self, f: impl FnOnce(&str) -> String) -> Self {
        self.message = f(&self.message);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_flag_finds_value() {
        let args = ["--to", "ironGate", "--type", "frago"];
        assert_eq!(extract_flag_value(&args, "--to"), Some("ironGate"));
        assert_eq!(extract_flag_value(&args, "--type"), Some("frago"));
        assert_eq!(extract_flag_value(&args, "--missing"), None);
    }

    #[test]
    fn split_repo_path_works() {
        let (org, name) = split_repo_path("ecoPrimals/bearDog").unwrap();
        assert_eq!(org, "ecoPrimals");
        assert_eq!(name, "bearDog");
    }

    #[test]
    fn split_repo_path_rejects_bare_name() {
        assert!(split_repo_path("bearDog").is_err());
    }

    #[test]
    fn context_slug_normalizes() {
        assert_eq!(context_slug("gardens/cellMembrane"), "gardens-cellmembrane");
        assert_eq!(context_slug("infra/wateringHole"), "infra-wateringhole");
    }

    #[test]
    fn parse_impulse_post_minimum_args() {
        let args = ["--to", "ironGate", "--subject", "test impulse"];
        let post = parse_impulse_post_args(&args).unwrap();
        assert_eq!(post.to_gates, vec!["ironGate"]);
        assert_eq!(post.subject, "test impulse");
    }

    #[test]
    fn parse_impulse_post_rejects_missing_to() {
        let args = ["--subject", "no target"];
        assert!(parse_impulse_post_args(&args).is_err());
    }
}
