// SPDX-License-Identifier: AGPL-3.0-or-later

//! Auxiliary health probes — depot freshness, VCS parity, rootpulse ledger, TLS cert expiry.

use super::StatusProbe;
use std::path::Path;

const STALE_THRESHOLD_DAYS: u64 = 7;
const SECS_PER_DAY: u64 = 86_400;
const SECS_PER_HOUR: u64 = 3_600;
const CERT_WARNING_THRESHOLD_DAYS: i64 = 14;
const MAX_CERT_PROBE_DOMAINS: usize = 5;

pub(crate) fn probe_depot_freshness(arch: &str) -> super::super::ProbeResult {
    let dest_root = super::super::resolve_plasmidbin_dir();
    let bin_dir = dest_root.join("primals").join(arch);

    if !bin_dir.is_dir() {
        return super::super::ProbeResult::fail(format!(
            "depot dir missing: {}",
            bin_dir.display()
        ));
    }

    let gate = super::super::resolve_local_gate_identity();
    let composition_primals = crate::plasmid::resolve_gate_primals(&gate);
    let primals: Vec<&str> = composition_primals.iter().map(String::as_str).collect();
    let mut present = 0u32;
    let mut missing = 0u32;
    let mut oldest_age_secs: u64 = 0;

    let now = std::time::SystemTime::now();
    for primal in &primals {
        let path = bin_dir.join(primal);
        if path.is_file() {
            present += 1;
            if let Ok(meta) = std::fs::metadata(&path)
                && let Ok(modified) = meta.modified()
                && let Ok(age) = now.duration_since(modified)
            {
                oldest_age_secs = oldest_age_secs.max(age.as_secs());
            }
        } else {
            missing += 1;
        }
    }

    let total = present + missing;
    let age_days = oldest_age_secs / SECS_PER_DAY;
    let ok = missing == 0 && age_days < STALE_THRESHOLD_DAYS;

    let age_str = if oldest_age_secs > 0 {
        if age_days > 0 {
            format!(", oldest {age_days}d")
        } else {
            let hours = oldest_age_secs / SECS_PER_HOUR;
            format!(", oldest {hours}h")
        }
    } else {
        String::new()
    };

    super::super::ProbeResult {
        ok,
        detail: format!("{present}/{total} binaries present{age_str}"),
    }
}

/// VCS parity probe: check that origin and forgejo are at the same commit for
/// locally-cloned repos. Reports drift count — any drift is a WARN that auto-
/// reconciliation should resolve within the next cascade cycle.
pub(crate) async fn probe_vcs_parity() -> StatusProbe {
    let Ok(workspace) = crate::temporal::resolve_workspace_root() else {
        return StatusProbe {
            name: "vcs.parity".into(),
            ok: true,
            detail: "workspace not found (VPS/minimal)".into(),
        };
    };

    let local_paths: Vec<String> = crate::manifest::load_from_workspace_async(&workspace)
        .await
        .map_or_else(
            |_| {
                vec![
                    cellmembrane_types::service::INFRA_PLASMID_BIN.into(),
                    cellmembrane_types::service::INFRA_WATERING_HOLE.into(),
                ]
            },
            |m| m.repos.values().map(|r| r.local_path.clone()).collect(),
        );

    let mut drift_count = 0u32;
    let mut checked = 0u32;

    for repo_path in &local_paths {
        let repo_dir = workspace.join(repo_path);
        if !repo_dir.join(".git").exists() {
            continue;
        }
        let origin_head = git_rev_parse(&repo_dir, "origin/main").await;
        let forgejo_head = git_rev_parse(&repo_dir, "forgejo/main").await;
        if let (Some(o), Some(f)) = (origin_head, forgejo_head) {
            checked += 1;
            if o != f {
                drift_count += 1;
            }
        }
    }

    let ok = drift_count == 0;
    let detail = format!("{checked} repos checked, {drift_count} drifted");
    StatusProbe {
        name: "vcs.parity".into(),
        ok,
        detail,
    }
}

