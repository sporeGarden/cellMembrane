// SPDX-License-Identifier: AGPL-3.0-or-later

use std::process::Command;

fn main() {
    println!("cargo::rerun-if-changed=../../.git/HEAD");
    println!("cargo::rerun-if-changed=../../.git/refs");

    if std::env::var("MEMBRANE_BUILD_SHA").is_ok() {
        return;
    }

    let sha = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    if !sha.is_empty() {
        println!("cargo::rustc-env=MEMBRANE_BUILD_SHA={sha}");
    }
}
