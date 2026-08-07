// SPDX-License-Identifier: AGPL-3.0-or-later

//! Plasmid dispatch commands — `pipeline`, `trigger`, and `status`.
//!
//! These are the high-level user-facing operations that compose lower-level
//! harvest, sandbox, and refresh primitives.

use super::harvest::{self, HarvestArgs, HarvestResult, HarvestStatus};
use super::sandbox;
use super::{RefreshArgs, refresh};

/// `plasmid.pipeline` — Full zero-touch harvest → refresh cycle.
///
/// Detects upstream changes, rebuilds, checksums, pushes to VPS,
/// and reports aggregated outcome. This is the end-to-end command
/// that replaces manual harvest+refresh cycles.
pub async fn pipeline(
    config: &crate::ShadowConfig,
    primal: Option<&str>,
    dry_run: bool,
) -> crate::error::Result<crate::ShadowOutcome> {
    let harvest_args = HarvestArgs {
        primal: primal.map(Into::into),
        force: false,
        dry_run,
        depot_dir: None,
        target: None,
        local: false,
        push: false,
        with_restart: false,
    };

    let harvest_outcome = super::harvest(&harvest_args).await?;

    if dry_run {
        return Ok(harvest_outcome);
    }

    let results: Vec<HarvestResult> = harvest_outcome
        .data
        .as_ref()
        .and_then(|d| serde_json::from_value(d.clone()).ok())
        .unwrap_or_default();

    let built_any = results
        .iter()
        .any(|r| matches!(r.status, HarvestStatus::Built));

    if !built_any {
        return Ok(crate::ShadowOutcome {
            ok: harvest_outcome.ok,
            message: format!("{} — no new binaries to push", harvest_outcome.message),
            data: harvest_outcome.data,
        });
    }

    let arch = super::detect_target_triple();
    let depot_dir = super::depot::resolve_depot(None)?;
    let bin_dir = depot_dir.join("primals").join(arch);

    for entry in results
        .iter()
        .filter(|r| matches!(r.status, HarvestStatus::Built))
    {
        let binary_path = bin_dir.join(&entry.binary);
        if !binary_path.exists() {
            continue;
        }

        let sandbox_args = sandbox::SandboxArgs {
            primal: entry.binary.clone(),
            commit: entry.detail.clone(),
            binary_path,
            timeout_secs: None,
        };

        match sandbox::validate_with_deps(&sandbox_args).await {
            Ok(result) if !result.health_ok => {
                return Ok(crate::ShadowOutcome {
                    ok: false,
                    message: format!(
                        "{} | sandbox FAIL for {} — {} ({}ms). Refresh aborted.",
                        harvest_outcome.message, entry.binary, result.detail, result.elapsed_ms
                    ),
                    data: Some(serde_json::to_value(&result).unwrap_or_default()),
                });
            }
            Err(e) => {
                tracing::warn!(primal = %entry.binary, error = %e, "sandbox infra error");
                return Ok(crate::ShadowOutcome {
                    ok: false,
                    message: format!(
                        "{} | sandbox INFRA ERROR for {} — {e}. Refresh aborted.",
                        harvest_outcome.message, entry.binary
                    ),
                    data: None,
                });
            }
            Ok(_) => {}
        }
    }

    let depot_source = Some(bin_dir.to_string_lossy().into_owned());

    let refresh_args = RefreshArgs {
        primal: primal.map(Into::into),
        dry_run: false,
        source_dir: depot_source,
    };

    let refresh_outcome = refresh(config, &refresh_args).await?;

    Ok(crate::ShadowOutcome {
        ok: refresh_outcome.ok,
        message: format!(
            "{} | sandbox: PASS | {}",
            harvest_outcome.message, refresh_outcome.message
        ),
        data: refresh_outcome.data,
    })
}

/// `plasmid.trigger` — Remotely trigger the VPS pipeline via SSH.
///
/// Kicks `systemctl start plasmid-pipeline.service` on the VPS, causing
/// an immediate harvest→refresh cycle there. Useful when an operator wants
/// the VPS to converge without running the full pipeline locally.
pub async fn trigger(config: &crate::ShadowConfig) -> crate::error::Result<crate::ShadowOutcome> {
    let cmd = "systemctl start plasmid-pipeline.service 2>&1; \
               sleep 1; \
               systemctl is-active plasmid-pipeline.service 2>&1 || \
               journalctl -u plasmid-pipeline.service --no-pager -n 3 2>&1";

    let (output, code) = crate::ssh::exec_raw(config, cmd).await?;

    if code == 0 || output.contains("activating") || output.contains("active") {
        Ok(crate::ShadowOutcome::ok(format!(
            "trigger: plasmid-pipeline.service started on {}\n{output}",
            config.ssh_host
        )))
    } else {
        Ok(crate::ShadowOutcome {
            ok: false,
            message: format!(
                "trigger: failed to start on {} (exit {code})\n{output}",
                config.ssh_host
            ),
            data: None,
        })
    }
}

