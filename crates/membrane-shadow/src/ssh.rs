// SPDX-License-Identifier: AGPL-3.0-or-later

//! SSH transport for shadow functions.
//!
//! Wraps `tokio::process::Command` around the system SSH client via an
//! `SshArgs` builder that centralises the common SSH/SCP option patterns.

use crate::config::ShadowConfig;
use crate::error::{Result, ShadowError};
use tokio::process::Command;

// ── Builder ──────────────────────────────────────────────────────────

/// Centralised SSH option builder. Every SSH/SCP call routes through this
/// to avoid duplicating `-o ConnectTimeout=… -o BatchMode=yes` boilerplate.
fn ssh_args(timeout: u32) -> Vec<String> {
    vec![
        "-o".into(),
        format!("ConnectTimeout={timeout}"),
        "-o".into(),
        "BatchMode=yes".into(),
    ]
}

/// Extended builder for provisioning hosts where we accept new host keys.
fn ssh_args_accept_new(timeout: u32) -> Vec<String> {
    let mut args = ssh_args(timeout);
    args.extend(["-o".into(), "StrictHostKeyChecking=accept-new".into()]);
    args
}

/// SCP keepalive args for long transfers.
fn scp_keepalive_args() -> Vec<String> {
    vec![
        "-o".into(),
        "ServerAliveInterval=15".into(),
        "-o".into(),
        "ServerAliveCountMax=3".into(),
        "-q".into(),
    ]
}

/// Run an SSH command and return the raw `Output`.
async fn run_ssh(host: &str, timeout: u32, command: &str) -> std::io::Result<std::process::Output> {
    let mut args = ssh_args(timeout);
    args.push(host.into());
    args.push(command.into());
    Command::new("ssh").args(&args).output().await
}

/// Run an SSH command on `user@host` with `accept-new` host key policy.
async fn run_ssh_accept_new(
    dest: &str,
    timeout: u32,
    command: &str,
) -> std::io::Result<std::process::Output> {
    let mut args = ssh_args_accept_new(timeout);
    args.push(dest.into());
    args.push(command.into());
    Command::new("ssh").args(&args).output().await
}

/// Extract exit code from `Output`, defaulting to -1 on signal termination.
fn exit_code(output: &std::process::Output) -> i32 {
    output.status.code().unwrap_or(-1)
}

// ── Public API ───────────────────────────────────────────────────────

/// Execute a command on the VPS via SSH, returning stdout.
pub async fn exec(config: &ShadowConfig, command: &str) -> Result<String> {
    let output = run_ssh(&config.ssh_host, config.ssh_timeout, command).await?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(ShadowError::Ssh(format!(
            "exit {}: {}",
            exit_code(&output),
            stderr.trim()
        )))
    }
}

/// Quick SSH connectivity check — returns true if the host is reachable.
pub async fn check_connectivity(host: &str) -> bool {
    run_ssh(host, 5, "true")
        .await
        .is_ok_and(|o| o.status.success())
}

/// Execute a command and return both stdout and exit code (non-fatal on failure).
pub async fn exec_raw(config: &ShadowConfig, command: &str) -> Result<(String, i32)> {
    exec_raw_on(&config.ssh_host, config.ssh_timeout, command).await
}

/// Execute a command on a specific host, returning stdout and exit code.
///
/// Use this when the target host differs from `config.ssh_host` (e.g. outer membrane)
/// to avoid cloning the full `ShadowConfig` just to swap the host field.
pub async fn exec_raw_on(host: &str, timeout: u32, command: &str) -> Result<(String, i32)> {
    let output = run_ssh(host, timeout, command).await?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    Ok((stdout, exit_code(&output)))
}

/// Transfer a local file to the VPS via SCP.
pub async fn scp_to(config: &ShadowConfig, local_path: &str, remote_path: &str) -> Result<()> {
    let dest = format!("{}:{}", config.ssh_host, remote_path);
    let mut args = ssh_args(config.ssh_timeout);
    args.extend(scp_keepalive_args());
    args.push(local_path.into());
    args.push(dest);

    let output = Command::new("scp").args(&args).output().await?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(ShadowError::Ssh(format!(
            "scp failed (exit {}): {}",
            exit_code(&output),
            stderr.trim()
        )))
    }
}

