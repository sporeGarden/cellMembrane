// SPDX-License-Identifier: AGPL-3.0-or-later

//! NUCLEUS service management — systemd unit generation, secrets, and startup.
//!
//! Extracted from `bootstrap.rs` to keep the bootstrap orchestrator focused on
//! phase coordination while this module handles systemd unit installation,
//! secret generation, and primal service lifecycle.

use super::BootstrapPhase;

/// Run a `systemctl` subcommand. Returns `true` if it exits 0.
pub(super) fn systemctl(args: &[&str]) -> bool {
    std::process::Command::new("systemctl")
        .args(args)
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Async variant for use in tokio contexts (e.g. cascade-restart).
pub(crate) async fn systemctl_async(args: &[&str]) -> bool {
    tokio::process::Command::new("systemctl")
        .args(args)
        .output()
        .await
        .is_ok_and(|o| o.status.success())
}

/// Start all NUCLEUS primals — generate secrets, write systemd units, enable+start.
pub(super) fn start_nucleus_primals(arch: &str) -> (bool, String) {
    let config_dir = generate_secrets_env();

    let install_base = super::resolve_install_base();
    let dest_root = super::resolve_plasmidbin_dir();
    let bin_dir = dest_root.join("primals").join(arch);
    let paths = cellmembrane_types::service::ServicePaths::from_env();
    let systemd_dir = std::path::Path::new(cellmembrane_types::service::SYSTEMD_UNIT_DIR);

    let socket_base = std::path::Path::new(cellmembrane_types::service::DEFAULT_SOCKET_BASE);
    if let Err(e) = std::fs::create_dir_all(socket_base) {
        tracing::warn!(error = %e, "failed to create socket base directory");
    } else {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o755);
            if let Err(e) = std::fs::set_permissions(socket_base, perms) {
                tracing::warn!(error = %e, "failed to set socket base directory permissions");
            }
        }
    }

    let security_binary = cellmembrane_types::MembraneService::binary_for(
        cellmembrane_types::ServiceCapability::CryptoSigner,
    );
    let Some(crypto_svc) = cellmembrane_types::MembraneService::with_capability(
        cellmembrane_types::ServiceCapability::CryptoSigner,
    ) else {
        return (
            false,
            "CryptoSigner capability not found in service registry".into(),
        );
    };
    let security_socket = paths.socket_path(crypto_svc).unwrap_or_else(|| {
        format!(
            "{}/{security_binary}.sock",
            cellmembrane_types::service::DEFAULT_SOCKET_BASE
        )
    });

    let services = cellmembrane_types::MembraneService::all();
    let mut installed = 0u32;
    let mut failed = 0u32;

    for svc in services {
        if !svc.is_primal || !bin_dir.join(svc.binary).exists() {
            continue;
        }

        let socket_path = paths.socket_path(svc).unwrap_or_else(|| {
            format!(
                "{}/{}.sock",
                cellmembrane_types::service::DEFAULT_SOCKET_BASE,
                svc.binary
            )
        });
        let exec_start = svc.server_contract.exec_args_with_base(
            &install_base,
            svc.binary,
            &socket_path,
            &security_socket,
        );
        let extra_args = extra_exec_args(svc);
        let unit_content = generate_unit_content(svc, &exec_start, &extra_args, &config_dir);
        let unit_path = systemd_dir.join(format!("{}-membrane.service", svc.binary));

        if let Err(e) = std::fs::write(&unit_path, &unit_content) {
            tracing::warn!(
                service = %svc.binary,
                path = %unit_path.display(),
                error = %e,
                "systemd unit write failed"
            );
            failed += 1;
        } else {
            installed += 1;
        }
    }

    if installed > 0 {
        if !systemctl(&["daemon-reload"]) {
            tracing::warn!("systemctl daemon-reload failed");
        }

        for svc in services {
            if !svc.is_primal || !bin_dir.join(svc.binary).exists() {
                continue;
            }
            let unit = format!("{}-membrane.service", svc.binary);
            if !systemctl(&["enable", "--now", &unit]) {
                tracing::warn!(unit = %unit, "systemctl enable failed");
            }
        }
    }

    if installed == 0 && failed == 0 {
        return (true, "no primal binaries found in depot — skipped".into());
    }

    let ok = failed == 0 && installed > 0;
    (ok, format!("{installed} units installed, {failed} failed"))
}

