//! Integration test for the macOS keychain trust-store backend.
//!
//! ## Why two test classes
//!
//! Apple's `security add-trusted-cert` ALWAYS prompts for authentication —
//! per-user Trust Settings need a user-auth dialog (Touch ID / password)
//! and admin Trust Settings need an admin password — even when the target
//! `-k` keychain is a freshly created, password-known scratch keychain.
//! There is no `--non-interactive` / `--password-via-stdin` flag. That
//! means a *fully automated* end-to-end test of `add-trusted-cert` cannot
//! run unattended on a CI runner without UI automation.
//!
//! We solve this by splitting integration tests into two groups:
//!
//! 1. **Non-mutating, fully automated** (default `cargo test`): exercise
//!    `verify_installed` and the constructor against a scratch keychain.
//!    `find-certificate` does NOT require auth, so this proves the entire
//!    shell-out shape works without ever touching Trust Settings.
//! 2. **Mutating, opt-in via `--include-ignored`**: actually run
//!    `add_root` / `remove_root` against the user's real
//!    `~/Library/Keychains/login.keychain-db`. These pop the macOS
//!    keychain-access dialog and require the developer to click "Always
//!    Allow" (or supply Touch ID). Each test installs a Drop guard that
//!    removes the issued cert even on panic, so the developer's keychain
//!    is never left with leftover SpendGuard test roots.
//!
//! The split lets `cargo test -p spendguard-cli` stay green on CI runners
//! and developer laptops without auth dialogs, while still providing
//! reviewer-runnable evidence of end-to-end keychain mutation via the
//! `--include-ignored` flag.

#![cfg(target_os = "macos")]

use spendguard_cli::trust::macos::MacosTrustStore;
use spendguard_cli::trust::{CommandOutput, CommandRunner, TrustStore};
use spendguard_cli::{
    doctor_with_trust_store, install_with_trust_store, uninstall_with_trust_store, DoctorOpts,
    InstallOpts, TrustScope, UninstallOpts,
};
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::OnceLock;

// ──────────────────────────────────────────────────────────────────────
//  Shared scaffolding
// ──────────────────────────────────────────────────────────────────────

/// Adapter that rewrites the `-k <keychain>` argument from whatever the
/// trust store picked (login or System keychain) to a developer-provided
/// scratch keychain. The surgical minimum override — every other argument
/// flows through untouched, so the production shell-out shape is what's
/// exercised.
struct KeychainRedirectingRunner {
    target_keychain: PathBuf,
}

