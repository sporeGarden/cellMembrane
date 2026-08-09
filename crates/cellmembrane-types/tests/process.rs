use cellmembrane_types::process::*;

#[test]
fn service_status_display() {
    assert_eq!(ServiceStatus::Running.to_string(), "running");
    assert_eq!(ServiceStatus::Stopped.to_string(), "stopped");
    assert_eq!(ServiceStatus::Failed.to_string(), "failed");
    assert_eq!(ServiceStatus::Unknown.to_string(), "unknown");
}

#[test]
fn service_outcome_constructors() {
    let ok = ServiceOutcome::success("3 units installed");
    assert!(ok.ok);
    assert_eq!(ok.detail, "3 units installed");

    let fail = ServiceOutcome::failure("daemon-reload failed");
    assert!(!fail.ok);
    assert_eq!(fail.detail, "daemon-reload failed");
}

#[test]
fn init_system_detect_returns_valid() {
    let init = InitSystem::detect();
    if cfg!(target_os = "linux") {
        assert!(
            init == InitSystem::Systemd || init == InitSystem::Bare,
            "Linux should detect systemd or bare, got: {init}"
        );
    }
}

#[test]
fn init_system_display() {
    assert_eq!(InitSystem::Systemd.to_string(), "systemd");
    assert_eq!(InitSystem::Launchd.to_string(), "launchd");
    assert_eq!(InitSystem::WindowsSCM.to_string(), "windows-scm");
    assert_eq!(InitSystem::Bare.to_string(), "bare");
}

#[test]
fn init_system_units_support() {
    assert!(InitSystem::Systemd.supports_units());
    assert!(InitSystem::Launchd.supports_units());
    assert!(InitSystem::WindowsSCM.supports_units());
    assert!(!InitSystem::Bare.supports_units());
}

#[test]
fn crash_loop_action_display() {
    assert_eq!(CrashLoopAction::Disabled.to_string(), "disabled");
    assert_eq!(CrashLoopAction::Logged.to_string(), "logged");
    assert_eq!(
        CrashLoopAction::FailedToDisable.to_string(),
        "failed-to-disable"
    );
}

#[test]
fn crash_loop_report_empty() {
    let report = CrashLoopReport {
        loops: vec![],
        threshold: 5,
        scanned: 10,
    };
    assert!(!report.has_loops());
    assert_eq!(report.disabled_count(), 0);
}

#[test]
fn crash_loop_report_with_entries() {
    let report = CrashLoopReport {
        loops: vec![
            CrashLoopEntry {
                unit: "nestgate-membrane.service".into(),
                restart_count: 17920,
                sub_state: "failed".into(),
                action: CrashLoopAction::Disabled,
            },
            CrashLoopEntry {
                unit: "biomeos-beacon.service".into(),
                restart_count: 11161,
                sub_state: "activating".into(),
                action: CrashLoopAction::FailedToDisable,
            },
        ],
        threshold: 5,
        scanned: 15,
    };
    assert!(report.has_loops());
    assert_eq!(report.disabled_count(), 1);
}

#[test]
fn crash_loop_report_serialization() {
    let report = CrashLoopReport {
        loops: vec![CrashLoopEntry {
            unit: "test.service".into(),
            restart_count: 100,
            sub_state: "failed".into(),
            action: CrashLoopAction::Disabled,
        }],
        threshold: 5,
        scanned: 1,
    };
    let json = serde_json::to_string(&report).unwrap();
    assert!(json.contains("\"action\":\"disabled\""));
    let parsed: CrashLoopReport = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.loops[0].restart_count, 100);
}

#[test]
fn restart_policy_default() {
    let rp = RestartPolicy::default();
    assert_eq!(rp.condition, "on-failure");
    assert_eq!(rp.restart_sec, 5);
    assert_eq!(rp.start_limit_interval_sec, 120);
    assert_eq!(rp.start_limit_burst, 10);
}

#[test]
fn service_spec_to_systemd_unit_has_sections() {
    let spec = ServiceSpec {
        binary: "beardog".into(),
        systemd_unit: "beardog-membrane.service".into(),
        description: "beardog primal (membrane NUCLEUS)".into(),
        exec_start: "/opt/membrane/beardog server --socket /run/membrane/beardog.sock".into(),
        extra_args: String::new(),
        environment: vec![],
        env_file: None,
        restart_policy: RestartPolicy::default(),
        after: vec!["network.target".into()],
        working_directory: None,
        umask: "0002".into(),
        runtime_directory: Some("membrane".into()),
        runtime_directory_mode: "0755".into(),
        limit_nofile: Some(65536),
    };
    let unit = spec.to_systemd_unit();
    assert!(unit.contains("[Unit]"));
    assert!(unit.contains("[Service]"));
    assert!(unit.contains("[Install]"));
    assert!(unit.contains("Description=beardog primal"));
    assert!(unit.contains("ExecStart=/opt/membrane/beardog server"));
    assert!(unit.contains("Restart=on-failure"));
    assert!(unit.contains("RuntimeDirectory=membrane"));
    assert!(unit.contains("LimitNOFILE=65536"));
    assert!(unit.contains("WantedBy=multi-user.target"));
}

