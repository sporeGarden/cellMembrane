// SPDX-License-Identifier: AGPL-3.0-or-later

use cellmembrane_types::validation::{Report, Severity};

#[test]
fn report_display_format() {
    let mut report = Report::new();
    report.pass("test.check", "passed");
    report.fail("test.fail", "something broke");
    let output = format!("{report}");
    assert!(output.contains("[PASS] test.check: passed"));
    assert!(output.contains("[FAIL] test.fail: something broke"));
    assert!(output.contains("--- 1 passed, 1 failed, 0 warnings"));
}

#[test]
fn report_total_checks() {
    let mut report = Report::new();
    report.pass("a", "ok");
    report.pass("b", "ok");
    report.fail("c", "fail");
    report.warn("d", "maybe");
    report.info("e", "fyi");
    assert_eq!(report.total_checks(), 3);
}

#[test]
fn report_merge() {
    let mut r1 = Report::new();
    r1.pass("a", "ok");
    let mut r2 = Report::new();
    r2.fail("b", "not ok");
    r2.warn("c", "meh");
    r1.merge(r2);
    assert_eq!(r1.entries.len(), 3);
    assert!(!r1.is_ok());
}

#[test]
fn report_summary() {
    let mut report = Report::new();
    report.pass("a", "ok");
    report.pass("b", "ok");
    report.fail("c", "nope");
    report.warn("d", "hmm");
    assert_eq!(report.summary(), "2 passed, 1 failed, 1 warnings");
}

#[test]
fn severity_display() {
    assert_eq!(format!("{}", Severity::Info), "INFO");
    assert_eq!(format!("{}", Severity::Warn), "WARN");
    assert_eq!(format!("{}", Severity::Fail), "FAIL");
    assert_eq!(format!("{}", Severity::Pass), "PASS");
}

#[test]
fn report_entry_display() {
    let entry = cellmembrane_types::validation::ReportEntry {
        severity: Severity::Warn,
        check: "net.port".to_string(),
        message: "port conflict detected".to_string(),
    };
    assert_eq!(
        format!("{entry}"),
        "[WARN] net.port: port conflict detected"
    );
}

#[test]
fn report_count_empty() {
    let report = Report::new();
    assert_eq!(report.count(Severity::Pass), 0);
    assert_eq!(report.count(Severity::Fail), 0);
    assert!(report.is_ok());
}
