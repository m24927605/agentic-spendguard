//! SLICE 8 (COV_12) — symmetric uninstall round-trip integration tests.
//!
//! These tests prove that `install_with_backends` + `uninstall_with_backends_full`
//! land their inverses on every artefact the install path touches:
//!
//! 1. The shell rc marker block lands at install time and is stripped at
//!    uninstall time.
//! 2. The four PEM files (CA + leaf + keys) land at install time and are
//!    deleted at uninstall time.
//! 3. The trust-store backend's `add_root` / `remove_root` are called
//!    symmetrically. We use a `NoopTrustStore` here (the real keychain
//!    tests live in `tests/trust_macos.rs` under `--include-ignored`).
//! 4. `doctor::run` post-uninstall returns a fully `Absent`/`NotInstalled`
//!    report — every check flips from its Present variant to its absent
//!    counterpart.
//!
//! ## `#[ignore]` gating
//!
//! These tests are NOT `#[ignore]`-gated by default — they're fully
//! hermetic. The install + uninstall both run against tempdir-rooted HOME
//! via the `home: Some(tmp)` seam on `install_with_backends` /
//! `uninstall_with_backends_full`. The only env mutation is wrapping the
//! `HOME` env var via a process-wide mutex during the test body, mirroring
//! the SLICE 5/6 `HomeGuard` pattern from `lib.rs::tests`.
//!
//! ## Anti-scope
//!
//! - These tests do NOT shell out to bash/zsh/fish/pwsh to source the rc
//!   file. The lib-level `strip_marker_block` tests already prove the
//!   text-level idempotency; the integration tests assert the higher
//!   level "install + uninstall = no SpendGuard residue" contract.
//! - These tests do NOT touch the real OS trust store. The `NoopTrustStore`
//!   records what was added/removed; the real keychain round-trip lives
//!   in `tests/trust_macos.rs` under `--include-ignored`.

use anyhow::Result;
use spendguard_cli::doctor::{
    self, CaCheckResult, DoctorEnv, DoctorOpts as Slice7DoctorOpts, GeminiCheckResult,
    ProxyCheckResult, ShellRcCheckResult,
};
use spendguard_cli::preflight::BaseEnv;
use spendguard_cli::shell::{EnvView, MARKER_BEGIN, MARKER_END};
use spendguard_cli::trust::TrustStore;
use spendguard_cli::{
    install_with_backends, uninstall_with_backends_full, InstallOpts, ShellKind, TrustScope,
    UninstallOpts, DEFAULT_PROXY_URL,
};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

// ──────────────────────────────────────────────────────────────────────
//  Shared scaffolding
// ──────────────────────────────────────────────────────────────────────

/// Inert TrustStore used by the round-trip integration tests so we don't
/// touch the developer's real keychain. Records what add/remove was
/// called with so the round-trip assertions can prove symmetry.
#[derive(Debug, Default)]
struct NoopTrustStore {
    added: Mutex<Vec<PathBuf>>,
    removed: Mutex<Vec<String>>,
    // Tracks whether the cert is "in the store" so verify_installed can
    // return the post-uninstall=false answer for the doctor probe.
    present: Mutex<bool>,
}

impl TrustStore for NoopTrustStore {
    fn add_root(&self, ca_pem_path: &Path, _scope: TrustScope) -> Result<Vec<PathBuf>> {
        self.added.lock().unwrap().push(ca_pem_path.to_path_buf());
        *self.present.lock().unwrap() = true;
        Ok(vec![PathBuf::from("/tmp/noop-trust-store")])
    }
    fn remove_root(&self, fp: &str, _scope: TrustScope) -> Result<Vec<PathBuf>> {
        self.removed.lock().unwrap().push(fp.to_string());
        let was_present = std::mem::replace(&mut *self.present.lock().unwrap(), false);
        if was_present {
            Ok(vec![PathBuf::from("/tmp/noop-trust-store")])
        } else {
            Ok(vec![])
        }
    }
    fn verify_installed(&self, _fp: &str, _scope: TrustScope) -> Result<bool> {
        Ok(*self.present.lock().unwrap())
    }
}

/// Atomic HOME setter for the round-trip tests. Mirrors `HomeGuard` in
/// `services/cli/src/lib.rs::tests` — single shared mutex so the tests
/// in this binary serialise their HOME mutations and the developer's
/// real shell is never racing them.
struct HomeGuard {
    prior: Option<std::ffi::OsString>,
    _lock: MutexGuard<'static, ()>,
}

