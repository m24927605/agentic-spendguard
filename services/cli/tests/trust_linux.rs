//! Integration tests for the Linux multi-distro trust-store backend.
//!
//! ## Why split from the lib unit tests
//!
//! The lib-side `services/cli/src/trust/linux.rs` unit tests drive the
//! `add_root` / `remove_root` / `verify_installed` argv shape with a
//! [`FakeRunner`], which is OS-independent and runs anywhere `cargo test`
//! runs — including the macOS dev laptop (the `linux` module is only
//! compiled under `#[cfg(target_os = "linux")]`, but the FakeRunner tests
//! live inside that module so they are skipped automatically off-target).
//!
//! The tests in *this* file exercise the production code against the real
//! system tools (`update-ca-certificates`, `update-ca-trust`, `trust
//! anchor`) and the real distro filesystem layout. They:
//!
//! 1. require root (writing to `/usr/local/share/ca-certificates/` etc.),
//! 2. mutate the host's trust store (the cert lands in
//!    `/etc/ssl/certs/ca-certificates.crt` after `update-ca-certificates`),
//! 3. are gated behind `#[ignore]` so default `cargo test` is hermetic,
//! 4. only matter on the per-distro CI matrix from `tests.md §8`
//!    (`ubuntu-24.04` / `fedora:42` / `archlinux:base` / `alpine:edge`).
//!
//! Run them locally with:
//!
//! ```sh
//! sudo cargo test -p spendguard-cli --test trust_linux -- --include-ignored
//! ```
//!
//! Each test installs a Drop-guard that strips the cert from the trust
//! store and re-runs the update tool, so the host is never left dirty.

#![cfg(target_os = "linux")]

use spendguard_cli::trust::linux::{detect_distro_from, LinuxDistro, LinuxTrustStore};
use spendguard_cli::trust::TrustStore;
use spendguard_cli::{install_with_trust_store, InstallOpts, TrustScope};
use std::path::PathBuf;

// ──────────────────────────────────────────────────────────────────────
//  Always-on tests — these don't mutate trust state, so they run on the
//  default `cargo test` invocation. They serve as smoke tests that the
//  module compiles and exposes the right surface on the Linux target.
// ──────────────────────────────────────────────────────────────────────

/// `LinuxTrustStore::new()` reads `/etc/os-release` without panicking,
/// even when the file is absent (treated as Unknown).
#[test]
fn linux_trust_store_constructor_is_infallible() {
    let _store = LinuxTrustStore::new();
}

/// Detection on the canonical Ubuntu 24.04 string matches Debian family.
/// Pure-function check; no FS access.
#[test]
fn detect_ubuntu_24_04_string_maps_to_debian() {
    let s = r#"PRETTY_NAME="Ubuntu 24.04 LTS"
NAME="Ubuntu"
VERSION_ID="24.04"
VERSION="24.04 LTS (Noble Numbat)"
ID=ubuntu
ID_LIKE=debian
"#;
    assert_eq!(detect_distro_from(s), LinuxDistro::Debian);
}

// ──────────────────────────────────────────────────────────────────────
//  Mutating, opt-in tests — `#[ignore]`-gated so default `cargo test`
//  stays hermetic. The CI matrix runs each test inside the right distro
//  container with the right system tools pre-installed.
// ──────────────────────────────────────────────────────────────────────

/// RAII guard that restores the host trust store on Drop. Best-effort:
/// removes the staged anchor file (if any) and re-runs the distro's
/// update tool. Each test installs one of these so a mid-test panic
/// doesn't leave SpendGuard residue.
struct TrustStoreCleanup {
    store: LinuxTrustStore,
    fingerprint: String,
}

impl Drop for TrustStoreCleanup {
    fn drop(&mut self) {
        let _ = self
            .store
            .remove_root(&self.fingerprint, TrustScope::System);
    }
}

/// End-to-end Debian/Ubuntu path: `install` lands the CA at
/// `/usr/local/share/ca-certificates/spendguard.crt` AND runs
/// `update-ca-certificates`, which flushes the cert into
/// `/etc/ssl/certs/ca-certificates.crt`. Verified by reading the bundle
/// and grepping for the cert PEM (the bundle inlines anchor PEMs).
///
/// Requires root + the `ca-certificates` package; gated to skip on
/// non-Debian hosts.
#[test]
#[ignore = "Mutates /usr/local/share/ca-certificates/; requires sudo + debian/ubuntu/alpine host. \
            Run with --include-ignored under the CI matrix or sudo locally."]
