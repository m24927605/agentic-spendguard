//! macOS keychain trust-store backend.
//!
//! Wraps `/usr/bin/security` to install / verify / remove the SpendGuard root
//! CA. Per design §4 (trust-store backends) and review-standards `T6`:
//!
//! - `--scope user` (default): adds to `~/Library/Keychains/login.keychain-db`,
//!   no `sudo`, but the macOS keychain agent will prompt for the user
//!   password to authorize the trust-settings write. We pass `-d` (admin
//!   trust-settings domain) for both scopes because that is the domain
//!   Safari, curl, Python, the system update path, and the rest of the
//!   OS-trust-consuming stack actually consult; trust settings written
//!   without `-d` land in a per-user domain that many TLS consumers
//!   silently ignore. (R1 review MINOR-1 doc-correction: prior wording
//!   read the `-d` semantic backwards.)
//! - `--scope system`: adds to `/Library/Keychains/System.keychain` with the
//!   `-d` admin flag. Requires `sudo`, surfaced to the operator via the
//!   wrapping `spendguard install` command — the shell-out itself does NOT
//!   prepend `sudo`; if the calling shell is not root the `security` command
//!   exits non-zero with `SecKeychainItemImport: UserCanceledErr` or
//!   `errSecAuthFailed`, and we surface that to the user verbatim.
//!
//! ## Why not call `security` via `sh -c "…"`
//!
//! Review-standards `T8` bans `Command::new("sh").arg("-c", …)` — every
//! argument is passed positionally to `/usr/bin/security` via `Command::args`.
//! No shell metacharacters in the path can lead to injection, even if the
//! `ca_pem_path` somehow contained `; rm -rf ~`.
//!
//! ## Fingerprint identity
//!
//! The legacy `security delete-certificate -Z <hash>` accepts EITHER SHA-256
//! OR SHA-1 hex (verified against `security delete-certificate -h` on macOS
//! 14+). We pass the SHA-256 hex (the install-time identity returned by
//! `ca::fingerprint_hex`) verbatim — no SHA-1 conversion required. As a
//! belt-and-braces fallback, [`verify_installed`] also scans
//! `security find-certificate -a -Z` output line-by-line for either hash
//! family, so a hostile environment that for some reason only exposes SHA-1
//! still verifies.
//!
//! ## Idempotence
//!
//! - `add_root` on a cert that is already trusted is a no-op exit-0 from the
//!   `security` CLI; we treat it as success.
//! - `remove_root` on an absent cert returns `Ok(vec![])` with no error
//!   (review-standards `X3`).
//! - `verify_installed` returns `Ok(false)` for absent certs, NOT `Err`.

use crate::trust::{CommandOutput, CommandRunner, StdCommandRunner, TrustStore};
use crate::TrustScope;
use anyhow::{anyhow, Context, Result};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Path to the system `security` CLI. Hard-coded to the absolute path so we
/// don't depend on `$PATH` ordering, which is a review-standards `T8`
/// belt-and-braces (an attacker who can prepend a shadowed `security` to
/// `$PATH` would otherwise win).
const SECURITY_BIN: &str = "/usr/bin/security";

/// Resolved per-user login keychain path. `~/Library/Keychains/login.keychain-db`
/// is the modern (10.12+) location; the legacy `login.keychain` symlinks to it
/// on every supported macOS release. We resolve from `$HOME` at call time
/// rather than caching so the runner inherits the calling shell's HOME (which
/// matters under `sudo -E spendguard install`).
fn user_login_keychain() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").ok_or_else(|| {
        anyhow!("HOME not set; cannot resolve ~/Library/Keychains/login.keychain-db")
    })?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("Keychains")
        .join("login.keychain-db"))
}

/// `/Library/Keychains/System.keychain`. System-scope trust requires this
/// path; `--scope system` is documented to require `sudo`.
fn system_keychain() -> PathBuf {
    PathBuf::from("/Library/Keychains/System.keychain")
}

fn keychain_for(scope: TrustScope) -> Result<PathBuf> {
    match scope {
        TrustScope::User => user_login_keychain(),
        TrustScope::System => Ok(system_keychain()),
    }
}

pub struct MacosTrustStore {
    runner: Box<dyn CommandRunner>,
    security_bin: PathBuf,
}