impl HomeGuard {
    fn set(home: &Path) -> Self {
        static HOME_LOCK: Mutex<()> = Mutex::new(());
        let lock = HOME_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let prior = std::env::var_os("HOME");
        // SAFETY: HOME_LOCK serialises all mutations. Drop restores.
        #[allow(unused_unsafe)]
        unsafe {
            std::env::set_var("HOME", home);
        }
        Self { prior, _lock: lock }
    }
}

impl Drop for HomeGuard {
    fn drop(&mut self) {
        #[allow(unused_unsafe)]
        unsafe {
            match self.prior.take() {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
        }
    }
}

/// Resolve the canonical CA dir under a tempdir HOME — mirrors
/// `paths::base_data_dir_from` for the relevant target_os so install
/// and uninstall both find the PEM at the same canonical location
/// without having to mutate `XDG_DATA_HOME` (avoids a second
/// process-env guard).
fn canonical_ca_dir_under(home: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        home.join("Library")
            .join("Application Support")
            .join("SpendGuard")
            .join("ca")
    }
    #[cfg(target_os = "linux")]
    {
        home.join(".local")
            .join("share")
            .join("spendguard")
            .join("ca")
    }
    #[cfg(target_os = "windows")]
    {
        let _ = home;
        // Windows resolves via APPDATA, not HOME — these tests run on
        // POSIX hosts where the macOS / Linux branches above apply.
        panic!("uninstall_smoke tests are not designed for Windows targets")
    }
}

// ──────────────────────────────────────────────────────────────────────
//  Test 1: full round-trip — install + uninstall leaves no residue
// ──────────────────────────────────────────────────────────────────────

