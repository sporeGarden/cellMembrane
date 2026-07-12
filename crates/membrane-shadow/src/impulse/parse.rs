// SPDX-License-Identifier: AGPL-3.0-or-later

use std::path::Path;

use serde::Deserialize;

use super::types::{
    ImpulseAck, ImpulseContent, ImpulseFile, ImpulseFrom, ImpulseMeta, ImpulseOpMeta, ImpulseTo,
};
use crate::error::{Result, ShadowError};

/// Parse a TOML file that may use either `[impulse]` or legacy `[signal]` table name.
pub(super) fn parse_impulse_or_signal(
    contents: &str,
) -> std::result::Result<ImpulseFile, toml::de::Error> {
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

/// Find an impulse by ID (exact match or filename stem contains).
pub(super) fn find_impulse_by_id(
    active_dir: &Path,
    impulse_id: &str,
) -> Result<(std::path::PathBuf, ImpulseFile)> {
    let entries = std::fs::read_dir(active_dir).map_err(ShadowError::Io)?;
    for entry in entries {
        let entry = entry.map_err(ShadowError::Io)?;
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "toml") {
            continue;
        }
        let contents = std::fs::read_to_string(&path).map_err(ShadowError::Io)?;
        if let Ok(impulse) = parse_impulse_or_signal(&contents) {
            if impulse.impulse.id == impulse_id
                || path
                    .file_stem()
                    .is_some_and(|s| s.to_string_lossy().contains(impulse_id))
            {
                return Ok((path, impulse));
            }
        }
    }
    Err(ShadowError::Parse(format!(
        "impulse not found: {impulse_id}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_IMPULSE_TOML: &str = r#"
[impulse]
id = "IMP-115-001"
type = "frago"
priority = "routine"
wave = 115

[from]
gate = "primalSpring"

[to]
gates = ["sporeGate"]

[content]
subject = "test impulse"
body = "this is a test"

[meta]
created = "2026-06-16T12:00:00Z"
"#;

    const LEGACY_SIGNAL_TOML: &str = r#"
[signal]
id = "SIG-114-002"
type = "status"
priority = "priority"
wave = 114

[from]
gate = "eastGate"

[to]
gates = ["sporeGate"]

[content]
subject = "legacy format"
body = "signal table name"

[meta]
created = "2026-06-16T13:00:00Z"
"#;

    #[test]
    fn parse_valid_impulse() {
        let result = parse_impulse_or_signal(VALID_IMPULSE_TOML);
        let impulse = result.expect("should parse impulse format");
        assert_eq!(impulse.impulse.id, "IMP-115-001");
        assert_eq!(impulse.from.gate, "primalSpring");
        assert_eq!(impulse.to.gates, vec!["sporeGate"]);
        assert_eq!(impulse.content.subject, "test impulse");
        assert_eq!(impulse.impulse.wave, 115);
    }

    #[test]
    fn parse_legacy_signal_format() {
        let result = parse_impulse_or_signal(LEGACY_SIGNAL_TOML);
        let impulse = result.expect("should parse legacy signal format");
        assert_eq!(impulse.impulse.id, "SIG-114-002");
        assert_eq!(impulse.from.gate, "eastGate");
        assert_eq!(impulse.content.subject, "legacy format");
        assert_eq!(impulse.impulse.wave, 114);
    }

    #[test]
    fn parse_invalid_toml_returns_error() {
        let result = parse_impulse_or_signal("not valid toml {{{");
        assert!(result.is_err());
    }

    #[test]
    fn parse_empty_string_returns_error() {
        let result = parse_impulse_or_signal("");
        assert!(result.is_err());
    }

    #[test]
    fn parse_missing_required_fields() {
        let incomplete = r#"
[impulse]
id = "IMP-003"
"#;
        let result = parse_impulse_or_signal(incomplete);
        assert!(result.is_err());
    }

    #[test]
    fn parsed_impulse_has_empty_acks_by_default() {
        let impulse = parse_impulse_or_signal(VALID_IMPULSE_TOML).unwrap();
        assert!(impulse.acks.is_empty());
    }
}
