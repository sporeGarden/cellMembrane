// SPDX-License-Identifier: AGPL-3.0-or-later

//! SSH transport for shadow functions.
//!
//! Wraps `tokio::process::Command` around the system SSH client.
//! Uses the `golgi` alias from `~/.ssh/config` for connection parameters.

use crate::config::ShadowConfig;
use crate::error::{Result, ShadowError};
use tokio::process::Command;

/// Execute a command on the VPS via SSH, returning stdout.
pub async fn exec(config: &ShadowConfig, command: &str) -> Result<String> {
    let output = Command::new("ssh")
        .args([
            "-o",
            &format!("ConnectTimeout={}", config.ssh_timeout),
            "-o",
            "BatchMode=yes",
            &config.ssh_host,
            command,
        ])
        .output()
        .await?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(ShadowError::Ssh(format!(
            "exit {}: {}",
            output.status.code().unwrap_or(-1),
            stderr.trim()
        )))
    }
}

/// Quick SSH connectivity check — returns true if the host is reachable.
pub async fn check_connectivity(host: &str) -> bool {
    Command::new("ssh")
        .args([
            "-o",
            "ConnectTimeout=5",
            "-o",
            "BatchMode=yes",
            host,
            "true",
        ])
        .output()
        .await
        .is_ok_and(|o| o.status.success())
}

/// Execute a command and return both stdout and exit code (non-fatal on failure).
pub async fn exec_raw(config: &ShadowConfig, command: &str) -> Result<(String, i32)> {
    let output = Command::new("ssh")
        .args([
            "-o",
            &format!("ConnectTimeout={}", config.ssh_timeout),
            "-o",
            "BatchMode=yes",
            &config.ssh_host,
            command,
        ])
        .output()
        .await?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let code = output.status.code().unwrap_or(-1);
    Ok((stdout, code))
}

/// Transfer a local file to the VPS via SCP.
pub async fn scp_to(config: &ShadowConfig, local_path: &str, remote_path: &str) -> Result<()> {
    let dest = format!("{}:{}", config.ssh_host, remote_path);
    let output = Command::new("scp")
        .args([
            "-o",
            &format!("ConnectTimeout={}", config.ssh_timeout),
            "-o",
            "BatchMode=yes",
            "-q",
            local_path,
            &dest,
        ])
        .output()
        .await?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(ShadowError::Ssh(format!(
            "scp failed (exit {}): {}",
            output.status.code().unwrap_or(-1),
            stderr.trim()
        )))
    }
}