fn debian_install_lands_cert_in_merged_bundle() {
    let store = LinuxTrustStore::new();
    if !matches!(store.distro(), LinuxDistro::Debian | LinuxDistro::Alpine) {
        eprintln!(
            "skipping: detected distro {:?}, this test is for Debian/Alpine family",
            store.distro()
        );
        return;
    }

    let tmp = tempfile::tempdir().expect("tempdir");
    let opts = InstallOpts {
        scope: TrustScope::System,
        ca_out: Some(tmp.path().to_path_buf()),
        shell: None,
        force_allow_gemini_oauth: false,
    };
    let report = install_with_trust_store(&opts, &store).expect("install");
    let _cleanup = TrustStoreCleanup {
        store: LinuxTrustStore::new(),
        fingerprint: report.ca_fingerprint_sha256.clone(),
    };

    // Anchor file landed where we expect.
    assert_eq!(report.trust_store_locations.len(), 1);
    assert!(report.trust_store_locations[0]
        .to_string_lossy()
        .contains("/ca-certificates/spendguard.crt"));

    // `update-ca-certificates` ran and the merged bundle now contains
    // the cert. We probe via the verify path so the assertion goes
    // through the same code the doctor report uses (`X3`).
    let present = store
        .verify_installed(&report.ca_fingerprint_sha256, TrustScope::System)
        .expect("verify_installed");
    assert!(
        present,
        "post-install verify must find fingerprint in merged bundle"
    );
}

/// End-to-end RHEL/Fedora path: `install` lands the CA at
/// `/etc/pki/ca-trust/source/anchors/spendguard.crt` AND runs
/// `update-ca-trust extract`, which writes the cert into
/// `/etc/pki/ca-trust/extracted/pem/tls-ca-bundle.pem`.
#[test]
#[ignore = "Mutates /etc/pki/ca-trust/source/anchors/; requires sudo + rhel/fedora host. \
            Run with --include-ignored under the CI matrix or sudo locally."]
fn rhel_install_lands_cert_in_extracted_bundle() {
    let store = LinuxTrustStore::new();
    if store.distro() != LinuxDistro::Rhel {
        eprintln!(
            "skipping: detected distro {:?}, this test is for RHEL family",
            store.distro()
        );
        return;
    }

    let tmp = tempfile::tempdir().expect("tempdir");
    let opts = InstallOpts {
        scope: TrustScope::System,
        ca_out: Some(tmp.path().to_path_buf()),
        shell: None,
        force_allow_gemini_oauth: false,
    };
    let report = install_with_trust_store(&opts, &store).expect("install");
    let _cleanup = TrustStoreCleanup {
        store: LinuxTrustStore::new(),
        fingerprint: report.ca_fingerprint_sha256.clone(),
    };

    assert_eq!(report.trust_store_locations.len(), 1);
    assert!(report.trust_store_locations[0]
        .to_string_lossy()
        .contains("/ca-trust/source/anchors/spendguard.crt"));

    let present = store
        .verify_installed(&report.ca_fingerprint_sha256, TrustScope::System)
        .expect("verify_installed");
    assert!(
        present,
        "post-install verify must find fingerprint in tls-ca-bundle.pem"
    );
}

/// End-to-end Arch path: `install` invokes `trust anchor --store <pem>`
/// against p11-kit; we then assert the cert appears in `trust list
/// --filter=ca-anchors` output via the production `verify_installed`.
#[test]
#[ignore = "Mutates p11-kit anchors; requires arch host with trust(1) installed. \
            Run with --include-ignored under the CI matrix or sudo locally."]
fn arch_install_lands_cert_in_p11_kit() {
    let store = LinuxTrustStore::new();
    if store.distro() != LinuxDistro::Arch {
        eprintln!(
            "skipping: detected distro {:?}, this test is for Arch",
            store.distro()
        );
        return;
    }

    let tmp = tempfile::tempdir().expect("tempdir");
    let opts = InstallOpts {
        scope: TrustScope::System,
        ca_out: Some(tmp.path().to_path_buf()),
        shell: None,
        force_allow_gemini_oauth: false,
    };
    let report = install_with_trust_store(&opts, &store).expect("install");
    let _cleanup = TrustStoreCleanup {
        store: LinuxTrustStore::new(),
        fingerprint: report.ca_fingerprint_sha256.clone(),
    };

    let present = store
        .verify_installed(&report.ca_fingerprint_sha256, TrustScope::System)
        .expect("verify_installed");
    assert!(
        present,
        "post-install `trust list` must include the fingerprint"
    );
}

/// Idempotent `verify_installed` on a fresh, never-installed-to system
/// returns `Ok(false)` rather than erroring — review-standards `X3` /
/// acceptance `A8.2`. Runs on any Linux host because it touches no
/// system mutating tools, only the read-side bundle parse.
#[test]
#[ignore = "Reads /etc/ssl/certs/ca-certificates.crt etc.; safe but distro-dependent. \
            Run with --include-ignored under the CI matrix for evidence."]
fn verify_installed_on_unmodified_host_for_bogus_fingerprint_is_false() {
    let store = LinuxTrustStore::new();
    // `verify_installed` on a bogus fingerprint MUST return Ok(false)
    // regardless of which distro we landed on — even Unknown.
    let bogus = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let present = store
        .verify_installed(bogus, TrustScope::System)
        .expect("verify must not error against unmodified host");
    assert!(
        !present,
        "bogus fingerprint must not be in the host trust bundle"
    );

    // PathBuf import suppression to satisfy clippy::needless_collect on
    // unused imports across feature flags.
    let _ = PathBuf::from("/");
}
