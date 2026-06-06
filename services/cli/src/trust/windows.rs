//! Windows trust-store backend (SLICE 4 / COV_08).
//!
//! Wraps the Windows `certutil` CLI to install / verify / remove the
//! SpendGuard root CA in the `Root` certificate store. Per design `§4`
//! (Windows backend table line 45) and review-standards `T6`:
//!
//! - `--scope user` (default) → `certutil -user -addstore -f Root <pem>`
//!   writes into `HKCU\Software\Microsoft\SystemCertificates\Root\Certificates`
//!   — no UAC consent prompt, no elevation. The cert is picked up by
//!   schannel / wininet / .NET / Edge / IE / curl-built-with-schannel.
//! - `--scope system` → `certutil -addstore -f Root <pem>` writes into
//!   `HKLM\Software\Microsoft\SystemCertificates\Root\Certificates`. The
//!   shell-out itself does NOT prepend `runas` / `Start-Process -Verb
//!   RunAs`; the operator must invoke `spendguard install` from an
//!   already-elevated shell. A non-elevated invocation returns
//!   `0x80070005 (E_ACCESSDENIED)`, which we surface verbatim with the
//!   stderr the `certutil` CLI emits.
//!
//! ## Why a `CommandRunner` indirection (T8 + hermetic tests + no
//! `windows`/`windows-sys` crate dep)
//!
//! Same pattern as [`macos`](super::macos) and [`linux`](super::linux):
//! every shell-out routes through [`crate::trust::CommandRunner`]. Two
//! reasons:
//!
//! 1. **T8 belt-and-braces**: `certutil` is invoked positionally with
//!    `args[]` — no shell metacharacter expansion, no `cmd.exe /c "…"`,
//!    no `powershell -Command "…"`. A fingerprint string carrying
//!    `; del C:\Windows\*` cannot reach a shell parser.
//! 2. **Compile + test on the macOS dev host**: the LOCKED-at-SLICE-3-R2
//!    module-decl pattern declares `pub mod windows;` **un-gated** so the
//!    FakeRunner-driven unit tests below run on every workspace member's
//!    `cargo test` invocation, including macOS. We therefore MUST NOT
//!    import `windows::*` or `windows-sys::*` types in any public
//!    signature (they don't exist on the macOS target). The
//!    `CommandRunner` trait + `std::path::Path` give us all the
//!    indirection we need.
//!
//! Integration tests that actually mutate the user's `Root` cert store
//! live in `services/cli/tests/trust_windows.rs` and are `#[cfg(target_os
//! = "windows")]`-gated plus `#[ignore]`-gated; the per-OS CI matrix opts
//! in to them via `cargo test -p spendguard-cli --test trust_windows --
//! --include-ignored`.
//!
//! ## Fingerprint identity
//!
//! Windows `certutil -delstore` historically accepted both SHA-1 and
//! SHA-256 thumbprints, the format detected by hex length. SLICE 4 ships
//! the SHA-256 hex (the install-time identity returned by
//! [`crate::ca::fingerprint_hex`]) verbatim. As a belt-and-braces
//! fallback, [`WindowsTrustStore::verify_installed`] scans
//! `certutil -store Root` output line-by-line for either hash family —
//! the CLI prints both `Cert Hash(sha1)` and (on Windows 10 1607+)
//! `Cert Hash(sha256)`.
//!
//! ## Idempotence
//!
//! - `add_root` on a cert that is already in the `Root` store is a
//!   `certutil` no-op exit-0; we treat it as success.
//! - `remove_root` on an absent cert returns `Ok(vec![])` with no error
//!   (review-standards `X3` / acceptance `A8.2`). We pre-check via
//!   `verify_installed` so an idempotent uninstall does not surface
//!   `0x80092004 (CRYPT_E_NOT_FOUND)` as a failure.
//! - `verify_installed` returns `Ok(false)` for absent certs, NOT `Err`.

