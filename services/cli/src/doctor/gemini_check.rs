//! SLICE 7 (COV_11) — Gemini CLI auth state probe.
//!
//! Wraps [`crate::preflight::gemini::detect`] from SLICE 6 — does NOT
//! reimplement the detection ladder. The mapping from
//! [`crate::preflight::gemini::GeminiPreflight`] to
//! [`GeminiCheckResult`] is the only logic here.
//!
//! ## Mapping
//!
//! | `GeminiPreflight` | `GeminiCheckResult` | Severity |
//! |-------------------|---------------------|----------|
//! | `NotInstalled` | `NotInstalled` | OK — Gemini absent is benign |
//! | `ApiKeyMode` | `Healthy { mode: ApiKey }` | OK |
//! | `ServiceAccountMode` | `Healthy { mode: ServiceAccount }` | OK |
//! | `OauthFreetierRefused` | `OauthFreetierWarning` | WARN — surfaces the refusal but doctor doesn't fail-closed (operator may have run install with `--force-allow-gemini-oauth`) |
//!
//! ## Why doctor warns rather than refuses
//!
//! The install-time gate already covered the refusal-or-bypass decision
//! (SLICE 6). Doctor's job is post-install diagnosis: if the operator
//! used `--force-allow-gemini-oauth`, they accepted that their Gemini
//! CLI may stop working — doctor should NOT re-refuse and exit-1 every
//! time they run `spendguard doctor`. Instead we surface the state as
//! a warning so the operator can see "yes, Gemini is in OAuth mode,
//! and yes, SpendGuard is up — re-auth if your Gemini CLI breaks".
//!
//! ## Exit-code implications
//!
//! Per slice prompt: exit code 2 when the Gemini OAuth refusal is
//! present. That's surfaced via `DoctorReport::exit_code()` in
//! [`crate::doctor`] — the `GeminiCheckResult::OauthFreetierWarning`
//! variant is the signal.

use crate::preflight::{gemini::GeminiPreflight, BaseEnv};

/// Outcome of the Gemini auth probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GeminiCheckResult {
    /// Gemini CLI is in API-key or service-account mode — SpendGuard's
    /// MITM proxy will work.
    Healthy { mode: GeminiAuthMode },
    /// Gemini CLI is in OAuth free-tier mode. Surfaced as a warning so
    /// `spendguard doctor` can exit 2 (matching the install refusal
    /// code) without making the install path itself unrecoverable.
    OauthFreetierWarning,
    /// Gemini CLI is not installed (no `~/.gemini/oauth_creds.json`).
    NotInstalled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeminiAuthMode {
    ApiKey,
    ServiceAccount,
}

impl GeminiCheckResult {
    /// One-line render.
    pub fn render(&self, use_color: bool) -> String {
        use crate::doctor::{paint, Color};
        match self {
            Self::Healthy { mode } => {
                let head = paint("OK", Color::Green, use_color);
                let mode_str = match mode {
                    GeminiAuthMode::ApiKey => "API key (GEMINI_API_KEY)",
                    GeminiAuthMode::ServiceAccount => {
                        "Vertex service account (GOOGLE_APPLICATION_CREDENTIALS)"
                    }
                };
                format!("{head} {mode_str}")
            }
            Self::OauthFreetierWarning => {
                let head = paint("WARN", Color::Yellow, use_color);
                format!(
                    "{head} Gemini CLI is signed into OAuth free-tier — \
                     MITM proxy will break the refresh handshake against \
                     accounts.google.com. Sign out + re-auth via \
                     `gemini auth use-api-key` or set \
                     GOOGLE_APPLICATION_CREDENTIALS."
                )
            }
            Self::NotInstalled => {
                let head = paint("OK", Color::Green, use_color);
                format!("{head} Gemini CLI not installed (nothing to check)")
            }
        }
    }
}

