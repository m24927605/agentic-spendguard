//! OS trust-store backend dispatch.
//!
//! This slice (SLICE 2 / COV_06) ships the macOS keychain backend; Linux
//! (SLICE 3) and Windows (SLICE 4) plug into the same [`TrustStore`] trait so
//! the [`install`](crate::install) / [`uninstall`](crate::uninstall) /
//! [`doctor`](crate::doctor) entry points are OS-agnostic.
//!
//! ## Why a `CommandRunner` indirection
//!
//! The macOS impl shells out to `/usr/bin/security`. To keep unit tests
//! hermetic and fast — and to keep `cargo test -p spendguard-cli --lib`
//! green on Linux CI runners — every shell-out goes through the
//! [`CommandRunner`] trait. Production code injects [`StdCommandRunner`]
//! (a thin wrapper around `std::process::Command`); tests inject a fake
//! that records invocations and returns canned output. Integration tests
//! that actually mutate the user keychain live in
//! `services/cli/tests/trust_macos.rs` and are `#[ignore]`-gated so they
//! only run when explicitly opted in (`cargo test -- --include-ignored`).
//!
//! This indirection also enforces review-standards `T8`: trust-store
//! mutations route through a typed shell-out surface, not `Command::new("sh")
//! .arg("-c", …)`, so user-controlled strings never reach a shell.

use crate::TrustScope;
use anyhow::Result;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
pub mod macos;

/// Capture of a shell-out invocation. `Output::status` is the OS exit code;
/// `Output::stdout` / `Output::stderr` are captured as `Vec<u8>` so the
/// caller decides on charset (the macOS `security` CLI emits ASCII).
#[derive(Debug)]
pub struct CommandOutput {
    pub status: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl CommandOutput {
    pub fn success(&self) -> bool {
        self.status == Some(0)
    }

    pub fn stdout_str(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.stdout)
    }

    pub fn stderr_str(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.stderr)
    }
}

/// Abstracts the production `std::process::Command` surface. Implementations
/// MUST NOT spawn a shell — every argument is passed positionally so review-
/// standards `T8` (no shell injection) holds even if the caller passes
/// attacker-controlled strings.
pub trait CommandRunner: Send + Sync {
    /// Run a binary with the given `args` and capture stdout / stderr.
    fn run(&self, program: &str, args: &[OsString]) -> Result<CommandOutput>;
}

/// Production `CommandRunner` — thin shim over `std::process::Command`.
#[derive(Debug, Default, Clone, Copy)]
pub struct StdCommandRunner;

impl CommandRunner for StdCommandRunner {
    fn run(&self, program: &str, args: &[OsString]) -> Result<CommandOutput> {
        let output = std::process::Command::new(program)
            .args(args)
            .output()
            .map_err(|e| anyhow::anyhow!("spawn {program}: {e}"))?;
        Ok(CommandOutput {
            status: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

/// OS trust-store interface. Methods are infallible-for-absent semantics
/// (review-standards `X3`): `verify_installed` returns `Ok(false)` when
/// the cert is simply not present, never `Err`.
pub trait TrustStore {
    /// Install `ca_pem_path` as a trust root in the keychain / cert store
    /// associated with `scope`. Returns the list of trust-store paths
    /// touched (for `InstallReport.trust_store_locations`).
    fn add_root(&self, ca_pem_path: &Path, scope: TrustScope) -> Result<Vec<PathBuf>>;

    /// Remove the cert identified by `fingerprint_sha256_hex` (lower-case,
    /// 64-char hex) from the scope's trust store. Returns the touched
    /// trust-store paths. Idempotent: if the cert is absent, returns an
    /// empty Vec without erroring.
    fn remove_root(&self, fingerprint_sha256_hex: &str, scope: TrustScope) -> Result<Vec<PathBuf>>;

    /// Return `Ok(true)` when `fingerprint_sha256_hex` is present in the
    /// scope's trust store.  Returns `Ok(false)` when the cert is absent.
    /// Errors only on backend failure (e.g. `security` binary missing).
    fn verify_installed(&self, fingerprint_sha256_hex: &str, scope: TrustScope) -> Result<bool>;
}

/// Dispatch to the platform-specific backend. Linux / Windows will plug in
/// here in SLICE 3 / SLICE 4 — for now, those platforms produce a clear
/// `unsupported on this slice` error from the constructor.
#[cfg(target_os = "macos")]
pub fn dispatch() -> Result<Box<dyn TrustStore>> {
    Ok(Box::new(macos::MacosTrustStore::new()))
}

#[cfg(not(target_os = "macos"))]
pub fn dispatch() -> Result<Box<dyn TrustStore>> {
    anyhow::bail!(
        "trust-store install is implemented for macOS in this slice (COV_06); \
         Linux lands in COV_07 (SLICE 3) and Windows in COV_08 (SLICE 4)"
    )
}