/// Full round-trip: install + uninstall + doctor returns Healthy/Absent.
/// Every install side-effect (shell rc block, 4 PEM files, trust-store
/// entry) is reversed and the doctor report flips to its Absent/NotInstalled
/// variants.
#[test]
fn round_trip_install_then_uninstall_leaves_no_residue() {
    let home_tmp = tempfile::tempdir().expect("home tempdir");
    let ca_dir = canonical_ca_dir_under(home_tmp.path());
    let _guard = HomeGuard::set(home_tmp.path());

    let backend = NoopTrustStore::default();
    let install_opts = InstallOpts {
        scope: TrustScope::User,
        // Point install at the canonical XDG-derived path so the
        // uninstall PEM-resolution step finds the same files. Avoids
        // a second env guard for XDG_DATA_HOME.
        ca_out: Some(ca_dir.clone()),
        shell: Some(ShellKind::Bash),
        force_allow_gemini_oauth: false,
    };
    let env = EnvView {
        shell: Some("/bin/bash"),
        ..Default::default()
    };

    // ─── Install ──────────────────────────────────────────────────────
    let install_report = install_with_backends(
        &install_opts,
        &backend,
        Some(home_tmp.path()),
        env,
        DEFAULT_PROXY_URL,
    )
    .expect("install");

    // All four PEM files landed.
    for p in [
        &install_report.ca_pem_path,
        &install_report.ca_key_path,
        &install_report.leaf_pem_path,
        &install_report.leaf_key_path,
    ] {
        assert!(p.exists(), "expected PEM at {p:?} after install");
    }
    // Shell rc block landed.
    let bashrc = home_tmp.path().join(".bashrc");
    let after_install = std::fs::read_to_string(&bashrc).expect("read .bashrc");
    assert!(
        after_install.contains(MARKER_BEGIN),
        "shell rc must carry SpendGuard marker block after install"
    );
    assert!(after_install.contains("HTTPS_PROXY"));
    // Trust-store add_root called once.
    assert_eq!(backend.added.lock().unwrap().len(), 1);

    // ─── Uninstall ────────────────────────────────────────────────────
    let uninstall_opts = UninstallOpts {
        scope: TrustScope::User,
        ca_fingerprint: None, // Auto-resolve from PEM
        keep_shell_rc: false,
        keep_ca_files: false,
    };
    let uninstall_report =
        uninstall_with_backends_full(&uninstall_opts, &backend, Some(home_tmp.path()), env)
            .expect("uninstall");

    // Full cleanup → no warnings → exit 0 contract.
    assert!(
        uninstall_report.warnings.is_empty(),
        "full round-trip must produce no warnings, got: {warnings:?}",
        warnings = uninstall_report.warnings,
    );

    // 1. Shell rc block stripped.
    let after_uninstall = std::fs::read_to_string(&bashrc).expect("read .bashrc");
    assert!(
        !after_uninstall.contains(MARKER_BEGIN),
        "shell rc must not carry SpendGuard marker after uninstall"
    );
    assert!(
        !after_uninstall.contains(MARKER_END),
        "shell rc must not carry SpendGuard end marker after uninstall"
    );
    assert!(
        !after_uninstall.contains("HTTPS_PROXY"),
        "HTTPS_PROXY export must be stripped"
    );

    // 2. Trust-store remove_root called with the re-derived fingerprint.
    let removed = backend.removed.lock().unwrap().clone();
    assert_eq!(
        removed.len(),
        1,
        "remove_root must have been called exactly once"
    );
    assert_eq!(
        removed[0], install_report.ca_fingerprint_sha256,
        "uninstall must re-derive the same fingerprint install wrote"
    );

    // 3. All four PEM files deleted.
    for p in [
        &install_report.ca_pem_path,
        &install_report.ca_key_path,
        &install_report.leaf_pem_path,
        &install_report.leaf_key_path,
    ] {
        assert!(
            !p.exists(),
            "expected PEM at {p:?} to be deleted after uninstall, but it persists"
        );
    }

    // 4. Backend tracks "not present" after the uninstall.
    let still_present = backend
        .verify_installed(&install_report.ca_fingerprint_sha256, TrustScope::User)
        .expect("verify post-uninstall");
    assert!(
        !still_present,
        "trust backend must report cert absent after uninstall"
    );

    // 5. doctor::run() post-uninstall reports every check at its absent
    //    variant. Per slice doc: "all checks Absent/NotInstalled" — the
    //    overall report is `Degraded` (not Healthy, because Healthy
    //    requires `CaCheckResult::Healthy`), but the per-check shape
    //    proves the install truly left no trace. Deviation #5: the
    //    slice doc's "is_healthy() returns Healthy" wording is
    //    contradictory with the doctor logic which requires CA-in-store
    //    for Healthy; we interpret the intent as "every check is at its
    //    Absent variant", and assert that here.
    let doctor_env = BaseEnv {
        home: Some(home_tmp.path()),
        gemini_api_key: None,
        google_application_credentials: None,
    };
    let doctor_opts = Slice7DoctorOpts::default();
    let doctor_env_inj = DoctorEnv {
        ca_pem_override: None, // Use the canonical path (PEM is deleted).
        shell_env: Some(EnvView::default()),
        tcp_probe: Some(doctor::proxy_reachable::TcpProbe::always_unreachable()),
    };
    let post_uninstall = doctor::run_with_env(&doctor_env, doctor_opts, doctor_env_inj);

    assert!(
        matches!(post_uninstall.ca, CaCheckResult::NotInstalled),
        "post-uninstall CA check must be NotInstalled, got: {ca:?}",
        ca = post_uninstall.ca,
    );
    assert!(
        matches!(
            post_uninstall.proxy,
            ProxyCheckResult::ProxyUnreachable { .. }
        ),
        "post-uninstall proxy check must be unreachable (no proxy running), got: {proxy:?}",
        proxy = post_uninstall.proxy,
    );
    assert!(
        matches!(post_uninstall.shell, ShellRcCheckResult::Absent { .. }),
        "post-uninstall shell check must be Absent (we passed empty EnvView), got: {shell:?}",
        shell = post_uninstall.shell,
    );
    assert!(
        matches!(post_uninstall.gemini, GeminiCheckResult::NotInstalled),
        "post-uninstall gemini check must be NotInstalled (no OAuth state seeded), got: {gemini:?}",
        gemini = post_uninstall.gemini,
    );
}

// ──────────────────────────────────────────────────────────────────────
//  Test 2: partial install + uninstall — cleans whatever's there
// ──────────────────────────────────────────────────────────────────────

