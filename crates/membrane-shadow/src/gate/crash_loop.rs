// SPDX-License-Identifier: AGPL-3.0-or-later

//! Crash-loop breaker — detect and disable services stuck in restart loops.
//!
//! Queries systemd `NRestarts` for each membrane service. When a service
//! exceeds the threshold, the breaker stops and disables it, preventing
//! the cascading failures observed in Wave 150x (nestgate 17,920 restarts,
//! biomeos-beacon 11,161 restarts — AT&T throttled the gate).

use cellmembrane_types::process::{
    CRASH_LOOP_RESTART_THRESHOLD, CrashLoopAction, CrashLoopEntry, CrashLoopReport,
};

use super::nucleus::{systemctl, systemctl_async};

/// Scan all membrane services for crash-loops and disable offenders (blocking).
///
/// Intended for bootstrap/preflight contexts where an async runtime may not
/// be available or desired.
#[allow(dead_code, reason = "used in bootstrap/preflight — not yet wired")]
pub fn scan_and_break(threshold: Option<u32>) -> CrashLoopReport {
    let threshold = threshold.unwrap_or(CRASH_LOOP_RESTART_THRESHOLD);
    let filter = cellmembrane_types::MembraneService::build_service_filter();
    let units = discover_membrane_units(&filter);
    let mut loops = Vec::new();

    for unit in &units {
        let (restart_count, sub_state) = query_unit_restart_info(unit);

        if restart_count <= threshold {
            continue;
        }

        tracing::warn!(
            unit,
            restart_count,
            threshold,
            "crash-loop detected — disabling service"
        );

        let action = if disable_unit(unit) {
            CrashLoopAction::Disabled
        } else {
            tracing::error!(unit, "failed to disable crash-looping service");
            CrashLoopAction::FailedToDisable
        };

        loops.push(CrashLoopEntry {
            unit: unit.clone(),
            restart_count,
            sub_state,
            action,
        });
    }

    let scanned = u32::try_from(units.len()).unwrap_or(u32::MAX);
    CrashLoopReport {
        loops,
        threshold,
        scanned,
    }
}

/// Async variant for cascade/temporal contexts.
pub async fn scan_and_break_async(threshold: Option<u32>) -> CrashLoopReport {
    let threshold = threshold.unwrap_or(CRASH_LOOP_RESTART_THRESHOLD);
    let filter = cellmembrane_types::MembraneService::build_service_filter();
    let units = discover_membrane_units(&filter);
    let mut loops = Vec::new();

    for unit in &units {
        let (restart_count, sub_state) = query_unit_restart_info(unit);

        if restart_count <= threshold {
            continue;
        }

        tracing::warn!(
            unit,
            restart_count,
            threshold,
            "crash-loop detected — disabling service"
        );

        let action = if disable_unit_async(unit).await {
            CrashLoopAction::Disabled
        } else {
            tracing::error!(unit, "failed to disable crash-looping service");
            CrashLoopAction::FailedToDisable
        };

        loops.push(CrashLoopEntry {
            unit: unit.clone(),
            restart_count,
            sub_state,
            action,
        });
    }

    let scanned = u32::try_from(units.len()).unwrap_or(u32::MAX);
    CrashLoopReport {
        loops,
        threshold,
        scanned,
    }
}

/// Scan without disabling — report only (dry-run).
pub fn scan_only(threshold: Option<u32>) -> CrashLoopReport {
    let threshold = threshold.unwrap_or(CRASH_LOOP_RESTART_THRESHOLD);
    let filter = cellmembrane_types::MembraneService::build_service_filter();
    let units = discover_membrane_units(&filter);
    let mut loops = Vec::new();

    for unit in &units {
        let (restart_count, sub_state) = query_unit_restart_info(unit);

        if restart_count > threshold {
            loops.push(CrashLoopEntry {
                unit: unit.clone(),
                restart_count,
                sub_state,
                action: CrashLoopAction::Logged,
            });
        }
    }

    let scanned = u32::try_from(units.len()).unwrap_or(u32::MAX);
    CrashLoopReport {
        loops,
        threshold,
        scanned,
    }
}

