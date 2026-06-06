//! CLI smoke tests — exercise the binary's `--help` + exit-code
//! discipline without hitting Postgres.
//!
//! Spec ancestor: `docs/calibration-report-spec-v1alpha1.md` §2.3
//! + SLICE_05 R1 "real CLI must work; not just framework".
//!
//! The tests use `assert_cmd`-style invocation through `std::process`
//! so we don't depend on the `assert_cmd` dev-dep. We rely on cargo's
//! `target/debug/spendguard-calibration-report` build artefact for
//! `cargo test`.

use std::process::Command;

fn bin_path() -> std::path::PathBuf {
    // CARGO_BIN_EXE_<name> is set by cargo when running integration
    // tests; points at the built binary.
    let exe = env!("CARGO_BIN_EXE_spendguard-calibration-report");
    std::path::PathBuf::from(exe)
}

#[test]
fn binary_prints_help() {
    let out = Command::new(bin_path())
        .arg("--help")
        .output()
        .expect("binary executes");
    assert!(out.status.success(), "--help should exit 0");
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("SpendGuard calibration-report CLI"));
    assert!(stdout.contains("--tenant"));
    assert!(stdout.contains("--from"));
    assert!(stdout.contains("--to"));
    assert!(stdout.contains("--format"));
    assert!(stdout.contains("--proof-mode"));
    assert!(stdout.contains("--verify-chain"));
}

#[test]
fn missing_canonical_url_exits_two() {
    // Per spec §2.3: cannot query → exit 2.
    let out = Command::new(bin_path())
        .args(["--tenant", "00000000-0000-4000-8000-000000000001"])
        .env_remove("SPENDGUARD_CALIBRATION_CANONICAL_URL")
        .output()
        .expect("binary executes");
    assert_eq!(
        out.status.code(),
        Some(2),
        "missing --canonical-url should exit 2; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn invalid_tenant_exits_two() {
    let out = Command::new(bin_path())
        .args([
            "--tenant",
            "not-a-uuid",
            "--canonical-url",
            "postgres://nonexistent:5432/db",
        ])
        .output()
        .expect("binary executes");
    assert_eq!(
        out.status.code(),
        Some(2),
        "invalid --tenant should exit 2; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn cross_tenant_exits_two() {
    let out = Command::new(bin_path())
        .args([
            "--tenant",
            "00000000-0000-4000-8000-000000000099",
            "--auth-tenants",
            "00000000-0000-4000-8000-000000000001",
            "--canonical-url",
            "postgres://nonexistent:5432/db",
        ])
        .output()
        .expect("binary executes");
    assert_eq!(
        out.status.code(),
        Some(2),
        "cross-tenant should exit 2 BEFORE attempting DB connect; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Stderr should mention the rejection (per spec §5.2).
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains("not authorised") || stderr.contains("§5.2"),
        "stderr must explain the rejection: {stderr}"
    );
}

#[test]
fn invalid_window_exits_two() {
    let out = Command::new(bin_path())
        .args([
            "--tenant",
            "00000000-0000-4000-8000-000000000001",
            "--from",
            "garbage",
            "--canonical-url",
            "postgres://nonexistent:5432/db",
        ])
        .output()
        .expect("binary executes");
    assert_eq!(
        out.status.code(),
        Some(2),
        "garbage --from should exit 2; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn db_connect_fail_exits_two() {
    // canonical_url points at a non-existent host → exit 2 per §2.3.
    let out = Command::new(bin_path())
        .args([
            "--tenant",
            "00000000-0000-4000-8000-000000000001",
            "--canonical-url",
            // intentionally invalid; should fail at connect time
            "postgres://hostnotfound.invalid:5432/db?connect_timeout=2",
        ])
        .output()
        .expect("binary executes");
    assert_eq!(
        out.status.code(),
        Some(2),
        "DB connect fail should exit 2; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn verify_chain_subcommand_help() {
    let out = Command::new(bin_path())
        .args(["verify-chain", "--help"])
        .output()
        .expect("binary executes");
    assert!(out.status.success(), "verify-chain --help should exit 0");
}
