// SPDX-License-Identifier: AGPL-3.0-or-later

//! Depot checksum verification — local (git) and WAN (HTTPS) sources.
//!
//! guideStone P3 (Self-Verifying): dual independent verification of binary integrity.

use super::resolve_plasmidbin_dir;
use tracing::warn;

/// Verify local depot binaries against the git-tracked `checksums.toml`.
#[must_use]
pub(crate) fn verify_local_depot(arch: &str) -> super::ProbeResult {
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

    let checksums_path = if dest_root.join(cellmembrane_types::service::CHECKSUMS_FILE).exists() {
        dest_root.join(cellmembrane_types::service::CHECKSUMS_FILE)
    } else if let Ok(workspace) = crate::temporal::resolve_workspace_root() {
        let ws_path = workspace
            .join(cellmembrane_types::service::INFRA_PLASMID_BIN)
            .join(cellmembrane_types::service::CHECKSUMS_FILE);
        if ws_path.exists() {
            ws_path
        } else {
            return super::ProbeResult::fail(
                "checksums.toml not found in depot or workspace",
            );
        }
    } else {
        return super::ProbeResult::fail("checksums.toml not found");
    };

    let Ok(content) = std::fs::read_to_string(&checksums_path) else {
        return super::ProbeResult::fail("cannot read checksums.toml");
    };

    let parsed = match toml::from_str::<ChecksumFile>(&content) {
        Ok(p) => p,
        Err(e) => return super::ProbeResult::fail(format!("checksums.toml parse error: {e}")),
    };

    let Some(entries) = parsed.targets.get(arch) else {
        return super::ProbeResult::fail(format!("no [{arch}] section in checksums.toml"));
    };

    let mut verified = 0u32;
    let mut failed = 0u32;
    let mut missing = 0u32;
    let nucleus_set: std::collections::HashSet<&str> =
        crate::plasmid::nucleus_primals().into_iter().collect();

    for (name, entry) in entries {
        let bin_path = bin_dir.join(name);
        if !bin_path.exists() {
            if nucleus_set.contains(name.as_str()) {
                missing += 1;
            }
            continue;
        }
        let Ok(hash) = crate::plasmid::compute_blake3_file(&bin_path) else {
            failed += 1;
            continue;
        };
        if hash == entry.blake3 {
            verified += 1;
        } else {
            failed += 1;
        }
    }

    let ok = failed == 0 && missing == 0;
    super::ProbeResult {
        ok,
        detail: format!("{verified} verified, {failed} hash mismatch, {missing} missing"),
    }
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
        let actual = match crate::plasmid::compute_blake3_file_async(&bin_path).await {
            Ok(h) => h,
            Err(e) => {
                mismatch += 1;
                warn!(name, error = %e, "WAN checksum verify: cannot hash local binary");
                continue;
            }
        };
        if actual == *expected_hash {
            verified += 1;
        } else {
            mismatch += 1;
            warn!(
                name,
                local = &actual[..12],
                wan = expected_hash,
                "WAN checksum mismatch"
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Mirror of the `ChecksumFile` / `ChecksumEntry` structs inside
    /// `verify_local_depot` — used to validate our TOML format expectations.
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

    #[test]
    fn checksums_toml_parses_correctly() {
        let toml_str = r#"
[x86_64-unknown-linux-musl]
beardog = { blake3 = "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890", size = 4096 }
songbird = { blake3 = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef", size = 8192 }

[aarch64-unknown-linux-musl]
beardog = { blake3 = "fedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321", size = 4100 }
"#;
        let parsed: ChecksumFile = toml::from_str(toml_str).unwrap();
        let x86 = parsed.targets.get("x86_64-unknown-linux-musl").unwrap();
        assert_eq!(x86.len(), 2);
        assert!(x86.contains_key("beardog"));
        assert!(x86.contains_key("songbird"));
        assert_eq!(x86["beardog"].blake3.len(), 64);

        let aarch64 = parsed.targets.get("aarch64-unknown-linux-musl").unwrap();
        assert_eq!(aarch64.len(), 1);
    }

    #[test]
    fn checksums_toml_missing_arch_section() {
        let toml_str = r#"
[x86_64-unknown-linux-musl]
beardog = { blake3 = "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890", size = 4096 }
"#;
        let parsed: ChecksumFile = toml::from_str(toml_str).unwrap();
        assert!(!parsed.targets.contains_key("riscv64"));
    }

    #[test]
    fn checksums_toml_rejects_invalid() {
        let bad = "this is not valid toml [[[";
        assert!(toml::from_str::<ChecksumFile>(bad).is_err());
    }

    #[test]
    fn checksums_toml_blake3_hash_roundtrip() {
        let content = b"hello membrane";
        let hash = blake3::hash(content).to_hex().to_string();
        assert_eq!(hash.len(), 64, "blake3 hex hash should be 64 chars");

        let toml_str = format!(
            "[x86_64-unknown-linux-musl]\nbeardog = {{ blake3 = \"{hash}\", size = {} }}\n",
            content.len()
        );
        let parsed: ChecksumFile = toml::from_str(&toml_str).unwrap();
        let entry = &parsed.targets["x86_64-unknown-linux-musl"]["beardog"];
        assert_eq!(entry.blake3, hash, "TOML round-trip should preserve hash");
    }

    #[test]
    fn checksums_toml_multiple_archs_and_binaries() {
        let toml_str = r#"
[x86_64-unknown-linux-musl]
beardog = { blake3 = "aaaa000000000000000000000000000000000000000000000000000000000000", size = 100 }
songbird = { blake3 = "bbbb000000000000000000000000000000000000000000000000000000000000", size = 200 }
nestgate = { blake3 = "cccc000000000000000000000000000000000000000000000000000000000000", size = 300 }

[aarch64-unknown-linux-musl]
beardog = { blake3 = "dddd000000000000000000000000000000000000000000000000000000000000", size = 110 }
"#;
        let parsed: ChecksumFile = toml::from_str(toml_str).unwrap();
        assert_eq!(parsed.targets.len(), 2);
        assert_eq!(parsed.targets["x86_64-unknown-linux-musl"].len(), 3);
        assert_eq!(parsed.targets["aarch64-unknown-linux-musl"].len(), 1);
    }

    #[tokio::test]
    async fn verify_wan_checksums_dry_run_returns_ok() {
        let phase = verify_wan_checksums("x86_64-unknown-linux-musl", true).await;
        assert!(phase.ok);
        assert_eq!(phase.name, "checksum.wan");
        assert!(phase.detail.contains("dry-run"));
    }
}
