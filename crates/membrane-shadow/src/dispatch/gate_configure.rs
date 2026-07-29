// SPDX-License-Identifier: AGPL-3.0-or-later

//! `gate.configure` / `gate.apply` — declarative service config generation.
//!
//! Builds `ServiceSpec` for every primal in a gate's composition,
//! renders to the detected init system, and optionally installs.

use crate::{ShadowOutcome, gate};

/// Resolve the gate name from CLI args or local identity.
/// Skips flag values (e.g. the `K=V` after `--env`).
pub(super) fn resolve_gate_name(args: &[&str]) -> String {
    let mut skip_next = false;
    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }
        if *arg == "--env" {
            skip_next = true;
            continue;
        }
        if !arg.starts_with('-') {
            return arg.to_string();
        }
    }
    gate::resolve_local_gate_identity()
}

/// Parse `--env K=V` flags from CLI args into key-value pairs.
pub(super) fn parse_env_overrides(args: &[&str]) -> Vec<(String, String)> {
    let mut envs = Vec::new();
    let mut iter = args.iter();
    while let Some(&arg) = iter.next() {
        if arg == "--env"
            && let Some(&val) = iter.next()
            && let Some((k, v)) = val.split_once('=')
        {
            envs.push((k.to_string(), v.to_string()));
        }
    }
    envs
}

/// Build `ServiceSpec` entries for all primals in a gate's composition.
fn build_service_specs(
    gate_name: &str,
    env_overrides: &[(String, String)],
) -> crate::Result<(Vec<cellmembrane_types::ServiceSpec>, String)> {
    let workspace_root = crate::temporal::resolve_workspace_root()?;
    let manifest = crate::manifest::load_from_workspace(&workspace_root)?;

    let comp_name = manifest
        .gates
        .get(gate_name)
        .and_then(|g| g.composition.as_deref())
        .unwrap_or("full");

    let primals: Vec<String> = manifest.gate_composition(gate_name).map_or_else(
        || {
            crate::plasmid::nucleus_primals()
                .into_iter()
                .map(Into::into)
                .collect()
        },
        |p| p.primals.clone(),
    );

    let install_base = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_INSTALL_BASE,
        cellmembrane_types::service::DEFAULT_INSTALL_BASE,
    );
    let socket_base = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_SOCKET_BASE,
        cellmembrane_types::service::DEFAULT_SOCKET_BASE,
    );
    let config_dir = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_CONFIG_DIR,
        cellmembrane_types::service::DEFAULT_CONFIG_DIR,
    );
    let paths = cellmembrane_types::service::ServicePaths::new(&install_base, &socket_base);
    let security_socket = gate::nucleus::resolve_security_socket(&paths);

    let mut specs = Vec::new();
    for primal_name in &primals {
        let Some(svc) = cellmembrane_types::MembraneService::for_binary(primal_name) else {
            continue;
        };
        let mut spec = cellmembrane_types::ServiceSpec::from_membrane_service(
            svc,
            &install_base,
            &socket_base,
            &security_socket,
            &config_dir,
        );
        let extra = gate::nucleus::extra_exec_args(svc);
        if !extra.is_empty() {
            spec.extra_args = extra;
        }
        for (k, v) in env_overrides {
            spec.environment.push((k.clone(), v.clone()));
        }
        specs.push(spec);
    }

    Ok((specs, comp_name.to_string()))
}

/// `gate.configure` — preview service configs for a gate's composition.
pub(super) fn dispatch_configure(args: &[&str]) -> crate::Result<ShadowOutcome> {
    use std::fmt::Write;

    let gate_name = resolve_gate_name(args);
    let env_overrides = parse_env_overrides(args);
    let (specs, comp_name) = build_service_specs(&gate_name, &env_overrides)?;

    let init = cellmembrane_types::InitSystem::detect();
    let mut output = format!(
        "gate.configure: {gate_name} (composition: {comp_name}, init: {init})\n\
         --- {} service(s) ---\n",
        specs.len()
    );

    for spec in &specs {
        let config = if init == cellmembrane_types::InitSystem::Launchd {
            spec.to_launchd_plist()
        } else {
            spec.to_systemd_unit()
        };
        let _ = write!(output, "\n### {}\n{config}", spec.binary);
    }

    Ok(ShadowOutcome::ok_with(
        format!(
            "gate.configure: {gate_name} — {} services ({comp_name}, {init})",
            specs.len()
        ),
        serde_json::json!({
            "gate": gate_name,
            "composition": comp_name,
            "init_system": init.to_string(),
            "services": specs.iter().map(|s| &s.binary).collect::<Vec<_>>(),
            "preview": output,
        }),
    ))
}

