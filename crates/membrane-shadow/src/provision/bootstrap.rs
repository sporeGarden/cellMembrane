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
    cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_PROVISION_SSH_USER,
        cellmembrane_types::service::DEFAULT_PROVISION_SSH_USER,
    )
}

/// Run a command on the remote host via SSH with retry logic for fresh droplets.
pub(super) async fn ssh_exec(ip: &str, command: &str) -> Result<String> {
    let user = provision_ssh_user();
    let (stdout, code) = crate::ssh::exec_on_host(&user, ip, command, 10).await?;
    if code == 0 {
        Ok(stdout)
    } else {
        Err(ShadowError::Ssh(format!(
            "command failed (exit {code}): {stdout}"
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
    Err(ShadowError::Ssh(format!(
        "not available on {ip} after {SSH_MAX_RETRIES} attempts"
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
    let base = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_INSTALL_BASE,
        cellmembrane_types::service::DEFAULT_INSTALL_BASE,
    );
    let relay_binary = cellmembrane_types::MembraneService::binary_for(
        cellmembrane_types::ServiceCapability::TurnServer,
    );
    let config_dir = cellmembrane_types::service::DEFAULT_CONFIG_DIR;
    let eco_root = cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT;
    let script = format!(
        r#"
        mkdir -p {base} /run/membrane {eco_root} \
                 {base}/sandbox /run/membrane/sandbox \
                 {base}/canary /run/membrane/canary \
                 /var/lib/membrane/{relay_binary} {config_dir}
        chmod 755 /run/membrane /run/membrane/sandbox /run/membrane/canary
        echo "directories created"
    "#
    );
    ssh_exec(ip, &script).await
}

/// Phase 3: Deploy binaries from local depot via SCP.
///
/// Scoped to the target gate's composition profile when available.
async fn deploy_binaries(ip: &str, gate_name: &str) -> Result<String> {
    let depot_dir = crate::plasmid::depot::resolve_depot(None)?;
    let arch = crate::plasmid::detect_target_triple();
    let bin_dir = depot_dir.join("primals").join(&arch);

    if !bin_dir.exists() {
        return Err(ShadowError::Config(format!(
            "depot bin dir not found: {}",
            bin_dir.display()
        )));
    }

    let primals = crate::plasmid::resolve_gate_primals(gate_name);
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
        match crate::ssh::scp_to_host(
            &provision_ssh_user(),
            ip,
            &local_path.to_string_lossy(),
            &remote_path,
            10,
        )
        .await
        {
            Ok(()) => deployed += 1,
            Err(e) => error!(primal, error = %e, "SCP failed"),
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
    let vps_peer = crate::manifest::resolve_federation_peer();
    let hub_id = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_MESH_HUB_ID,
        cellmembrane_types::service::DEFAULT_MESH_HUB_ID,
    );
    let install_base = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_INSTALL_BASE,
        cellmembrane_types::service::DEFAULT_INSTALL_BASE,
    );

    let umask = cellmembrane_types::service::DEFAULT_SERVICE_UMASK;
    let rtd_mode = cellmembrane_types::service::DEFAULT_RUNTIME_DIRECTORY_MODE;

    let bearer_unit = format!(
        r"[Unit]
Description={spine} Crypto Spine (Membrane Tower)
After=network-online.target
Wants=network-online.target
StartLimitIntervalSec=60
StartLimitBurst=5

[Service]
Type=simple
UMask={umask}
ExecStart={install_base}/{spine} server --socket /run/membrane/{spine}.sock --audit-dir /var/lib/membrane/{spine}
Environment={spine_upper}_NODE_ID={gate_name}
Environment={spine_upper}_LOG_LEVEL=info
Restart=always
RestartSec=3
RuntimeDirectory=membrane
RuntimeDirectoryMode={rtd_mode}
RuntimeDirectoryPreserve=yes
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
UMask={umask}
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
RuntimeDirectory=membrane
RuntimeDirectoryMode={rtd_mode}
RuntimeDirectoryPreserve=yes
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
UMask={umask}
ExecStart={install_base}/%i server --socket /run/membrane/%i.sock --security-socket /run/membrane/{spine}.sock --pid-dir /run/membrane
Restart=always
RestartSec=5
RuntimeDirectory=membrane
RuntimeDirectoryMode={rtd_mode}
RuntimeDirectoryPreserve=yes
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
async fn start_services(ip: &str, gate_name: &str) -> Result<String> {
    let spine = cellmembrane_types::MembraneService::binary_for(
        cellmembrane_types::ServiceCapability::CryptoSigner,
    );
    let relay = cellmembrane_types::MembraneService::binary_for(
        cellmembrane_types::ServiceCapability::MeshRelay,
    );

    let composition = crate::plasmid::resolve_gate_primals(gate_name);
    let nucleus_others: Vec<&str> = composition
        .iter()
        .map(String::as_str)
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
    let vps_peer = crate::manifest::resolve_federation_peer();
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
    match deploy_binaries(&ip, gate_name).await {
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
    match start_services(&ip, gate_name).await {
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

    let has_failures = phases.iter().any(|p| p.contains("FAIL"));
    ProvisionOutcome {
        success: !has_failures,
        droplet: Some(droplet.clone()),
        message: if has_failures {
            format!("bootstrap PARTIAL for {gate_name} at {ip} — check phases")
        } else {
            format!("bootstrap complete for {gate_name} at {ip}")
        },
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