/// Maximum age (in days) before `plasmid.status` flags the depot as stale.
const DEPOT_STALE_THRESHOLD_DAYS: u64 = 7;

/// Parse an ISO-8601 `generated` timestamp into days since that date.
///
/// Returns `None` if the timestamp is unparseable or missing.
fn parse_staleness_days(generated: &str) -> Option<u64> {
    let date_part = generated.split('T').next()?;
    let parts: Vec<&str> = date_part.split('-').collect();
    if parts.len() < 3 {
        return None;
    }
    let year: i64 = parts[0].parse().ok()?;
    let month: i64 = parts[1].parse().ok()?;
    let day: i64 = parts[2].parse().ok()?;

    let now = crate::utc_now_iso8601();
    let now_date = now.split('T').next()?;
    let now_parts: Vec<&str> = now_date.split('-').collect();
    if now_parts.len() < 3 {
        return None;
    }
    let now_year: i64 = now_parts[0].parse().ok()?;
    let now_month: i64 = now_parts[1].parse().ok()?;
    let now_day: i64 = now_parts[2].parse().ok()?;

    let gen_days = year * 365 + month * 30 + day;
    let now_days = now_year * 365 + now_month * 30 + now_day;
    let diff = now_days.saturating_sub(gen_days);
    u64::try_from(diff).ok()
}

/// `plasmid.status` — Report depot freshness and upstream drift.
///
/// Reads provenance.toml for last build timestamp, then checks each
/// primal's HEAD against the recorded commit to identify drift.
/// Warns when the depot is older than `DEPOT_STALE_THRESHOLD_DAYS`.
pub async fn status() -> crate::error::Result<crate::ShadowOutcome> {
    let depot_dir = harvest::resolve_depot(None)?;
    let sources = harvest::load_sources(&depot_dir)?;
    let provenance = harvest::load_provenance(&depot_dir);

    let generated = provenance
        .as_ref()
        .and_then(|p| p.generated.clone())
        .unwrap_or_else(|| cellmembrane_types::service::UNKNOWN_LABEL.into());

    let target = provenance
        .as_ref()
        .and_then(|p| p.target.clone())
        .unwrap_or_else(|| cellmembrane_types::service::UNKNOWN_LABEL.into());

    let registry_primals = super::nucleus_primals();
    let total = registry_primals.len();

    let mut drifted: Vec<String> = Vec::new();
    let mut current = 0usize;

    for &primal in &registry_primals {
        if let Some(source) = sources.get(primal) {
            let changed = super::drift::has_upstream_changes_lenient(
                primal,
                source,
                provenance.as_ref(),
                &depot_dir,
            )
            .await;
            if changed {
                drifted.push(primal.to_string());
            } else {
                current += 1;
            }
        }
    }

    let stale_days = parse_staleness_days(&generated);
    let stale_warning = stale_days.is_some_and(|d| d > DEPOT_STALE_THRESHOLD_DAYS);

    let age_suffix = match stale_days {
        Some(d) if d > DEPOT_STALE_THRESHOLD_DAYS => format!(" | ⚠ STALE ({d} days old)"),
        Some(d) => format!(" | age: {d}d"),
        None => String::new(),
    };

    let msg = format!(
        "depot: {current}/{total} current, {} drifted | built: {generated} | target: {target}{age_suffix}",
        drifted.len()
    );

    let data = serde_json::json!({
        "total": total,
        "current": current,
        "drifted": drifted,
        "generated": generated,
        "target": target,
        "stale_days": stale_days,
        "stale": stale_warning,
    });

    Ok(crate::ShadowOutcome {
        ok: drifted.is_empty() && !stale_warning,
        message: msg,
        data: Some(data),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_staleness_recent() {
        let ts = crate::utc_now_iso8601();
        let days = parse_staleness_days(&ts);
        assert_eq!(days, Some(0), "today's timestamp should be 0 days old");
    }

    #[test]
    fn parse_staleness_old() {
        let days = parse_staleness_days("2020-01-01T00:00:00Z");
        assert!(days.is_some());
        assert!(days.unwrap() > 365, "2020 should be years ago");
    }

    #[test]
    fn parse_staleness_unparseable() {
        assert!(parse_staleness_days("unknown").is_none());
        assert!(parse_staleness_days("").is_none());
        assert!(parse_staleness_days("not-a-date").is_none());
    }

    #[test]
    fn stale_threshold_is_7_days() {
        assert_eq!(DEPOT_STALE_THRESHOLD_DAYS, 7);
    }
}
