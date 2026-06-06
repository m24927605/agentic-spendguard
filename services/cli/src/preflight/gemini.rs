//! SLICE 6 (COV_10) — Gemini OAuth free-tier refusal detector.
//!
//! Per `design.md` §3.5 (line 77), refuse `HTTPS_PROXY` setup when Gemini
//! CLI is signed in via Google's free OAuth tier:
//!
//! > Detection: `~/.gemini/oauth_creds.json` present AND `GEMINI_API_KEY`
//! > unset AND `GOOGLE_APPLICATION_CREDENTIALS` unset.
//!
//! Rationale: Gemini's free-tier OAuth tokens are minted against
//! `accounts.google.com`. A forward HTTPS proxy with a self-signed CA
//! breaks the OAuth refresh handshake, and the user has no API key to
//! fall back on. Better to refuse cleanly with a 3-option recovery hint
//! than silently break the user's existing Gemini access.
//!
//! ## Detection order (read-only)
//!
//! 1. Probe `~/.gemini/oauth_creds.json` existence (filesystem read).
//! 2. Read `GEMINI_API_KEY` from injected env (no `std::env::var`).
//! 3. Read `GOOGLE_APPLICATION_CREDENTIALS` from injected env.
//!
//! Empty string env vars (`GEMINI_API_KEY=""`) are treated as unset —
//! the Gemini CLI itself treats them this way, and we mirror.
//!
//! ## Anti-scope
//!
//! - No reading of `oauth_creds.json` contents — existence check only,
//!   per slice doc anti-scope.
//! - No `which gemini` probe — the spec gates on the credential state,
//!   not the binary's presence. (A user with `~/.gemini/oauth_creds.json`
//!   but no `gemini` binary is mid-uninstall and should still refuse,
//!   matching `design.md` §3.5 semantics.)

use crate::preflight::{BaseEnv, PreflightRefusal};
use std::path::PathBuf;

/// Outcome of probing for Gemini CLI's auth mode.
///
/// Variant ordering matches the slice-doc detection ladder so reading the
/// enum top-to-bottom mirrors the logic in [`detect`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeminiPreflight {
    /// `~/.gemini/oauth_creds.json` absent. The Gemini CLI either was
    /// never installed or the user has signed out.
    NotInstalled,
    /// `GEMINI_API_KEY` is set to a non-empty value. Gemini CLI prefers
    /// API-key mode over OAuth when both are present, so the proxy can
    /// safely MITM.
    ApiKeyMode,
    /// `GOOGLE_APPLICATION_CREDENTIALS` is set to a non-empty value
    /// (Vertex AI service-account JSON path). Gemini CLI uses the
    /// service-account token, not OAuth, so the proxy can safely MITM.
    ServiceAccountMode,
    /// `~/.gemini/oauth_creds.json` present AND both env vars unset
    /// (or empty). This is the legal red line — refuse install per
    /// `design.md` §6.
    OauthFreetierRefused,
}

/// Probe Gemini CLI's auth mode from the injected env. Read-only — no
/// filesystem mutations, no process-env mutations.
///
/// **Order matters**: `GEMINI_API_KEY` is checked before
/// `GOOGLE_APPLICATION_CREDENTIALS` because the Gemini CLI itself prefers
/// API-key auth when both are set (per its CLI docs). The detection
/// outcome surfaces whichever route the CLI would take.
pub fn detect(env: &BaseEnv<'_>) -> GeminiPreflight {
    let creds_path = oauth_creds_path(env);
    let creds_exists = std::fs::metadata(&creds_path).is_ok();
    if !creds_exists {
        return GeminiPreflight::NotInstalled;
    }
    if env_var_is_set(env, "GEMINI_API_KEY") {
        return GeminiPreflight::ApiKeyMode;
    }
    if env_var_is_set(env, "GOOGLE_APPLICATION_CREDENTIALS") {
        return GeminiPreflight::ServiceAccountMode;
    }
    GeminiPreflight::OauthFreetierRefused
}

/// Run [`detect`] and translate the refusal outcome into a typed
/// [`PreflightRefusal`]. Used by [`crate::preflight::run_preflight`].
pub fn detect_gemini_oauth_freetier(env: &BaseEnv<'_>) -> Result<(), PreflightRefusal> {
    match detect(env) {
        GeminiPreflight::OauthFreetierRefused => Err(PreflightRefusal::GeminiOauthFreetier(
            REFUSAL_MESSAGE.to_string(),
        )),
        // NotInstalled / ApiKeyMode / ServiceAccountMode all pass.
        _ => Ok(()),
    }
}

/// Compute `~/.gemini/oauth_creds.json` relative to the injected HOME.
fn oauth_creds_path(env: &BaseEnv<'_>) -> PathBuf {
    env.home().join(".gemini").join("oauth_creds.json")
}