use crate::trust::{CommandOutput, CommandRunner, StdCommandRunner, TrustStore};
use crate::TrustScope;
use anyhow::{anyhow, Context, Result};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Default `certutil` binary name. Resolved via `$PATH` because Windows
/// ships it at `%SystemRoot%\System32\certutil.exe` on every supported
/// release (Windows 10 1607+ / Server 2016+), and `%SystemRoot%\System32`
/// is in `%PATH%` by default. Pinning the absolute path would require
/// reading `%SystemRoot%` at runtime — and `certutil.exe` is on `PATH`
/// for the same reason cmd.exe is. Test injection of the binary name
/// goes through [`WindowsTrustStore::with_runner`] anyway.
const CERTUTIL_BIN: &str = "certutil";

/// Conceptual "trust-store path" returned in `InstallReport.trust_store_locations`
/// so operators + the doctor report see the canonical registry hive that
/// the cert landed in. We never actually write to these paths — `certutil`
/// owns the registry write — but they're the closest filesystem-shaped
/// identifier of what got mutated. Pattern mirrors the Arch p11-kit
/// conceptual anchor path in [`super::linux`].
fn conceptual_store_path(scope: TrustScope) -> PathBuf {
    match scope {
        TrustScope::User => {
            PathBuf::from("HKCU\\Software\\Microsoft\\SystemCertificates\\Root\\Certificates")
        }
        TrustScope::System => {
            PathBuf::from("HKLM\\Software\\Microsoft\\SystemCertificates\\Root\\Certificates")
        }
    }
}

/// Windows trust-store backend. Pattern mirrors [`super::linux::LinuxTrustStore`]:
/// a [`CommandRunner`] is the only injection point so unit tests stay
/// hermetic on the macOS dev host AND the production path never imports
/// `windows`/`windows-sys` crate types.
pub struct WindowsTrustStore {
    runner: Box<dyn CommandRunner>,
    certutil_bin: PathBuf,
}

impl WindowsTrustStore {
    /// Production constructor — `certutil` resolved via `$PATH` +
    /// `std::process::Command` shell-out.
    pub fn new() -> Self {
        Self {
            runner: Box::new(StdCommandRunner),
            certutil_bin: PathBuf::from(CERTUTIL_BIN),
        }
    }

    /// Test-only constructor — same shape as
    /// [`super::macos::MacosTrustStore::with_runner`] +
    /// [`super::linux::LinuxTrustStore::with_runner`]. Hidden from
    /// rustdoc; visible enough that `tests/trust_windows.rs` can build a
    /// store targeting a controlled `certutil` binary.
    #[doc(hidden)]
    pub fn with_runner<R: CommandRunner + 'static>(runner: R, certutil_bin: PathBuf) -> Self {
        Self {
            runner: Box::new(runner),
            certutil_bin,
        }
    }

    fn certutil(&self) -> &str {
        self.certutil_bin.to_str().unwrap_or(CERTUTIL_BIN)
    }

    fn run(&self, args: Vec<OsString>) -> Result<CommandOutput> {
        self.runner.run(self.certutil(), &args)
    }

    /// Build the leading `-user` arg slot — present on `TrustScope::User`,
    /// omitted on `TrustScope::System`. Centralised so add / remove /
    /// verify always agree on the scope flag position.
    fn scope_arg(scope: TrustScope) -> Option<OsString> {
        match scope {
            TrustScope::User => Some(OsString::from("-user")),
            TrustScope::System => None,
        }
    }
}

impl Default for WindowsTrustStore {
    fn default() -> Self {
        Self::new()
    }
}

impl TrustStore for WindowsTrustStore {
    fn add_root(&self, ca_pem_path: &Path, scope: TrustScope) -> Result<Vec<PathBuf>> {
        // Argv shape per design §4 line 45:
        //   user   → `certutil -user -addstore -f Root <pem_path>`
        //   system → `certutil -addstore -f Root <pem_path>`
        //
        // `-f` (force) is set on both scopes so re-installing over an
        // already-trusted cert is a no-op exit-0 (idempotence). Without
        // `-f`, `certutil` returns `0x800B0109 (CERT_E_UNTRUSTEDROOT)`
        // when the target store already contains the cert and Windows
        // applies its policy heuristic; `-f` is the documented escape.
        let mut args: Vec<OsString> = Vec::with_capacity(5);
        if let Some(s) = Self::scope_arg(scope) {
            args.push(s);
        }
        args.push("-addstore".into());
        args.push("-f".into());
        args.push("Root".into());
        args.push(ca_pem_path.as_os_str().to_owned());

        let out = self.run(args).context("invoke certutil -addstore Root")?;
        if !out.success() {
            return Err(anyhow!(
                "certutil -addstore Root exited {:?}: stderr={}",
                out.status,
                out.stderr_str().trim()
            ));
        }
        Ok(vec![conceptual_store_path(scope)])
    }