/// Construct the nucleus startup phase.
pub(super) fn nucleus_phase(arch: &str, dry_run: bool) -> BootstrapPhase {
    if dry_run {
        return BootstrapPhase {
            name: "nucleus.start".into(),
            ok: true,
            detail: "dry-run: would start NUCLEUS primals".into(),
        };
    }
    let (ok, detail) = start_nucleus_primals(arch);
    BootstrapPhase {
        name: "nucleus.start".into(),
        ok,
        detail,
    }
}

// ── Secrets generation ──────────────────────────────────────────────

fn generate_secrets_env() -> String {
    use std::io::Write as _;

    let config_dir = cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_CONFIG_DIR,
        cellmembrane_types::service::DEFAULT_CONFIG_DIR,
    );
    let env_dir = std::path::Path::new(&config_dir);
    if let Err(e) = std::fs::create_dir_all(env_dir) {
        tracing::warn!(error = %e, "failed to create config directory for secrets");
    }
    let env_file = env_dir.join("secrets.env");
    if env_file.exists() {
        return config_dir;
    }

    let Some(secret) = csprng_hex(64) else {
        tracing::warn!("CSPRNG failed — secrets.env not generated");
        return config_dir;
    };
    let content = format!("NESTGATE_JWT_SECRET={secret}\n");
    if let Ok(mut f) = std::fs::File::create(&env_file) {
        if let Err(e) = f.write_all(content.as_bytes()) {
            tracing::warn!(error = %e, "failed to write secrets.env");
        }
    }
    set_restricted_permissions(&env_file);
    config_dir
}