/// Execute a command on a host using `user@ip` form (provisioning/enrollment).
///
/// Accepts new host keys on first connect (`StrictHostKeyChecking=accept-new`).
pub async fn exec_on_host(
    user: &str,
    host: &str,
    command: &str,
    timeout: u32,
) -> Result<(String, i32)> {
    let dest = format!("{user}@{host}");
    let output = run_ssh_accept_new(&dest, timeout, command).await?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    Ok((stdout, exit_code(&output)))
}

/// Transfer a local file to a host using `user@ip` form (provisioning/enrollment).
///
/// Accepts new host keys on first connect.
pub async fn scp_to_host(
    user: &str,
    host: &str,
    local_path: &str,
    remote_path: &str,
    timeout: u32,
) -> Result<()> {
    let dest = format!("{user}@{host}:{remote_path}");
    let mut args = ssh_args_accept_new(timeout);
    args.push(local_path.into());
    args.push(dest);

    let output = Command::new("scp").args(&args).output().await?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(ShadowError::Ssh(format!(
            "scp to {host} failed (exit {}): {}",
            exit_code(&output),
            stderr.trim()
        )))
    }
}

/// Execute a command on a named gate, resolving the SSH target from the
/// ecosystem manifest (host → `lan_ip` → `wg_ip` priority chain).
///
/// Falls back to the gate name as a hostname if the manifest is unavailable.
pub async fn exec_on_gate(gate: &str, command: &str, timeout: u32) -> Result<(String, i32)> {
    let (host, user) = resolve_gate_ssh(gate);
    let dest = format!("{user}@{host}");
    let output = run_ssh(&dest, timeout, command).await?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    Ok((stdout, exit_code(&output)))
}

/// Resolve the SSH target and user for a gate from the manifest.
fn resolve_gate_ssh(gate: &str) -> (String, String) {
    let workspace = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_ECOPRIMALS_ROOT,
        cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT,
    );
    if let Ok(manifest) = crate::manifest::load_from_workspace(std::path::Path::new(&workspace)) {
        let host = manifest
            .ssh_target_for(gate)
            .map_or_else(|| gate.to_string(), String::from);
        let user = manifest.ssh_user_for(gate).to_string();
        return (host, user);
    }
    (gate.into(), "root".into())
}

/// Fetch a remote file's contents via SSH `cat` (depot binary download).
pub async fn cat_remote(host: &str, remote_path: &str, timeout: u32) -> Result<Vec<u8>> {
    let output = run_ssh(host, timeout, &format!("cat {remote_path}")).await?;

    if output.status.success() {
        Ok(output.stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(ShadowError::Ssh(format!(
            "ssh cat {remote_path} failed (exit {}): {}",
            exit_code(&output),
            stderr.trim()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_args_includes_timeout_and_batch() {
        let args = ssh_args(10);
        assert_eq!(args, ["-o", "ConnectTimeout=10", "-o", "BatchMode=yes"]);
    }

    #[test]
    fn ssh_args_accept_new_extends_base() {
        let args = ssh_args_accept_new(5);
        assert_eq!(args.len(), 6);
        assert!(args.contains(&"StrictHostKeyChecking=accept-new".to_string()));
    }

    #[test]
    fn scp_keepalive_includes_quiet() {
        let args = scp_keepalive_args();
        assert!(args.contains(&"-q".to_string()));
        assert!(args.contains(&"ServerAliveInterval=15".to_string()));
    }

    #[tokio::test]
    async fn check_connectivity_unreachable_returns_false() {
        let ok = check_connectivity("nonexistent.invalid.host.test").await;
        assert!(!ok, "unreachable host should return false");
    }

    #[tokio::test]
    async fn exec_raw_on_invalid_host_returns_error_or_failure() {
        let result = exec_raw_on("nonexistent.invalid.host.test", 1, "true").await;
        if let Ok((_out, code)) = result {
            assert_ne!(code, 0, "invalid host should fail");
        }
    }

    #[test]
    fn shadow_config_default_timeout() {
        let config = ShadowConfig::default();
        assert!(config.ssh_timeout > 0, "timeout should be positive");
    }

    #[test]
    fn resolve_gate_ssh_falls_back_to_gate_name() {
        let (host, user) = resolve_gate_ssh("nonexistent_gate_xyz");
        assert_eq!(user, "root");
        assert!(!host.is_empty());
    }
}
