use super::*;

#[test]
fn relay_config_respects_defaults() {
    let config = RelayConfig {
        ecoprimals_root: PathBuf::from(cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT),
        forgejo_remote: Cow::Borrowed("forgejo"),
        github_remote: Cow::Borrowed("origin"),
        golgi_ext_host: Cow::Borrowed("golgi-ext"),
    };
    assert_eq!(
        config.ecoprimals_root,
        PathBuf::from(cellmembrane_types::service::DEFAULT_ECOPRIMALS_ROOT)
    );
    assert_eq!(&*config.forgejo_remote, "forgejo");
    assert_eq!(&*config.github_remote, "origin");
    assert_eq!(&*config.golgi_ext_host, "golgi-ext");
}

#[test]
fn relay_config_custom_values() {
    let config = RelayConfig {
        ecoprimals_root: PathBuf::from("/tmp/test-eco"),
        forgejo_remote: Cow::Borrowed("forgejo"),
        github_remote: Cow::Owned("github".to_string()),
        golgi_ext_host: Cow::Owned("custom-ext".to_string()),
    };
    assert_eq!(config.ecoprimals_root, PathBuf::from("/tmp/test-eco"));
    assert_eq!(&*config.forgejo_remote, "forgejo");
    assert_eq!(&*config.github_remote, "github");
    assert_eq!(&*config.golgi_ext_host, "custom-ext");
}

#[tokio::test]
async fn mediate_skips_nonexistent_repos() {
    let config = RelayConfig {
        ecoprimals_root: PathBuf::from("/tmp/nonexistent-relay-test"),
        forgejo_remote: Cow::Borrowed("forgejo"),
        github_remote: Cow::Borrowed("origin"),
        golgi_ext_host: Cow::Borrowed("test"),
    };
    let (pulled, failures) = mediate(&config, &["no/such/repo"]).await;
    assert!(pulled.is_empty());
    assert!(failures.is_empty());
}

#[tokio::test]
async fn absorb_skips_nonexistent_repos() {
    let config = RelayConfig {
        ecoprimals_root: PathBuf::from("/tmp/nonexistent-absorb-test"),
        forgejo_remote: Cow::Borrowed("forgejo"),
        github_remote: Cow::Borrowed("origin"),
        golgi_ext_host: Cow::Borrowed("test"),
    };
    let absorbed = absorb_extracellular(&config, &["no/such/repo"]).await;
    assert!(absorbed.is_empty());
}

#[tokio::test]
async fn parity_skips_nonexistent_repos() {
    let config = RelayConfig {
        ecoprimals_root: PathBuf::from("/tmp/nonexistent-parity-test"),
        forgejo_remote: Cow::Borrowed("forgejo"),
        github_remote: Cow::Borrowed("origin"),
        golgi_ext_host: Cow::Borrowed("test"),
    };
    let reports = check_parity(&config, &["no/such/repo"]).await;
    assert_eq!(reports.len(), 1);
    assert!(
        reports[0].at_parity,
        "non-existent repos should count as parity"
    );
    assert!(reports[0].detail.contains("not cloned"));
}

#[test]
fn relay_result_serializes() {
    let result = RelayResult {
        absorbed: vec!["songBird".into()],
        pulled: vec!["bearDog".into()],
        pull_failures: vec![],
        impulses_sensed: 2,
        pushed: vec!["bearDog".into()],
        push_skipped: vec![],
        push_failures: vec!["songBird".into()],
    };
    let json = serde_json::to_value(&result).unwrap();
    assert_eq!(json["absorbed"][0], "songBird");
    assert_eq!(json["pulled"][0], "bearDog");
    assert_eq!(json["impulses_sensed"], 2);
    assert_eq!(json["push_failures"][0], "songBird");
}

#[test]
fn parity_report_serializes() {
    let report = ParityReport {
        repo: "songBird".into(),
        at_parity: false,
        detail: "GitHub 3 ahead of Forgejo".into(),
    };
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["repo"], "songBird");
    assert_eq!(json["at_parity"], false);
    assert!(json["detail"].as_str().unwrap().contains("GitHub"));
}

#[test]
fn ship_result_variants() {
    assert!(matches!(ShipResult::Pushed, ShipResult::Pushed));
    assert!(matches!(ShipResult::Skipped, ShipResult::Skipped));
    assert!(matches!(ShipResult::Failed, ShipResult::Failed));
}

#[test]
fn absorb_outcome_variants() {
    assert!(matches!(
        AbsorbOutcome::Absorbed(3),
        AbsorbOutcome::Absorbed(3)
    ));
    assert!(matches!(AbsorbOutcome::AtParity, AbsorbOutcome::AtParity));
    assert!(matches!(
        AbsorbOutcome::FetchFailed,
        AbsorbOutcome::FetchFailed
    ));
    assert!(matches!(
        AbsorbOutcome::PushFailed,
        AbsorbOutcome::PushFailed
    ));
    assert!(matches!(
        AbsorbOutcome::NoGitHubRemote,
        AbsorbOutcome::NoGitHubRemote
    ));
}