#[test]
fn service_spec_env_file_included() {
    let spec = ServiceSpec {
        binary: "nestgate".into(),
        systemd_unit: "nestgate-membrane.service".into(),
        description: "nestgate primal".into(),
        exec_start: "/opt/membrane/nestgate server".into(),
        extra_args: String::new(),
        environment: vec![],
        env_file: Some("/etc/membrane/secrets.env".into()),
        restart_policy: RestartPolicy::default(),
        after: vec!["network.target".into()],
        working_directory: None,
        umask: "0002".into(),
        runtime_directory: Some("membrane".into()),
        runtime_directory_mode: "0755".into(),
        limit_nofile: Some(65536),
    };
    let unit = spec.to_systemd_unit();
    assert!(unit.contains("EnvironmentFile=-/etc/membrane/secrets.env"));
}

#[test]
fn service_spec_environment_vars() {
    let spec = ServiceSpec {
        binary: "songbird".into(),
        systemd_unit: "songbird-relay.service".into(),
        description: "songbird primal".into(),
        exec_start: "/opt/membrane/songbird server".into(),
        extra_args: String::new(),
        environment: vec![
            ("MEMBRANE_GATE_NAME".into(), "sporeGate".into()),
            ("MESH_IP".into(), "10.13.37.1".into()),
        ],
        env_file: None,
        restart_policy: RestartPolicy::default(),
        after: vec!["network.target".into()],
        working_directory: None,
        umask: "0002".into(),
        runtime_directory: None,
        runtime_directory_mode: "0755".into(),
        limit_nofile: Some(65536),
    };
    let unit = spec.to_systemd_unit();
    assert!(unit.contains("Environment=MEMBRANE_GATE_NAME=sporeGate"));
    assert!(unit.contains("Environment=MESH_IP=10.13.37.1"));
}

#[test]
fn service_spec_to_systemd_override() {
    let spec = ServiceSpec {
        binary: "songbird".into(),
        systemd_unit: "songbird-relay.service".into(),
        description: String::new(),
        exec_start: String::new(),
        extra_args: String::new(),
        environment: vec![("ROUTES".into(), "sporeprint:9500".into())],
        env_file: Some("/etc/membrane/secrets.env".into()),
        restart_policy: RestartPolicy::default(),
        after: vec![],
        working_directory: None,
        umask: "0002".into(),
        runtime_directory: None,
        runtime_directory_mode: "0755".into(),
        limit_nofile: None,
    };
    let ovr = spec.to_systemd_override();
    assert!(ovr.contains("[Service]"));
    assert!(ovr.contains("Environment=ROUTES=sporeprint:9500"));
    assert!(ovr.contains("EnvironmentFile=-/etc/membrane/secrets.env"));
}

#[test]
fn service_spec_to_launchd_plist() {
    let spec = ServiceSpec {
        binary: "beardog".into(),
        systemd_unit: "beardog-membrane.service".into(),
        description: "beardog primal".into(),
        exec_start: "/opt/membrane/beardog server --socket /tmp/bd.sock".into(),
        extra_args: String::new(),
        environment: vec![("FAMILY_SEED".into(), "abc123".into())],
        env_file: None,
        restart_policy: RestartPolicy::default(),
        after: vec![],
        working_directory: None,
        umask: "0002".into(),
        runtime_directory: None,
        runtime_directory_mode: "0755".into(),
        limit_nofile: Some(65536),
    };
    let plist = spec.to_launchd_plist();
    assert!(plist.contains("<key>Label</key>"));
    assert!(plist.contains("eco.primals.beardog"));
    assert!(plist.contains("<key>Program</key>"));
    assert!(plist.contains("/opt/membrane/beardog"));
    assert!(plist.contains("<key>KeepAlive</key>"));
    assert!(plist.contains("FAMILY_SEED"));
}

#[test]
fn service_spec_from_membrane_service() {
    let svc = cellmembrane_types::MembraneService::with_capability(
        cellmembrane_types::ServiceCapability::CryptoSigner,
    )
    .expect("CryptoSigner must exist");
    let spec = ServiceSpec::from_membrane_service(
        svc,
        "/opt/membrane",
        "/run/membrane",
        "/run/membrane/security.sock",
        "/etc/membrane",
    );
    assert_eq!(spec.binary, svc.binary);
    assert!(spec.exec_start.contains(svc.binary));
    assert!(spec.description.contains("NUCLEUS"));
}

#[test]
fn init_scope_env_var_name() {
    assert_eq!(
        cellmembrane_types::service::ENV_INIT_SCOPE,
        "MEMBRANE_INIT_SCOPE"
    );
}

#[test]
fn init_system_detect_without_env_override() {
    if std::env::var(cellmembrane_types::service::ENV_INIT_SCOPE).is_err() {
        let init = InitSystem::detect();
        if cfg!(target_os = "linux") {
            assert!(
                init == InitSystem::Systemd || init == InitSystem::Bare,
                "Linux should detect systemd or bare, got: {init}"
            );
        }
    }
}
