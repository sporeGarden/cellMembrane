// SPDX-License-Identifier: AGPL-3.0-or-later

use std::path::Path;

use serde::Deserialize;

use super::types::*;
use crate::error::{Result, ShadowError};

/// Parse a TOML file that may use either `[impulse]` or legacy `[signal]` table name.
pub(crate) fn parse_impulse_or_signal(
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
pub(crate) fn find_impulse_by_id(
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
