// SPDX-License-Identifier: AGPL-3.0-or-later

//! Post-parse schema validation for `EcosystemManifest`.
//!
//! Serde catches structural TOML issues at deserialization time, but
//! cross-field integrity (repo↔gate references, duplicate paths, count
//! mismatches) can only be checked after a successful parse.

use super::EcosystemManifest;
use std::collections::BTreeMap;

impl EcosystemManifest {
    /// Validate manifest integrity beyond what serde catches.
    ///
    /// Returns a list of validation issues. An empty list means the manifest
    /// is structurally sound. Checks performed:
    /// - `meta.version` is non-empty
    /// - `meta.total_repos` matches actual repo count
    /// - Every repo has non-empty `org` and `local_path`
    /// - No duplicate `local_path` values across repos
    /// - Gate repo references resolve to existing `[repos.*]` entries
    #[must_use]
    pub fn validate(&self) -> Vec<String> {
        let mut issues = Vec::new();

        if self.meta.version.is_empty() {
            issues.push("meta.version is empty".into());
        }

        let actual = u32::try_from(self.repos.len()).unwrap_or(u32::MAX);
        if self.meta.total_repos != 0 && self.meta.total_repos != actual {
            issues.push(format!(
                "meta.total_repos={} but manifest contains {} repo entries",
                self.meta.total_repos, actual,
            ));
        }

        let mut local_paths: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (name, entry) in &self.repos {
            if entry.org.is_empty() {
                issues.push(format!("repos.{name}: org is empty"));
            }
            if entry.local_path.is_empty() {
                issues.push(format!("repos.{name}: local_path is empty"));
            } else {
                local_paths
                    .entry(entry.local_path.as_str())
                    .or_default()
                    .push(name.as_str());
            }
        }
        for (path, names) in &local_paths {
            if names.len() > 1 {
                issues.push(format!(
                    "duplicate local_path \"{path}\" in repos: {}",
                    names.join(", "),
                ));
            }
        }

        for (gate_name, profile) in &self.gates {
            for repo_ref in &profile.repos {
                if !self.repos.contains_key(repo_ref.as_str()) {
                    issues.push(format!(
                        "gates.{gate_name}: references unknown repo \"{repo_ref}\""
                    ));
                }
            }
        }

        for (name, entry) in &self.repos {
            if let Some(pkg) = &entry.package {
                if pkg.is_empty() {
                    issues.push(format!(
                        "repos.{name}: package is empty (omit or set a value)"
                    ));
                }
            }
            if let Some(linker) = &entry.linker {
                if linker.is_empty() {
                    issues.push(format!(
                        "repos.{name}: linker is empty (omit or set a value)"
                    ));
                }
            }
            if entry.gpu && entry.category != cellmembrane_types::RepoCategory::Primal {
                issues.push(format!(
                    "repos.{name}: gpu=true but category is \"{}\", expected \"primal\"",
                    entry.category,
                ));
            }
        }

        issues
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_clean_manifest_returns_empty() {
        let toml_str = r#"
[meta]
version = "2.5.0"
total_repos = 2

[sync]
forgejo_ssh = "ssh://git@git.primals.eco:2222"

[repos.bearDog]
org = "ecoPrimals"
local_path = "primals/bearDog"

[repos.cellMembrane]
org = "sporeGarden"
local_path = "gardens/cellMembrane"

[gates.eastGate]
repos = ["bearDog", "cellMembrane"]
"#;
        let m: EcosystemManifest = toml::from_str(toml_str).unwrap();
        let issues = m.validate();
        assert!(issues.is_empty(), "expected no issues, got: {issues:?}");
    }

    #[test]
    fn validate_catches_empty_version() {
        let toml_str = r#"
[meta]
version = ""
[sync]
"#;
        let m: EcosystemManifest = toml::from_str(toml_str).unwrap();
        let issues = m.validate();
        assert!(issues.iter().any(|i| i.contains("version is empty")));
    }

    #[test]
    fn validate_catches_total_repos_mismatch() {
        let toml_str = r#"
[meta]
version = "1.0.0"
total_repos = 99

[sync]

[repos.bearDog]
org = "ecoPrimals"
local_path = "primals/bearDog"
"#;
        let m: EcosystemManifest = toml::from_str(toml_str).unwrap();
        let issues = m.validate();
        assert!(issues.iter().any(|i| i.contains("total_repos=99")));
    }

    #[test]
    fn validate_catches_gate_unknown_repo_ref() {
        let toml_str = r#"
[meta]
version = "1.0.0"
[sync]

[repos.bearDog]
org = "ecoPrimals"
local_path = "primals/bearDog"

[gates.eastGate]
repos = ["bearDog", "phantomRepo"]
"#;
        let m: EcosystemManifest = toml::from_str(toml_str).unwrap();
        let issues = m.validate();
        assert!(issues.iter().any(|i| i.contains("phantomRepo")));
        assert!(
            !issues.iter().any(|i| i.contains("bearDog")),
            "bearDog exists in repos and should not be flagged"
        );
    }

    #[test]
    fn validate_catches_empty_org() {
        let toml_str = r#"
[meta]
version = "1.0.0"
[sync]

[repos.broken]
org = ""
local_path = "primals/broken"
"#;
        let m: EcosystemManifest = toml::from_str(toml_str).unwrap();
        let issues = m.validate();
        assert!(
            issues
                .iter()
                .any(|i| i.contains("repos.broken: org is empty"))
        );
    }

    #[test]
    fn validate_catches_duplicate_local_path() {
        let toml_str = r#"
[meta]
version = "1.0.0"
[sync]

[repos.alpha]
org = "eco"
local_path = "primals/same"

[repos.beta]
org = "eco"
local_path = "primals/same"
"#;
        let m: EcosystemManifest = toml::from_str(toml_str).unwrap();
        let issues = m.validate();
        assert!(issues.iter().any(|i| i.contains("duplicate local_path")));
    }

    #[test]
    fn validate_catches_empty_package() {
        let toml_str = r#"
[meta]
version = "1.0.0"
[sync]

[repos.biomeOS]
org = "ecoPrimals"
local_path = "primals/biomeOS"
package = ""
"#;
        let m: EcosystemManifest = toml::from_str(toml_str).unwrap();
        let issues = m.validate();
        assert!(
            issues
                .iter()
                .any(|i| i.contains("repos.biomeOS: package is empty"))
        );
    }

    #[test]
    fn validate_catches_empty_linker() {
        let toml_str = r#"
[meta]
version = "1.0.0"
[sync]

[repos.nestGate]
org = "ecoPrimals"
local_path = "primals/nestGate"
linker = ""
"#;
        let m: EcosystemManifest = toml::from_str(toml_str).unwrap();
        let issues = m.validate();
        assert!(
            issues
                .iter()
                .any(|i| i.contains("repos.nestGate: linker is empty"))
        );
    }

    #[test]
    fn validate_catches_gpu_on_non_primal() {
        let toml_str = r#"
[meta]
version = "1.0.0"
[sync]

[repos.cellMembrane]
org = "sporeGarden"
local_path = "gardens/cellMembrane"
category = "garden"
gpu = true
"#;
        let m: EcosystemManifest = toml::from_str(toml_str).unwrap();
        let issues = m.validate();
        assert!(issues.iter().any(|i| i.contains("gpu=true but category")));
    }

    #[test]
    fn validate_accepts_valid_build_config() {
        let toml_str = r#"
[meta]
version = "1.0.0"
total_repos = 3
[sync]

[repos.biomeOS]
org = "ecoPrimals"
local_path = "primals/biomeOS"
category = "primal"
package = "biomeos-unibin"

[repos.nestGate]
org = "ecoPrimals"
local_path = "primals/nestGate"
category = "primal"
linker = "ld.lld"

[repos.barraCuda]
org = "ecoPrimals"
local_path = "primals/barraCuda"
category = "primal"
gpu = true
"#;
        let m: EcosystemManifest = toml::from_str(toml_str).unwrap();
        let issues = m.validate();
        assert!(issues.is_empty(), "expected no issues, got: {issues:?}");
    }

    #[test]
    fn validate_total_repos_zero_skips_count_check() {
        let toml_str = r#"
[meta]
version = "2.0.0"
total_repos = 0
[sync]

[repos.foo]
org = "eco"
local_path = "foo"
"#;
        let m: EcosystemManifest = toml::from_str(toml_str).unwrap();
        let issues = m.validate();
        assert!(!issues.iter().any(|i| i.contains("total_repos")));
    }
}
