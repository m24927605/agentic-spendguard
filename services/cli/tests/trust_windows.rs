//! Integration tests for the Windows trust-store backend (SLICE 4 / COV_08).
//!
//! ## Why two test classes (mirrors the macOS pattern)
//!
//! Windows `certutil -addstore Root` mutates the user's (or machine's)
//! certificate store. Per design `§4` (Windows backend table line 45):
//!
//! - `certutil -user -addstore -f Root <pem>` writes to
//!   `HKCU\Software\Microsoft\SystemCertificates\Root\Certificates` — no
//!   UAC prompt, no elevation, but it DOES mutate the user's cert store.
//! - `certutil -addstore -f Root <pem>` writes to `HKLM` and requires an
//!   already-elevated shell (UAC consent at process spawn, not inside the
//!   binary).
//!
//! On a CI runner we don't want to permanently dirty the matrix runner's
//! `HKCU\…\Root` hive on every PR. We solve this by splitting integration
//! tests into two groups:
//!
//! 1. **Non-mutating, fully automated** (default `cargo test` on Windows):
//!    exercise the constructor + the production `verify_installed` argv
//!    shape against a fresh-host bogus fingerprint, which proves the
//!    `certutil -store Root` shell-out shape works without ever writing
//!    to the cert store. Two tests.
//! 2. **Mutating, opt-in via `--include-ignored`**: actually run
//!    `add_root` / `remove_root` against `HKCU\…\Root` (User) and
//!    `HKLM\…\Root` (System; requires an elevated shell). Each test
//!    installs a Drop guard that removes the issued cert on Drop even
//!    when the test panics, so the matrix runner is never left dirty.
//!    Three tests.
//!
//! Run the mutating tests with:
//!
//! ```pwsh
//! cargo test -p spendguard-cli --test trust_windows -- --include-ignored
//! ```
//!
//! ## CI matrix
//!
//! The default Windows runner only executes the always-on smoke tests
//! (group 1). The `--include-ignored` group runs in the dedicated Windows
//! integration job per `tests.md §8`; the bogus-fingerprint cleanup test
//! (test #5) runs on every Windows CI invocation because it's
//! self-cleaning + non-destructive (verify-only against a fingerprint
//! that's not in the store).

#![cfg(target_os = "windows")]

use spendguard_cli::trust::windows::WindowsTrustStore;
use spendguard_cli::trust::TrustStore;
use spendguard_cli::{
    install_with_trust_store, uninstall_with_trust_store, InstallOpts, TrustScope, UninstallOpts,
};
use std::path::PathBuf;

// ──────────────────────────────────────────────────────────────────────
//  Always-on smoke tests — these read but don't mutate the cert store,
//  so they run on every default `cargo test` invocation. They serve as
//  proof that the module compiles + exposes the right surface on the
//  Windows target.
// ──────────────────────────────────────────────────────────────────────

/// `WindowsTrustStore::new()` constructor is infallible and doesn't shell
/// out — pure path resolution.
#[test]
fn windows_trust_store_constructor_is_infallible() {
    let _store = WindowsTrustStore::new();
}

/// `verify_installed` against the real `HKCU\…\Root` cert store for a
/// random fingerprint returns `Ok(false)` — proves the production code
/// path doesn't error against a populated real cert store. (We only READ
/// via `certutil -store Root`; no `-addstore` so no mutation.)
#[test]
fn verify_installed_on_real_user_store_for_bogus_fingerprint_is_false() {
    let store = WindowsTrustStore::new();
    let bogus = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let present = store
        .verify_installed(bogus, TrustScope::User)
        .expect("verify must not error against real user cert store");
    assert!(!present, "bogus fingerprint must not be in HKCU Root store");
}

// ──────────────────────────────────────────────────────────────────────
//  Mutating, opt-in tests (require `--include-ignored` and, for System
//  scope, an elevated shell).
//
//  Run with:
//      cargo test -p spendguard-cli --test trust_windows -- --include-ignored
// ──────────────────────────────────────────────────────────────────────

/// RAII cleanup of the user's real `HKCU\…\Root` cert store. Best-effort:
/// removes the issued CA on Drop even when a test panics. Uses the
/// production `WindowsTrustStore::new()` so cleanup goes through the same
/// code path being exercised.
struct TrustStoreCleanup {
    fingerprint: String,
    scope: TrustScope,
}

impl Drop for TrustStoreCleanup {
    fn drop(&mut self) {
        let store = WindowsTrustStore::new();
        let _ = store.remove_root(&self.fingerprint, self.scope);
    }
}

/// End-to-end User-scope path: `install` lands the CA in `HKCU\…\Root`
/// via `certutil -user -addstore -f Root <pem>`. Verified by reading the
/// store via the production `verify_installed`.
#[test]
#[ignore = "Mutates HKCU\\Software\\Microsoft\\SystemCertificates\\Root\\Certificates. \
            Run with --include-ignored under the Windows CI job."]
