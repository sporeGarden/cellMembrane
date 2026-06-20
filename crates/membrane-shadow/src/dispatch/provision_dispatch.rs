// SPDX-License-Identifier: AGPL-3.0-or-later

//! Provision domain dispatch — cloud gate lifecycle (create, status, destroy, verify).

use crate::ShadowOutcome;
use crate::cli;
use tracing::info;

#[cfg(feature = "http")]
pub(super) async fn dispatch_provision(args: &[&str]) -> crate::Result<ShadowOutcome> {
    use crate::provision::{self, ProvisionRequest, digitalocean};

    let provider_str = cli::extract_flag_value(args, "--provider").unwrap_or("digitalocean");
    let provider: provision::Provider = provider_str
        .parse()
        .map_err(|e: String| crate::ShadowError::Config(e))?;

    if matches!(provider, provision::Provider::Hetzner) {
        return Ok(ShadowOutcome::fail(
            "Hetzner provider not yet implemented — use --provider digitalocean",
        ));
    }

    let name = cli::extract_flag_value(args, "--name")
        .unwrap_or("membrane-canary")
        .to_string();
    let region = cli::extract_flag_value(args, "--region")
        .unwrap_or("nyc1")
        .to_string();
    let size = cli::extract_flag_value(args, "--size")
        .unwrap_or("s-1vcpu-2gb")
        .to_string();
    let profile = cli::extract_flag_value(args, "--profile")
        .unwrap_or("canary-fieldmouse")
        .to_string();
    let dry_run = args.contains(&"--dry-run");

    if dry_run {
        return Ok(ShadowOutcome::ok(format!(
            "dry-run: would provision {name} ({size}) in {region} with profile {profile}"
        )));
    }

    info!("resolving SSH keys");
    let keys = digitalocean::list_ssh_keys().await?;
    let ssh_key_ids: Vec<String> = keys.iter().map(|k| k.id.to_string()).collect();

    if ssh_key_ids.is_empty() {
        return Ok(ShadowOutcome::fail(
            "no SSH keys found on DO account — add one first",
        ));
    }

    info!(
        name = %name,
        size = %size,
        region = %region,
        ssh_keys = ssh_key_ids.len(),
        "creating droplet"
    );

    let req = ProvisionRequest {
        name: name.clone(),
        region,
        size,
        image: "debian-12-x64".into(),
        profile: profile.clone(),
        ssh_keys: ssh_key_ids,
        tags: vec!["membrane".into(), "canary".into(), "ecoprimals".into()],
    };

    let droplet = digitalocean::create_droplet(&req).await?;
    info!(
        name = %droplet.name,
        id = droplet.id,
        "droplet created, waiting for active"
    );

    let active = digitalocean::wait_until_active(droplet.id, &profile).await?;
    let ip = active.ip.as_deref().unwrap_or("").to_string();
    info!(ip = %ip, "droplet active");

    info!("bootstrapping");
    let outcome = provision::bootstrap::bootstrap_droplet(&active, &name).await;

    let msg = format!(
        "provision: {} (id={}, ip={ip})\n  phases:\n    {}",
        outcome.message,
        active.id,
        outcome.phases.join("\n    ")
    );

    Ok(ShadowOutcome::ok_with(msg, serde_json::to_value(&outcome)?))
}

