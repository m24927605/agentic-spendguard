//! SLICE 6 (COV_10) — Install-time preflight gate.
//!
//! Today the only gate is the Gemini OAuth free-tier refusal per
//! `design.md` §3.5 (line 77): when `~/.gemini/oauth_creds.json` is present
//! but neither `GEMINI_API_KEY` nor `GOOGLE_APPLICATION_CREDENTIALS` is set,
//! Gemini CLI is signed into Google's free OAuth tier and SpendGuard's
//! HTTPS proxy will break the OAuth refresh handshake against
//! `accounts.google.com`. We refuse cleanly with a pointed actionable
//! message rather than silently breaking the user's Gemini access.
//!
//! ## Threat model
//!
//! Per `review-standards.md` `T10`, the Gemini gate MUST run **before**
//! any trust-store mutation. The wiring in [`crate::install_with_backends`]
//! calls [`run_preflight`] between the env-var build and the trust install
//! so a refusal never leaves a CA on disk or in the keychain.
//!
//! ## Why a typed refusal, not `anyhow::Error`
//!
//! The CLI's `main.rs` matches on the refusal variant to print the
//! actionable user-facing message verbatim — letting `anyhow` collapse it
//! into a generic chain would lose the slice-specified 3-option recovery
//! list. The variant carries the message as a `String` so future gates can
//! supply their own copy without adding more enum arms (additive growth).
//!
//! ## Forward-compat — deviation #2
//!
//! SLICE 6 ships only the Gemini gate. Future slices (or POST_GA) can add
//! more gates by extending `PreflightOverrides` and the dispatcher loop in
//! [`run_preflight`] without touching this module's public types — every
//! gate produces its own [`PreflightRefusal`] variant.
//!
//! ## Env injection
//!
//! [`BaseEnv`] is a borrowed view that lets tests construct env literals
//! without `std::env::set_var` (same B3 pattern as `paths::BaseEnv` and
//! `shell::EnvView`). Production callers go through
//! [`BaseEnv::from_process`].

pub mod gemini;

use std::path::{Path, PathBuf};

/// Borrowed view of the env vars + HOME the preflight gates need. Pure-
/// function input so tests can construct literals (B3 pattern).
///
/// The lifetimes are tied to the strings the caller owns —
/// `std::env::var(...).ok()` at runtime, `&'static str` literals in tests.
#[derive(Debug, Clone, Copy, Default)]
pub struct BaseEnv<'a> {
    pub home: Option<&'a Path>,
    pub gemini_api_key: Option<&'a str>,
    pub google_application_credentials: Option<&'a str>,
}

impl<'a> BaseEnv<'a> {
    /// Resolve `$HOME` (POSIX) / `%USERPROFILE%` (Windows). Returns a
    /// borrowed path with `'static` lifetime by leaking — only called
    /// once per process from production code so the leak is bounded.
    pub fn from_process() -> BaseEnv<'static> {
        // Leak the env strings into 'static. Only called once per `install`
        // / `uninstall` invocation, so bounded by the number of process
        // launches.
        fn leak_path(s: std::ffi::OsString) -> &'static Path {
            let owned: PathBuf = s.into();
            Box::leak(owned.into_boxed_path())
        }
        fn leak_str(s: String) -> &'static str {
            Box::leak(s.into_boxed_str())
        }
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(leak_path);
        let gemini_api_key = std::env::var("GEMINI_API_KEY").ok().map(leak_str);
        let google_application_credentials = std::env::var("GOOGLE_APPLICATION_CREDENTIALS")
            .ok()
            .map(leak_str);
        BaseEnv {
            home,
            gemini_api_key,
            google_application_credentials,
        }
    }

    /// Lookup an env var by name. Today only `GEMINI_API_KEY` and
    /// `GOOGLE_APPLICATION_CREDENTIALS` are surfaced; the slice doc seam
    /// keeps the signature generic so future gates can probe additional
    /// vars without breaking [`BaseEnv`] layout.
    pub fn var(&self, name: &str) -> Option<&str> {
        match name {
            "GEMINI_API_KEY" => self.gemini_api_key,
            "GOOGLE_APPLICATION_CREDENTIALS" => self.google_application_credentials,
            _ => None,
        }
    }

    /// Resolved `$HOME` path; gates compute `~/.gemini/oauth_creds.json`
    /// off this. Panics with a clear message rather than silently
    /// degrading detection — a missing HOME on the install path is
    /// already surfaced by the SLICE 5 shell-rc resolver, and the Gemini
    /// detector treats `home() = "<unset>"` as "not installed" (the
    /// oauth_creds.json path will not exist).
    pub fn home(&self) -> &Path {
        // Returning a sentinel "/" rather than panicking keeps the
        // detector pure — the metadata probe below will simply return
        // NotFound and we report NotInstalled. This matches the SLICE 5
        // pattern where missing HOME degrades to "no rc written".
        static FALLBACK: &str = "/";
        self.home.unwrap_or_else(|| Path::new(FALLBACK))
    }
}

/// Per-flag overrides surfaced via `spendguard install` CLI args. Each
/// flag short-circuits exactly one gate so the operator's `--force-allow-*`
/// surface is greppable and auditable.
#[derive(Debug, Clone, Copy, Default)]
pub struct PreflightOverrides {
    /// Set by `spendguard install --force-allow-gemini-oauth`. When `true`
    /// the Gemini OAuth gate is skipped and the install proceeds; the
    /// operator has accepted that their Gemini CLI may stop working until
    /// they sign out and re-authenticate.
    pub allow_gemini_oauth: bool,
}