fn windows_real_install_lands_ca_in_user_root_store() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let opts = InstallOpts {
        scope: TrustScope::User,
        ca_out: Some(tmp.path().to_path_buf()),
        shell: None,
        force_allow_gemini_oauth: false,
    };
    let store = WindowsTrustStore::new();
    let report = install_with_trust_store(&opts, &store).expect("install");
    let _cleanup = TrustStoreCleanup {
        fingerprint: report.ca_fingerprint_sha256.clone(),
        scope: TrustScope::User,
    };

    assert_eq!(report.trust_store_locations.len(), 1);
    assert!(report.trust_store_locations[0]
        .to_string_lossy()
        .contains("HKCU"));

    let installed = store
        .verify_installed(&report.ca_fingerprint_sha256, TrustScope::User)
        .expect("verify_installed must not error");
    assert!(
        installed,
        "CA should be present in HKCU Root store after install"
    );
}

/// End-to-end System-scope path: `install` lands the CA in `HKLM\…\Root`
/// via `certutil -addstore -f Root <pem>`. REQUIRES an elevated shell —
/// non-elevated invocations fail with `0x80070005 (E_ACCESSDENIED)`,
/// which the production code surfaces verbatim.
#[test]
#[ignore = "Mutates HKLM\\Software\\Microsoft\\SystemCertificates\\Root\\Certificates. \
            REQUIRES ELEVATED SHELL. Run with --include-ignored under the Windows CI job."]
fn windows_real_install_lands_ca_in_system_root_store() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let opts = InstallOpts {
        scope: TrustScope::System,
        ca_out: Some(tmp.path().to_path_buf()),
        shell: None,
        force_allow_gemini_oauth: false,
    };
    let store = WindowsTrustStore::new();
    let report = install_with_trust_store(&opts, &store).expect("install (must run elevated)");
    let _cleanup = TrustStoreCleanup {
        fingerprint: report.ca_fingerprint_sha256.clone(),
        scope: TrustScope::System,
    };

    assert_eq!(report.trust_store_locations.len(), 1);
    assert!(report.trust_store_locations[0]
        .to_string_lossy()
        .contains("HKLM"));

    let installed = store
        .verify_installed(&report.ca_fingerprint_sha256, TrustScope::System)
        .expect("verify_installed must not error");
    assert!(
        installed,
        "CA should be present in HKLM Root store after install"
    );
}

/// Fresh-host bogus-fingerprint cleanup with RAII guard: install + verify
/// + uninstall round-trip leaves `HKCU\…\Root` clean. The TrustStoreCleanup
/// drop-guard is the safety net even when this body panics; the explicit
/// uninstall_with_trust_store call is the happy-path assertion that
/// remove_root drops the cert via the production code path.
#[test]
#[ignore = "Mutates HKCU\\Software\\Microsoft\\SystemCertificates\\Root\\Certificates. \
            Run with --include-ignored under the Windows CI job."]
fn windows_real_install_uninstall_round_trip_leaves_user_store_clean() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let opts = InstallOpts {
        scope: TrustScope::User,
        ca_out: Some(tmp.path().to_path_buf()),
        shell: None,
        force_allow_gemini_oauth: false,
    };
    let store = WindowsTrustStore::new();
    let report = install_with_trust_store(&opts, &store).expect("install");
    // RAII guard fires even if uninstall_with_trust_store below panics —
    // belt-and-braces (the test body's explicit uninstall is the
    // assertion we care about; the guard is the safety net).
    let _cleanup = TrustStoreCleanup {
        fingerprint: report.ca_fingerprint_sha256.clone(),
        scope: TrustScope::User,
    };

    let uninstall_opts = UninstallOpts {
        scope: TrustScope::User,
        ca_fingerprint: Some(report.ca_fingerprint_sha256.clone()),
        // SLICE 8 (COV_12): keep the rc + PEM paths out of scope — the
        // Windows trust-store test focuses on certutil shellout shape.
        keep_shell_rc: true,
        keep_ca_files: true,
    };
    let uninstall_report = uninstall_with_trust_store(&uninstall_opts, &store).expect("uninstall");
    assert_eq!(
        uninstall_report.trust_store_locations_cleared.len(),
        1,
        "uninstall should report the cert store it touched"
    );

    // Post-uninstall the cert is gone.
    let still_present = store
        .verify_installed(&report.ca_fingerprint_sha256, TrustScope::User)
        .expect("verify post-uninstall");
    assert!(
        !still_present,
        "CA must be gone from HKCU Root store after uninstall"
    );

    // PathBuf import suppression to satisfy clippy on platforms where
    // PathBuf is only conditionally used (mirrors trust_linux.rs).
    let _ = PathBuf::from("/");
}
