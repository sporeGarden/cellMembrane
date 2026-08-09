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
///
/// G69 Phase 2: validates per-entry builder and blake3 when available, falling
/// back to file-level builder for legacy provenance files.
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
    let entry = provenance.as_ref().and_then(|p| p.entries.get(primal));

    let provenance_ok = entry.and_then(|e| e.commit.as_ref()).is_some();

    let builder_ok = entry.and_then(|e| e.builder.as_ref()).map_or_else(
        || {
            provenance
                .as_ref()
                .is_some_and(|p| !p.builder.as_ref().is_some_and(String::is_empty))
        },
        |b| !b.is_empty(),
    );

    let blake3_cross_ok = entry
        .and_then(|e| e.blake3.as_ref())
        .is_none_or(|prov_hash| {
            super::compute_blake3_file(&bin_path).is_ok_and(|actual| actual == *prov_hash)
        });

    if checksum_ok && provenance_ok && builder_ok && blake3_cross_ok {
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
    if !blake3_cross_ok {
        reasons.push("provenance blake3 does not match binary");
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