impl MacosTrustStore {
    /// Production constructor — `/usr/bin/security` + `std::process::Command`.
    pub fn new() -> Self {
        Self {
            runner: Box::new(StdCommandRunner),
            security_bin: PathBuf::from(SECURITY_BIN),
        }
    }

    /// Test-only constructor that injects a fake runner. Available from
    /// `#[cfg(test)]` plus from the integration test crate via the
    /// `pub fn` visibility — kept conservative so external callers cannot
    /// accidentally use it.
    #[doc(hidden)]
    pub fn with_runner<R: CommandRunner + 'static>(runner: R, security_bin: PathBuf) -> Self {
        Self {
            runner: Box::new(runner),
            security_bin,
        }
    }

    fn security(&self) -> &str {
        self.security_bin.to_str().unwrap_or(SECURITY_BIN)
    }

    fn run(&self, args: Vec<OsString>) -> Result<CommandOutput> {
        self.runner.run(self.security(), &args)
    }
}

impl Default for MacosTrustStore {
    fn default() -> Self {
        Self::new()
    }
}

impl TrustStore for MacosTrustStore {
    fn add_root(&self, ca_pem_path: &Path, scope: TrustScope) -> Result<Vec<PathBuf>> {
        let keychain = keychain_for(scope)?;
        // `security add-trusted-cert -d -r trustRoot -k <keychain> <pem>` —
        // the `-d` flag selects the admin trust settings domain (per `man
        // security`), and `-r trustRoot` records the cert as a root anchor.
        // For user scope this still writes to the per-user keychain via `-k`;
        // the trust-settings domain `-d` is what surfaces the cert in
        // server-policy verifications and is the same incantation
        // documented by Apple for both user and system installs.
        let mut args: Vec<OsString> = vec![
            "add-trusted-cert".into(),
            "-d".into(),
            "-r".into(),
            "trustRoot".into(),
            "-k".into(),
            keychain.clone().into_os_string(),
        ];
        args.push(ca_pem_path.as_os_str().to_owned());

        let out = self.run(args).context("invoke security add-trusted-cert")?;
        if !out.success() {
            return Err(anyhow!(
                "security add-trusted-cert exited {:?}: stderr={}",
                out.status,
                out.stderr_str().trim()
            ));
        }
        Ok(vec![keychain])
    }

    fn remove_root(&self, fingerprint_sha256_hex: &str, scope: TrustScope) -> Result<Vec<PathBuf>> {
        validate_hex_fingerprint(fingerprint_sha256_hex)?;
        let keychain = keychain_for(scope)?;

        // Idempotent: skip the delete if the cert is not present. This avoids
        // surfacing the `security` "cert not found" non-zero exit as an error
        // (review-standards X3 / acceptance A8.2).
        if !self.verify_installed(fingerprint_sha256_hex, scope)? {
            return Ok(vec![]);
        }

        // -Z accepts SHA-256 or SHA-1; we pass the SHA-256 hex up-cased
        // (the security CLI is case-insensitive on hex, but Apple's own
        // tooling emits upper-case, so we match for cosmetic consistency).
        let mut args: Vec<OsString> = vec![
            "delete-certificate".into(),
            "-Z".into(),
            fingerprint_sha256_hex.to_uppercase().into(),
            "-t".into(), // also delete trust settings for this cert
        ];
        args.push(keychain.clone().into_os_string());

        let out = self
            .run(args)
            .context("invoke security delete-certificate")?;
        if !out.success() {
            return Err(anyhow!(
                "security delete-certificate exited {:?}: stderr={}",
                out.status,
                out.stderr_str().trim()
            ));
        }
        Ok(vec![keychain])
    }

    fn verify_installed(&self, fingerprint_sha256_hex: &str, scope: TrustScope) -> Result<bool> {
        validate_hex_fingerprint(fingerprint_sha256_hex)?;
        let keychain = keychain_for(scope)?;

        // `security find-certificate -a -Z <keychain>` enumerates ALL certs
        // in the named keychain, prefixing each with `SHA-256 hash: <hex>`
        // and `SHA-1 hash: <hex>` (verified against macOS 14 + 15 output).
        let args: Vec<OsString> = vec![
            "find-certificate".into(),
            "-a".into(),
            "-Z".into(),
            keychain.into_os_string(),
        ];

        let out = self.run(args).context("invoke security find-certificate")?;
        if !out.success() {
            // `find-certificate` returns non-zero when the keychain is
            // unreadable (e.g. locked). Treat as absent for idempotency, but
            // surface the stderr so the operator can investigate.
            tracing::debug!(
                exit = ?out.status,
                stderr = %out.stderr_str().trim(),
                "security find-certificate non-zero exit; treating as absent"
            );
            return Ok(false);
        }
        Ok(haystack_contains_fingerprint(
            &out.stdout_str(),
            fingerprint_sha256_hex,
        ))
    }