/// `gate.apply` — write service configs to the init system config directory.
pub(super) async fn dispatch_apply(args: &[&str]) -> crate::Result<ShadowOutcome> {
    let gate_name = resolve_gate_name(args);
    let env_overrides = parse_env_overrides(args);
    let (specs, comp_name) = build_service_specs(&gate_name, &env_overrides)?;

    let init = cellmembrane_types::InitSystem::detect();
    let mut installed = 0usize;
    let mut failed = 0usize;
    let mut details = Vec::new();

    match init {
        cellmembrane_types::InitSystem::Systemd => {
            let unit_dir = cellmembrane_types::service::SYSTEMD_UNIT_DIR;
            for spec in &specs {
                let unit_name = &spec.systemd_unit;
                let unit_path = format!("{unit_dir}/{unit_name}");
                let content = spec.to_systemd_unit();

                match tokio::fs::write(&unit_path, &content).await {
                    Ok(()) => {
                        installed += 1;
                        details.push(format!("  installed: {unit_name}"));
                    }
                    Err(e) => {
                        failed += 1;
                        details.push(format!("  FAILED: {unit_name} — {e}"));
                    }
                }
            }
            if installed > 0 {
                gate::nucleus::systemctl(&["daemon-reload"]);
            }
        }
        cellmembrane_types::InitSystem::Launchd => {
            let plist_dir = "/Library/LaunchDaemons";
            for spec in &specs {
                let plist_name = format!("eco.primals.{}.plist", spec.binary);
                let plist_path = format!("{plist_dir}/{plist_name}");
                let content = spec.to_launchd_plist();

                match tokio::fs::write(&plist_path, &content).await {
                    Ok(()) => {
                        installed += 1;
                        details.push(format!("  installed: {plist_name}"));
                    }
                    Err(e) => {
                        failed += 1;
                        details.push(format!("  FAILED: {plist_name} — {e}"));
                    }
                }
            }
        }
        _ => {
            details.push("  bare mode: service configs written as reference only".into());
            let config_dir = cellmembrane_types::service::env_or(
                cellmembrane_types::service::ENV_CONFIG_DIR,
                cellmembrane_types::service::DEFAULT_CONFIG_DIR,
            );
            let services_dir = format!("{config_dir}/services");
            let _ = tokio::fs::create_dir_all(&services_dir).await;
            for spec in &specs {
                let toml_name = format!("{}.toml", spec.binary);
                let toml_path = format!("{services_dir}/{toml_name}");
                let content = format!(
                    "# Auto-generated by gate.apply (bare mode)\n\
                     binary = \"{}\"\n\
                     exec_start = \"{}{}\"\n",
                    spec.binary, spec.exec_start, spec.extra_args
                );
                match tokio::fs::write(&toml_path, &content).await {
                    Ok(()) => {
                        installed += 1;
                        details.push(format!("  wrote: {toml_name}"));
                    }
                    Err(e) => {
                        failed += 1;
                        details.push(format!("  FAILED: {toml_name} — {e}"));
                    }
                }
            }
        }
    }

    let ok = failed == 0;
    let msg = format!(
        "gate.apply: {gate_name} ({comp_name}, {init}) — {installed} installed, {failed} failed\n{}",
        details.join("\n")
    );

    Ok(ShadowOutcome {
        ok,
        message: msg,
        data: Some(serde_json::json!({
            "gate": gate_name,
            "composition": comp_name,
            "init_system": init.to_string(),
            "installed": installed,
            "failed": failed,
        })),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_gate_name_from_args() {
        let name = resolve_gate_name(&["eastGate"]);
        assert_eq!(name, "eastGate");
    }

    #[test]
    fn resolve_gate_name_skips_flags() {
        let name = resolve_gate_name(&["--env", "K=V", "westGate"]);
        assert_eq!(name, "westGate");
    }

    #[test]
    fn parse_env_overrides_extracts_pairs() {
        let envs = parse_env_overrides(&["--env", "FOO=bar", "--env", "BAZ=qux"]);
        assert_eq!(envs.len(), 2);
        assert_eq!(envs[0], ("FOO".into(), "bar".into()));
        assert_eq!(envs[1], ("BAZ".into(), "qux".into()));
    }

    #[test]
    fn parse_env_overrides_empty_when_no_flags() {
        let envs = parse_env_overrides(&["eastGate"]);
        assert!(envs.is_empty());
    }

    #[test]
    fn configure_generates_specs_for_local_gate() {
        let result = dispatch_configure(&[]);
        if let Ok(outcome) = result {
            assert!(outcome.ok);
            let data = outcome.data.unwrap();
            let services = data["services"].as_array().unwrap();
            assert!(!services.is_empty(), "should have at least one service");
            assert!(data["init_system"].is_string());
        }
    }

    #[test]
    fn configure_with_env_overrides() {
        let result = dispatch_configure(&["--env", "MEMBRANE_LOG=debug"]);
        if let Ok(outcome) = result {
            assert!(outcome.ok);
            let preview = outcome.data.unwrap()["preview"]
                .as_str()
                .unwrap()
                .to_string();
            assert!(
                preview.contains("MEMBRANE_LOG=debug"),
                "env override should appear in preview"
            );
        }
    }
}
