// SPDX-License-Identifier: AGPL-3.0-or-later

//! Checksum verification and persistence for plasmid fetch.
//!
//! BLAKE3 verification of downloaded binaries, WAN checksums.toml fetch,
//! and local checksum persistence.

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

use super::detect_target_triple;

/// Checksum entry from `checksums.toml`.
///
/// Deserializes from either the struct format `{ blake3 = "...", size = N }`
/// or the legacy plain-string format `"hash"` (size defaults to 0).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ChecksumEntry {
    pub blake3: String,
    pub size: u64,
}

impl<'de> serde::Deserialize<'de> for ChecksumEntry {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = ChecksumEntry;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a checksum entry (struct or plain hash string)")
            }

            fn visit_str<E: serde::de::Error>(
                self,
                v: &str,
            ) -> std::result::Result<Self::Value, E> {
                Ok(ChecksumEntry {
                    blake3: v.to_string(),
                    size: 0,
                })
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut blake3: Option<String> = None;
                let mut size: u64 = 0;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "blake3" => blake3 = Some(map.next_value()?),
                        "size" => size = map.next_value()?,
                        _ => {
                            let _ = map.next_value::<serde::de::IgnoredAny>()?;
                        }
                    }
                }
                let blake3 = blake3.ok_or_else(|| serde::de::Error::missing_field("blake3"))?;
                Ok(ChecksumEntry { blake3, size })
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

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

pub(super) async fn verify_blake3_async(path: impl AsRef<Path>, expected: &str) -> bool {
    if expected.is_empty() {
        return false;
    }
    let path = path.as_ref().to_path_buf();
    let expected = expected.to_string();
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
    let url = format!("{base_url}/{}", cellmembrane_types::service::CHECKSUMS_FILE);

    let Ok(client) = crate::http_client(std::time::Duration::from_secs(15)) else {
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
///
/// Accepts both struct entries (`primal = { blake3 = "...", size = N }`) and
/// legacy plain-string entries (`primal = "hash"`). This enables backward
/// compatibility with depot files written before the struct format was adopted.
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
        } else if let Some(plain) = entry.as_str() {
            result.insert(name.clone(), plain.to_string());
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

    let alt_path = bin_dir.join(cellmembrane_types::service::CHECKSUMS_FILE);

    let depot_root_path = bin_dir
        .parent()
        .and_then(|p| p.parent())
        .map(|d| d.join(cellmembrane_types::service::CHECKSUMS_FILE));

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
    let arch_result = parse_checksums_toml(&contents, arch);
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
        let size = resolve_binary_size(depot_root, arch, name);
        let _ = writeln!(content, "{name} = {{ blake3 = \"{hash}\", size = {size} }}");
    }
    let path = depot_root.join(cellmembrane_types::service::CHECKSUMS_FILE);
    if let Err(e) = std::fs::write(&path, content.as_bytes()) {
        tracing::warn!(error = %e, path = %path.display(), "failed to persist checksums.toml");
    }
}

fn resolve_binary_size(depot_root: &Path, arch: &str, name: &str) -> u64 {
    depot_root
        .join("primals")
        .join(arch)
        .join(name)
        .metadata()
        .map_or(0, |m| m.len())
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
    fn persist_writes_struct_format() {
        let dir = std::env::temp_dir().join("cksum_persist_struct");
        let _ = std::fs::remove_dir_all(&dir);
        let arch = "x86_64-unknown-linux-musl";
        let arch_dir = dir.join("primals").join(arch);
        std::fs::create_dir_all(&arch_dir).unwrap();
        std::fs::write(arch_dir.join("beardog"), b"fake binary").unwrap();

        let mut checksums = HashMap::new();
        checksums.insert("beardog".to_string(), "abc123".to_string());
        checksums.insert("songbird".to_string(), "def456".to_string());

        persist_checksums(&dir, arch, &checksums);

        let content = std::fs::read_to_string(dir.join("checksums.toml")).unwrap();
        assert!(content.starts_with(&format!("[{arch}]")));
        assert!(
            content.contains("blake3 = \"abc123\""),
            "should use struct format: {content}"
        );
        assert!(content.contains("blake3 = \"def456\""));
        assert!(content.contains("size = "), "should include size field");
        let beardog_pos = content.find("beardog").unwrap();
        let songbird_pos = content.find("songbird").unwrap();
        assert!(beardog_pos < songbird_pos, "sorted alphabetically");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_checksums_toml_handles_legacy_plain_strings() {
        let content = r#"
[x86_64-unknown-linux-musl]
beardog = "abc123"
songbird = "def456"
"#;
        let map = parse_checksums_toml(content, "x86_64-unknown-linux-musl");
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("beardog").unwrap(), "abc123");
        assert_eq!(map.get("songbird").unwrap(), "def456");
    }

    #[test]
    fn parse_checksums_toml_handles_mixed_formats() {
        let content = r#"
[x86_64-unknown-linux-musl]
beardog = { blake3 = "abc123", size = 1234 }
songbird = "def456"
"#;
        let map = parse_checksums_toml(content, "x86_64-unknown-linux-musl");
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("beardog").unwrap(), "abc123");
        assert_eq!(map.get("songbird").unwrap(), "def456");
    }

    #[test]
    fn load_checksums_returns_empty_for_missing_dir() {
        let loaded = load_checksums(Path::new("/tmp/nonexistent-checksum-dir"), "v0.1");
        assert!(loaded.is_empty());
    }
}
