// SPDX-License-Identifier: AGPL-3.0-or-later

//! Tests for nucleus service management — extracted from `nucleus.rs` for line
//! budget management.

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
fn generate_unit_content_songbird_depends_on_beardog() {
    let svc = MembraneService::with_capability(ServiceCapability::MeshRelay)
        .expect("MeshRelay must exist");
    let content = generate_unit_content(svc, "/bin/songbird server", "", "/etc/membrane");
    let beardog_svc = MembraneService::with_capability(ServiceCapability::CryptoSigner)
        .expect("CryptoSigner must exist");
    assert!(
        content.contains(beardog_svc.systemd_unit),
        "songbird unit should have After= dependency on beardog: {}",
        content
            .lines()
            .find(|l| l.starts_with("After="))
            .unwrap_or("no After= line")
    );
}

#[test]
fn generate_unit_content_beardog_has_no_primal_deps() {
    let svc = MembraneService::with_capability(ServiceCapability::CryptoSigner)
        .expect("CryptoSigner must exist");
    let content = generate_unit_content(svc, "/bin/beardog server", "", "/etc/membrane");
    let after_line = content
        .lines()
        .find(|l| l.starts_with("After="))
        .expect("should have After= line");
    assert_eq!(
        after_line, "After=network.target",
        "beardog should only depend on network.target"
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

#[test]
fn init_system_detect_is_valid() {
    let init = cellmembrane_types::InitSystem::detect();
    let valid = matches!(
        init,
        cellmembrane_types::InitSystem::Systemd
            | cellmembrane_types::InitSystem::Launchd
            | cellmembrane_types::InitSystem::WindowsSCM
            | cellmembrane_types::InitSystem::Bare
    );
    assert!(
        valid,
        "detect() must return a known init system, got: {init}"
    );
}

#[test]
fn load_env_file_parses_key_value_pairs() {
    let dir = std::env::temp_dir().join(format!("nucleus_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("test.env");
    std::fs::write(&path, "FOO=bar\n# comment\nBAZ=qux\n\n").expect("write");
    let pairs = load_env_file(&path);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(pairs.len(), 2);
    assert_eq!(pairs[0], ("FOO".to_string(), "bar".to_string()));
    assert_eq!(pairs[1], ("BAZ".to_string(), "qux".to_string()));
}

#[test]
fn load_env_file_returns_empty_on_missing() {
    let pairs = load_env_file(std::path::Path::new("/nonexistent/secrets.env"));
    assert!(pairs.is_empty());
}

#[test]
fn resolve_security_socket_returns_valid_path() {
    let paths = cellmembrane_types::service::ServicePaths::from_env();
    let socket = resolve_security_socket(&paths);
    assert!(
        socket.contains(".sock"),
        "should be a .sock path, got: {socket}"
    );
}

#[test]
fn stop_bare_process_returns_false_on_missing_pid() {
    assert!(!stop_bare_process("nonexistent-binary-xyz"));
}