#[cfg(feature = "http")]
pub(super) async fn dispatch_provision_status(args: &[&str]) -> crate::Result<ShadowOutcome> {
    use crate::plasmid::canary;
    use crate::provision::digitalocean;

    if let Some(id_str) = cli::extract_flag_value(args, "--id") {
        let id: u64 = id_str.parse().map_err(|e| {
            crate::ShadowError::Config(format!("invalid droplet id '{id_str}': {e}"))
        })?;
        let state = digitalocean::get_droplet(id).await?;
        let msg = format!(
            "{} (id={}) — {} @ {}",
            state.name,
            state.id,
            state.status,
            state.ip.as_deref().unwrap_or("no-ip")
        );
        Ok(ShadowOutcome::ok_with(msg, serde_json::to_value(&state)?))
    } else {
        let droplets = digitalocean::list_membrane_droplets().await?;
        let remote_canaries = canary::list_remote_canaries().await;

        if droplets.is_empty() && remote_canaries.is_empty() {
            return Ok(ShadowOutcome::ok("no membrane droplets found"));
        }

        let mut lines: Vec<String> = droplets
            .iter()
            .map(|d| {
                format!(
                    "  {} (id={}) — {} @ {} [{}]",
                    d.name,
                    d.id,
                    d.status,
                    d.ip.as_deref().unwrap_or("no-ip"),
                    d.region
                )
            })
            .collect();

        if !remote_canaries.is_empty() {
            lines.push(String::new());
            lines.push("  remote canary registry:".into());
            for rc in &remote_canaries {
                lines.push(format!(
                    "    {} @ {} (id={:?}, primals={})",
                    rc.gate_name,
                    rc.ip,
                    rc.droplet_id,
                    rc.primals.len()
                ));
            }
        }

        let msg = format!(
            "{} droplet(s), {} registered canary(ies):\n{}",
            droplets.len(),
            remote_canaries.len(),
            lines.join("\n")
        );
        Ok(ShadowOutcome::ok_with(
            msg,
            serde_json::to_value(&droplets)?,
        ))
    }
}

#[cfg(feature = "http")]
pub(super) async fn dispatch_provision_destroy(args: &[&str]) -> crate::Result<ShadowOutcome> {
    use crate::plasmid::canary;
    use crate::provision::digitalocean;

    let id_str = cli::extract_flag_value(args, "--id").ok_or_else(|| {
        crate::ShadowError::Config("gate.provision.destroy requires --id <droplet-id>".into())
    })?;
    let id: u64 = id_str
        .parse()
        .map_err(|e| crate::ShadowError::Config(format!("invalid droplet id '{id_str}': {e}")))?;

    let gate_name = cli::extract_flag_value(args, "--gate");

    let dry_run = args.contains(&"--dry-run");
    if dry_run {
        return Ok(ShadowOutcome::ok(format!(
            "dry-run: would destroy droplet id={id}"
        )));
    }

    digitalocean::destroy_droplet(id).await?;

    if let Some(name) = gate_name {
        canary::deregister_remote_canary(name).await;
    } else {
        let registry = canary::load_remote_canaries().await;
        if let Some(entry) = registry.entries.iter().find(|e| e.droplet_id == Some(id)) {
            canary::deregister_remote_canary(&entry.gate_name).await;
        }
    }

    Ok(ShadowOutcome::ok(format!("DESTROYED droplet id={id}")))
}

#[cfg(feature = "http")]
pub(super) async fn dispatch_provision_verify(args: &[&str]) -> crate::Result<ShadowOutcome> {
    use crate::plasmid::canary;
    use crate::provision::verify;

    let ip = cli::extract_flag_value(args, "--ip");
    let gate = cli::extract_flag_value(args, "--gate");

    let (target_ip, gate_name) = match (ip, gate) {
        (Some(ip), name) => (ip.to_string(), name.unwrap_or("unknown").to_string()),
        (None, Some(name)) => {
            let registry = canary::load_remote_canaries().await;
            let entry = registry
                .entries
                .iter()
                .find(|e| e.gate_name == name)
                .ok_or_else(|| {
                    crate::ShadowError::Config(format!(
                        "gate '{name}' not found in remote canary registry"
                    ))
                })?;
            (entry.ip.clone(), name.to_string())
        }
        (None, None) => {
            return Err(crate::ShadowError::Config(
                "gate.provision.verify requires --ip <addr> or --gate <name>".into(),
            ));
        }
    };

    let profile = cli::extract_flag_value(args, "--profile");
    let outcome = verify::verify_remote_gate(&target_ip, &gate_name, profile).await;
    let summary = outcome
        .phases
        .iter()
        .map(|p| format!("  {p}"))
        .collect::<Vec<_>>()
        .join("\n");

    let status = if outcome.success { "PASS" } else { "FAIL" };
    let msg = format!("gate.provision.verify [{status}] {gate_name}@{target_ip}\n{summary}");

    if outcome.success {
        Ok(ShadowOutcome::ok(msg))
    } else {
        Ok(ShadowOutcome::fail(msg))
    }
}