    fn remove_root(&self, fingerprint_sha256_hex: &str, scope: TrustScope) -> Result<Vec<PathBuf>> {
        validate_hex_fingerprint(fingerprint_sha256_hex)?;

        // Idempotent: skip the delete when the cert is not in the store.
        // `certutil -delstore` returns `0x80092004 (CRYPT_E_NOT_FOUND)`
        // on miss, which we DO NOT want to surface as an error (review-
        // standards X3 / acceptance A8.2).
        if !self.verify_installed(fingerprint_sha256_hex, scope)? {
            return Ok(vec![]);
        }

        // Argv shape per design §4 line 45 + spec slice doc:
        //   user   → `certutil -user -delstore Root <thumbprint>`
        //   system → `certutil -delstore Root <thumbprint>`
        //
        // `certutil` matches the thumbprint case-insensitively but
        // Microsoft's own tooling emits upper-case hex, so we match.
        let mut args: Vec<OsString> = Vec::with_capacity(4);
        if let Some(s) = Self::scope_arg(scope) {
            args.push(s);
        }
        args.push("-delstore".into());
        args.push("Root".into());
        args.push(fingerprint_sha256_hex.to_uppercase().into());

        let out = self.run(args).context("invoke certutil -delstore Root")?;
        if !out.success() {
            return Err(anyhow!(
                "certutil -delstore Root exited {:?}: stderr={}",
                out.status,
                out.stderr_str().trim()
            ));
        }
        Ok(vec![conceptual_store_path(scope)])
    }

    fn verify_installed(&self, fingerprint_sha256_hex: &str, scope: TrustScope) -> Result<bool> {
        validate_hex_fingerprint(fingerprint_sha256_hex)?;

        // We enumerate the Root store via `certutil -store` and grep the
        // stdout for the fingerprint. We deliberately do NOT shell out to
        // `certutil -verifystore Root <fingerprint>` — `-verifystore` runs
        // a chain-build + revocation check against the cert, which can
        // touch the network and surface a non-zero exit for reasons
        // orthogonal to "is it in the store" (e.g. revocation responder
        // unreachable). The `-store` enumeration is a pure registry read.
        //
        // Argv shape:
        //   user   → `certutil -user -store Root`
        //   system → `certutil -store Root`
        let mut args: Vec<OsString> = Vec::with_capacity(3);
        if let Some(s) = Self::scope_arg(scope) {
            args.push(s);
        }
        args.push("-store".into());
        args.push("Root".into());

        let out = self.run(args).context("invoke certutil -store Root")?;
        if !out.success() {
            // `certutil -store Root` returns non-zero when the registry
            // hive is locked or the calling user has no read access to
            // HKLM (rare; documented for non-interactive service
            // contexts). Treat as absent for idempotency, but surface the
            // stderr at debug so the operator can investigate via the
            // doctor report.
            tracing::debug!(
                exit = ?out.status,
                stderr = %out.stderr_str().trim(),
                "certutil -store non-zero exit; treating as absent"
            );
            return Ok(false);
        }
        Ok(haystack_contains_fingerprint(
            &out.stdout_str(),
            fingerprint_sha256_hex,
        ))
    }
}