async fn git_rev_parse(repo_dir: &Path, refspec: &str) -> Option<String> {
    crate::git_ops::git_output_opt(repo_dir, &["rev-parse", refspec]).await
}

/// Probe rootpulse ledger state — checks if a session has been committed on this gate.
///
/// A missing session is a soft warning, not a failure — gates that haven't
/// run cascade with freshness yet are healthy but un-attested.
pub(crate) fn probe_rootpulse_ledger() -> StatusProbe {
    crate::temporal::post_sync::load_rootpulse_session().map_or_else(
        || StatusProbe {
            name: "rootpulse.ledger".into(),
            ok: true,
            detail: "no session yet — will populate on next cascade with freshness".into(),
        },
        |s| StatusProbe {
            name: "rootpulse.ledger".into(),
            ok: true,
            detail: format!("last session: {s}"),
        },
    )
}

/// Probe TLS cert expiry for publicly-served domains.
///
/// Only runs on gates that serve TLS (have a `caddy_tls` or `tls_terminator`
/// role in the manifest). Returns `None` if TLS is not locally relevant.
///
/// Uses `openssl s_client` to probe each domain's cert expiry. Any cert
/// with <14 days remaining triggers a probe failure (EXP-03 monitoring).
pub(crate) async fn probe_tls_cert_expiry() -> Option<StatusProbe> {
    let workspace = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_ECOPRIMALS_ROOT,
        cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT,
    );
    let manifest = match crate::manifest::load_from_workspace(std::path::Path::new(&workspace)) {
        Ok(m) => m,
        Err(e) => {
            tracing::debug!(error = %e, "TLS cert probe: manifest load failed");
            return None;
        }
    };
    let gate = super::super::resolve_local_gate_identity();

    let profile = manifest.gates.get(&gate)?;
    let is_tls_gate = profile
        .roles
        .iter()
        .any(cellmembrane_types::GateRole::is_tls);
    if !is_tls_gate {
        return None;
    }

    let domains: Vec<String> = profile
        .domains
        .clone()
        .unwrap_or_default()
        .into_iter()
        .take(MAX_CERT_PROBE_DOMAINS)
        .collect();

    if domains.is_empty() {
        return Some(StatusProbe {
            name: "tls.cert_expiry".into(),
            ok: true,
            detail: "no domains configured".into(),
        });
    }

    let mut results: Vec<String> = Vec::new();
    let mut any_expiring = false;

    for domain in &domains {
        let d = domain.clone();
        let days = tokio::task::spawn_blocking(move || check_cert_days(&d))
            .await
            .unwrap_or(-1);
        if days < 0 {
            results.push(format!("{domain}: EXPIRED/unreachable"));
            any_expiring = true;
        } else if days < CERT_WARNING_THRESHOLD_DAYS {
            results.push(format!("{domain}: {days}d remaining (WARNING)"));
            any_expiring = true;
        } else {
            results.push(format!("{domain}: {days}d remaining"));
        }
    }

    Some(StatusProbe {
        name: "tls.cert_expiry".into(),
        ok: !any_expiring,
        detail: results.join(", "),
    })
}

/// Check TLS cert days remaining for a domain via local openssl probe.
pub(super) fn check_cert_days(domain: &str) -> i64 {
    let https_port = cellmembrane_types::service::DEFAULT_HTTPS_PORT;
    let cmd = format!(
        "echo | openssl s_client -connect {domain}:{https_port} -servername {domain} 2>/dev/null \
         | openssl x509 -noout -enddate 2>/dev/null"
    );
    let Ok(result) = std::process::Command::new("sh").args(["-c", &cmd]).output() else {
        return -1;
    };
    if !result.status.success() {
        return -1;
    }

    let stdout = String::from_utf8_lossy(&result.stdout);
    let not_after = stdout
        .lines()
        .find(|l| l.starts_with("notAfter="))
        .map_or("", |l| l.trim_start_matches("notAfter=").trim());

    crate::caddy::parse_days_remaining(not_after)
}
