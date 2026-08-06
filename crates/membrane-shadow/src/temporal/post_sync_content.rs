// SPDX-License-Identifier: AGPL-3.0-or-later

//! Post-cascade content, sovereignty, and drift pipelines.
//!
//! Extracted from `post_sync.rs` — these functions handle rootpulse
//! sovereignty verification, commit drift detection, content rebuilds,
//! and depot freshness reporting.

/// Commit cascade state to rootPulse and verify sovereignty.
pub(super) async fn run_rootpulse_sovereignty(
    wave_id: u32,
    gate: &str,
    heads: &std::collections::BTreeMap<String, String>,
    lines: &mut Vec<String>,
) {
    match crate::freshness::rootpulse_commit(wave_id, gate, heads).await {
        Ok(session) => {
            lines.push(format!("  [rootpulse] COMMITTED {session}"));
            persist_rootpulse_session(wave_id, gate, &session);
        }
        Err(e) => {
            lines.push(format!("  [rootpulse] SKIP: {e}"));
        }
    }

    let checks = crate::freshness::sovereignty_verify(wave_id, heads).await;
    if !checks.is_empty() {
        let verified = checks.iter().filter(|c| c.verified).count();
        let total = checks.len();
        if verified == total {
            lines.push(format!("  [sovereignty] VERIFIED {verified}/{total}"));
        } else {
            lines.push(format!("  [sovereignty] {verified}/{total} verified"));
            for check in &checks {
                if !check.verified {
                    lines.push(format!("    \u{26a0} {}: {}", check.repo, check.detail));
                }
            }
        }
    }
}

/// Collect tree hashes for all cloned repos in the cascade set.
///
/// Uses `HEAD^{tree}` (content-addressed) so identical file states at
/// different commit SHAs produce identical entries — this is what makes
/// convergence comparison a DAG rather than a cyclic graph.
pub async fn collect_cascade_heads(
    root: &std::path::Path,
    repos: &[(&str, &crate::manifest::RepoEntry)],
) -> std::collections::BTreeMap<String, String> {
    let mut heads = std::collections::BTreeMap::new();
    for (name, entry) in repos {
        let repo_dir = root.join(&entry.local_path);
        if repo_dir.join(".git").exists() {
            #[allow(clippy::literal_string_with_formatting_args)]
            if let Ok(tree) =
                crate::git_ops::git_output(&repo_dir, &["rev-parse", "HEAD^{tree}"]).await
            {
                heads.insert((*name).to_string(), tree);
            }
        }
    }
    heads
}

/// Quick depot freshness summary — reports how many binaries exist and are recent.
pub(super) fn summarize_depot_freshness() -> String {
    let depot_dir = crate::plasmid::resolve_path(
        None,
        cellmembrane_types::service::ENV_PLASMIDBIN_DEPOT,
        || {
            std::path::PathBuf::from(cellmembrane_types::service::env_or(
                cellmembrane_types::service::ENV_ECOPRIMALS_ROOT,
                cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT,
            ))
            .join(cellmembrane_types::service::PLASMID_BIN_DIR)
        },
    );

    let arch = crate::plasmid::detect_target_triple();
    let primals_dir = depot_dir.join("primals").join(arch);
    if !primals_dir.is_dir() {
        return String::new();
    }

    let mut present = 0u32;
    let mut total = 0u32;
    let mut stale = 0u32;
    let now = std::time::SystemTime::now();
    let stale_threshold = std::time::Duration::from_secs(
        cellmembrane_types::service::DEFAULT_STALENESS_THRESHOLD_SECS,
    );

    for name in crate::plasmid::nucleus_primals() {
        total += 1;
        let path = primals_dir.join(name);
        if path.exists() {
            present += 1;
            if let Ok(meta) = std::fs::metadata(&path) {
                if let Ok(modified) = meta.modified() {
                    if now.duration_since(modified).unwrap_or_default() > stale_threshold {
                        stale += 1;
                    }
                }
            }
        }
    }

    let missing = total - present;
    let suffix = match (missing, stale) {
        (0, 0) => String::new(),
        (0, s) => format!(" ({s} stale — run with --with-rebuild to auto-fix)"),
        (m, 0) => format!(" ({m} missing)"),
        (m, s) => format!(" ({m} missing, {s} stale)"),
    };
    format!("  [depot] {present}/{total} binaries present{suffix}")
}

/// Check if this gate is the designated freshness publisher.
pub(super) fn is_freshness_publisher() -> bool {
    std::env::var(cellmembrane_types::service::ENV_FRESHNESS_PUBLISHER)
        .is_ok_and(|v| matches!(v.as_str(), "1" | "true" | "yes"))
}