/// Treat an empty-string env var as unset — Gemini CLI itself does this,
/// and the slice tests assert it.
fn env_var_is_set(env: &BaseEnv<'_>, name: &str) -> bool {
    env.var(name).map(|v| !v.is_empty()).unwrap_or(false)
}

/// User-facing refusal message. Lifted verbatim from the slice doc §3.5
/// callout so the operator's terminal output matches the design spec
/// byte-for-byte. The three recovery options (API key / service account /
/// `--force-allow-gemini-oauth`) are required by the slice prompt.
pub const REFUSAL_MESSAGE: &str = "Refusing to install: Gemini CLI is using OAuth free-tier authentication (~/.gemini/oauth_creds.json present, no GEMINI_API_KEY, no GOOGLE_APPLICATION_CREDENTIALS).

SpendGuard's HTTPS proxy with a self-signed CA breaks Gemini's OAuth refresh handshake against accounts.google.com.

To proceed:
  1. Set GEMINI_API_KEY (paid API tier), or
  2. Set GOOGLE_APPLICATION_CREDENTIALS (Vertex AI service account), or
  3. Use 'spendguard install --force-allow-gemini-oauth' to override
     (your Gemini CLI may stop working until you sign out + re-auth).";

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// Seed a tempdir with `~/.gemini/oauth_creds.json`. Returns the tempdir
    /// (which the caller must keep alive for the duration of the test).
    fn seed_oauth_creds() -> tempfile::TempDir {
        let home = tempfile::tempdir().expect("home tempdir");
        std::fs::create_dir_all(home.path().join(".gemini")).expect("create .gemini dir");
        std::fs::write(
            home.path().join(".gemini").join("oauth_creds.json"),
            br#"{"access_token":"fake","refresh_token":"fake"}"#,
        )
        .expect("seed oauth_creds.json");
        home
    }

    /// Detection ladder rule 1: oauth_creds.json present + both env vars
    /// unset → OauthFreetierRefused.
    #[test]
    fn oauth_creds_present_no_env_vars_returns_oauth_freetier_refused() {
        let home = seed_oauth_creds();
        let env = BaseEnv {
            home: Some(home.path()),
            gemini_api_key: None,
            google_application_credentials: None,
        };
        assert_eq!(detect(&env), GeminiPreflight::OauthFreetierRefused);
    }

    /// Detection ladder rule 2: oauth_creds.json present + GEMINI_API_KEY
    /// set → ApiKeyMode (API key wins over OAuth).
    #[test]
    fn oauth_creds_present_with_api_key_returns_api_key_mode() {
        let home = seed_oauth_creds();
        let env = BaseEnv {
            home: Some(home.path()),
            gemini_api_key: Some("AIzaSy-real-key"),
            google_application_credentials: None,
        };
        assert_eq!(detect(&env), GeminiPreflight::ApiKeyMode);
    }

    /// Detection ladder rule 3: oauth_creds.json present + service
    /// account env set (and no API key) → ServiceAccountMode.
    #[test]
    fn oauth_creds_present_with_service_account_returns_service_account_mode() {
        let home = seed_oauth_creds();
        let env = BaseEnv {
            home: Some(home.path()),
            gemini_api_key: None,
            google_application_credentials: Some("/path/to/sa.json"),
        };
        assert_eq!(detect(&env), GeminiPreflight::ServiceAccountMode);
    }

    /// Detection short-circuit: oauth_creds.json absent → NotInstalled
    /// regardless of env vars.
    #[test]
    fn oauth_creds_absent_returns_not_installed() {
        let home = tempfile::tempdir().expect("home tempdir");
        let env = BaseEnv {
            home: Some(home.path()),
            gemini_api_key: None,
            google_application_credentials: None,
        };
        assert_eq!(detect(&env), GeminiPreflight::NotInstalled);
    }

    /// Both env vars set → ApiKeyMode wins (API key is checked first).
    #[test]
    fn both_env_vars_set_returns_api_key_mode() {
        let home = seed_oauth_creds();
        let env = BaseEnv {
            home: Some(home.path()),
            gemini_api_key: Some("AIzaSy-real-key"),
            google_application_credentials: Some("/path/to/sa.json"),
        };
        assert_eq!(detect(&env), GeminiPreflight::ApiKeyMode);
    }

    /// Empty `GEMINI_API_KEY=""` is treated as unset.
    #[test]
    fn empty_gemini_api_key_is_treated_as_unset() {
        let home = seed_oauth_creds();
        let env = BaseEnv {
            home: Some(home.path()),
            gemini_api_key: Some(""),
            google_application_credentials: None,
        };
        assert_eq!(detect(&env), GeminiPreflight::OauthFreetierRefused);
    }

    /// Empty `GOOGLE_APPLICATION_CREDENTIALS=""` is treated as unset.
    #[test]
    fn empty_google_application_credentials_is_treated_as_unset() {
        let home = seed_oauth_creds();
        let env = BaseEnv {
            home: Some(home.path()),
            gemini_api_key: None,
            google_application_credentials: Some(""),
        };
        assert_eq!(detect(&env), GeminiPreflight::OauthFreetierRefused);
    }

    /// `detect_gemini_oauth_freetier` returns `Err(...)` carrying the
    /// REFUSAL_MESSAGE for the refusal case.
    #[test]
    fn detect_gemini_oauth_freetier_err_carries_refusal_message() {
        let home = seed_oauth_creds();
        let env = BaseEnv {
            home: Some(home.path()),
            gemini_api_key: None,
            google_application_credentials: None,
        };
        let err = detect_gemini_oauth_freetier(&env).expect_err("must refuse");
        let msg = format!("{err}");
        // Slice-prompt-mandated content: NOT "CURL_CA_BUNDLE" (which is
        // the Linux trust-store wording) but the Gemini-specific tokens.
        assert!(
            msg.contains("GEMINI_API_KEY"),
            "refusal message must reference GEMINI_API_KEY env var"
        );
        assert!(
            msg.contains("GOOGLE_APPLICATION_CREDENTIALS"),
            "refusal message must reference GOOGLE_APPLICATION_CREDENTIALS env var"
        );
        assert!(
            msg.contains("--force-allow-gemini-oauth"),
            "refusal message must surface the override flag"
        );
        assert!(
            !msg.contains("CURL_CA_BUNDLE"),
            "refusal message MUST NOT reference the Linux trust-store wording"
        );
    }

    /// `REFUSAL_MESSAGE` lists all 3 recovery options.
    #[test]
    fn refusal_message_lists_all_three_recovery_options() {
        // Three numbered options per slice doc.
        assert!(REFUSAL_MESSAGE.contains("1. Set GEMINI_API_KEY"));
        assert!(REFUSAL_MESSAGE.contains("2. Set GOOGLE_APPLICATION_CREDENTIALS"));
        assert!(REFUSAL_MESSAGE.contains("3. Use 'spendguard install --force-allow-gemini-oauth'"));
        // Plus the OAuth diagnostic header.
        assert!(REFUSAL_MESSAGE.starts_with("Refusing to install: Gemini CLI is using OAuth"));
        // And the accounts.google.com root-cause explanation.
        assert!(REFUSAL_MESSAGE.contains("accounts.google.com"));
    }

    /// Custom HOME via BaseEnv mock → preflight reads from the mock HOME.
    /// Verifies the detector has no hardcoded `~/` or
    /// `std::env::var("HOME")` reads.
    #[test]
    fn custom_home_via_base_env_drives_oauth_creds_lookup() {
        // Create two homes — one with the OAuth file, one without. Point
        // BaseEnv at the SECOND home; the detector MUST NOT find the
        // file in the first.
        let oauth_home = seed_oauth_creds();
        let clean_home = tempfile::tempdir().expect("clean home tempdir");

        // Sanity: the OAuth file landed where seed_oauth_creds put it.
        assert!(oauth_home
            .path()
            .join(".gemini")
            .join("oauth_creds.json")
            .exists());

        let env = BaseEnv {
            home: Some(clean_home.path()),
            gemini_api_key: None,
            google_application_credentials: None,
        };
        // Detector reads from clean_home, finds nothing → NotInstalled.
        assert_eq!(detect(&env), GeminiPreflight::NotInstalled);

        // Now point at oauth_home → OauthFreetierRefused.
        let env2 = BaseEnv {
            home: Some(oauth_home.path()),
            gemini_api_key: None,
            google_application_credentials: None,
        };
        assert_eq!(detect(&env2), GeminiPreflight::OauthFreetierRefused);
    }

    /// `oauth_creds_path` joins `.gemini/oauth_creds.json` onto the
    /// injected HOME — sanity that the path-join is correct under
    /// macOS/Linux/Windows path semantics.
    #[test]
    fn oauth_creds_path_joins_dot_gemini_subdir() {
        let env = BaseEnv {
            home: Some(Path::new("/Users/test")),
            gemini_api_key: None,
            google_application_credentials: None,
        };
        let path = oauth_creds_path(&env);
        assert!(
            path.ends_with(".gemini/oauth_creds.json")
                || path.ends_with(r".gemini\oauth_creds.json"),
            "expected .gemini/oauth_creds.json suffix, got {path:?}"
        );
        assert!(path.starts_with("/Users/test"));
    }
}
