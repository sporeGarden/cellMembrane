// SPDX-License-Identifier: AGPL-3.0-or-later

//! Depot lineage validation — BLAKE3 checksum + provenance + builder authority.
//!
//! `PostPrimordial` primals are hard-blocked on lineage failure; other primals
//! get a warning but are allowed through.

/// Result of lineage validation for a primal binary.
#[derive(Debug, Clone)]
pub(crate) enum LineageResult {
    /// Checksum, provenance, and builder all verified.
    Verified,
    /// `PostPrimordial` primal with broken lineage — must not be installed.
    Blocked(String),
    /// Non-critical primal with incomplete lineage — warn but allow.
    Warned(String),
}

/// Validate depot lineage for a primal: BLAKE3 checksum, provenance commit,
/// and builder authority. `PostPrimordial` primals are hard-blocked on failure;
/// other primals get a warning.
pub(crate) fn validate_lineage(primal: &str, depot_dir: &std::path::Path) -> LineageResult {
    let is_critical = cellmembrane_types::service::is_post_primordial(primal);
    let arch = super::detect_target_triple();
    let bin_path = depot_dir.join("primals").join(arch).join(primal);

    let checksum_ok = if bin_path.exists() {
        verify_checksum_against_depot(primal, &bin_path, depot_dir, arch)
    } else {
        false
    };

    let provenance = super::depot::load_provenance(depot_dir);
    let provenance_ok = provenance
        .as_ref()
        .and_then(|p| p.entries.get(primal))
        .and_then(|e| e.commit.as_ref())
        .is_some();

    let builder_ok = provenance
        .as_ref()
        .is_some_and(|p| !p.builder.as_ref().is_some_and(String::is_empty));

    if checksum_ok && provenance_ok && builder_ok {
        return LineageResult::Verified;
    }

    let mut reasons = Vec::new();
    if !checksum_ok {
        reasons.push("BLAKE3 mismatch or missing");
    }
    if !provenance_ok {
        reasons.push("no provenance commit");
    }
    if !builder_ok {
        reasons.push("no builder identity");
    }
    let detail = format!("{primal}: lineage incomplete — {}", reasons.join(", "));

    if is_critical {
        LineageResult::Blocked(format!(
            "{primal} is postPrimordial — depot lineage validation FAILED ({detail})"
        ))
    } else {
        LineageResult::Warned(detail)
    }
}

fn verify_checksum_against_depot(
    primal: &str,
    bin_path: &std::path::Path,
    depot_dir: &std::path::Path,
    arch: &str,
) -> bool {
    let checksums_path = depot_dir.join(cellmembrane_types::service::CHECKSUMS_FILE);
    let Ok(content) = std::fs::read_to_string(&checksums_path) else {
        return false;
    };

    let map = super::checksum::parse_checksums_toml(&content, arch);
    let Some(expected) = map.get(primal) else {
        return false;
    };

    super::compute_blake3_file(bin_path).is_ok_and(|actual| actual == *expected)
}