/// Persist rootpulse session to gate-local state.
pub(crate) fn persist_rootpulse_session(wave_id: u32, gate: &str, session_id: &str) {
    let Ok(root) = crate::temporal::resolve_workspace_root() else {
        return;
    };
    let state_path = root
        .join(cellmembrane_types::service::INFRA_WATERING_HOLE)
        .join(".rootpulse_state.toml");
    let content = format!(
        "# Last rootpulse commit — auto-generated, do not edit\n\
         wave = {wave_id}\n\
         gate = \"{gate}\"\n\
         session = \"{session_id}\"\n\
         timestamp = \"{}\"\n",
        crate::utc_now_iso8601()
    );
    if let Err(e) = std::fs::write(&state_path, &content) {
        tracing::warn!(
            path = %state_path.display(),
            error = %e,
            "rootpulse: failed to persist session state"
        );
    }
}

/// Load the last rootpulse session ID from gate-local state.
pub(crate) fn load_rootpulse_session() -> Option<String> {
    let root = crate::temporal::resolve_workspace_root().ok()?;
    let state_path = root
        .join(cellmembrane_types::service::INFRA_WATERING_HOLE)
        .join(".rootpulse_state.toml");
    let contents = std::fs::read_to_string(&state_path).ok()?;
    let table: toml::Table = contents.parse().ok()?;
    table
        .get("session")
        .and_then(|v| v.as_str())
        .map(String::from)
}

/// Rebuild sporePrint static site if this gate has the content dir and Zola installed.
pub(super) async fn run_content_rebuild_if_needed(root: &std::path::Path, lines: &mut Vec<String>) {
    let site_dir = root.join(cellmembrane_types::service::SPOREPRINT_CONTENT_DIR);
    if !site_dir.join("config.toml").exists() {
        return;
    }

    let result = tokio::process::Command::new("zola")
        .arg("build")
        .current_dir(&site_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await;

    match result {
        Ok(output) if output.status.success() => {
            lines.push(format!(
                "  [content] REBUILT sporePrint ({})",
                site_dir.display()
            ));
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let first_line = stderr.lines().next().unwrap_or("unknown error");
            lines.push(format!("  [content] zola build FAIL — {first_line}"));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            lines.push(format!("  [content] zola exec error — {e}"));
        }
    }
}

/// Commit drift pipeline: detect primals with source changes not yet in the depot.
pub(super) async fn run_commit_drift_pipeline(lines: &mut Vec<String>) {
    let drifted = crate::plasmid::detect_commit_drift().await;

    if drifted.is_empty() {
        lines.push("  [drift] all primals current with depot".into());
        return;
    }

    let total = crate::plasmid::nucleus_primals().len();
    lines.push(format!(
        "  [drift] {}/{total} primals have source changes not in depot: [{}]",
        drifted.len(),
        drifted.join(", ")
    ));

    if !is_build_authority() {
        return;
    }

    crate::plasmid::notify_mesh_build_pending(&drifted).await;
    lines.push("  [drift] build authority — auto-harvesting drifted primals".into());

    for primal in &drifted {
        let harvest_args = crate::plasmid::HarvestArgs {
            primal: Some(primal.clone()),
            force: true,
            dry_run: false,
            depot_dir: None,
            target: None,
            local: true,
            push: false,
            with_restart: false,
        };
        match crate::plasmid::harvest(&harvest_args).await {
            Ok(o) => lines.push(format!("  [drift] {primal}: {}", o.message)),
            Err(e) => lines.push(format!("  [drift] {primal}: FAIL — {e}")),
        }
    }

    let passed = super::post_sync_harvest::run_post_cascade_sandbox(&drifted, lines).await;
    if !passed.is_empty() {
        match super::post_sync_harvest::run_post_cascade_refresh(Some(&passed), lines).await {
            Ok(pushed) => {
                lines.push(format!(
                    "  [drift] {} rebuilt, {pushed} pushed to depot",
                    drifted.len()
                ));
            }
            Err(e) => lines.push(format!("  [drift] refresh FAIL: {e}")),
        }
    }
}

/// Whether this gate is a build authority (builds and publishes depot binaries).
pub(super) fn is_build_authority() -> bool {
    std::env::var(cellmembrane_types::service::ENV_BUILD_AUTHORITY)
        .is_ok_and(|v| matches!(v.as_str(), "1" | "true" | "yes"))
}

/// Post-rebuild health check: verify sporePrint `public/` is non-empty.
pub(super) async fn check_content_health(root: &std::path::Path, lines: &mut Vec<String>) {
    let public_dir = root
        .join(cellmembrane_types::service::SPOREPRINT_CONTENT_DIR)
        .join("public");
    if !public_dir.exists() {
        return;
    }

    let index = public_dir.join("index.html");
    if !index.exists() {
        lines.push(
            "  [content] WARNING: sporePrint public/ missing index.html — root will 404".into(),
        );
        return;
    }

    let size = tokio::fs::metadata(&index).await.map_or(0, |m| m.len());
    if size == 0 {
        lines.push("  [content] WARNING: sporePrint public/index.html is empty".into());
        return;
    }

    let file_count = std::fs::read_dir(&public_dir).map_or(0, std::iter::Iterator::count);
    lines.push(format!(
        "  [content] sporePrint healthy ({file_count} files, index {size}B)"
    ));
}