/// Probe Gemini CLI auth state. Pure-function over [`BaseEnv`]; no
/// process-env reads beyond what's already in `env`.
pub fn check(env: &BaseEnv<'_>) -> GeminiCheckResult {
    match crate::preflight::gemini::detect(env) {
        GeminiPreflight::NotInstalled => GeminiCheckResult::NotInstalled,
        GeminiPreflight::ApiKeyMode => GeminiCheckResult::Healthy {
            mode: GeminiAuthMode::ApiKey,
        },
        GeminiPreflight::ServiceAccountMode => GeminiCheckResult::Healthy {
            mode: GeminiAuthMode::ServiceAccount,
        },
        GeminiPreflight::OauthFreetierRefused => GeminiCheckResult::OauthFreetierWarning,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `check` maps the SLICE 6 ApiKeyMode to Healthy::ApiKey. Reuses
    /// the SLICE 6 OAuth creds seed pattern so we know we're hitting
    /// the same detection ladder.
    #[test]
    fn check_maps_api_key_mode_to_healthy_api_key() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(tmp.path().join(".gemini")).expect("mkdir .gemini");
        std::fs::write(
            tmp.path().join(".gemini").join("oauth_creds.json"),
            br#"{"access_token":"fake"}"#,
        )
        .expect("seed");
        let env = BaseEnv {
            home: Some(tmp.path()),
            gemini_api_key: Some("AIzaSy-fake"),
            google_application_credentials: None,
        };
        assert_eq!(
            check(&env),
            GeminiCheckResult::Healthy {
                mode: GeminiAuthMode::ApiKey,
            }
        );
    }

    /// `check` maps OauthFreetierRefused → OauthFreetierWarning.
    #[test]
    fn check_maps_oauth_freetier_to_warning_not_refusal() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(tmp.path().join(".gemini")).expect("mkdir .gemini");
        std::fs::write(
            tmp.path().join(".gemini").join("oauth_creds.json"),
            br#"{"access_token":"fake"}"#,
        )
        .expect("seed");
        let env = BaseEnv {
            home: Some(tmp.path()),
            gemini_api_key: None,
            google_application_credentials: None,
        };
        assert_eq!(check(&env), GeminiCheckResult::OauthFreetierWarning);
    }

    /// `check` maps absent oauth_creds.json → NotInstalled.
    #[test]
    fn check_maps_no_oauth_creds_to_not_installed() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let env = BaseEnv {
            home: Some(tmp.path()),
            gemini_api_key: None,
            google_application_credentials: None,
        };
        assert_eq!(check(&env), GeminiCheckResult::NotInstalled);
    }

    /// `check` maps ServiceAccountMode (GOOGLE_APPLICATION_CREDENTIALS
    /// set, no API key) → Healthy::ServiceAccount.
    #[test]
    fn check_maps_service_account_mode_to_healthy_service_account() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(tmp.path().join(".gemini")).expect("mkdir .gemini");
        std::fs::write(
            tmp.path().join(".gemini").join("oauth_creds.json"),
            br#"{"access_token":"fake"}"#,
        )
        .expect("seed");
        let env = BaseEnv {
            home: Some(tmp.path()),
            gemini_api_key: None,
            google_application_credentials: Some("/path/to/sa.json"),
        };
        assert_eq!(
            check(&env),
            GeminiCheckResult::Healthy {
                mode: GeminiAuthMode::ServiceAccount,
            }
        );
    }

    /// Renderer status tokens.
    #[test]
    fn render_emits_expected_status_tokens() {
        let healthy = GeminiCheckResult::Healthy {
            mode: GeminiAuthMode::ApiKey,
        };
        let plain = healthy.render(false);
        assert!(plain.contains("OK"));
        assert!(plain.contains("API key"));

        let warning = GeminiCheckResult::OauthFreetierWarning;
        let warn = warning.render(false);
        assert!(warn.contains("WARN"));
        assert!(warn.contains("OAuth free-tier"));
        assert!(warn.contains("gemini auth use-api-key"));

        let absent = GeminiCheckResult::NotInstalled;
        assert!(absent.render(false).contains("OK"));
    }
}
