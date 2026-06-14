// SPDX-License-Identifier: AGPL-3.0-or-later
//! Post-provision bootstrap — SSH into a fresh droplet and bring it to operational state.
//!
//! Orchestrates: hardening -> directory setup -> binary deployment -> systemd install ->
//! gate.bootstrap -> mesh join -> health sweep.

use super::{DropletState, ProvisionOutcome};
use crate::error::{Result, ShadowError};

const SSH_RETRY_DELAY_SECS: u64 = 10;
const SSH_MAX_RETRIES: u32 = 12;

/// Run a command on the remote host via SSH with retry logic for fresh droplets.
async fn ssh_exec(ip: &str, command: &str) -> Result<String> {
    let output = tokio::process::Command::new("ssh")
        .args([
            "-o",
            "ConnectTimeout=10",
            "-o",
            "BatchMode=yes",
            "-o",
            "StrictHostKeyChecking=accept-new",
            &format!("root@{ip}"),
            command,
        ])
        .output()
        .await
        .map_err(|e| ShadowError::Parse(format!("SSH exec failed: {e}")))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(ShadowError::Parse(format!(
            "SSH command failed (exit {}): {stderr}",
            output.status.code().unwrap_or(-1)
        )))
    }
}

/// Wait for SSH to become available on a fresh droplet.
async fn wait_for_ssh(ip: &str) -> Result<()> {
    for attempt in 0..SSH_MAX_RETRIES {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(SSH_RETRY_DELAY_SECS)).await;
        }
        if ssh_exec(ip, "echo ready").await.is_ok() {
            return Ok(());
        }
        eprintln!(
            "provision: SSH not ready on {ip} (attempt {}/{})",
            attempt + 1,
            SSH_MAX_RETRIES
        );
    }
    Err(ShadowError::Parse(format!(
        "SSH not available on {ip} after {SSH_MAX_RETRIES} attempts"
    )))
}

/// Phase 1: OS hardening (fail2ban, unattended-upgrades, remove provider agent).
async fn harden(ip: &str) -> Result<String> {
    let script = r#"
        export DEBIAN_FRONTEND=noninteractive
        apt-get update -qq
        apt-get install -y -qq fail2ban unattended-upgrades socat > /dev/null 2>&1
        systemctl enable --now fail2ban
        systemctl enable --now unattended-upgrades
        apt-get remove -y -qq droplet-agent > /dev/null 2>&1 || true
        systemctl disable --now snapd > /dev/null 2>&1 || true
        echo "hardened"
    "#;
    ssh_exec(ip, script).await
}

/// Phase 2: Create directory structure.
async fn setup_directories(ip: &str) -> Result<String> {
    let script = r#"
        mkdir -p /opt/membrane /run/membrane /opt/ecoPrimals \
                 /opt/membrane/sandbox /run/membrane/sandbox \
                 /opt/membrane/canary /run/membrane/canary \
                 /var/lib/membrane/songbird /etc/membrane
        echo "directories created"
    "#;
    ssh_exec(ip, script).await
}

/// Phase 3: Deploy binaries from local depot via SCP.
async fn deploy_binaries(ip: &str) -> Result<String> {
    let depot_dir = crate::plasmid::depot::resolve_depot(None)?;
    let arch = crate::plasmid::detect_target_triple();
    let bin_dir = depot_dir.join("primals").join(&arch);

    if !bin_dir.exists() {
        return Err(ShadowError::Parse(format!(
            "depot bin dir not found: {}",
            bin_dir.display()
        )));
    }

    let primals = crate::plasmid::nucleus_primals();
    let mut deployed = 0u32;

    for primal in &primals {
        let local_path = bin_dir.join(primal);
        if !local_path.exists() {
            continue;
        }

        let remote_path = format!(
            "{}/{primal}",
            cellmembrane_types::service::DEFAULT_INSTALL_BASE
        );
        let scp_result = tokio::process::Command::new("scp")
            .args([
                "-o",
                "ConnectTimeout=10",
                "-o",
                "BatchMode=yes",
                "-o",
                "StrictHostKeyChecking=accept-new",
                &local_path.to_string_lossy(),
                &format!("root@{ip}:{remote_path}"),
            ])
            .output()
            .await;

        match scp_result {
            Ok(output) if output.status.success() => deployed += 1,
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                eprintln!("provision: SCP {primal} failed: {stderr}");
            }
            Err(e) => eprintln!("provision: SCP {primal} error: {e}"),
        }
    }

    ssh_exec(ip, "chmod +x /opt/membrane/*").await?;

    Ok(format!("{deployed}/{} binaries deployed", primals.len()))
}

