// SPDX-License-Identifier: AGPL-3.0-or-later

//! Depot checksum verification — local (git) and WAN (HTTPS) sources.
//!
//! guideStone P3 (Self-Verifying): dual independent verification of binary integrity.

use super::health::resolve_plasmidbin_dir;

/// Verify local depot binaries against the git-tracked `checksums.toml`.
#[must_use]
pub fn verify_local_depot(arch: &str) -> (bool, String) {
    #[derive(serde::Deserialize)]
    struct ChecksumFile {
        #[serde(flatten)]
        targets:
            std::collections::BTreeMap<String, std::collections::BTreeMap<String, ChecksumEntry>>,
    }
    #[derive(serde::Deserialize)]
    struct ChecksumEntry {
        blake3: String,
        #[serde(rename = "size")]
        _size: u64,
    }

    let dest_root = resolve_plasmidbin_dir();
    let bin_dir = dest_root.join("primals").join(arch);

    let checksums_path = if dest_root.join("checksums.toml").exists() {
        dest_root.join("checksums.toml")
    } else if let Ok(workspace) = crate::temporal::resolve_workspace_root() {
        let ws_path = workspace.join("infra/plasmidBin/checksums.toml");
        if ws_path.exists() {
            ws_path
        } else {
            return (
                false,
                "checksums.toml not found in depot or workspace".into(),
            );
        }
    } else {
        return (false, "checksums.toml not found".into());
    };

    let Ok(content) = std::fs::read_to_string(&checksums_path) else {
        return (false, "cannot read checksums.toml".into());
    };

    let parsed: ChecksumFile = match toml::from_str(&content) {
        Ok(p) => p,
        Err(e) => return (false, format!("parse error: {e}")),
    };

    let Some(entries) = parsed.targets.get(arch) else {
        return (false, format!("no [{arch}] section in checksums.toml"));
    };

    let mut verified = 0u32;
    let mut failed = 0u32;
    let mut missing = 0u32;

    for (name, entry) in entries {
        let bin_path = bin_dir.join(name);
        if !bin_path.exists() {
            missing += 1;
            continue;
        }
        let hash = crate::plasmid::compute_blake3_file(&bin_path);
        if hash == entry.blake3 {
            verified += 1;
        } else {
            failed += 1;
        }
    }

    let ok = failed == 0 && missing == 0;
    (
        ok,
        format!("{verified} verified, {failed} hash mismatch, {missing} missing"),
    )
}

/// Cross-verify local binaries against the WAN-served checksums.toml.
///
/// Provides a second independent verification source. Even if the git-tracked
/// checksums are stale or compromised, the WAN endpoint serves the authoritative
/// hashes published by the VPS depot sync pipeline.
pub async fn verify_wan_checksums(arch: &str, dry_run: bool) -> super::bootstrap::BootstrapPhase {
    if dry_run {
        return super::bootstrap::BootstrapPhase {
            name: "checksum.wan".into(),
            ok: true,
            detail: "dry-run: would cross-verify against WAN depot checksums".into(),
        };
    }

    let wan_hashes = crate::plasmid::fetch_wan_checksums(arch).await;

    if wan_hashes.is_empty() {
        return super::bootstrap::BootstrapPhase {
            name: "checksum.wan".into(),
            ok: true,
            detail: "WAN checksums unavailable (offline or no http feature) — skipped".into(),
        };
    }

    let dest_root = resolve_plasmidbin_dir();
    let bin_dir = dest_root.join("primals").join(arch);

    let mut verified = 0u32;
    let mut mismatch = 0u32;
    let mut missing = 0u32;

    for (name, expected_hash) in &wan_hashes {
        let bin_path = bin_dir.join(name);
        if !bin_path.exists() {
            missing += 1;
            continue;
        }
        let actual = crate::plasmid::compute_blake3_file(&bin_path);
        if actual == *expected_hash {
            verified += 1;
        } else {
            mismatch += 1;
            eprintln!(
                "warn: WAN checksum mismatch for {name}: local={} wan={expected_hash}",
                &actual[..12]
            );
        }
    }

    let ok = mismatch == 0;
    super::bootstrap::BootstrapPhase {
        name: "checksum.wan".into(),
        ok,
        detail: format!(
            "{verified} verified, {mismatch} mismatch, {missing} missing (WAN cross-check)"
        ),
    }
}
