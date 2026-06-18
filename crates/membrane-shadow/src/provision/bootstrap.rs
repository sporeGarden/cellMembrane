// SPDX-License-Identifier: AGPL-3.0-or-later
//! Post-provision bootstrap — SSH into a fresh droplet and bring it to operational state.
//!
//! Orchestrates: hardening -> directory setup -> binary deployment -> systemd install ->
//! gate.bootstrap -> mesh join -> health sweep.

use super::{DropletState, ProvisionOutcome};
use crate::error::{Result, ShadowError};
use tracing::{error, info, warn};

const SSH_RETRY_DELAY_SECS: u64 = 10;
const SSH_MAX_RETRIES: u32 = 12;

fn provision_ssh_user() -> String {
    std::env::var(cellmembrane_types::service::ENV_PROVISION_SSH_USER)
        .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_PROVISION_SSH_USER.into())
}

/// Run a command on the remote host via SSH with retry logic for fresh droplets.
pub(super) async fn ssh_exec(ip: &str, command: &str) -> Result<String> {
    let user = provision_ssh_user();
    let output = tokio::process::Command::new("ssh")
        .args([
            "-o",
            "ConnectTimeout=10",
            "-o",
            "BatchMode=yes",
            "-o",
            "StrictHostKeyChecking=accept-new",
            &format!("{user}@{ip}"),
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
        warn!(
            ip,
            attempt = attempt + 1,
            max_retries = SSH_MAX_RETRIES,
            "SSH not ready"
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
    let base = std::env::var(cellmembrane_types::service::ENV_INSTALL_BASE)
        .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_INSTALL_BASE.into());
    let relay_binary = cellmembrane_types::MembraneService::binary_for(
        cellmembrane_types::ServiceCapability::TurnServer,
    );
    let config_dir = cellmembrane_types::service::DEFAULT_CONFIG_DIR;
    let script = format!(
        r#"
        mkdir -p {base} /run/membrane /opt/ecoPrimals \
                 {base}/sandbox /run/membrane/sandbox \
                 {base}/canary /run/membrane/canary \
                 /var/lib/membrane/{relay_binary} {config_dir}
        echo "directories created"
    "#
    );
    ssh_exec(ip, &script).await
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
                &format!("{}@{ip}:{remote_path}", provision_ssh_user()),
            ])
            .output()
            .await;

        match scp_result {
            Ok(output) if output.status.success() => deployed += 1,
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                error!(primal, stderr = %stderr, "SCP failed");
            }
            Err(e) => error!(primal, error = %e, "SCP error"),
        }
    }

    ssh_exec(
        ip,
        &format!(
            "chmod +x {}/*",
            cellmembrane_types::service::DEFAULT_INSTALL_BASE
        ),
    )
    .await?;

    Ok(format!("{deployed}/{} binaries deployed", primals.len()))
}

/// Generate systemd unit content for the Tower atomic services.
fn generate_systemd_units(gate_name: &str) -> (String, String, String) {
    let spine = cellmembrane_types::MembraneService::binary_for(
        cellmembrane_types::ServiceCapability::CryptoSigner,
    );
    let relay = cellmembrane_types::MembraneService::binary_for(
        cellmembrane_types::ServiceCapability::MeshRelay,
    );
    let spine_upper = spine.to_uppercase();
    let relay_upper = relay.to_uppercase();
    let federation_port = cellmembrane_types::service::DEFAULT_FEDERATION_PORT;
    let vps_peer =
        std::env::var(cellmembrane_types::service::ENV_VPS_MESH_PEER).unwrap_or_else(|_| {
            format!(
                "{}:{}",
                cellmembrane_types::service::DEFAULT_VPS_HOST,
                cellmembrane_types::service::DEFAULT_FEDERATION_PORT
            )
        });
    let hub_id = std::env::var(cellmembrane_types::service::ENV_MESH_HUB_ID)
        .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_MESH_HUB_ID.into());
    let install_base = std::env::var(cellmembrane_types::service::ENV_INSTALL_BASE)
        .unwrap_or_else(|_| cellmembrane_types::service::DEFAULT_INSTALL_BASE.into());

    let bearer_unit = format!(
        r"[Unit]
Description={spine} Crypto Spine (Membrane Tower)
After=network-online.target
Wants=network-online.target
StartLimitIntervalSec=60
StartLimitBurst=5

[Service]
Type=simple
ExecStart={install_base}/{spine} server --socket /run/membrane/{spine}.sock --audit-dir /var/lib/membrane/{spine}
Environment={spine_upper}_NODE_ID={gate_name}
Environment={spine_upper}_LOG_LEVEL=info
Restart=always
RestartSec=3
MemoryMax=64M

[Install]
WantedBy=multi-user.target
"
    );

    let bind_all = cellmembrane_types::service::BIND_ALL;
    let relay_unit = format!(
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
ExecStart={install_base}/{relay} server \
    --socket /run/membrane/{relay}.sock \
    --security-socket /run/membrane/{spine}.sock \
    --federation-port {federation_port} \
    --bind {bind_all} \
    --dark-forest \
    --pid-dir /run/membrane
Environment={relay_upper}_NODE_ID={gate_name}
Environment={relay_upper}_LOG_LEVEL=info
Environment={relay_upper}_DARK_FOREST=true
Environment={relay_upper}_SECURITY_PROVIDER={spine}
Environment={relay_upper}_PID_DIR=/run/membrane
Environment={relay_upper}_FEDERATION_PORT={federation_port}
Environment={relay_upper}_FEDERATION_ENABLED=true
Environment={relay_upper}_PEERS={hub_id}@{vps_peer}
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
ExecStart={install_base}/%i server --socket /run/membrane/%i.sock --security-socket /run/membrane/{spine}.sock --pid-dir /run/membrane
Restart=always
RestartSec=5
MemoryMax=128M

[Install]
WantedBy=multi-user.target
"
    );

    (bearer_unit, relay_unit, nucleus_template)
}

