// SPDX-License-Identifier: AGPL-3.0-or-later

//! Plasmid binary lifecycle — fetch, refresh, harvest, and deploy primal binaries.
//!
//! Manages the binary supply chain for membrane services:
//! - `fetch` — Download binaries from sovereign or external sources (GitHub, VPS, Forgejo)
//! - `harvest` — Build from source, detect changes, checksum, stage to depot
//! - `refresh` — Push local pre-built binaries to VPS with atomic replacement
//! - `pipeline` — End-to-end zero-touch: harvest → refresh → alive
//! - `status` — Report depot freshness and drift against upstream
//!
//! BLAKE3 checksums are verified in-process using the `blake3` crate.

mod depot;
mod fetch;
mod harvest;
mod refresh;

pub use fetch::*;
pub use harvest::{HarvestArgs, HarvestResult, HarvestStatus, harvest};
pub use refresh::{RefreshArgs, RefreshResult, RefreshStatus, refresh};

pub use depot::{StalenessEntry, StalenessReport};

/// Compute BLAKE3 hash of a file, returning hex string.
pub(crate) fn compute_blake3_file(path: &std::path::Path) -> String {
    depot::compute_blake3_file(path)
}

/// Detect stale primals in the depot. Resolves depot path from env/defaults.
pub fn detect_depot_staleness() -> crate::error::Result<StalenessReport> {
    let depot_dir = depot::resolve_depot(None)?;
    depot::detect_stale_primals(&depot_dir)
}

use std::path::PathBuf;

/// Primal binary names derived from the service registry at compile time.
///
/// Previously a hand-maintained `const` list — now sourced directly from
/// `cellmembrane-types::MembraneService::all()` so additions/removals to the
/// registry propagate automatically with zero manual sync.
pub(crate) fn nucleus_primals() -> Vec<&'static str> {
    cellmembrane_types::MembraneService::all()
        .iter()
        .filter(|s| s.is_primal)
        .map(|s| s.binary)
        .collect()
}

/// Detect the local platform's Rust target triple.
pub(crate) fn detect_target_triple() -> String {
    let arch = if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "unknown"
    };
    format!("{arch}-unknown-linux-musl")
}

/// Resolve a path with priority: explicit override → env var → computed default.
pub(crate) fn resolve_path(
    explicit: Option<&str>,
    env_var: &str,
    default_fn: impl FnOnce() -> PathBuf,
) -> PathBuf {
    if let Some(dir) = explicit {
        return PathBuf::from(dir);
    }
    if let Ok(val) = std::env::var(env_var) {
        return PathBuf::from(val);
    }
    default_fn()
}

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
    };

    let harvest_outcome = harvest(&harvest_args).await?;

    if dry_run {
        return Ok(harvest_outcome);
    }

    let built_any = harvest_outcome
        .data
        .as_ref()
        .and_then(|d| d.as_array())
        .is_some_and(|arr| {
            arr.iter().any(|r| {
                r.get("status")
                    .and_then(|s| s.as_str())
                    .is_some_and(|s| s == "Built")
            })
        });

    if !built_any {
        return Ok(crate::ShadowOutcome {
            ok: harvest_outcome.ok,
            message: format!("{} — no new binaries to push", harvest_outcome.message),
            data: harvest_outcome.data,
        });
    }

    let depot_source = depot::resolve_depot(None).ok().map(|d| {
        let arch = detect_target_triple();
        d.join("primals").join(arch).to_string_lossy().into_owned()
    });

    let refresh_args = RefreshArgs {
        primal: primal.map(Into::into),
        dry_run: false,
        source_dir: depot_source,
    };

    let refresh_outcome = refresh(config, &refresh_args).await?;

    Ok(crate::ShadowOutcome {
        ok: refresh_outcome.ok,
        message: format!("{} | {}", harvest_outcome.message, refresh_outcome.message),
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

/// `plasmid.depot_sync` — Sync inner membrane binaries to the WAN depot directory.
///
/// After `plasmid.refresh` pushes binaries to the install dir (e.g. `/opt/membrane/`),
/// the WAN depot directory (`/opt/ecoPrimals/plasmidBin/primals/{arch}/`) may be stale.
/// This command ensures the depot serves the same binaries that are running, by
/// hard-linking (or copying) from install dir to depot dir on the VPS.
pub async fn depot_sync(
    config: &crate::ShadowConfig,
) -> crate::error::Result<crate::ShadowOutcome> {
    let install_dir = std::env::var(cellmembrane_types::service::ENV_INSTALL_BASE)
        .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_INSTALL_BASE.into());
    let depot_root = format!(
        "{}/plasmidBin/primals",
        cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT
    );
    let arch = detect_target_triple();
    let depot_dir = format!("{depot_root}/{arch}");

    let primals = nucleus_primals();
    let primal_list = primals.join(" ");

    let cmd = format!(
        "mkdir -p {depot_dir} && \
         synced=0; failed=0; \
         for p in {primal_list}; do \
           src=\"{install_dir}/$p\"; \
           dst=\"{depot_dir}/$p\"; \
           if [ -f \"$src\" ]; then \
             cp -f \"$src\" \"$dst\" && synced=$((synced+1)) || failed=$((failed+1)); \
           fi; \
         done; \
         echo \"synced=$synced failed=$failed\""
    );

    let (output, code) = crate::ssh::exec_raw(config, &cmd).await?;

    if code != 0 {
        return Ok(crate::ShadowOutcome {
            ok: false,
            message: format!("depot_sync failed (exit {code}): {}", output.trim()),
            data: None,
        });
    }

    let synced = output
        .split("synced=")
        .nth(1)
        .and_then(|s| s.split_whitespace().next())
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);
    let failed = output
        .split("failed=")
        .nth(1)
        .and_then(|s| s.split_whitespace().next())
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);

    Ok(crate::ShadowOutcome {
        ok: failed == 0,
        message: format!("depot_sync: {synced} binaries synced to {depot_dir}, {failed} failed"),
        data: Some(serde_json::json!({
            "synced": synced,
            "failed": failed,
            "depot_dir": depot_dir,
            "install_dir": install_dir,
            "arch": arch,
        })),
    })
}

