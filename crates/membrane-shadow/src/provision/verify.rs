// SPDX-License-Identifier: AGPL-3.0-or-later

//! Remote gate verification — health, federation, canary, and identity checks.
//!
//! Used by `gate.provision.verify` to validate a remote gate without needing
//! physical access. Profile-based expectations determine pass/fail criteria.

use super::{DropletState, ProvisionOutcome};

use super::bootstrap::{health_sweep, ssh_exec, verify_federation, write_gate_identity};

/// Remotely verify a provisioned gate — checks health, federation, and canary state.
///
/// When `profile` is provided, health expectations are derived from the composition tier.
pub async fn verify_remote_gate(
    ip: &str,
    gate_name: &str,
    profile: Option<&str>,
) -> ProvisionOutcome {
    let mut phases: Vec<String> = Vec::new();

    let expected_healthy: u32 = profile
        .and_then(cellmembrane_types::MembraneComposition::parse_name)
        .map_or(0, |comp| {
            u32::try_from(comp.spec().primals.len()).unwrap_or(u32::MAX)
        });
    if expected_healthy > 0 {
        phases.push(format!(
            "profile: expects {expected_healthy} healthy ({})",
            profile.unwrap_or("unknown")
        ));
    }

    if ssh_exec(ip, "echo ready").await.is_err() {
        return ProvisionOutcome {
            success: false,
            droplet: None,
            message: format!("SSH unreachable at {ip}"),
            phases,
        };
    }
    phases.push("ssh: reachable".into());

    let health_count = match health_sweep(ip).await {
        Ok(detail) => {
            let count = parse_health_count(&detail);
            phases.push(format!("health: {detail}"));
            count
        }
        Err(e) => {
            phases.push(format!("health: FAIL — {e}"));
            0
        }
    };

    let has_federation = match verify_federation(ip).await {
        Ok(detail) => {
            let trimmed = detail.trim().to_string();
            let has_peers = trimmed.contains("peers=1") || trimmed.contains("peers=2");
            phases.push(trimmed);
            has_peers
        }
        Err(e) => {
            phases.push(format!("federation: {e}"));
            false
        }
    };

    let canary_result = ssh_exec(
        ip,
        &format!(
            r#"
STALE=0
for bin in {install_base}/*; do
    [ -x "$bin" ] || continue
    NAME=$(basename "$bin")
    SOCK="{socket_base}/${{NAME}}.sock"
    [ -S "$SOCK" ] || {{ STALE=$((STALE + 1)); continue; }}
    RESP=$(echo '{{"jsonrpc":"2.0","method":"health","id":1}}' | socat - UNIX-CONNECT:"$SOCK" 2>/dev/null)
    echo "$RESP" | grep -q '"status":"healthy"' || STALE=$((STALE + 1))
done
echo "canary.audit: stale=$STALE"
"#,
            install_base = cellmembrane_types::service::DEFAULT_INSTALL_BASE,
            socket_base = cellmembrane_types::service::DEFAULT_SOCKET_BASE
        ),
    )
    .await;
    match canary_result {
        Ok(detail) => phases.push(detail.trim().to_string()),
        Err(e) => phases.push(format!("canary.audit: FAIL — {e}")),
    }

    let identity_result = ssh_exec(
        ip,
        "cat /etc/membrane/gate_identity 2>/dev/null || echo 'no identity file'",
    )
    .await;
    match identity_result {
        Ok(detail) => phases.push(format!("identity: {}", detail.trim())),
        Err(e) => phases.push(format!("identity: {e}")),
    }

    let health_ok = expected_healthy == 0 || health_count >= expected_healthy;
    let success = has_federation && health_ok && !phases.iter().any(|p| p.contains("health: FAIL"));

    if !health_ok {
        phases.push(format!(
            "EXPECTATION MISS: health {health_count}/{expected_healthy} (profile requires {expected_healthy})"
        ));
    }

    ProvisionOutcome {
        success,
        droplet: None,
        message: format!("verify complete for {gate_name} at {ip}"),
        phases,
    }
}

/// Parse healthy count from a health sweep detail string like "13/13 healthy".
fn parse_health_count(detail: &str) -> u32 {
    detail
        .split('/')
        .next()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .unwrap_or(0)
}

/// Register a remote canary and write gate identity during bootstrap.
pub(super) async fn finalize_bootstrap(
    droplet: &DropletState,
    gate_name: &str,
    ip: &str,
    phases: &mut Vec<String>,
) {
    let primals: Vec<String> = crate::plasmid::nucleus_primals()
        .into_iter()
        .map(Into::into)
        .collect();
    crate::plasmid::canary::register_remote_canary(gate_name, ip, Some(droplet.id), primals).await;
    phases.push("registry: remote canary registered".into());

    match write_gate_identity(ip, gate_name, &droplet.profile).await {
        Ok(_) => phases.push("identity: written".into()),
        Err(e) => phases.push(format!("identity: FAIL — {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_health_count_standard_format() {
        assert_eq!(parse_health_count("13/13 healthy"), 13);
        assert_eq!(parse_health_count("5/13 healthy"), 5);
        assert_eq!(parse_health_count("0/13 healthy"), 0);
    }

    #[test]
    fn parse_health_count_edge_cases() {
        assert_eq!(parse_health_count(""), 0);
        assert_eq!(parse_health_count("not a number"), 0);
        assert_eq!(parse_health_count("/13"), 0);
    }

    #[test]
    fn parse_health_count_large_values() {
        assert_eq!(parse_health_count("100/100 services"), 100);
    }

    #[test]
    fn expected_healthy_from_composition() {
        let comp = cellmembrane_types::MembraneComposition::parse_name("nucleus");
        assert!(comp.is_some(), "nucleus should be a known composition");
        let primals = comp.unwrap().spec().primals.len();
        assert!(primals > 0, "nucleus should have primals");
    }
}