/// Phase 4: Install systemd service units for Tower atomic.
///
/// Unit names and paths are derived from the service registry — the crypto spine
/// and mesh relay are discovered by capability, not hardcoded by name.
async fn install_systemd_units(ip: &str, gate_name: &str) -> Result<String> {
    let spine =
        cellmembrane_types::MembraneService::binary_for(cellmembrane_types::ServiceCapability::CryptoSigner);
    let relay =
        cellmembrane_types::MembraneService::binary_for(cellmembrane_types::ServiceCapability::MeshRelay);
    let spine_upper = spine.to_uppercase();
    let relay_upper = relay.to_uppercase();
    let federation_port = cellmembrane_types::service::DEFAULT_FEDERATION_PORT;

    let bearer_unit = format!(
        r"[Unit]
Description={spine} Crypto Spine (Membrane Tower)
After=network-online.target
Wants=network-online.target
StartLimitIntervalSec=60
StartLimitBurst=5

[Service]
Type=simple
ExecStart=/opt/membrane/{spine} server --socket /run/membrane/{spine}.sock --pid-dir /run/membrane
Environment={spine_upper}_NODE_ID={gate_name}
Environment={spine_upper}_LOG_LEVEL=info
Restart=always
RestartSec=3
MemoryMax=64M

[Install]
WantedBy=multi-user.target
"
    );

    let songbird_unit = format!(
        r"[Unit]
Description={relay} Discovery + Federation (Membrane Tower)
After=network-online.target {spine}-membrane.service
Wants=network-online.target
Requires={spine}-membrane.service
StartLimitIntervalSec=60
StartLimitBurst=5

[Service]
Type=simple
ExecStartPre=-/bin/rm -f /run/membrane/{relay}.sock
ExecStart=/opt/membrane/{relay} server \
    --socket /run/membrane/{relay}.sock \
    --security-socket /run/membrane/{spine}.sock \
    --federation-port {federation_port} \
    --bind 0.0.0.0 \
    --dark-forest \
    --pid-dir /run/membrane
Environment={relay_upper}_NODE_ID={gate_name}
Environment={relay_upper}_LOG_LEVEL=info
Environment={relay_upper}_DARK_FOREST=true
Environment={relay_upper}_SECURITY_PROVIDER={spine}
Environment={relay_upper}_PID_DIR=/run/membrane
Environment={relay_upper}_FEDERATION_PORT={federation_port}
Environment={relay_upper}_FEDERATION_ENABLED=true
Restart=always
RestartSec=5
MemoryMax=128M

[Install]
WantedBy=multi-user.target
"
    );

    let nucleus_template = format!(
        r"[Unit]
Description=NUCLEUS Primal %i (Membrane)
After={spine}-membrane.service {relay}-membrane.service
Wants={relay}-membrane.service

[Service]
Type=simple
ExecStart=/opt/membrane/%i server --socket /run/membrane/%i.sock --security-socket /run/membrane/{spine}.sock --pid-dir /run/membrane
Restart=always
RestartSec=5
MemoryMax=128M

[Install]
WantedBy=multi-user.target
"
    );

    let install_script = format!(
        r#"
cat > /etc/systemd/system/{spine}-membrane.service << 'UNIT'
{bearer_unit}
UNIT

cat > /etc/systemd/system/{relay}-membrane.service << 'UNIT'
{songbird_unit}
UNIT

cat > /etc/systemd/system/membrane-nucleus@.service << 'UNIT'
{nucleus_template}
UNIT

systemctl daemon-reload
systemctl enable {spine}-membrane {relay}-membrane
echo "units installed"
"#
    );

    ssh_exec(ip, &install_script).await
}

/// Phase 5: Start Tower services and NUCLEUS primals.
///
/// Tower services (crypto spine + mesh relay) are started first via their
/// dedicated systemd units. Remaining NUCLEUS primals are started via the
/// template unit, discovered from the service registry rather than hardcoded.
async fn start_services(ip: &str) -> Result<String> {
    let spine =
        cellmembrane_types::MembraneService::binary_for(cellmembrane_types::ServiceCapability::CryptoSigner);
    let relay =
        cellmembrane_types::MembraneService::binary_for(cellmembrane_types::ServiceCapability::MeshRelay);

    // Non-Tower NUCLEUS primals to start via template unit
    let nucleus_others: Vec<&str> = crate::plasmid::nucleus_primals()
        .into_iter()
        .filter(|p| *p != spine && *p != relay)
        .collect();
    let primal_list = nucleus_others.join(" ");

    let script = format!(
        r"
        systemctl start {spine}-membrane
        sleep 2
        systemctl start {relay}-membrane
        sleep 2

        for primal in {primal_list}; do
            if [ -f '/opt/membrane/$primal' ]; then
                systemctl enable --now 'membrane-nucleus@$primal' 2>/dev/null || true
            fi
        done

        echo 'services started'
    "
    );
    ssh_exec(ip, &script).await
}