/// `plasmid.status` — Report depot freshness and upstream drift.
///
/// Reads provenance.toml for last build timestamp, then checks each
/// primal's HEAD against the recorded commit to identify drift.
pub async fn status() -> crate::error::Result<crate::ShadowOutcome> {
    let depot_dir = harvest::resolve_depot(None)?;
    let sources = harvest::load_sources(&depot_dir)?;
    let provenance = harvest::load_provenance(&depot_dir);

    let generated = provenance
        .as_ref()
        .and_then(|p| p.generated.clone())
        .unwrap_or_else(|| "unknown".into());

    let target = provenance
        .as_ref()
        .and_then(|p| p.target.clone())
        .unwrap_or_else(|| "unknown".into());

    let registry_primals = nucleus_primals();
    let total = registry_primals.len();

    let mut drifted: Vec<String> = Vec::new();
    let mut current = 0usize;

    for &primal in &registry_primals {
        if let Some(source) = sources.get(primal) {
            let changed =
                harvest::has_upstream_changes_pub(primal, source, provenance.as_ref(), &depot_dir)
                    .await;
            if changed {
                drifted.push(primal.to_string());
            } else {
                current += 1;
            }
        }
    }

    let msg = format!(
        "depot: {current}/{total} current, {} drifted | built: {generated} | target: {target}",
        drifted.len()
    );

    let data = serde_json::json!({
        "total": total,
        "current": current,
        "drifted": drifted,
        "generated": generated,
        "target": target,
    });

    Ok(crate::ShadowOutcome {
        ok: drifted.is_empty(),
        message: msg,
        data: Some(data),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nucleus_primals_returns_13() {
        let primals = nucleus_primals();
        assert_eq!(primals.len(), 13, "expected 13 nucleus primals");
        assert!(primals.contains(&"beardog"));
        assert!(primals.contains(&"songbird"));
        assert!(primals.contains(&"squirrel"));
    }

    #[test]
    fn detect_target_triple_contains_musl() {
        let triple = detect_target_triple();
        assert!(
            triple.ends_with("-unknown-linux-musl"),
            "expected musl target, got: {triple}"
        );
    }

    #[test]
    fn resolve_path_explicit_overrides_env() {
        let result = resolve_path(Some("/explicit"), "NONEXISTENT_VAR_XYZ", || {
            PathBuf::from("/default")
        });
        assert_eq!(result, PathBuf::from("/explicit"));
    }

    #[test]
    fn resolve_path_uses_default_when_no_env() {
        let result = resolve_path(None, "NONEXISTENT_VAR_XYZ_ABC", || {
            PathBuf::from("/fallback")
        });
        assert_eq!(result, PathBuf::from("/fallback"));
    }

    #[tokio::test]
    async fn status_reports_depot_state() {
        let result = status().await;
        match result {
            Ok(outcome) => {
                assert!(outcome.message.contains("depot:"));
                assert!(outcome.message.contains("current"));
                let data = outcome.data.unwrap();
                assert!(data.get("total").is_some());
                assert!(data.get("drifted").is_some());
            }
            Err(e) => {
                let msg = format!("{e}");
                assert!(
                    msg.contains("depot not found") || msg.contains("cannot read"),
                    "unexpected error: {msg}"
                );
            }
        }
    }
}
