use super::super::resolve::*;
use super::*;

#[test]
fn env_or_returns_default_when_unset() {
    let val = env_or("_CELLMEMBRANE_TEST_UNSET_VAR_xyz", "fallback");
    assert_eq!(val, "fallback");
}

#[test]
fn default_socket_base_is_absolute() {
    assert!(DEFAULT_SOCKET_BASE.starts_with('/'));
}

#[test]
fn default_push_remotes_nonempty() {
    assert!(!DEFAULT_PUSH_REMOTES.is_empty());
    assert!(DEFAULT_PUSH_REMOTES.contains(&"forgejo"));
}

#[test]
fn lan_dns_name_lowercases_gate() {
    assert_eq!(lan_dns_name("sporeGate"), "sporegate.primals.local");
    assert_eq!(lan_dns_name("eastGate"), "eastgate.primals.local");
    assert_eq!(lan_dns_name("golgi"), "golgi.primals.local");
}

#[test]
#[allow(clippy::assertions_on_constants)]
fn timeout_constants_reasonable() {
    assert!(DEFAULT_SSH_TIMEOUT_SECS >= 5);
    assert!(DEFAULT_GIT_OP_TIMEOUT_SECS >= 30);
    assert!(DEFAULT_FETCH_TIMEOUT_SECS >= 60);
}

#[test]
fn post_primordial_contains_core_stack() {
    assert!(is_post_primordial("beardog"));
    assert!(is_post_primordial("songbird"));
    assert!(is_post_primordial("skunkbat"));
    assert!(is_post_primordial("nestgate"));
    assert!(is_post_primordial("cellmembrane"));
    assert!(is_post_primordial("biomeos"));
}

#[test]
fn post_primordial_excludes_non_core() {
    assert!(!is_post_primordial("squirrel"));
    assert!(!is_post_primordial("petaltongue"));
    assert!(!is_post_primordial("loamspine"));
    assert!(!is_post_primordial("toadstool"));
}

#[test]
fn post_primordial_count() {
    assert_eq!(POST_PRIMORDIAL_PRIMALS.len(), 6);
}

#[test]
fn resolve_systemd_unit_dir_defaults_to_system() {
    if std::env::var(ENV_SYSTEMD_UNIT_DIR).is_err() && std::env::var(ENV_INIT_SCOPE).is_err() {
        let dir = resolve_systemd_unit_dir();
        assert_eq!(dir, SYSTEMD_UNIT_DIR);
    }
}

#[test]
fn env_init_scope_constant_defined() {
    assert_eq!(ENV_INIT_SCOPE, "MEMBRANE_INIT_SCOPE");
}

#[test]
fn systemd_user_unit_dir_is_relative() {
    assert!(!SYSTEMD_USER_UNIT_DIR.starts_with('/'));
    assert!(SYSTEMD_USER_UNIT_DIR.contains("systemd/user"));
}

#[test]
fn songbird_socket_matches_registry() {
    let relay_binary = crate::MembraneService::binary_for(crate::ServiceCapability::MeshRelay);
    let expected = format!("{DEFAULT_SOCKET_BASE}/{relay_binary}.sock");
    assert_eq!(
        DEFAULT_SONGBIRD_SOCKET, expected,
        "DEFAULT_SONGBIRD_SOCKET must match registry-derived path"
    );
}

#[test]
fn env_songbird_socket_is_capability_neutral() {
    assert!(
        !ENV_SONGBIRD_SOCKET.contains("BEARDOG"),
        "env var should not embed primal names"
    );
    assert!(
        !ENV_SONGBIRD_SOCKET.contains("SONGBIRD"),
        "env var should use capability-neutral naming"
    );
}

#[test]
fn resolve_socket_base_defaults_to_system() {
    if std::env::var(ENV_SOCKET_BASE).is_err()
        && std::env::var(ENV_BIOMEOS_SOCKET_DIR).is_err()
        && std::env::var(ENV_INIT_SCOPE).is_err()
    {
        let base = resolve_socket_base();
        assert_eq!(base, DEFAULT_SOCKET_BASE);
    }
}

#[test]
fn resolve_socket_base_returns_absolute() {
    let base = resolve_socket_base();
    assert!(
        base.starts_with('/'),
        "socket base must be absolute: {base}"
    );
}

#[test]
fn env_gate_name_uses_membrane_prefix() {
    assert!(
        ENV_GATE_NAME.starts_with("MEMBRANE_"),
        "ENV_GATE_NAME must use MEMBRANE_ prefix: {ENV_GATE_NAME}"
    );
}

#[test]
fn env_gate_name_legacy_is_backward_compat() {
    assert_eq!(ENV_GATE_NAME_LEGACY, "GATE_NAME");
}

#[test]
fn resolve_gate_name_env_returns_none_when_unset() {
    if std::env::var(ENV_GATE_NAME).is_err() && std::env::var(ENV_GATE_NAME_LEGACY).is_err() {
        assert!(resolve_gate_name_env().is_none());
    }
}