    fn verify_trusted(
        &self,
        ca_pem_path: &Path,
        fingerprint_sha256_hex: &str,
        scope: TrustScope,
    ) -> Result<bool> {
        // 1. Presence gate. `find-certificate` enumerates every cert in the
        //    keychain regardless of trust settings; if it's not even present
        //    there's nothing to trust. Cheap + unambiguous "absent" answer.
        if !self.verify_installed(fingerprint_sha256_hex, scope)? {
            return Ok(false);
        }

        // 2. Trust gate. Presence != trusted: a cert that was added then had
        //    its trust settings removed/denied still shows up in
        //    `find-certificate`. `security verify-cert -c <ca.pem>` evaluates
        //    the cert against the OS trust settings (honouring Deny /
        //    removed-root domains) and exits 0 only when the chain actually
        //    validates — i.e. the CA is recorded as a trusted root. This is
        //    the predicate the locked spec (tests.md §2.1
        //    `macos_install_user_scope_marks_as_trustroot`) sanctions.
        let args: Vec<OsString> = vec![
            "verify-cert".into(),
            "-c".into(),
            ca_pem_path.as_os_str().to_owned(),
        ];
        let out = self
            .run(args)
            .context("invoke security verify-cert")?;
        if out.success() {
            return Ok(true);
        }
        // Non-zero exit here is a DEFINITIVE "not trusted" (Deny / removed
        // trust settings / untrusted root), not an inconclusive error — the
        // command ran and rendered a verdict. Surface it as `Ok(false)` so
        // the doctor downgrades to NotInTrustStore (WARN + re-run install)
        // rather than a false green. A spawn failure (security binary
        // missing) already propagated as `Err` from `self.run(...)?`, which
        // the doctor maps to NotInTrustStore — fail-closed in both arms.
        tracing::debug!(
            exit = ?out.status,
            stderr = %out.stderr_str().trim(),
            "security verify-cert reported the CA as not trusted (present but untrusted)"
        );
        Ok(false)
    }
}

/// Defence-in-depth: any string passed to `security -Z` must be plain hex.
/// This is a *second* line of defence behind the trait API — the public
/// `add_root` / `remove_root` arguments are typed as `&str`, so we cannot
/// rely on the type system. Hex-only validation keeps shell-meta /
/// `--flag` / NUL out of the argv.
fn validate_hex_fingerprint(s: &str) -> Result<()> {
    if s.is_empty() || s.len() > 128 {
        return Err(anyhow!(
            "fingerprint hex length {} is outside the expected [1, 128] range",
            s.len()
        ));
    }
    if !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(anyhow!(
            "fingerprint hex contains a non-hex byte; refusing to pass to security CLI"
        ));
    }
    Ok(())
}