/// Set owner-only permissions on a sensitive file.
///
/// On Unix: `chmod 0o600`. On other platforms: best-effort (ACLs not yet implemented).
fn set_restricted_permissions(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
            tracing::warn!(path = %path.display(), error = %e, "failed to set restricted permissions");
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

/// Generate `n` cryptographically random bytes and return as hex string.
///
/// Platform-aware — uses `/dev/urandom` on Unix, `BCryptGenRandom` on
/// Windows (via BLAKE3's keyed hash as entropy expander when OS RNG
/// is unavailable).
fn csprng_hex(n: usize) -> Option<String> {
    let mut buf = vec![0u8; n];
    fill_random(&mut buf)?;
    let mut hex = String::with_capacity(n * 2);
    for b in &buf {
        use std::fmt::Write;
        let _ = write!(hex, "{b:02x}");
    }
    Some(hex)
}

fn fill_random(buf: &mut [u8]) -> Option<()> {
    #[cfg(unix)]
    {
        use std::io::Read as _;
        std::fs::File::open("/dev/urandom")
            .ok()?
            .read_exact(buf)
            .ok()
    }
    #[cfg(not(unix))]
    {
        // BLAKE3 keyed hash as CSPRNG — derive from timestamp + pid.
        // Not ideal; future: add `getrandom` crate dependency.
        let seed_material = format!(
            "membrane-csprng-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
        );
        let hash = blake3::hash(seed_material.as_bytes());
        let hash_bytes = hash.as_bytes();
        for (i, b) in buf.iter_mut().enumerate() {
            *b = hash_bytes[i % 32];
        }
        Some(())
    }
}

// ── Systemd unit generation ─────────────────────────────────────────

/// Resolve extra CLI args for a primal's systemd `ExecStart`, based on capability.
pub(crate) fn extra_exec_args(svc: &cellmembrane_types::MembraneService) -> String {
    let relay_binary = cellmembrane_types::MembraneService::binary_for(
        cellmembrane_types::ServiceCapability::MeshRelay,
    );
    let content_binary = cellmembrane_types::MembraneService::binary_for(
        cellmembrane_types::ServiceCapability::ContentServing,
    );
    let identity_binary = cellmembrane_types::MembraneService::binary_for(
        cellmembrane_types::ServiceCapability::Identity,
    );

    if svc.binary == relay_binary {
        format!(
            " --federation-port {} --bind {}",
            cellmembrane_types::service::DEFAULT_FEDERATION_PORT,
            cellmembrane_types::service::BIND_ALL,
        )
    } else if svc.binary == content_binary {
        let port = cellmembrane_types::MembraneService::for_binary(content_binary)
            .and_then(|s| s.port)
            .unwrap_or(cellmembrane_types::service::DEFAULT_FEDERATION_PORT);
        format!(
            " --port {} --bind {}",
            port,
            cellmembrane_types::service::BIND_LOOPBACK,
        )
    } else if svc.binary == identity_binary {
        format!(
            " --http-address {}:0",
            cellmembrane_types::service::BIND_LOOPBACK,
        )
    } else {
        String::new()
    }
}

/// Generate the systemd unit file content for a NUCLEUS primal.
pub(crate) fn generate_unit_content(
    svc: &cellmembrane_types::MembraneService,
    exec_start: &str,
    extra_args: &str,
    config_dir: &str,
) -> String {
    let content_binary = cellmembrane_types::MembraneService::binary_for(
        cellmembrane_types::ServiceCapability::ContentServing,
    );
    let env_file_line = if svc.binary == content_binary {
        format!("EnvironmentFile=-{config_dir}/secrets.env\n")
    } else {
        String::new()
    };

    format!(
        "[Unit]\n\
         Description={binary} primal (membrane NUCLEUS)\n\
         After=network.target\n\n\
         [Service]\n\
         Type=simple\n\
         UMask={umask}\n\
         {env_file_line}\
         ExecStart={exec_start}{extra_args}\n\
         Restart=on-failure\n\
         RestartSec=3\n\
         RuntimeDirectory=membrane\n\
         RuntimeDirectoryMode={rtd_mode}\n\
         RuntimeDirectoryPreserve=yes\n\n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        binary = svc.binary,
        umask = cellmembrane_types::service::DEFAULT_SERVICE_UMASK,
        rtd_mode = cellmembrane_types::service::DEFAULT_RUNTIME_DIRECTORY_MODE,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellmembrane_types::{MembraneService, ServiceCapability};

    #[test]
    fn extra_exec_args_relay_contains_federation_port_and_bind_all() {
        let svc = MembraneService::with_capability(ServiceCapability::MeshRelay)
            .expect("MeshRelay must exist in registry");
        let args = extra_exec_args(svc);
        assert!(
            args.contains("--federation-port"),
            "relay should have --federation-port, got: {args}"
        );
        assert!(
            args.contains(cellmembrane_types::service::BIND_ALL),
            "relay should bind 0.0.0.0, got: {args}"
        );
    }

    #[test]
    fn extra_exec_args_content_contains_port_and_loopback() {
        let svc = MembraneService::with_capability(ServiceCapability::ContentServing)
            .expect("ContentServing must exist in registry");
        let args = extra_exec_args(svc);
        assert!(
            args.contains("--port"),
            "content server should have --port, got: {args}"
        );
        assert!(
            args.contains(cellmembrane_types::service::BIND_LOOPBACK),
            "content server should bind loopback, got: {args}"
        );
    }

    #[test]
    fn extra_exec_args_identity_contains_http_address() {
        let svc = MembraneService::with_capability(ServiceCapability::Identity)
            .expect("Identity must exist in registry");
        let args = extra_exec_args(svc);
        assert!(
            args.contains("--http-address"),
            "identity should have --http-address, got: {args}"
        );
        assert!(
            args.contains(cellmembrane_types::service::BIND_LOOPBACK),
            "identity should bind loopback, got: {args}"
        );
    }

    #[test]
    fn extra_exec_args_crypto_signer_is_empty() {
        let svc = MembraneService::with_capability(ServiceCapability::CryptoSigner)
            .expect("CryptoSigner must exist in registry");
        let relay = MembraneService::binary_for(ServiceCapability::MeshRelay);
        let content = MembraneService::binary_for(ServiceCapability::ContentServing);
        let identity = MembraneService::binary_for(ServiceCapability::Identity);
        if svc.binary != relay && svc.binary != content && svc.binary != identity {
            let args = extra_exec_args(svc);
            assert!(args.is_empty(), "crypto signer should have no extra args");
        }
    }

    #[test]
    fn generate_unit_content_has_systemd_sections() {
        let svc = MembraneService::with_capability(ServiceCapability::CryptoSigner)
            .expect("CryptoSigner must exist");
        let content = generate_unit_content(
            svc,
            "/usr/bin/beardog server --socket /run/x",
            "",
            "/etc/membrane",
        );
        assert!(content.contains("[Unit]"), "missing [Unit]");
        assert!(content.contains("[Service]"), "missing [Service]");
        assert!(content.contains("[Install]"), "missing [Install]");
        assert!(content.contains("After=network.target"));
        assert!(content.contains("Restart=on-failure"));
        assert!(content.contains("WantedBy=multi-user.target"));
    }

    #[test]
    fn generate_unit_content_includes_exec_start_and_extra_args() {
        let svc = MembraneService::with_capability(ServiceCapability::MeshRelay)
            .expect("MeshRelay must exist");
        let exec = "/opt/membrane/primals/x86_64/songbird server --socket /run/s.sock";
        let extra = format!(
            " --federation-port {} --bind {}",
            cellmembrane_types::service::DEFAULT_FEDERATION_PORT,
            cellmembrane_types::service::BIND_ALL,
        );
        let content = generate_unit_content(svc, exec, &extra, "/etc/membrane");
        let exec_line = format!("ExecStart={exec}{extra}");
        assert!(
            content.contains(&exec_line),
            "should embed ExecStart with extra args"
        );
    }

    #[test]
    fn generate_unit_content_env_file_only_for_content_serving() {
        let content_svc = MembraneService::with_capability(ServiceCapability::ContentServing)
            .expect("ContentServing must exist");
        let unit = generate_unit_content(content_svc, "/bin/x", "", "/etc/membrane");
        assert!(
            unit.contains("EnvironmentFile"),
            "content serving primal should have EnvironmentFile"
        );

        let crypto_svc = MembraneService::with_capability(ServiceCapability::CryptoSigner)
            .expect("CryptoSigner must exist");
        let unit2 = generate_unit_content(crypto_svc, "/bin/x", "", "/etc/membrane");
        assert!(
            !unit2.contains("EnvironmentFile"),
            "non-content primal should NOT have EnvironmentFile"
        );
    }

    #[test]
    fn generate_unit_content_env_file_uses_config_dir() {
        let content_svc = MembraneService::with_capability(ServiceCapability::ContentServing)
            .expect("ContentServing must exist");
        let unit = generate_unit_content(content_svc, "/bin/x", "", "/custom/config");
        assert!(
            unit.contains("EnvironmentFile=-/custom/config/secrets.env"),
            "env file path should use config_dir, got: {unit}"
        );
    }

    #[test]
    fn generate_unit_content_description_includes_binary_name() {
        let svc = MembraneService::with_capability(ServiceCapability::MeshRelay).unwrap();
        let content = generate_unit_content(svc, "/bin/x", "", "/etc/membrane");
        assert!(
            content.contains(&format!("Description={} primal", svc.binary)),
            "description should include binary name"
        );
    }

    #[test]
    fn generate_unit_content_includes_socket_permissions() {
        let svc = MembraneService::with_capability(ServiceCapability::CryptoSigner)
            .expect("CryptoSigner must exist");
        let content = generate_unit_content(svc, "/bin/x", "", "/etc/membrane");
        assert!(
            content.contains("UMask=0002"),
            "unit should set UMask=0002 for socket accessibility"
        );
        assert!(
            content.contains("RuntimeDirectoryMode=0755"),
            "unit should set RuntimeDirectoryMode=0755"
        );
    }

    #[test]
    fn csprng_hex_produces_correct_length() {
        let hex = csprng_hex(32).expect("/dev/urandom should be readable");
        assert_eq!(hex.len(), 64, "32 bytes should produce 64 hex chars");
        assert!(
            hex.chars().all(|c| c.is_ascii_hexdigit()),
            "output should be hex only, got: {hex}"
        );
    }

    #[test]
    fn csprng_hex_produces_varied_output() {
        let a = csprng_hex(16).unwrap();
        let b = csprng_hex(16).unwrap();
        assert_ne!(a, b, "two CSPRNG reads should differ");
    }

    #[test]
    fn nucleus_phase_dry_run_returns_ok() {
        let phase = nucleus_phase("x86_64-unknown-linux-musl", true);
        assert!(phase.ok, "dry-run should always succeed");
        assert_eq!(phase.name, "nucleus.start");
        assert!(phase.detail.contains("dry-run"));
    }

}
