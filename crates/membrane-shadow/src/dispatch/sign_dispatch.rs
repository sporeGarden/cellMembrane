// SPDX-License-Identifier: AGPL-3.0-or-later

//! Sign domain dispatch — depot signing activation, verification, and status.

use crate::cli;
use crate::error::ShadowError;
use crate::ShadowOutcome;

pub(super) fn dispatch_sign(cmd: &str, args: &[&str]) -> crate::Result<ShadowOutcome> {
    match cmd {
        "sign.activate" => dispatch_activate(args),
        "sign.verify" => dispatch_verify(args),
        "sign.status" => dispatch_status(args),
        _ => Ok(ShadowOutcome::fail(format!("unknown sign command: {cmd}"))),
    }
}

/// `sign.activate` — Sign the depot's checksums.toml via bearDog.
///
/// Ensures checksums.toml exists (generating if needed), then requests
/// an ed25519 signature from bearDog's UDS endpoint and persists the
/// result in signatures.toml.
///
/// Usage:
///   membrane sign.activate                   # sign default depot
///   membrane sign.activate --depot /path     # sign specific depot
///   membrane sign.activate --dry-run         # preflight only (no signing)
fn dispatch_activate(args: &[&str]) -> crate::Result<ShadowOutcome> {
    let depot_dir =
        crate::plasmid::depot::resolve_depot(cli::extract_flag_value(args, "--depot"))?;
    let dry_run = args.contains(&"--dry-run");

    let checksums_path = depot_dir.join("checksums.toml");
    if !checksums_path.exists() {
        return Ok(ShadowOutcome::fail(format!(
            "checksums.toml not found at {} — run `depot.integrity` first",
            checksums_path.display()
        )));
    }

    let signer_socket = signer_socket_name();
    let socket_available = crate::impulse::discover_socket(&signer_socket).is_some();

    if dry_run {
        return Ok(ShadowOutcome::ok_with(
            format!(
                "sign.activate preflight: depot={}, checksums=OK, bearDog={}",
                depot_dir.display(),
                if socket_available { "REACHABLE" } else { "UNAVAILABLE" }
            ),
            serde_json::json!({
                "depot": depot_dir.display().to_string(),
                "checksums_exist": true,
                "beardog_reachable": socket_available,
                "dry_run": true,
            }),
        ));
    }

    if !socket_available {
        return Ok(ShadowOutcome::fail(format!(
            "bearDog signer socket not found (looked for {signer_socket}) — is bearDog running?"
        )));
    }

    if crate::plasmid::signing::sign_and_persist(&depot_dir) {
        let sigs = load_signatures_summary(&depot_dir);
        Ok(ShadowOutcome::ok_with(
            format!(
                "depot signed — {} signature(s) in signatures.toml",
                sigs.signatures.len()
            ),
            serde_json::json!({
                "depot": depot_dir.display().to_string(),
                "signed": true,
                "latest": format_latest_signature(&sigs),
                "total_signatures": sigs.signatures.len(),
            }),
        ))
    } else {
        Ok(ShadowOutcome::fail(
            "signing failed — check bearDog logs and socket connectivity",
        ))
    }
}

/// `sign.verify` — Verify depot signatures against checksums.toml.
///
/// Usage:
///   membrane sign.verify                             # verify-if-present (default)
///   membrane sign.verify --policy require-signed     # fail without valid sig
///   membrane sign.verify --policy integrity-only     # skip sig verification
///   membrane sign.verify --depot /path               # specific depot
fn dispatch_verify(args: &[&str]) -> crate::Result<ShadowOutcome> {
    let depot_dir =
        crate::plasmid::depot::resolve_depot(cli::extract_flag_value(args, "--depot"))?;

    let policy = match cli::extract_flag_value(args, "--policy") {
        Some("require-signed" | "require_signed") => {
            cellmembrane_types::DepotTrustPolicy::RequireSigned
        }
        Some("integrity-only" | "integrity_only") => {
            cellmembrane_types::DepotTrustPolicy::IntegrityOnly
        }
        Some("verify-if-present" | "verify_if_present") | None => {
            cellmembrane_types::DepotTrustPolicy::VerifyIfPresent
        }
        Some(other) => {
            return Err(ShadowError::Config(format!(
                "unknown trust policy: {other} (expected: require-signed, verify-if-present, integrity-only)"
            )));
        }
    };

    let sigs = load_signatures_summary(&depot_dir);
    let valid = crate::plasmid::signing::verify_depot_with_policy(&depot_dir, policy);

    let status = if valid { "PASS" } else { "FAIL" };
    let policy_name = match policy {
        cellmembrane_types::DepotTrustPolicy::RequireSigned => "require-signed",
        cellmembrane_types::DepotTrustPolicy::VerifyIfPresent => "verify-if-present",
        cellmembrane_types::DepotTrustPolicy::IntegrityOnly => "integrity-only",
    };

    Ok(ShadowOutcome {
        ok: valid,
        message: format!(
            "sign.verify: {status} (policy={policy_name}, signatures={})",
            sigs.signatures.len()
        ),
        data: Some(serde_json::json!({
            "valid": valid,
            "policy": policy_name,
            "depot": depot_dir.display().to_string(),
            "signatures_count": sigs.signatures.len(),
            "latest": format_latest_signature(&sigs),
        })),
    })
}

