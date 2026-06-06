//! SLICE 5 (COV_09) shell integration tests.
//!
//! `#[ignore]`-gated by default — these tests actually shell out to
//! bash / zsh / fish / pwsh to source the generated rc file and assert
//! the env vars land in the child process. Run with:
//!
//! ```bash
//! cargo test --manifest-path services/cli/Cargo.toml \
//!     --test shell_integration -- --include-ignored
//! ```
//!
//! The hermetic lib tests under `services/cli/src/shell/*` already cover
//! the marker block shape, idempotency, and quoting; these tests prove
//! the end-to-end "rc file -> sourced shell -> env var present" loop
//! on hosts that have the relevant shell installed. CI matrix opts in
//! per platform (`tests.md` §8).

use spendguard_cli::shell::{dispatch_writer, DetectedShell};
use std::path::PathBuf;
use std::process::Command;

fn vars() -> Vec<(String, String)> {
    vec![
        ("SLICE5_HTTPS_PROXY".into(), "https://localhost:8443".into()),
        (
            "SLICE5_NODE_EXTRA_CA_CERTS".into(),
            "/tmp/spendguard-test/ca.pem".into(),
        ),
    ]
}

/// Find a binary on PATH and return its path, or skip the test.
fn require(binary: &str) -> Option<PathBuf> {
    Command::new("which")
        .arg(binary)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()))
}

#[test]
#[ignore = "Sources real bash; opt-in via --include-ignored."]
fn bash_round_trip_source_and_assert_env_present() {
    let Some(bash) = require("bash") else {
        eprintln!("bash not on PATH; skipping");
        return;
    };
    let tmp = tempfile::tempdir().expect("tempdir");
    let rc = tmp.path().join(".bashrc");
    let writer = dispatch_writer(DetectedShell::Bash);
    writer.write_rc(&rc, &vars()).expect("write bashrc");

    let out = Command::new(bash)
        .args([
            "-c",
            &format!(
                "source {rc} && echo $SLICE5_HTTPS_PROXY && echo $SLICE5_NODE_EXTRA_CA_CERTS",
                rc = rc.display()
            ),
        ])
        .output()
        .expect("spawn bash");
    assert!(out.status.success(), "bash failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("https://localhost:8443"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("/tmp/spendguard-test/ca.pem"),
        "stdout: {stdout}"
    );
}

#[test]
#[ignore = "Sources real zsh; opt-in via --include-ignored."]
fn zsh_round_trip_source_and_assert_env_present() {
    let Some(zsh) = require("zsh") else {
        eprintln!("zsh not on PATH; skipping");
        return;
    };
    let tmp = tempfile::tempdir().expect("tempdir");
    let rc = tmp.path().join(".zshrc");
    let writer = dispatch_writer(DetectedShell::Zsh);
    writer.write_rc(&rc, &vars()).expect("write zshrc");

    let out = Command::new(zsh)
        .args([
            "-c",
            &format!(
                "source {rc} && echo $SLICE5_HTTPS_PROXY && echo $SLICE5_NODE_EXTRA_CA_CERTS",
                rc = rc.display()
            ),
        ])
        .output()
        .expect("spawn zsh");
    assert!(out.status.success(), "zsh failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("https://localhost:8443"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("/tmp/spendguard-test/ca.pem"),
        "stdout: {stdout}"
    );
}

#[test]
#[ignore = "Sources real fish; opt-in via --include-ignored."]
fn fish_round_trip_source_and_assert_env_present() {
    let Some(fish) = require("fish") else {
        eprintln!("fish not on PATH; skipping");
        return;
    };
    let tmp = tempfile::tempdir().expect("tempdir");
    let rc = tmp
        .path()
        .join(".config")
        .join("fish")
        .join("conf.d")
        .join("spendguard.fish");
    let writer = dispatch_writer(DetectedShell::Fish);
    writer.write_rc(&rc, &vars()).expect("write fish conf.d");

    let out = Command::new(fish)
        .args([
            "-c",
            &format!(
                "source {rc} ; echo $SLICE5_HTTPS_PROXY ; echo $SLICE5_NODE_EXTRA_CA_CERTS",
                rc = rc.display()
            ),
        ])
        .output()
        .expect("spawn fish");
    assert!(out.status.success(), "fish failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("https://localhost:8443"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("/tmp/spendguard-test/ca.pem"),
        "stdout: {stdout}"
    );
}

#[test]
#[ignore = "Sources real pwsh; opt-in via --include-ignored."]
fn pwsh_round_trip_dot_source_and_assert_env_present() {
    let Some(pwsh) = require("pwsh") else {
        eprintln!("pwsh not on PATH; skipping");
        return;
    };
    let tmp = tempfile::tempdir().expect("tempdir");
    let profile = tmp.path().join("Profile.ps1");
    let writer = dispatch_writer(DetectedShell::Pwsh);
    writer
        .write_rc(&profile, &vars())
        .expect("write pwsh profile");

    let out = Command::new(pwsh)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-Command",
            &format!(
                ". '{profile}'; Write-Output $env:SLICE5_HTTPS_PROXY; Write-Output $env:SLICE5_NODE_EXTRA_CA_CERTS",
                profile = profile.display()
            ),
        ])
        .output()
        .expect("spawn pwsh");
    assert!(out.status.success(), "pwsh failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("https://localhost:8443"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("/tmp/spendguard-test/ca.pem"),
        "stdout: {stdout}"
    );
}