/// Defence-in-depth: any string passed to `certutil` argv MUST be plain
/// hex. The public `add_root` / `remove_root` API types fingerprints as
/// `&str`, so we cannot rely on the type system. Hex-only validation
/// keeps shell-meta, `--flag`, NUL, and Windows path-traversal
/// (`..\..\…`) tokens out of the argv. Mirrors the macOS + Linux
/// validators verbatim so the T8 belt-and-braces surface is consistent
/// across backends.
fn validate_hex_fingerprint(s: &str) -> Result<()> {
    if s.is_empty() || s.len() > 128 {
        return Err(anyhow!(
            "fingerprint hex length {} is outside the expected [1, 128] range",
            s.len()
        ));
    }
    if !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(anyhow!(
            "fingerprint hex contains a non-hex byte; refusing to pass to certutil CLI"
        ));
    }
    Ok(())
}

/// Case-insensitive substring match for `fingerprint_hex` against the
/// `certutil -store Root` haystack. `certutil` may upper- or lower-case
/// the thumbprint, and may interleave spaces every two hex bytes in the
/// `Cert Hash(sha1)` / `Cert Hash(sha256)` lines; we normalise the
/// haystack by stripping ASCII whitespace before substring-matching so
/// `e6 91 …` matches `e691…`.
fn haystack_contains_fingerprint(haystack: &str, fingerprint_hex: &str) -> bool {
    let needle = fingerprint_hex.to_ascii_lowercase();
    haystack.lines().any(|line| {
        let normalised: String = line
            .chars()
            .filter(|c| !c.is_ascii_whitespace())
            .collect::<String>()
            .to_ascii_lowercase();
        normalised.contains(&needle)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    // ──────────────────────────────────────────────────────────────────
    //  Fake `CommandRunner` mirroring the macOS + Linux test pattern.
    //  Records every call (program + argv) and returns scripted output;
    //  exhausted scripts panic to surface unexpected extra calls.
    // ──────────────────────────────────────────────────────────────────

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

    fn store_with(runner: Arc<FakeRunner>) -> WindowsTrustStore {
        WindowsTrustStore::with_runner(runner, PathBuf::from("certutil"))
    }

    const FINGERPRINT: &str = "68f37edc0d2063597abb39b2f99b02562210ce9f433c50201de6b7e1296ed371";

    // ──────────────────────────────────────────────────────────────────
    //  add_root argv shape — user scope + system scope + non-zero exit.
    // ──────────────────────────────────────────────────────────────────

    /// `add_root` user scope invokes
    /// `certutil -user -addstore -f Root <pem>` — `-user` is the FIRST
    /// argument because `certutil` requires it ahead of the verb.
    #[test]
    fn add_root_user_scope_invokes_certutil_user_addstore() {
        let runner = FakeRunner::new();
        runner.push(ok(""));
        let store = store_with(runner.clone());

        let ca = PathBuf::from("C:\\Temp\\ca.pem");
        let touched = store
            .add_root(&ca, TrustScope::User)
            .expect("add_root user");
        assert_eq!(
            touched,
            vec![PathBuf::from(
                "HKCU\\Software\\Microsoft\\SystemCertificates\\Root\\Certificates"
            )]
        );

        let calls = runner.calls();
        assert_eq!(calls.len(), 1, "exactly one certutil shell-out: {calls:?}");
        let (program, args) = &calls[0];
        assert_eq!(program, "certutil");
        assert_eq!(args[0], OsString::from("-user"));
        assert_eq!(args[1], OsString::from("-addstore"));
        assert_eq!(args[2], OsString::from("-f"));
        assert_eq!(args[3], OsString::from("Root"));
        assert_eq!(args[4], OsString::from("C:\\Temp\\ca.pem"));
        assert_eq!(args.len(), 5);
    }

    /// `add_root` system scope omits the `-user` flag so `certutil`
    /// targets `HKLM`. Argv collapses by exactly one element.
    #[test]
    fn add_root_system_scope_omits_user_flag() {
        let runner = FakeRunner::new();
        runner.push(ok(""));
        let store = store_with(runner.clone());

        let ca = PathBuf::from("C:\\Temp\\ca.pem");
        let touched = store
            .add_root(&ca, TrustScope::System)
            .expect("add_root system");
        assert_eq!(
            touched,
            vec![PathBuf::from(
                "HKLM\\Software\\Microsoft\\SystemCertificates\\Root\\Certificates"
            )]
        );

        let calls = runner.calls();
        assert_eq!(calls.len(), 1);
        let (program, args) = &calls[0];
        assert_eq!(program, "certutil");
        assert_eq!(args[0], OsString::from("-addstore"));
        assert_eq!(args[1], OsString::from("-f"));
        assert_eq!(args[2], OsString::from("Root"));
        assert_eq!(args[3], OsString::from("C:\\Temp\\ca.pem"));
        assert_eq!(args.len(), 4, "system scope must not carry -user");
        // Explicit absence check: -user must not appear anywhere in argv.
        for arg in args {
            assert_ne!(
                *arg,
                OsString::from("-user"),
                "system scope must not carry -user, got: {args:?}"
            );
        }
    }

    /// `add_root` surfaces a non-zero `certutil` exit with stderr context
    /// so the doctor / operator can diagnose. Mirrors macOS + Linux.
    #[test]
    fn add_root_surfaces_certutil_nonzero_exit() {
        let runner = FakeRunner::new();
        runner.push(err(
            5,
            "Access is denied. 0x80070005 (WIN32: 5 ERROR_ACCESS_DENIED)",
        ));
        let store = store_with(runner);

        let err = store
            .add_root(&PathBuf::from("C:\\Temp\\ca.pem"), TrustScope::System)
            .expect_err("non-zero must error");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("Access is denied"),
            "stderr context missing: {msg}"
        );
        assert!(
            msg.contains("certutil -addstore"),
            "command context missing: {msg}"
        );
    }

    // ──────────────────────────────────────────────────────────────────
    //  remove_root argv shape — user scope + system scope + idempotent
    //  no-op when cert is absent.
    // ──────────────────────────────────────────────────────────────────

    /// `remove_root` user scope is a two-call sequence:
    /// 1. `certutil -user -store Root` (the verify pre-check).
    /// 2. `certutil -user -delstore Root <thumbprint>` (the delete).
    ///
    /// Argv assertions on call #2 — `-user` first, then `-delstore Root`,
    /// then the upper-case hex thumbprint.
    #[test]
    fn remove_root_user_scope_invokes_certutil_delstore_when_present() {
        let runner = FakeRunner::new();
        // verify pre-check → present
        runner.push(ok(&format!(
            "================ Certificate 0 ================\r\n\
             Cert Hash(sha256): {}\r\n",
            FINGERPRINT.to_uppercase()
        )));
        // delstore → ok
        runner.push(ok(""));
        let store = store_with(runner.clone());

        let touched = store
            .remove_root(FINGERPRINT, TrustScope::User)
            .expect("remove_root user");
        assert_eq!(
            touched,
            vec![PathBuf::from(
                "HKCU\\Software\\Microsoft\\SystemCertificates\\Root\\Certificates"
            )]
        );

        let calls = runner.calls();
        assert_eq!(calls.len(), 2, "verify then delstore");

        // Call #2 — the delstore.
        let (program, args) = &calls[1];
        assert_eq!(program, "certutil");
        assert_eq!(args[0], OsString::from("-user"));
        assert_eq!(args[1], OsString::from("-delstore"));
        assert_eq!(args[2], OsString::from("Root"));
        assert_eq!(args[3], OsString::from(FINGERPRINT.to_uppercase()));
        assert_eq!(args.len(), 4);
    }

    /// `remove_root` system scope omits `-user` on both the verify
    /// pre-check AND the delstore so the cert is removed from `HKLM`.
    #[test]
    fn remove_root_system_scope_omits_user_flag() {
        let runner = FakeRunner::new();
        runner.push(ok(&format!(
            "Cert Hash(sha256): {}\r\n",
            FINGERPRINT.to_uppercase()
        )));
        runner.push(ok(""));
        let store = store_with(runner.clone());

        let touched = store
            .remove_root(FINGERPRINT, TrustScope::System)
            .expect("remove_root system");
        assert_eq!(
            touched,
            vec![PathBuf::from(
                "HKLM\\Software\\Microsoft\\SystemCertificates\\Root\\Certificates"
            )]
        );

        let calls = runner.calls();
        assert_eq!(calls.len(), 2);

        // Both calls must omit -user.
        let (_, store_args) = &calls[0];
        assert_eq!(store_args[0], OsString::from("-store"));
        assert_eq!(store_args[1], OsString::from("Root"));
        assert_eq!(store_args.len(), 2);

        let (_, del_args) = &calls[1];
        assert_eq!(del_args[0], OsString::from("-delstore"));
        assert_eq!(del_args[1], OsString::from("Root"));
        assert_eq!(del_args[2], OsString::from(FINGERPRINT.to_uppercase()));
        assert_eq!(del_args.len(), 3);
    }

    /// `remove_root` short-circuits when the cert is absent — no
    /// `-delstore` shell-out, just the verify pre-check. The FakeRunner
    /// scripted-output exhaustion is the guarantee: if `remove_root`
    /// tried to call `-delstore`, the FakeRunner would panic on
    /// exhausted output.
    #[test]
    fn remove_root_is_noop_when_cert_absent() {
        let runner = FakeRunner::new();
        // verify pre-check → absent (empty store output)
        runner.push(ok("CertUtil: -store command completed successfully.\r\n"));
        let store = store_with(runner.clone());

        let touched = store
            .remove_root(FINGERPRINT, TrustScope::User)
            .expect("remove_root");
        assert!(touched.is_empty(), "no-op should return empty Vec");

        let calls = runner.calls();
        assert_eq!(
            calls.len(),
            1,
            "verify_installed only, no delstore: {calls:?}"
        );
    }

    /// `remove_root` surfaces a non-zero `certutil -delstore` exit with
    /// stderr context. T6 idempotence does NOT extend to "swallow errors";
    /// we only swallow `verify=absent`, not delstore failures.
    #[test]
    fn remove_root_surfaces_certutil_delstore_nonzero_exit() {
        let runner = FakeRunner::new();
        // verify pre-check → present
        runner.push(ok(&format!(
            "Cert Hash(sha256): {}\r\n",
            FINGERPRINT.to_uppercase()
        )));
        // delstore → access denied
        runner.push(err(5, "Access is denied. 0x80070005"));
        let store = store_with(runner);

        let err = store
            .remove_root(FINGERPRINT, TrustScope::System)
            .expect_err("non-zero delstore must error");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("Access is denied"),
            "stderr context missing: {msg}"
        );
        assert!(
            msg.contains("certutil -delstore"),
            "command context missing: {msg}"
        );
    }

    // ──────────────────────────────────────────────────────────────────
    //  verify_installed argv shape + present / absent semantics.
    // ──────────────────────────────────────────────────────────────────

    /// `verify_installed` user scope invokes `certutil -user -store Root`
    /// and returns true when the fingerprint appears in stdout.
    #[test]
    fn verify_installed_user_scope_returns_true_when_present() {
        let runner = FakeRunner::new();
        runner.push(ok(&format!(
            "================ Certificate 0 ================\r\n\
             Issuer: CN=SpendGuard Local Root R1\r\n\
             Cert Hash(sha1): 89FA3228BF3088DD96772998EB49C1BE87CD9B3A\r\n\
             Cert Hash(sha256): {}\r\n\
             CertUtil: -store command completed successfully.\r\n",
            FINGERPRINT.to_uppercase()
        )));
        let store = store_with(runner.clone());

        let present = store
            .verify_installed(FINGERPRINT, TrustScope::User)
            .expect("verify");
        assert!(present, "fingerprint should be detected case-insensitively");

        let calls = runner.calls();
        assert_eq!(calls.len(), 1);
        let (program, args) = &calls[0];
        assert_eq!(program, "certutil");
        assert_eq!(args[0], OsString::from("-user"));
        assert_eq!(args[1], OsString::from("-store"));
        assert_eq!(args[2], OsString::from("Root"));
        assert_eq!(args.len(), 3);
    }

    /// `verify_installed` returns `Ok(false)` (not `Err`) when the
    /// fingerprint is absent — review-standards `X3` / acceptance `A8.2`.
    #[test]
    fn verify_installed_returns_false_when_absent() {
        let runner = FakeRunner::new();
        runner.push(ok(
            "================ Certificate 0 ================\r\n\
             Cert Hash(sha256): DEADBEEFDEADBEEFDEADBEEFDEADBEEFDEADBEEFDEADBEEFDEADBEEFDEADBEEF\r\n\
             CertUtil: -store command completed successfully.\r\n",
        ));
        let store = store_with(runner);

        let present = store
            .verify_installed(FINGERPRINT, TrustScope::User)
            .expect("verify");
        assert!(!present);
    }

    /// `verify_installed` system scope omits `-user`.
    #[test]
    fn verify_installed_system_scope_omits_user_flag() {
        let runner = FakeRunner::new();
        runner.push(ok(&format!(
            "Cert Hash(sha256): {}\r\n",
            FINGERPRINT.to_uppercase()
        )));
        let store = store_with(runner.clone());

        let present = store
            .verify_installed(FINGERPRINT, TrustScope::System)
            .expect("verify");
        assert!(present);

        let calls = runner.calls();
        assert_eq!(calls.len(), 1);
        let (_, args) = &calls[0];
        assert_eq!(args[0], OsString::from("-store"));
        assert_eq!(args[1], OsString::from("Root"));
        assert_eq!(args.len(), 2);
        // Explicit absence check.
        for arg in args {
            assert_ne!(*arg, OsString::from("-user"));
        }
    }

    /// `verify_installed` treats `certutil -store` non-zero exit as
    /// absent rather than erroring — mirrors the macOS treatment of
    /// `find-certificate` failure (locked keychain → idempotent install
    /// shouldn't break).
    #[test]
    fn verify_installed_returns_false_when_certutil_nonzero() {
        let runner = FakeRunner::new();
        runner.push(err(1, "CertUtil: -store command FAILED: 0x80090016"));
        let store = store_with(runner);

        let present = store
            .verify_installed(FINGERPRINT, TrustScope::User)
            .expect("verify must not error on non-zero exit");
        assert!(!present);
    }

    /// `verify_installed` handles `certutil`'s "spaces every two bytes"
    /// hex format. Some Windows versions emit
    /// `Cert Hash(sha256): 68 f3 7e dc ...`; the haystack normaliser
    /// strips ASCII whitespace before substring-matching so the
    /// fingerprint still hits.
    #[test]
    fn verify_installed_matches_certutil_spaced_hex_format() {
        // Build a haystack with `certutil`'s spaced-hex form (every two
        // bytes separated by a space).
        let spaced: String = FINGERPRINT
            .as_bytes()
            .chunks(2)
            .map(|c| std::str::from_utf8(c).unwrap())
            .collect::<Vec<_>>()
            .join(" ");
        let runner = FakeRunner::new();
        runner.push(ok(&format!("Cert Hash(sha256): {}\r\n", spaced)));
        let store = store_with(runner);

        let present = store
            .verify_installed(FINGERPRINT, TrustScope::User)
            .expect("verify spaced hex");
        assert!(
            present,
            "spaced hex must still hit after whitespace normalise"
        );
    }

    // ──────────────────────────────────────────────────────────────────
    //  T8 belt-and-braces — fingerprint hex validator rejects shell-meta,
    //  flag injection, newline injection, unicode, and overlong input.
    //  Mirrors the macOS + Linux validators.
    // ──────────────────────────────────────────────────────────────────

    /// `validate_hex_fingerprint` rejects shell metacharacters and
    /// flag-shaped strings before they reach `certutil` argv. T8.
    #[test]
    fn validate_hex_fingerprint_rejects_non_hex() {
        for bad in [
            "",                               // empty
            "abc; del C:\\Windows\\*",        // shell injection attempt
            "--insert-evil-flag",             // flag injection attempt
            "abc\r\nCert Hash(sha256): DEAD", // CRLF injection
            "🦀rust",                         // unicode
            "..\\..\\Windows\\System32",      // path traversal
            "a".repeat(129).as_str(),         // overlong
        ] {
            let err = validate_hex_fingerprint(bad).expect_err("non-hex must be rejected");
            let msg = format!("{err}");
            assert!(
                msg.contains("hex") || msg.contains("length") || msg.contains("range"),
                "expected hex-validation error for {bad:?}, got: {msg}"
            );
        }
    }

    /// `validate_hex_fingerprint` accepts both lower- and upper-case hex
    /// (and the mixed case `certutil` itself can emit).
    #[test]
    fn validate_hex_fingerprint_accepts_case_variants() {
        validate_hex_fingerprint(FINGERPRINT).expect("lowercase");
        validate_hex_fingerprint(&FINGERPRINT.to_uppercase()).expect("uppercase");
        validate_hex_fingerprint("AbCdEf0123456789").expect("mixed");
    }

    /// `remove_root` rejects a non-hex fingerprint at the boundary —
    /// even before the verify pre-check shells out. Belt-and-braces:
    /// FakeRunner has no scripted output, so any shell-out below would
    /// panic and surface as a different test failure.
    #[test]
    fn remove_root_rejects_non_hex_fingerprint() {
        let runner = FakeRunner::new();
        // No scripted output — any shell-out attempt panics FakeRunner.
        let store = store_with(runner.clone());

        let err = store
            .remove_root("abc; del *", TrustScope::User)
            .expect_err("non-hex must be rejected");
        let msg = format!("{err}");
        assert!(
            msg.contains("hex") || msg.contains("range"),
            "expected hex validation error, got: {msg}"
        );
        assert!(runner.calls().is_empty(), "no shell-out for invalid input");
    }

    /// `verify_installed` rejects a non-hex fingerprint at the boundary
    /// (no `-store` shell-out for a guaranteed-invalid input).
    #[test]
    fn verify_installed_rejects_non_hex_fingerprint() {
        let runner = FakeRunner::new();
        let store = store_with(runner.clone());

        let err = store
            .verify_installed("--evil-flag", TrustScope::User)
            .expect_err("non-hex must be rejected");
        let msg = format!("{err}");
        assert!(
            msg.contains("hex") || msg.contains("range"),
            "expected hex validation error, got: {msg}"
        );
        assert!(runner.calls().is_empty(), "no shell-out for invalid input");
    }

    // ──────────────────────────────────────────────────────────────────
    //  Constructor + scope_arg helper coverage.
    // ──────────────────────────────────────────────────────────────────

    /// `WindowsTrustStore::new()` is infallible and uses the default
    /// `certutil` binary name. No shell-out happens at construction time.
    #[test]
    fn windows_trust_store_constructor_is_infallible() {
        let store = WindowsTrustStore::new();
        // We can't assert against `runner` directly (it's boxed), so
        // probe via the `certutil_bin` field shape through `certutil()`.
        assert_eq!(store.certutil(), "certutil");
    }

    /// `WindowsTrustStore::with_runner` accepts a custom certutil path —
    /// `tests/trust_windows.rs` uses this to target a controlled binary.
    #[test]
    fn windows_trust_store_with_runner_uses_supplied_certutil_path() {
        let runner = FakeRunner::new();
        let store =
            WindowsTrustStore::with_runner(runner, PathBuf::from("C:\\Tools\\certutil-stub.exe"));
        assert_eq!(store.certutil(), "C:\\Tools\\certutil-stub.exe");
    }

    /// `WindowsTrustStore::scope_arg` returns the `-user` flag for User
    /// and `None` for System — single source of truth so add / remove /
    /// verify can't drift on the scope flag position.
    #[test]
    fn scope_arg_returns_user_flag_only_for_user_scope() {
        assert_eq!(
            WindowsTrustStore::scope_arg(TrustScope::User),
            Some(OsString::from("-user"))
        );
        assert_eq!(WindowsTrustStore::scope_arg(TrustScope::System), None);
    }

    /// `Default::default()` is the same as `WindowsTrustStore::new()`.
    #[test]
    fn windows_trust_store_default_equivalent_to_new() {
        let _default: WindowsTrustStore = Default::default();
    }
}