/// Typed preflight refusal. Carries the user-facing message verbatim so
/// `main.rs` can print it without wrapping. Implements `Error` via a
/// manual impl (we don't add a `thiserror` dep just for this — deviation
/// #1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreflightRefusal {
    /// `~/.gemini/oauth_creds.json` present, neither GEMINI_API_KEY nor
    /// GOOGLE_APPLICATION_CREDENTIALS set. Carries the full multi-line
    /// refusal message from the slice doc verbatim.
    GeminiOauthFreetier(String),
}

impl std::fmt::Display for PreflightRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PreflightRefusal::GeminiOauthFreetier(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for PreflightRefusal {}

/// Run every preflight gate. Returns `Ok(())` only when ALL gates pass
/// (or are bypassed via `force_allow`); the first refusal short-circuits.
///
/// **Threat-model `T10`**: callers MUST invoke this before any trust-store
/// or filesystem mutation. The wiring in [`crate::install_with_backends`]
/// satisfies this by placing the call between the env-var build (read-only)
/// and the `resolve_shell` / `trust_backend.add_root` calls.
pub fn run_preflight(
    env: &BaseEnv<'_>,
    force_allow: PreflightOverrides,
) -> Result<(), PreflightRefusal> {
    if !force_allow.allow_gemini_oauth {
        gemini::detect_gemini_oauth_freetier(env)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// `run_preflight` with `allow_gemini_oauth=true` bypasses the gate
    /// even when the OAuth refusal condition is met.
    #[test]
    fn run_preflight_bypasses_gemini_when_force_allow_true() {
        let home = tempfile::tempdir().expect("home tempdir");
        // Seed the OAuth refusal condition: oauth_creds.json present, no
        // env vars set.
        std::fs::create_dir_all(home.path().join(".gemini")).expect("create .gemini dir");
        std::fs::write(
            home.path().join(".gemini").join("oauth_creds.json"),
            br#"{"access_token":"fake"}"#,
        )
        .expect("seed oauth_creds.json");

        let env = BaseEnv {
            home: Some(home.path()),
            gemini_api_key: None,
            google_application_credentials: None,
        };
        let overrides = PreflightOverrides {
            allow_gemini_oauth: true,
        };
        assert!(
            run_preflight(&env, overrides).is_ok(),
            "force_allow MUST bypass the Gemini gate"
        );
    }

    /// `run_preflight` returns `Err(GeminiOauthFreetier)` when the
    /// refusal condition is met and no force flag is set.
    #[test]
    fn run_preflight_refuses_when_gemini_oauth_detected() {
        let home = tempfile::tempdir().expect("home tempdir");
        std::fs::create_dir_all(home.path().join(".gemini")).expect("create .gemini dir");
        std::fs::write(
            home.path().join(".gemini").join("oauth_creds.json"),
            br#"{"access_token":"fake"}"#,
        )
        .expect("seed oauth_creds.json");

        let env = BaseEnv {
            home: Some(home.path()),
            gemini_api_key: None,
            google_application_credentials: None,
        };
        let overrides = PreflightOverrides::default();
        let err = run_preflight(&env, overrides).expect_err("must refuse");
        assert!(matches!(err, PreflightRefusal::GeminiOauthFreetier(_)));
    }

    /// `run_preflight` returns `Ok(())` when Gemini is in API-key mode
    /// (oauth_creds.json present but GEMINI_API_KEY set).
    #[test]
    fn run_preflight_passes_when_gemini_in_api_key_mode() {
        let home = tempfile::tempdir().expect("home tempdir");
        std::fs::create_dir_all(home.path().join(".gemini")).expect("create .gemini dir");
        std::fs::write(
            home.path().join(".gemini").join("oauth_creds.json"),
            br#"{"access_token":"fake"}"#,
        )
        .expect("seed oauth_creds.json");

        let env = BaseEnv {
            home: Some(home.path()),
            gemini_api_key: Some("AIzaSy-fake-key"),
            google_application_credentials: None,
        };
        let overrides = PreflightOverrides::default();
        assert!(run_preflight(&env, overrides).is_ok());
    }

    /// `BaseEnv::home()` returns the fallback `/` when `home` is None —
    /// the gemini detector then treats the path as "not installed".
    #[test]
    fn base_env_home_returns_fallback_when_unset() {
        let env = BaseEnv::default();
        assert_eq!(env.home(), Path::new("/"));
    }

    /// `BaseEnv::var()` lookups for the two supported names return what
    /// the caller stored; unknown names return None.
    #[test]
    fn base_env_var_lookup_returns_stored_value_and_none_for_unknown() {
        let env = BaseEnv {
            home: None,
            gemini_api_key: Some("k1"),
            google_application_credentials: Some("/path/to/sa.json"),
        };
        assert_eq!(env.var("GEMINI_API_KEY"), Some("k1"));
        assert_eq!(
            env.var("GOOGLE_APPLICATION_CREDENTIALS"),
            Some("/path/to/sa.json")
        );
        assert_eq!(env.var("UNKNOWN_VAR"), None);
    }
}