/// Partial install (only the shell rc, no PEM files written) + uninstall
/// cleans the rc and returns success even though the PEM step has nothing
/// to do. Mirrors the operator who edits ~/.bashrc by hand and then runs
/// `spendguard uninstall` to clear it.
#[test]
fn partial_install_then_uninstall_cleans_rc_only() {
    let home_tmp = tempfile::tempdir().expect("home tempdir");
    let _guard = HomeGuard::set(home_tmp.path());

    // Hand-seed a marker block in ~/.bashrc — simulates an operator
    // who copied the breadcrumb from a prior install report.
    let bashrc = home_tmp.path().join(".bashrc");
    std::fs::write(
        &bashrc,
        format!(
            "alias l='ls -la'\n{MARKER_BEGIN}\nexport HTTPS_PROXY=\"https://localhost:8443\"\n{MARKER_END}\n",
        ),
    )
    .expect("seed .bashrc");

    let backend = NoopTrustStore::default();
    let uninstall_opts = UninstallOpts {
        scope: TrustScope::User,
        ca_fingerprint: None,
        keep_shell_rc: false,
        keep_ca_files: false,
    };
    let env = EnvView {
        shell: Some("/bin/bash"),
        ..Default::default()
    };

    let report =
        uninstall_with_backends_full(&uninstall_opts, &backend, Some(home_tmp.path()), env)
            .expect("uninstall");

    // Rc stripped.
    let after = std::fs::read_to_string(&bashrc).expect("read");
    assert!(!after.contains(MARKER_BEGIN));
    assert!(after.contains("alias l='ls -la'"));

    // PEM-side step warned (no PEM to read for fingerprint), trust step
    // skipped — but the call returned Ok and the rc step DID land.
    assert_eq!(report.removed_files, vec![bashrc.clone()]);
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("trust-store removal skipped")),
        "missing PEM must surface trust-store-skipped warning"
    );
}

// ──────────────────────────────────────────────────────────────────────
//  Test 3: --keep-shell-rc preserves the rc marker block
// ──────────────────────────────────────────────────────────────────────

/// `--keep-shell-rc` preserves the rc marker block while still removing
/// the trust-store entry + deleting PEM files. Demonstrates the opt-out
/// flag is wired through to `strip_shell_rc`.
#[test]
fn keep_shell_rc_preserves_marker_block_after_uninstall() {
    let home_tmp = tempfile::tempdir().expect("home tempdir");
    let ca_dir = canonical_ca_dir_under(home_tmp.path());
    let _guard = HomeGuard::set(home_tmp.path());

    let backend = NoopTrustStore::default();
    let install_opts = InstallOpts {
        scope: TrustScope::User,
        ca_out: Some(ca_dir),
        shell: Some(ShellKind::Bash),
        force_allow_gemini_oauth: false,
    };
    let env = EnvView {
        shell: Some("/bin/bash"),
        ..Default::default()
    };

    let install_report = install_with_backends(
        &install_opts,
        &backend,
        Some(home_tmp.path()),
        env,
        DEFAULT_PROXY_URL,
    )
    .expect("install");

    let bashrc = home_tmp.path().join(".bashrc");
    let before = std::fs::read_to_string(&bashrc).expect("read");
    assert!(before.contains(MARKER_BEGIN));

    // Uninstall with --keep-shell-rc.
    let uninstall_opts = UninstallOpts {
        scope: TrustScope::User,
        ca_fingerprint: None,
        keep_shell_rc: true,
        keep_ca_files: false,
    };
    let report =
        uninstall_with_backends_full(&uninstall_opts, &backend, Some(home_tmp.path()), env)
            .expect("uninstall with --keep-shell-rc");
    assert!(report.warnings.is_empty(), "no warnings expected");

    // Rc block PRESERVED.
    let after = std::fs::read_to_string(&bashrc).expect("read");
    assert!(
        after.contains(MARKER_BEGIN),
        "--keep-shell-rc must preserve the marker block"
    );
    assert!(after.contains("HTTPS_PROXY"));

    // Trust step still ran.
    assert_eq!(backend.removed.lock().unwrap().len(), 1);

    // PEM files DELETED — opposite of keep flag.
    assert!(!install_report.ca_pem_path.exists());
    assert!(!install_report.ca_key_path.exists());
}

// ──────────────────────────────────────────────────────────────────────
//  Test 4: --keep-ca-files preserves PEM blobs
// ──────────────────────────────────────────────────────────────────────