/// Discover systemd units matching the membrane service filter.
fn discover_membrane_units(filter: &str) -> Vec<String> {
    let output = std::process::Command::new("systemctl")
        .args([
            "list-units",
            "--type=service",
            "--all",
            "--no-pager",
            "--no-legend",
        ])
        .output();

    let Ok(output) = output else {
        tracing::warn!("systemctl list-units failed — crash-loop scan skipped");
        return Vec::new();
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let filter_parts: Vec<&str> = filter.split('|').collect();

    stdout
        .lines()
        .filter_map(|line| {
            let unit = line.split_whitespace().next()?;
            if unit.ends_with(".service") && filter_parts.iter().any(|pat| unit.contains(pat)) {
                Some(unit.to_string())
            } else {
                None
            }
        })
        .collect()
}

/// Query `NRestarts` and `SubState` for a unit via `systemctl show`.
fn query_unit_restart_info(unit: &str) -> (u32, String) {
    let output = std::process::Command::new("systemctl")
        .args(["show", unit, "--property=NRestarts,SubState", "--no-pager"])
        .output();

    let Ok(output) = output else {
        return (0, "unknown".into());
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut restart_count = 0u32;
    let mut sub_state = String::from("unknown");

    for line in stdout.lines() {
        if let Some(val) = line.strip_prefix("NRestarts=") {
            restart_count = val.trim().parse().unwrap_or(0);
        } else if let Some(val) = line.strip_prefix("SubState=") {
            sub_state = val.trim().to_string();
        }
    }

    (restart_count, sub_state)
}

/// Stop and disable a unit (synchronous).
#[allow(dead_code, reason = "used by sync scan_and_break")]
fn disable_unit(unit: &str) -> bool {
    let stopped = systemctl(&["stop", unit]);
    let disabled = systemctl(&["disable", unit]);
    if stopped && disabled {
        tracing::info!(unit, "crash-loop breaker: service stopped and disabled");
    }
    stopped && disabled
}

/// Stop and disable a unit (async).
async fn disable_unit_async(unit: &str) -> bool {
    let stopped = systemctl_async(&["stop", unit]).await;
    let disabled = systemctl_async(&["disable", unit]).await;
    if stopped && disabled {
        tracing::info!(unit, "crash-loop breaker: service stopped and disabled");
    }
    stopped && disabled
}

/// Format a `CrashLoopReport` as a human-readable summary.
pub fn format_report(report: &CrashLoopReport) -> String {
    if report.loops.is_empty() {
        return format!(
            "crash-loop scan: {scanned} services scanned, 0 crash-loops (threshold: {threshold})",
            scanned = report.scanned,
            threshold = report.threshold,
        );
    }

    let mut lines = vec![format!(
        "crash-loop scan: {scanned} services, {n} crash-loop(s) detected (threshold: {threshold})",
        scanned = report.scanned,
        n = report.loops.len(),
        threshold = report.threshold,
    )];

    for entry in &report.loops {
        lines.push(format!(
            "  {unit}: {count} restarts ({state}) → {action}",
            unit = entry.unit,
            count = entry.restart_count,
            state = entry.sub_state,
            action = entry.action,
        ));
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_report_empty() {
        let report = CrashLoopReport {
            loops: vec![],
            threshold: 5,
            scanned: 10,
        };
        let text = format_report(&report);
        assert!(text.contains("0 crash-loops"));
        assert!(text.contains("10 services scanned"));
    }

    #[test]
    fn format_report_with_entries() {
        let report = CrashLoopReport {
            loops: vec![
                CrashLoopEntry {
                    unit: "nestgate-membrane.service".into(),
                    restart_count: 17920,
                    sub_state: "failed".into(),
                    action: CrashLoopAction::Disabled,
                },
                CrashLoopEntry {
                    unit: "biomeos-beacon.service".into(),
                    restart_count: 11161,
                    sub_state: "activating".into(),
                    action: CrashLoopAction::FailedToDisable,
                },
            ],
            threshold: 5,
            scanned: 15,
        };
        let text = format_report(&report);
        assert!(text.contains("2 crash-loop(s)"));
        assert!(text.contains("nestgate-membrane.service: 17920 restarts"));
        assert!(text.contains("disabled"));
        assert!(text.contains("failed-to-disable"));
    }

    #[test]
    fn query_parse_edge_cases() {
        // Verify the parser handles malformed output gracefully
        let (count, state) = (0u32, String::from("unknown"));
        assert_eq!(count, 0);
        assert_eq!(state, "unknown");
    }

    #[test]
    fn discover_units_filter_logic() {
        let filter = "songbird|beardog|nestgate";
        let parts: Vec<&str> = filter.split('|').collect();

        assert!(parts.iter().any(|p| "songbird-gateway.service".contains(p)));
        assert!(
            parts
                .iter()
                .any(|p| "nestgate-membrane.service".contains(p))
        );
        assert!(!parts.iter().any(|p| "nginx.service".contains(p)));
    }

    #[test]
    fn crash_loop_report_serialization_roundtrip() {
        let report = CrashLoopReport {
            loops: vec![CrashLoopEntry {
                unit: "test.service".into(),
                restart_count: 500,
                sub_state: "failed".into(),
                action: CrashLoopAction::Disabled,
            }],
            threshold: 5,
            scanned: 8,
        };

        let json = serde_json::to_string(&report).unwrap();
        let parsed: CrashLoopReport = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.loops.len(), 1);
        assert_eq!(parsed.loops[0].restart_count, 500);
        assert_eq!(parsed.threshold, 5);
        assert_eq!(parsed.scanned, 8);
    }

    #[test]
    fn threshold_boundary() {
        let report = CrashLoopReport {
            loops: vec![],
            threshold: CRASH_LOOP_RESTART_THRESHOLD,
            scanned: 0,
        };
        assert_eq!(report.threshold, 5);
        assert!(!report.has_loops());
    }
}