/// `sign.status` — Show current depot signing state.
///
/// Usage:
///   membrane sign.status                # default depot
///   membrane sign.status --depot /path  # specific depot
fn dispatch_status(args: &[&str]) -> crate::Result<ShadowOutcome> {
    let depot_dir =
        crate::plasmid::depot::resolve_depot(cli::extract_flag_value(args, "--depot"))?;

    let checksums_path = depot_dir.join("checksums.toml");
    let checksums_exist = checksums_path.exists();

    let sigs = load_signatures_summary(&depot_dir);

    let signer_socket = signer_socket_name();
    let socket_available = crate::impulse::discover_socket(&signer_socket).is_some();

    let sig_details: Vec<serde_json::Value> = sigs
        .signatures
        .iter()
        .map(|s| {
            serde_json::json!({
                "signer_gate": &s.signer_gate,
                "algorithm": format!("{:?}", s.algorithm),
                "signed_at": &s.signed_at,
                "public_key": truncate_hex(&s.public_key, 16),
                "checksums_blake3": truncate_hex(&s.checksums_blake3, 16),
            })
        })
        .collect();

    let msg = if sigs.signatures.is_empty() {
        format!(
            "depot: {} — no signatures (checksums={}, bearDog={})",
            depot_dir.display(),
            if checksums_exist { "present" } else { "MISSING" },
            if socket_available {
                "reachable"
            } else {
                "unavailable"
            }
        )
    } else {
        let latest = &sigs.signatures[0];
        format!(
            "depot: {} — {} signature(s), latest by {} at {}",
            depot_dir.display(),
            sigs.signatures.len(),
            latest.signer_gate,
            latest.signed_at
        )
    };

    Ok(ShadowOutcome::ok_with(
        msg,
        serde_json::json!({
            "depot": depot_dir.display().to_string(),
            "checksums_exist": checksums_exist,
            "beardog_reachable": socket_available,
            "signatures": sig_details,
        }),
    ))
}

fn signer_socket_name() -> String {
    let binary = cellmembrane_types::MembraneService::binary_for(
        cellmembrane_types::ServiceCapability::CryptoSigner,
    );
    format!("{binary}.sock")
}

fn load_signatures_summary(
    depot_dir: &std::path::Path,
) -> cellmembrane_types::signing::SignaturesFile {
    let sigs_path = depot_dir.join("signatures.toml");
    std::fs::read_to_string(&sigs_path)
        .ok()
        .and_then(|content| toml::from_str(&content).ok())
        .unwrap_or_default()
}

fn format_latest_signature(
    sigs: &cellmembrane_types::signing::SignaturesFile,
) -> serde_json::Value {
    sigs.latest().map_or(serde_json::Value::Null, |s| {
        serde_json::json!({
            "signer_gate": &s.signer_gate,
            "signed_at": &s.signed_at,
            "algorithm": format!("{:?}", s.algorithm),
            "public_key": truncate_hex(&s.public_key, 16),
        })
    })
}

fn truncate_hex(hex: &str, max_chars: usize) -> String {
    if hex.len() <= max_chars {
        hex.to_string()
    } else {
        format!("{}…", &hex[..max_chars])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_hex_short_string() {
        assert_eq!(truncate_hex("abcd", 16), "abcd");
    }

    #[test]
    fn truncate_hex_long_string() {
        let long = "a".repeat(64);
        let result = truncate_hex(&long, 16);
        assert_eq!(result.len(), 19); // 16 + "…" (3 bytes UTF-8)
        assert!(result.ends_with('…'));
    }

    #[test]
    fn signer_socket_name_contains_beardog() {
        let name = signer_socket_name();
        assert!(name.contains("beardog"), "expected beardog in {name}");
        assert!(
            std::path::Path::new(&name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("sock")),
            "expected .sock extension in {name}"
        );
    }

    #[test]
    fn format_latest_empty() {
        let sigs = cellmembrane_types::signing::SignaturesFile::default();
        assert_eq!(format_latest_signature(&sigs), serde_json::Value::Null);
    }

    #[test]
    fn dispatch_unknown_command() {
        let result = dispatch_sign("sign.nonexistent", &[]).unwrap();
        assert!(!result.ok);
        assert!(result.message.contains("unknown sign command"));
    }

    #[test]
    fn dispatch_verify_rejects_bad_policy() {
        let result = dispatch_verify(&["--policy", "garbage"]);
        assert!(result.is_err());
    }
}
