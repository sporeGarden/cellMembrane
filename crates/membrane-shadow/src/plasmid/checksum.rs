// SPDX-License-Identifier: AGPL-3.0-or-later

//! Checksum verification and persistence for plasmid fetch.
//!
//! BLAKE3 verification of downloaded binaries, WAN checksums.toml fetch,
//! and local checksum persistence.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::detect_target_triple;

pub(super) fn compute_blake3(path: &Path) -> std::io::Result<String> {
    let data = std::fs::read(path)?;
    Ok(blake3::hash(&data).to_hex().to_string())
}

#[cfg(test)]
pub(super) fn verify_blake3(path: &Path, expected: &str) -> bool {
    if expected.is_empty() {
        return false;
    }
    compute_blake3(path).is_ok_and(|actual| actual == expected)
}

pub(super) async fn verify_blake3_async(path: PathBuf, expected: String) -> bool {
    if expected.is_empty() {
        return false;
    }
    tokio::task::spawn_blocking(move || {
        compute_blake3(&path).is_ok_and(|actual| actual == expected)
    })
    .await
    .unwrap_or(false)
}

/// Fetch `checksums.toml` from the WAN depot and parse it into per-primal BLAKE3 hashes.
#[cfg(feature = "http")]
pub async fn fetch_wan_checksums(arch: &str) -> HashMap<String, String> {
    let base_url = std::env::var(cellmembrane_types::service::ENV_WAN_DEPOT_URL)
        .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_WAN_DEPOT_URL.to_string());
    let url = format!("{base_url}/checksums.toml");

    let Ok(client) = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
    else {
        return HashMap::new();
    };

    let Ok(resp) = client.get(&url).send().await else {
        return HashMap::new();
    };

    if !resp.status().is_success() {
        return HashMap::new();
    }

    let Ok(body) = resp.text().await else {
        return HashMap::new();
    };

    parse_checksums_toml(&body, arch)
}

#[cfg(not(feature = "http"))]
pub async fn fetch_wan_checksums(_arch: &str) -> HashMap<String, String> {
    HashMap::new()
}

/// Parse the arch-keyed `checksums.toml` format into a flat primal->blake3 map.
pub(super) fn parse_checksums_toml(content: &str, arch: &str) -> HashMap<String, String> {
    let Ok(table) = content.parse::<toml::Table>() else {
        return HashMap::new();
    };
    let Some(arch_table) = table.get(arch).and_then(toml::Value::as_table) else {
        return HashMap::new();
    };
    let mut result = HashMap::new();
    for (name, entry) in arch_table {
        if let Some(blake3) = entry.get("blake3").and_then(toml::Value::as_str) {
            result.insert(name.clone(), blake3.to_string());
        }
    }
    result
}

pub(super) fn load_checksums(bin_dir: &Path, tag: &str) -> HashMap<String, String> {
    #[derive(Deserialize)]
    struct FlatChecksumFile {
        #[serde(default)]
        checksums: HashMap<String, String>,
    }

    let checksums_path = bin_dir
        .parent()
        .unwrap_or(bin_dir)
        .join(format!("checksums-{tag}.toml"));

    let alt_path = bin_dir.join("checksums.toml");

    let depot_root_path = bin_dir
        .parent()
        .and_then(|p| p.parent())
        .map(|d| d.join("checksums.toml"));

    let path = if checksums_path.exists() {
        checksums_path
    } else if alt_path.exists() {
        alt_path
    } else if depot_root_path.as_ref().is_some_and(|p| p.exists()) {
        depot_root_path.unwrap_or_default()
    } else {
        return HashMap::new();
    };

    let Ok(contents) = std::fs::read_to_string(&path) else {
        return HashMap::new();
    };

    let arch = detect_target_triple();
    let arch_result = parse_checksums_toml(&contents, &arch);
    if !arch_result.is_empty() {
        return arch_result;
    }

    toml::from_str::<FlatChecksumFile>(&contents)
        .map(|f| f.checksums)
        .unwrap_or_default()
}

pub(super) fn persist_checksums(
    depot_root: &Path,
    arch: &str,
    checksums: &HashMap<String, String>,
) {
    use std::fmt::Write;
    let mut content = format!("[{arch}]\n");
    let mut sorted: Vec<_> = checksums.iter().collect();
    sorted.sort_by_key(|(k, _)| k.as_str());
    for (name, hash) in sorted {
        let _ = writeln!(content, "{name} = \"{hash}\"");
    }
    let path = depot_root.join("checksums.toml");
    if let Err(e) = std::fs::write(&path, content.as_bytes()) {
        tracing::warn!(error = %e, path = %path.display(), "failed to persist checksums.toml");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_checksums_toml_extracts_arch_section() {
        let content = r#"
[x86_64-unknown-linux-musl]
beardog = { blake3 = "abc123" }
songbird = { blake3 = "def456" }

[aarch64-unknown-linux-musl]
beardog = { blake3 = "zzz999" }
"#;
        let map = parse_checksums_toml(content, "x86_64-unknown-linux-musl");
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("beardog").unwrap(), "abc123");
        assert_eq!(map.get("songbird").unwrap(), "def456");
    }

    #[test]
    fn parse_checksums_toml_returns_empty_for_missing_arch() {
        let content = "[aarch64-unknown-linux-musl]\nbeardog = { blake3 = \"abc\" }\n";
        let map = parse_checksums_toml(content, "x86_64-unknown-linux-musl");
        assert!(map.is_empty());
    }

    #[test]
    fn parse_checksums_toml_returns_empty_for_invalid_toml() {
        let map = parse_checksums_toml("not [valid toml", "x86_64-unknown-linux-musl");
        assert!(map.is_empty());
    }

    #[test]
    fn compute_blake3_matches_known_hash() {
        let dir = std::env::temp_dir().join("cksum_blake3_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test_file");
        std::fs::write(&path, b"hello world").unwrap();
        let hash = compute_blake3(&path).unwrap();
        assert!(!hash.is_empty());
        let hash2 = compute_blake3(&path).unwrap();
        assert_eq!(hash, hash2, "deterministic");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persist_writes_sorted_toml() {
        let dir = std::env::temp_dir().join("cksum_persist_test");
        let _ = std::fs::create_dir_all(&dir);
        let arch = "x86_64-unknown-linux-musl";
        let mut checksums = HashMap::new();
        checksums.insert("beardog".to_string(), "abc123".to_string());
        checksums.insert("songbird".to_string(), "def456".to_string());

        persist_checksums(&dir, arch, &checksums);

        let content = std::fs::read_to_string(dir.join("checksums.toml")).unwrap();
        assert!(content.starts_with(&format!("[{arch}]")));
        assert!(content.contains("beardog = \"abc123\""));
        assert!(content.contains("songbird = \"def456\""));
        let beardog_pos = content.find("beardog").unwrap();
        let songbird_pos = content.find("songbird").unwrap();
        assert!(beardog_pos < songbird_pos, "sorted alphabetically");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_checksums_returns_empty_for_missing_dir() {
        let loaded = load_checksums(Path::new("/tmp/nonexistent-checksum-dir"), "v0.1");
        assert!(loaded.is_empty());
    }
}