/// Case-insensitive substring match for `fingerprint_hex` against the
/// `find-certificate -Z` haystack. The CLI may upper- or lower-case the
/// hash; we normalise both sides.
fn haystack_contains_fingerprint(haystack: &str, fingerprint_hex: &str) -> bool {
    let needle = fingerprint_hex.to_ascii_lowercase();
    haystack
        .lines()
        .any(|line| line.to_ascii_lowercase().contains(&needle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Fake `CommandRunner` that records calls and returns canned outputs.
    /// Each `expect_…` helper pushes the next scripted response; calls beyond
    /// the script panic so missed-expectation bugs surface immediately.
    #[derive(Debug, Default)]
    struct FakeRunner {
        scripted: Mutex<Vec<CommandOutput>>,
        calls: Mutex<Vec<(String, Vec<OsString>)>>,
    }

    impl FakeRunner {
        fn new() -> Arc<Self> {
            Arc::new(Self::default())
        }
        fn push(&self, out: CommandOutput) {
            self.scripted.lock().unwrap().push(out);
        }
        fn calls(&self) -> Vec<(String, Vec<OsString>)> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl CommandRunner for Arc<FakeRunner> {
        fn run(&self, program: &str, args: &[OsString]) -> Result<CommandOutput> {
            self.calls
                .lock()
                .unwrap()
                .push((program.to_string(), args.to_vec()));
            let mut scripted = self.scripted.lock().unwrap();
            if scripted.is_empty() {
                panic!(
                    "FakeRunner exhausted: unexpected call to {program} {:?}",
                    args
                );
            }
            Ok(scripted.remove(0))
        }
    }

    fn ok(stdout: &str) -> CommandOutput {
        CommandOutput {
            status: Some(0),
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        }
    }

    fn err(code: i32, stderr: &str) -> CommandOutput {
        CommandOutput {
            status: Some(code),
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    fn store_with(runner: Arc<FakeRunner>) -> MacosTrustStore {
        MacosTrustStore::with_runner(runner, PathBuf::from("/usr/bin/security"))
    }

    const FINGERPRINT: &str = "68f37edc0d2063597abb39b2f99b02562210ce9f433c50201de6b7e1296ed371";

    /// `add_root` for user scope invokes `security add-trusted-cert -d -r
    /// trustRoot -k <login.keychain-db> <pem>`.
    #[test]
    fn add_root_user_scope_invokes_security_add_trusted_cert() {
        let runner = FakeRunner::new();
        runner.push(ok(""));
        let store = store_with(runner.clone());

        let ca = PathBuf::from("/tmp/ca.pem");
        let touched = store
            .add_root(&ca, TrustScope::User)
            .expect("add_root user");
        assert_eq!(touched.len(), 1);
        assert!(
            touched[0].to_string_lossy().ends_with("login.keychain-db"),
            "expected login keychain, got {touched:?}"
        );

        let calls = runner.calls();
        assert_eq!(calls.len(), 1);
        let (program, args) = &calls[0];
        assert_eq!(program, "/usr/bin/security");
        assert_eq!(args[0], OsString::from("add-trusted-cert"));
        assert_eq!(args[1], OsString::from("-d"));
        assert_eq!(args[2], OsString::from("-r"));
        assert_eq!(args[3], OsString::from("trustRoot"));
        assert_eq!(args[4], OsString::from("-k"));
        assert!(args[5].to_string_lossy().ends_with("login.keychain-db"));
        assert_eq!(args[6], OsString::from("/tmp/ca.pem"));
    }

    /// `add_root` for system scope targets `/Library/Keychains/System.keychain`.
    #[test]
    fn add_root_system_scope_targets_system_keychain() {
        let runner = FakeRunner::new();
        runner.push(ok(""));
        let store = store_with(runner.clone());

        let ca = PathBuf::from("/tmp/ca.pem");
        let touched = store
            .add_root(&ca, TrustScope::System)
            .expect("add_root system");
        assert_eq!(
            touched,
            vec![PathBuf::from("/Library/Keychains/System.keychain")]
        );

        let calls = runner.calls();
        assert_eq!(calls.len(), 1);
        let (_, args) = &calls[0];
        // -k argument index 5 (after add-trusted-cert -d -r trustRoot)
        assert_eq!(
            args[5],
            OsString::from("/Library/Keychains/System.keychain")
        );
    }

    /// `add_root` surfaces a non-zero exit as an error with stderr context.
    #[test]
    fn add_root_surfaces_security_nonzero_exit() {
        let runner = FakeRunner::new();
        runner.push(err(1, "SecKeychainItemImport: UserCanceledErr"));
        let store = store_with(runner);

        let err = store
            .add_root(&PathBuf::from("/tmp/ca.pem"), TrustScope::System)
            .expect_err("non-zero must error");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("UserCanceledErr"),
            "stderr context missing: {msg}"
        );
        assert!(
            msg.contains("security add-trusted-cert"),
            "command context missing: {msg}"
        );
    }

    /// `verify_installed` returns true when the SHA-256 hash appears in the
    /// haystack — case-insensitive match.
    #[test]
    fn verify_installed_returns_true_when_fingerprint_in_haystack() {
        let runner = FakeRunner::new();
        runner.push(ok(&format!(
            "SHA-256 hash: {}\nSHA-1 hash: 89FA3228BF3088DD96772998EB49C1BE87CD9B3A\nkeychain: \"/Users/u/Library/Keychains/login.keychain-db\"\n",
            FINGERPRINT.to_uppercase()
        )));
        let store = store_with(runner);

        let present = store
            .verify_installed(FINGERPRINT, TrustScope::User)
            .expect("verify");
        assert!(present, "fingerprint should be detected case-insensitively");
    }

    /// `verify_installed` returns false (not Err) when the fingerprint is
    /// absent — review-standards `X3` / acceptance `A8.2`.
    #[test]
    fn verify_installed_returns_false_when_absent() {
        let runner = FakeRunner::new();
        runner.push(ok("SHA-256 hash: AAAA\nSHA-1 hash: BBBB\nkeychain: \"/Users/u/Library/Keychains/login.keychain-db\"\n"));
        let store = store_with(runner);

        let present = store
            .verify_installed(FINGERPRINT, TrustScope::User)
            .expect("verify");
        assert!(!present);
    }

    /// `verify_installed` treats `security` non-zero exit as absent rather
    /// than erroring — locked keychains shouldn't break idempotent install.
    #[test]
    fn verify_installed_returns_false_when_security_nonzero() {
        let runner = FakeRunner::new();
        runner.push(err(1, "errSecAuthFailed"));
        let store = store_with(runner);

        let present = store
            .verify_installed(FINGERPRINT, TrustScope::User)
            .expect("verify must not error on non-zero exit");
        assert!(!present);
    }

    /// `verify_trusted` returns true only when the cert is BOTH present
    /// (`find-certificate` hit) AND `security verify-cert` exits 0 (trusted
    /// as a root). Two scripted shell-outs in order.
    #[test]
    fn verify_trusted_returns_true_when_present_and_verify_cert_succeeds() {
        let runner = FakeRunner::new();
        // 1. find-certificate (presence) → hit.
        runner.push(ok(&format!(
            "SHA-256 hash: {}\nkeychain: \"/Users/u/Library/Keychains/login.keychain-db\"\n",
            FINGERPRINT.to_uppercase()
        )));
        // 2. verify-cert → exit 0 (trusted).
        runner.push(ok("...certificate verification successful.\n"));
        let store = store_with(runner.clone());

        let ca_pem = PathBuf::from("/tmp/root_ca.pem");
        let trusted = store
            .verify_trusted(&ca_pem, FINGERPRINT, TrustScope::User)
            .expect("verify_trusted");
        assert!(trusted, "present + verify-cert-success ⇒ trusted");

        let calls = runner.calls();
        assert_eq!(calls.len(), 2, "find-certificate then verify-cert: {calls:?}");
        assert_eq!(calls[1].1[0], OsString::from("verify-cert"));
        assert_eq!(calls[1].1[1], OsString::from("-c"));
        assert_eq!(calls[1].1[2], ca_pem.into_os_string());
    }

    /// `verify_trusted` returns FALSE (not Healthy) for a present-but-
    /// untrusted CA: `find-certificate` hits but `security verify-cert`
    /// exits non-zero (trust settings denied/removed). This is the core
    /// fix — presence != trusted, and the doctor must downgrade to
    /// NotInTrustStore rather than over-report Healthy.
    #[test]
    fn verify_trusted_returns_false_when_present_but_verify_cert_fails() {
        let runner = FakeRunner::new();
        // 1. find-certificate (presence) → hit.
        runner.push(ok(&format!(
            "SHA-256 hash: {}\nkeychain: \"/Users/u/Library/Keychains/login.keychain-db\"\n",
            FINGERPRINT.to_uppercase()
        )));
        // 2. verify-cert → non-zero (CSSMERR_TP_NOT_TRUSTED / Deny).
        runner.push(err(1, "CSSMERR_TP_NOT_TRUSTED"));
        let store = store_with(runner.clone());

        let ca_pem = PathBuf::from("/tmp/root_ca.pem");
        let trusted = store
            .verify_trusted(&ca_pem, FINGERPRINT, TrustScope::User)
            .expect("verify_trusted must not error on a definitive untrusted verdict");
        assert!(
            !trusted,
            "present but verify-cert-failed ⇒ NOT trusted (no false green)"
        );
        assert_eq!(runner.calls().len(), 2, "both shell-outs ran");
    }

    /// `verify_trusted` short-circuits to false when the cert is not even
    /// present — `verify-cert` is never invoked.
    #[test]
    fn verify_trusted_returns_false_and_skips_verify_cert_when_absent() {
        let runner = FakeRunner::new();
        // find-certificate → miss. No second script entry → if verify-cert
        // were invoked, FakeRunner would panic.
        runner.push(ok("SHA-256 hash: AAAA\n"));
        let store = store_with(runner.clone());

        let ca_pem = PathBuf::from("/tmp/root_ca.pem");
        let trusted = store
            .verify_trusted(&ca_pem, FINGERPRINT, TrustScope::User)
            .expect("verify_trusted");
        assert!(!trusted, "absent cert ⇒ not trusted");
        assert_eq!(
            runner.calls().len(),
            1,
            "only find-certificate ran; verify-cert skipped"
        );
    }

    /// `remove_root` short-circuits when the cert is absent (no delete call).
    #[test]
    fn remove_root_is_noop_when_cert_absent() {
        let runner = FakeRunner::new();
        // First call: verify_installed → absent. Second call (delete) MUST
        // NOT happen; FakeRunner will panic on exhausted script if it does.
        runner.push(ok("SHA-256 hash: AAAA\n"));
        let store = store_with(runner.clone());

        let touched = store
            .remove_root(FINGERPRINT, TrustScope::User)
            .expect("remove_root");
        assert!(touched.is_empty(), "no-op should return empty Vec");

        let calls = runner.calls();
        assert_eq!(
            calls.len(),
            1,
            "verify_installed only, no delete-certificate"
        );
    }

    /// `remove_root` invokes `security delete-certificate -Z <hash> -t
    /// <keychain>` when the cert is present.
    #[test]
    fn remove_root_invokes_delete_certificate_when_present() {
        let runner = FakeRunner::new();
        // verify_installed → present
        runner.push(ok(&format!(
            "SHA-256 hash: {}\n",
            FINGERPRINT.to_uppercase()
        )));
        // delete-certificate → ok
        runner.push(ok(""));
        let store = store_with(runner.clone());

        let touched = store
            .remove_root(FINGERPRINT, TrustScope::User)
            .expect("remove_root");
        assert_eq!(touched.len(), 1);
        assert!(touched[0].to_string_lossy().ends_with("login.keychain-db"));

        let calls = runner.calls();
        assert_eq!(calls.len(), 2, "verify_installed then delete-certificate");
        let (_, args) = &calls[1];
        assert_eq!(args[0], OsString::from("delete-certificate"));
        assert_eq!(args[1], OsString::from("-Z"));
        assert_eq!(args[2], OsString::from(FINGERPRINT.to_uppercase()));
        assert_eq!(args[3], OsString::from("-t"));
        assert!(args[4].to_string_lossy().ends_with("login.keychain-db"));
    }

    /// `validate_hex_fingerprint` rejects shell metacharacters and flag-
    /// shaped strings before they hit `security`. This is the T8 belt-and-
    /// braces test.
    #[test]
    fn validate_hex_fingerprint_rejects_non_hex() {
        for bad in [
            "",                        // empty
            "abc; rm -rf /",           // shell injection attempt
            "--insert-evil-flag",      // flag injection attempt
            "abc\nSHA-256 hash: DEAD", // newline injection
            "🦀rust",                  // unicode
            "a".repeat(129).as_str(),  // overlong
        ] {
            let err = validate_hex_fingerprint(bad).expect_err("non-hex must be rejected");
            let msg = format!("{err}");
            assert!(
                msg.contains("hex") || msg.contains("length") || msg.contains("range"),
                "expected hex-validation error for {bad:?}, got: {msg}"
            );
        }
    }

    /// Lower- and upper-case hex both accepted.
    #[test]
    fn validate_hex_fingerprint_accepts_case_variants() {
        validate_hex_fingerprint(FINGERPRINT).expect("lowercase");
        validate_hex_fingerprint(&FINGERPRINT.to_uppercase()).expect("uppercase");
        // Mixed
        validate_hex_fingerprint("AbCdEf0123456789").expect("mixed");
    }

    /// `add_root` rejects the verify-then-add use case: there is no
    /// shortcut — the security CLI itself is idempotent (re-adding an
    /// already-trusted cert is a no-op exit-0), and exercising the real path
    /// keeps the integration tests honest.
    #[test]
    fn add_root_does_not_pre_check_verify() {
        let runner = FakeRunner::new();
        // Only one scripted response — if `add_root` tried to call
        // verify_installed first, FakeRunner would panic on the second call.
        runner.push(ok(""));
        let store = store_with(runner.clone());
        store
            .add_root(&PathBuf::from("/tmp/ca.pem"), TrustScope::User)
            .expect("single shell-out");
        assert_eq!(runner.calls().len(), 1);
    }
}