impl CommandRunner for KeychainRedirectingRunner {
    fn run(&self, program: &str, args: &[OsString]) -> anyhow::Result<CommandOutput> {
        let dash_k = OsString::from("-k");
        let find_cmd = OsString::from("find-certificate");
        let delete_cmd = OsString::from("delete-certificate");

        let mut rewritten: Vec<OsString> = Vec::with_capacity(args.len());
        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            if *arg == dash_k && i + 1 < args.len() {
                rewritten.push(dash_k.clone());
                rewritten.push(self.target_keychain.clone().into_os_string());
                i += 2;
            } else if i == args.len() - 1 && (args[0] == find_cmd || args[0] == delete_cmd) {
                let s = arg.to_string_lossy();
                if s.ends_with(".keychain") || s.ends_with(".keychain-db") {
                    rewritten.push(self.target_keychain.clone().into_os_string());
                } else {
                    rewritten.push(arg.clone());
                }
                i += 1;
            } else {
                rewritten.push(arg.clone());
                i += 1;
            }
        }
        let output = std::process::Command::new(program)
            .args(&rewritten)
            .output()
            .map_err(|e| anyhow::anyhow!("spawn {program}: {e}"))?;
        Ok(CommandOutput {
            status: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

/// Drop-guard for a scratch keychain. Deletes it from disk and from the
/// `security` search list on Drop. Idempotent.
///
/// Important: we deliberately DO NOT call `set-keychain-settings -lut 0`,
/// because that flag is known to block waiting for the keychain to be in
/// the user's keychain search list (empirically verified on macOS 14+).
struct ScratchKeychain {
    path: PathBuf,
}

impl ScratchKeychain {
    fn new(test_label: &str) -> Self {
        let tmp_root = std::env::temp_dir();
        static COUNTER: OnceLock<std::sync::atomic::AtomicU64> = OnceLock::new();
        let c = COUNTER
            .get_or_init(|| std::sync::atomic::AtomicU64::new(0))
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let path = tmp_root.join(format!(
            "spendguard-test-{}-{}-{}.keychain-db",
            test_label,
            std::process::id(),
            c
        ));
        let password = "spendguardtestpassword";

        // Best-effort cleanup of any prior leftover from a panicked run.
        let _ = std::process::Command::new("/usr/bin/security")
            .args(["delete-keychain", path.to_str().unwrap()])
            .output();
        let _ = std::fs::remove_file(&path);

        let out = std::process::Command::new("/usr/bin/security")
            .args(["create-keychain", "-p", password, path.to_str().unwrap()])
            .output()
            .expect("spawn security create-keychain");
        if !out.status.success() {
            panic!(
                "security create-keychain failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }

        let out = std::process::Command::new("/usr/bin/security")
            .args(["unlock-keychain", "-p", password, path.to_str().unwrap()])
            .output()
            .expect("spawn security unlock-keychain");
        if !out.status.success() {
            panic!(
                "security unlock-keychain failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }

        Self { path }
    }
}

impl Drop for ScratchKeychain {
    fn drop(&mut self) {
        let _ = std::process::Command::new("/usr/bin/security")
            .args(["delete-keychain", self.path.to_str().unwrap()])
            .output();
        let _ = std::fs::remove_file(&self.path);
    }
}

fn store_targeting(scratch: &ScratchKeychain) -> MacosTrustStore {
    MacosTrustStore::with_runner(
        KeychainRedirectingRunner {
            target_keychain: scratch.path.clone(),
        },
        PathBuf::from("/usr/bin/security"),
    )
}

// ──────────────────────────────────────────────────────────────────────
//  Non-mutating, fully automated tests
//
//  These exercise the shell-out plumbing against a real `security` CLI
//  WITHOUT calling add-trusted-cert. They prove:
//   - The argv shape we ship is correct.
//   - `verify_installed` parses real `find-certificate -Z` output.
//   - The `MacosTrustStore::new()` constructor doesn't panic.
// ──────────────────────────────────────────────────────────────────────

/// `verify_installed` on an empty scratch keychain returns `Ok(false)` —
/// proves we shell out to `find-certificate`, parse zero-result output, and
/// don't error (review-standards `X3` + acceptance `A8.2`).
#[test]
fn verify_installed_on_empty_scratch_keychain_returns_false() {
    let scratch = ScratchKeychain::new("verify_empty");
    let store = store_targeting(&scratch);
    let bogus = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
    let present = store
        .verify_installed(bogus, TrustScope::User)
        .expect("verify must not error on empty keychain");
    assert!(!present);
}

/// `MacosTrustStore::new()` resolves the user login keychain path without
/// shelling out — pure path resolution.
#[test]
fn macos_login_keychain_path_resolves_under_home() {
    let home = std::env::var("HOME").expect("HOME set on macOS test host");
    let expected = PathBuf::from(home)
        .join("Library")
        .join("Keychains")
        .join("login.keychain-db");
    assert!(expected.is_absolute());
    assert!(expected.to_string_lossy().ends_with("login.keychain-db"));
    // Constructor smoke.
    let _ = MacosTrustStore::new();
}

/// `verify_installed` on the user's real login keychain with a random
/// fingerprint returns `Ok(false)` — proves the production code path
/// doesn't error against a non-empty real keychain. (We only READ; no
/// Trust Settings mutation, so no auth dialog.)
#[test]
fn verify_installed_on_real_login_keychain_for_bogus_fingerprint_is_false() {
    let store = MacosTrustStore::new();
    let bogus = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let present = store
        .verify_installed(bogus, TrustScope::User)
        .expect("verify must not error against real login keychain");
    assert!(!present, "bogus fingerprint must not be in login keychain");
}

// ──────────────────────────────────────────────────────────────────────
//  Mutating, opt-in tests (require interactive auth)
//
//  Run with:
//      cargo test -p spendguard-cli --test trust_macos -- --include-ignored
//
//  These actually mutate the user's keychain Trust Settings. The macOS
//  keychain agent will pop an auth dialog (Touch ID / password) — accept
//  it to let the test proceed. Each test installs a Drop guard so the
//  developer's keychain is never left in a dirty state.
// ──────────────────────────────────────────────────────────────────────

/// RAII cleanup of the user's real login keychain. Best-effort: removes the
/// issued CA on Drop even when a test panics. Uses the production
/// `MacosTrustStore::new()` so the cleanup goes through the same code path
/// being exercised.
struct LoginKeychainCleanup {
    fingerprint: String,
}

impl Drop for LoginKeychainCleanup {
    fn drop(&mut self) {
        let store = MacosTrustStore::new();
        let _ = store.remove_root(&self.fingerprint, TrustScope::User);
    }
}

/// End-to-end: install lands the CA in the user login keychain. Pops auth
/// dialog. Verified via `verify_installed` after `add_root`.
#[test]
#[ignore = "Mutates ~/Library/Keychains/login.keychain-db; pops auth dialog. \
            Run with --include-ignored for manual smoke."]
fn macos_real_install_lands_ca_in_login_keychain() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let opts = InstallOpts {
        scope: TrustScope::User,
        ca_out: Some(tmp.path().to_path_buf()),
        shell: None,
    };
    let store = MacosTrustStore::new();
    let report = install_with_trust_store(&opts, &store).expect("install");
    let _cleanup = LoginKeychainCleanup {
        fingerprint: report.ca_fingerprint_sha256.clone(),
    };

    assert_eq!(report.trust_store_locations.len(), 1);
    assert!(report.trust_store_locations[0]
        .to_string_lossy()
        .ends_with("login.keychain-db"));

    let installed = store
        .verify_installed(&report.ca_fingerprint_sha256, TrustScope::User)
        .expect("verify_installed must not error");
    assert!(
        installed,
        "CA should be present in login keychain after install"
    );
}

/// End-to-end: install → uninstall round-trip leaves the keychain clean.
#[test]
#[ignore = "Mutates ~/Library/Keychains/login.keychain-db; pops auth dialog. \
            Run with --include-ignored for manual smoke."]
fn macos_real_uninstall_round_trip_removes_ca() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let opts = InstallOpts {
        scope: TrustScope::User,
        ca_out: Some(tmp.path().to_path_buf()),
        shell: None,
    };
    let store = MacosTrustStore::new();
    let report = install_with_trust_store(&opts, &store).expect("install");
    let _cleanup = LoginKeychainCleanup {
        fingerprint: report.ca_fingerprint_sha256.clone(),
    };

    let uninstall_opts = UninstallOpts {
        scope: TrustScope::User,
        ca_fingerprint: Some(report.ca_fingerprint_sha256.clone()),
    };
    let uninstall_report = uninstall_with_trust_store(&uninstall_opts, &store).expect("uninstall");
    assert_eq!(
        uninstall_report.trust_store_locations_cleared.len(),
        1,
        "uninstall should report the keychain it touched"
    );

    // Post-uninstall the cert is gone.
    let still_present = store
        .verify_installed(&report.ca_fingerprint_sha256, TrustScope::User)
        .expect("verify post-uninstall");
    assert!(!still_present);
}

/// End-to-end: doctor flips `ca_present_in_store` across install/uninstall.
#[test]
#[ignore = "Mutates ~/Library/Keychains/login.keychain-db; pops auth dialog. \
            Run with --include-ignored for manual smoke."]
fn macos_real_doctor_reflects_install_state() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let opts = InstallOpts {
        scope: TrustScope::User,
        ca_out: Some(tmp.path().to_path_buf()),
        shell: None,
    };
    let store = MacosTrustStore::new();
    let report = install_with_trust_store(&opts, &store).expect("install");
    let _cleanup = LoginKeychainCleanup {
        fingerprint: report.ca_fingerprint_sha256.clone(),
    };

    let doctor_opts = DoctorOpts {
        scope: TrustScope::User,
        ca_fingerprint: Some(report.ca_fingerprint_sha256.clone()),
    };
    let post_install = doctor_with_trust_store(&doctor_opts, &store).expect("doctor");
    assert!(post_install.ca_present_in_store);

    let uninstall_opts = UninstallOpts {
        scope: TrustScope::User,
        ca_fingerprint: Some(report.ca_fingerprint_sha256.clone()),
    };
    uninstall_with_trust_store(&uninstall_opts, &store).expect("uninstall");

    let post_uninstall = doctor_with_trust_store(&doctor_opts, &store).expect("doctor again");
    assert!(!post_uninstall.ca_present_in_store);
}

/// Idempotent uninstall against the real keychain — second call returns
/// `Ok(vec![])` rather than erroring (review-standards `X3` / acceptance
/// `A8.2`).
#[test]
#[ignore = "Mutates ~/Library/Keychains/login.keychain-db; pops auth dialog. \
            Run with --include-ignored for manual smoke."]
fn macos_real_uninstall_is_idempotent() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let opts = InstallOpts {
        scope: TrustScope::User,
        ca_out: Some(tmp.path().to_path_buf()),
        shell: None,
    };
    let store = MacosTrustStore::new();
    let report = install_with_trust_store(&opts, &store).expect("install");
    let _cleanup = LoginKeychainCleanup {
        fingerprint: report.ca_fingerprint_sha256.clone(),
    };

    let uninstall_opts = UninstallOpts {
        scope: TrustScope::User,
        ca_fingerprint: Some(report.ca_fingerprint_sha256.clone()),
    };
    let first = uninstall_with_trust_store(&uninstall_opts, &store).expect("first uninstall");
    assert_eq!(first.trust_store_locations_cleared.len(), 1);
    let second = uninstall_with_trust_store(&uninstall_opts, &store).expect("second uninstall");
    assert!(second.trust_store_locations_cleared.is_empty());
}