/// Phase 6: Join the mesh by peering with main production VPS.
async fn join_mesh(ip: &str) -> Result<String> {
    let vps_peer =
        std::env::var(cellmembrane_types::service::ENV_VPS_MESH_PEER).unwrap_or_else(|_| {
            format!(
                "{}:{}",
                cellmembrane_types::service::DEFAULT_VPS_HOST,
                cellmembrane_types::service::DEFAULT_FEDERATION_PORT
            )
        });
    let relay =
        cellmembrane_types::MembraneService::binary_for(cellmembrane_types::ServiceCapability::MeshRelay);

    let script = format!(
        r#"
sleep 3
echo '{{"jsonrpc":"2.0","method":"mesh.init","params":{{"peers":"{vps_peer}"}},"id":1}}' | \
    socat - UNIX-CONNECT:/run/membrane/{relay}.sock 2>/dev/null || echo "mesh.init deferred"
"#
    );
    ssh_exec(ip, &script).await
}

/// Phase 7: Health sweep — verify critical services are responding.
async fn health_sweep(ip: &str) -> Result<String> {
    let script = r#"
        HEALTHY=0
        TOTAL=0
        for sock in /run/membrane/*.sock; do
            [ -S "$sock" ] || continue
            TOTAL=$((TOTAL + 1))
            RESP=$(echo '{"jsonrpc":"2.0","method":"health","id":1}' | socat - UNIX-CONNECT:"$sock" 2>/dev/null)
            if echo "$RESP" | grep -q '"status":"healthy"'; then
                HEALTHY=$((HEALTHY + 1))
            fi
        done
        echo "$HEALTHY/$TOTAL healthy"
    "#;
    ssh_exec(ip, script).await
}

/// Full bootstrap pipeline — takes a fresh droplet from bare OS to operational gate.
pub async fn bootstrap_droplet(droplet: &DropletState, gate_name: &str) -> ProvisionOutcome {
    let ip = match &droplet.ip {
        Some(ip) => ip.clone(),
        None => {
            return ProvisionOutcome {
                success: false,
                droplet: Some(droplet.clone()),
                message: "droplet has no IP — cannot bootstrap".into(),
                phases: Vec::new(),
            };
        }
    };

    let mut phases: Vec<String> = Vec::new();

    eprintln!("provision: waiting for SSH on {ip}...");
    if let Err(e) = wait_for_ssh(&ip).await {
        return ProvisionOutcome {
            success: false,
            droplet: Some(droplet.clone()),
            message: format!("SSH unavailable: {e}"),
            phases,
        };
    }
    phases.push("ssh: ready".into());

    eprintln!("provision: hardening...");
    match harden(&ip).await {
        Ok(_) => phases.push("harden: done".into()),
        Err(e) => {
            phases.push(format!("harden: FAIL — {e}"));
            return ProvisionOutcome {
                success: false,
                droplet: Some(droplet.clone()),
                message: "hardening failed".into(),
                phases,
            };
        }
    }

    eprintln!("provision: creating directories...");
    match setup_directories(&ip).await {
        Ok(_) => phases.push("directories: done".into()),
        Err(e) => phases.push(format!("directories: FAIL — {e}")),
    }

    eprintln!("provision: deploying binaries...");
    match deploy_binaries(&ip).await {
        Ok(detail) => phases.push(format!("binaries: {detail}")),
        Err(e) => {
            phases.push(format!("binaries: FAIL — {e}"));
            return ProvisionOutcome {
                success: false,
                droplet: Some(droplet.clone()),
                message: "binary deployment failed".into(),
                phases,
            };
        }
    }

    eprintln!("provision: installing systemd units...");
    match install_systemd_units(&ip, gate_name).await {
        Ok(_) => phases.push("systemd: installed".into()),
        Err(e) => phases.push(format!("systemd: FAIL — {e}")),
    }

    eprintln!("provision: starting services...");
    match start_services(&ip).await {
        Ok(_) => phases.push("services: started".into()),
        Err(e) => phases.push(format!("services: FAIL — {e}")),
    }

    eprintln!("provision: joining mesh...");
    match join_mesh(&ip).await {
        Ok(detail) => phases.push(format!("mesh: {detail}")),
        Err(e) => phases.push(format!("mesh: deferred — {e}")),
    }

    eprintln!("provision: health sweep...");
    match health_sweep(&ip).await {
        Ok(detail) => phases.push(format!("health: {detail}")),
        Err(e) => phases.push(format!("health: FAIL — {e}")),
    }

    // Register as remote canary for failover discovery
    let primals: Vec<String> = crate::plasmid::nucleus_primals()
        .into_iter()
        .map(Into::into)
        .collect();
    crate::plasmid::canary::register_remote_canary(gate_name, &ip, Some(droplet.id), primals);
    phases.push("registry: remote canary registered".into());

    ProvisionOutcome {
        success: true,
        droplet: Some(droplet.clone()),
        message: format!("bootstrap complete for {gate_name} at {ip}"),
        phases,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_fails_without_ip() {
        let droplet = DropletState {
            id: 1,
            name: "test".into(),
            status: "active".into(),
            ip: None,
            region: "nyc1".into(),
            profile: "canary-fieldmouse".into(),
            created_at: "2026-06-12T00:00:00Z".into(),
        };

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = rt.block_on(bootstrap_droplet(&droplet, "test-canary"));
        assert!(!result.success);
        assert!(result.message.contains("no IP"));
    }
}