/// Phase 4: Install systemd service units for Tower atomic.
///
/// Unit names and paths are derived from the service registry — the crypto spine
/// and mesh relay are discovered by capability, not hardcoded by name.
/// Non-Tower primals get individual units based on their `ServerContract`.
async fn install_systemd_units(ip: &str, gate_name: &str) -> Result<String> {
    let spine = cellmembrane_types::MembraneService::binary_for(
        cellmembrane_types::ServiceCapability::CryptoSigner,
    );
    let relay = cellmembrane_types::MembraneService::binary_for(
        cellmembrane_types::ServiceCapability::MeshRelay,
    );
    let (bearer_unit, relay_unit, _nucleus_template) = generate_systemd_units(gate_name);

    let socket_base = cellmembrane_types::service::DEFAULT_SOCKET_BASE;
    let spine_socket = format!("{socket_base}/{spine}.sock");

    let mut primal_units = String::new();
    for svc in cellmembrane_types::MembraneService::all() {
        if !svc.is_primal || svc.binary == spine || svc.binary == relay {
            continue;
        }
        let socket_path = format!("{socket_base}/{}.sock", svc.binary);
        let exec_start = svc
            .server_contract
            .exec_args(svc.binary, &socket_path, &spine_socket);
        let unit = format!(
            r"
cat > /etc/systemd/system/{bin}-membrane.service << 'UNIT'
[Unit]
Description={bin} NUCLEUS Primal (Membrane)
After={spine}-membrane.service {relay}-membrane.service
Wants={relay}-membrane.service

[Service]
Type=simple
ExecStart={exec_start}
Restart=always
RestartSec=5
MemoryMax=128M

[Install]
WantedBy=multi-user.target
UNIT
",
            bin = svc.binary,
        );
        primal_units.push_str(&unit);
    }

    let install_script = format!(
        r#"
cat > /etc/systemd/system/{spine}-membrane.service << 'UNIT'
{bearer_unit}
UNIT

cat > /etc/systemd/system/{relay}-membrane.service << 'UNIT'
{relay_unit}
UNIT

{primal_units}

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
/// dedicated systemd units. Remaining NUCLEUS primals are started via their
/// individual units, discovered from the service registry rather than hardcoded.
async fn start_services(ip: &str) -> Result<String> {
    let spine = cellmembrane_types::MembraneService::binary_for(
        cellmembrane_types::ServiceCapability::CryptoSigner,
    );
    let relay = cellmembrane_types::MembraneService::binary_for(
        cellmembrane_types::ServiceCapability::MeshRelay,
    );

    let nucleus_others: Vec<&str> = crate::plasmid::nucleus_primals()
        .into_iter()
        .filter(|p| *p != spine && *p != relay)
        .collect();
    let enable_cmds: String = nucleus_others
        .iter()
        .map(|p| format!("systemctl enable --now '{p}-membrane' 2>/dev/null || true"))
        .collect::<Vec<_>>()
        .join("\n        ");

    let script = format!(
        r"
        systemctl start {spine}-membrane
        sleep 2
        systemctl start {relay}-membrane
        sleep 2

        {enable_cmds}

        echo 'services started'
    "
    );
    ssh_exec(ip, &script).await
}

/// Phase 6: Join the mesh by peering with main production VPS.
async fn join_mesh(ip: &str, gate_name: &str) -> Result<String> {
    let vps_peer =
        std::env::var(cellmembrane_types::service::ENV_VPS_MESH_PEER).unwrap_or_else(|_| {
            format!(
                "{}:{}",
                cellmembrane_types::service::DEFAULT_VPS_HOST,
                cellmembrane_types::service::DEFAULT_FEDERATION_PORT
            )
        });
    let relay = cellmembrane_types::MembraneService::binary_for(
        cellmembrane_types::ServiceCapability::MeshRelay,
    );

    let script = format!(
        r#"
sleep 3
echo '{{"jsonrpc":"2.0","method":"mesh.init","params":{{"node_id":"{gate_name}","peers":["{vps_peer}"]}},"id":1}}' | \
    socat - UNIX-CONNECT:/run/membrane/{relay}.sock 2>/dev/null || echo "mesh.init deferred"
"#
    );
    ssh_exec(ip, &script).await
}

/// Phase 6b: Verify federation mesh enrollment after join.
pub(super) async fn verify_federation(ip: &str) -> Result<String> {
    let relay = cellmembrane_types::MembraneService::binary_for(
        cellmembrane_types::ServiceCapability::MeshRelay,
    );

    let script = format!(
        r#"
sleep 3
RESP=$(echo '{{"jsonrpc":"2.0","method":"mesh.status","params":{{}},"id":1}}' | \
    socat - UNIX-CONNECT:/run/membrane/{relay}.sock 2>/dev/null)
if [ -z "$RESP" ]; then
    echo "federation: no response from relay"
    exit 0
fi
PEERS=$(echo "$RESP" | grep -o '"reachable_peers":[0-9]*' | grep -o '[0-9]*')
CONNS=$(echo "$RESP" | grep -o '"active_connections":[0-9]*' | grep -o '[0-9]*')
echo "federation: peers=${{PEERS:-0}} connections=${{CONNS:-0}}"
"#
    );
    ssh_exec(ip, &script).await
}

/// Phase 7: Health sweep — verify critical services are responding.
pub(super) async fn health_sweep(ip: &str) -> Result<String> {
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

    info!(ip = %ip, "waiting for SSH");
    if let Err(e) = wait_for_ssh(&ip).await {
        return ProvisionOutcome {
            success: false,
            droplet: Some(droplet.clone()),
            message: format!("SSH unavailable: {e}"),
            phases,
        };
    }
    phases.push("ssh: ready".into());

    info!("hardening");
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

    info!("creating directories");
    match setup_directories(&ip).await {
        Ok(_) => phases.push("directories: done".into()),
        Err(e) => phases.push(format!("directories: FAIL — {e}")),
    }

    info!("deploying binaries");
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

    info!("installing systemd units");
    match install_systemd_units(&ip, gate_name).await {
        Ok(_) => phases.push("systemd: installed".into()),
        Err(e) => phases.push(format!("systemd: FAIL — {e}")),
    }

    info!("starting services");
    match start_services(&ip).await {
        Ok(_) => phases.push("services: started".into()),
        Err(e) => phases.push(format!("services: FAIL — {e}")),
    }

    info!("joining mesh");
    match join_mesh(&ip, gate_name).await {
        Ok(detail) => phases.push(format!("mesh: {detail}")),
        Err(e) => phases.push(format!("mesh: deferred — {e}")),
    }

    info!("verifying federation");
    match verify_federation(&ip).await {
        Ok(detail) => phases.push(detail.trim().to_string()),
        Err(e) => phases.push(format!("federation: verify failed — {e}")),
    }

    info!("health sweep");
    match health_sweep(&ip).await {
        Ok(detail) => phases.push(format!("health: {detail}")),
        Err(e) => phases.push(format!("health: FAIL — {e}")),
    }

    super::verify::finalize_bootstrap(droplet, gate_name, &ip, &mut phases).await;

    ProvisionOutcome {
        success: true,
        droplet: Some(droplet.clone()),
        message: format!("bootstrap complete for {gate_name} at {ip}"),
        phases,
    }
}

/// Write a gate identity file at `/etc/membrane/gate_identity` for remote verification.
pub(super) async fn write_gate_identity(
    ip: &str,
    gate_name: &str,
    profile: &str,
) -> Result<String> {
    let now = chrono::Utc::now().to_rfc3339();
    let script = format!(
        r#"
mkdir -p /etc/membrane
cat > /etc/membrane/gate_identity << 'IDENTITY'
[gate]
name = "{gate_name}"
profile = "{profile}"
provisioned_at = "{now}"
arch = "x86_64-unknown-linux-musl"
IDENTITY
echo "identity written"
"#
    );
    ssh_exec(ip, &script).await
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
