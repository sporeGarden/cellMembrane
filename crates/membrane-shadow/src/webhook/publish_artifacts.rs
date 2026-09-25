//! Post-build artifact generation — invoke site-specific build tools.
//!
//! Each [`PublishSite`] can declare an `artifact_command` — a subprocess that
//! runs in the worktree after Zola build to generate derived files
//! (graph.json, api/site.json, llms.txt, llms-full.txt, content-manifest.toml).
//!
//! This keeps the membrane generic: domain-specific generation logic lives in
//! each site's own build tool (e.g. `detroit-build`).

use crate::seo::PublishSite;
use tracing::{info, warn};

/// Run the artifact generation command for a site, if configured.
///
/// Returns `Ok(Some(msg))` on success, `Ok(None)` if no command configured,
/// or `Err` on failure.
pub async fn generate(
    site: &PublishSite,
) -> Result<Option<String>, String> {
    let Some(cmd_parts) = site.artifact_command else {
        return Ok(None);
    };

    if cmd_parts.is_empty() {
        return Ok(None);
    }

    let program = cmd_parts[0];
    let args = &cmd_parts[1..];
    let work_dir = site.worktree;

    info!(
        repo = %site.repo_name,
        command = %program,
        worktree = %work_dir,
        "publish_artifacts: running artifact generation"
    );

    let output = tokio::process::Command::new(program)
        .args(args)
        .current_dir(work_dir)
        .env_remove("GIT_DIR")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("failed to spawn {program}: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if output.status.success() {
        let last_line = stdout
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("(no output)");
        info!(
            repo = %site.repo_name,
            "publish_artifacts: succeeded — {last_line}"
        );
        Ok(Some(format!("artifacts generated: {last_line}")))
    } else {
        let code = output.status.code().unwrap_or(-1);
        warn!(
            repo = %site.repo_name,
            exit_code = code,
            stderr = %stderr.trim(),
            "publish_artifacts: command failed"
        );
        Err(format!(
            "{program} exited {code}: {}",
            stderr.lines().last().unwrap_or("(no stderr)")
        ))
    }
}