/// `--keep-ca-files` preserves the four PEM files while still stripping
/// the rc + removing the trust-store entry. Mirrors the operator who
/// wants to retain the CA for audit / rotation but disable the install.
#[test]
fn keep_ca_files_preserves_pem_blobs_after_uninstall() {
    let home_tmp = tempfile::tempdir().expect("home tempdir");
    let ca_dir = canonical_ca_dir_under(home_tmp.path());
    let _guard = HomeGuard::set(home_tmp.path());

    let backend = NoopTrustStore::default();
    let install_opts = InstallOpts {
        scope: TrustScope::User,
        ca_out: Some(ca_dir),
        shell: Some(ShellKind::Bash),
        force_allow_gemini_oauth: false,
    };
    let env = EnvView {
        shell: Some("/bin/bash"),
        ..Default::default()
    };

    let install_report = install_with_backends(
        &install_opts,
        &backend,
        Some(home_tmp.path()),
        env,
        DEFAULT_PROXY_URL,
    )
    .expect("install");

    let bashrc = home_tmp.path().join(".bashrc");

    // Uninstall with --keep-ca-files.
    let uninstall_opts = UninstallOpts {
        scope: TrustScope::User,
        ca_fingerprint: None,
        keep_shell_rc: false,
        keep_ca_files: true,
    };
    let report =
        uninstall_with_backends_full(&uninstall_opts, &backend, Some(home_tmp.path()), env)
            .expect("uninstall with --keep-ca-files");
    assert!(report.warnings.is_empty());

    // Rc block STRIPPED.
    let after = std::fs::read_to_string(&bashrc).expect("read");
    assert!(!after.contains(MARKER_BEGIN));

    // Trust step ran with the auto-resolved fingerprint.
    let removed = backend.removed.lock().unwrap().clone();
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0], install_report.ca_fingerprint_sha256);

    // PEM files PRESERVED.
    for p in [
        &install_report.ca_pem_path,
        &install_report.ca_key_path,
        &install_report.leaf_pem_path,
        &install_report.leaf_key_path,
    ] {
        assert!(
            p.exists(),
            "--keep-ca-files must preserve PEM at {p:?}, but it was deleted"
        );
    }
}

// ──────────────────────────────────────────────────────────────────────
//  Test 5: idempotent uninstall — second call is a no-op
// ──────────────────────────────────────────────────────────────────────

/// Running uninstall twice is safe — the second invocation finds nothing
/// to clean and returns success (with warnings noting the absent PEM,
/// matching the partial-install pattern). Proves review-standards X3
/// "absent → Ok" semantics propagate through every step.
#[test]
fn uninstall_is_idempotent() {
    let home_tmp = tempfile::tempdir().expect("home tempdir");
    let ca_dir = canonical_ca_dir_under(home_tmp.path());
    let _guard = HomeGuard::set(home_tmp.path());

    let backend = NoopTrustStore::default();
    let install_opts = InstallOpts {
        scope: TrustScope::User,
        ca_out: Some(ca_dir),
        shell: Some(ShellKind::Bash),
        force_allow_gemini_oauth: false,
    };
    let env = EnvView {
        shell: Some("/bin/bash"),
        ..Default::default()
    };

    install_with_backends(
        &install_opts,
        &backend,
        Some(home_tmp.path()),
        env,
        DEFAULT_PROXY_URL,
    )
    .expect("install");

    let uninstall_opts = UninstallOpts {
        scope: TrustScope::User,
        ca_fingerprint: None,
        keep_shell_rc: false,
        keep_ca_files: false,
    };

    // First uninstall — full cleanup.
    let first = uninstall_with_backends_full(&uninstall_opts, &backend, Some(home_tmp.path()), env)
        .expect("first uninstall");
    assert!(first.warnings.is_empty(), "first uninstall is clean");
    assert!(!first.removed_files.is_empty());

    // Second uninstall — everything already absent. Returns Ok with a
    // warning about the missing fingerprint source (no PEM to hash).
    let second =
        uninstall_with_backends_full(&uninstall_opts, &backend, Some(home_tmp.path()), env)
            .expect("second uninstall");
    assert!(
        second.removed_files.is_empty(),
        "second uninstall has nothing to remove"
    );
    assert!(
        second.trust_store_locations_cleared.is_empty(),
        "second uninstall trust step is no-op"
    );
    assert!(
        second
            .warnings
            .iter()
            .any(|w| w.contains("trust-store removal skipped")),
        "second uninstall surfaces missing-PEM warning"
    );
}
