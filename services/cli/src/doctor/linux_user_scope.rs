//! SLICE 7 (COV_11) — Linux user-scope `CURL_CA_BUNDLE` redirect.
//!
//! Implements design `§3 line 47`:
//!
//! > The SLICE 7 doctor surfaces the `CURL_CA_BUNDLE` / `SSL_CERT_FILE`
//! > env-var redirect.
//!
//! This is the operator-facing complement to the SLICE 3 R2 fail-closed
//! contract: when the user attempted `spendguard install --scope user`
//! on Linux, the install path errored before touching any system trust
//! store but DID write the PEM to `$HOME/.local/share/ca-certificates/`
//! as a courtesy. Doctor sees that file, infers the user-scope
//! attempt, and surfaces the canonical env-var redirect with copy-
//! paste-able commands.
//!
//! ## Read-only
//!
//! We do not export or set the env vars on the operator's behalf. The
//! renderer prints the literal `export CURL_CA_BUNDLE=… ;
//! export SSL_CERT_FILE=…` commands so the operator can paste them.
//!
//! ## Anti-scope
//!
//! - We don't probe `which curl` or any of the CLIs that consume
//!   `CURL_CA_BUNDLE` — that's a tools-detector concern, not a
//!   trust-store concern.
//! - We don't recommend `--scope system` from this surface — the
//!   redirect IS the per-user solution.

use crate::preflight::BaseEnv;
use std::path::PathBuf;

/// Marker that the user-scope install was attempted, plus the canonical
/// recovery commands the operator should run.
#[derive(Debug, Clone)]
pub struct LinuxUserScopeRedirect {
    /// Where SpendGuard staged the PEM as a user-scope courtesy.
    pub cert_path: PathBuf,
}

impl LinuxUserScopeRedirect {
    /// Render the multi-line redirect surface. The operator can copy-
    /// paste any of the four commands into their shell.
    pub fn render(&self, use_color: bool) -> String {
        use crate::doctor::{paint, Color};
        let head = paint("REDIRECT", Color::Yellow, use_color);
        let cert = self.cert_path.display();
        format!(
            "{head} per-user trust at {cert} — Linux trust tools do not honour \
             $HOME-scoped anchors. Use the env-var redirect:\n\
             \n\
             \texport CURL_CA_BUNDLE=\"{cert}\"\n\
             \texport SSL_CERT_FILE=\"{cert}\"\n\
             \texport REQUESTS_CA_BUNDLE=\"{cert}\"\n\
             \texport NODE_EXTRA_CA_CERTS=\"{cert}\""
        )
    }
}

/// Probe `$HOME/.local/share/ca-certificates/spendguard.crt`. Returns
/// `Some(redirect)` when the file is present (i.e. the SLICE 3 R2
/// user-scope courtesy path landed); `None` otherwise.
///
/// Cross-OS note: the file path is Linux-conventional, but we do NOT
/// gate the probe on `target_os = "linux"` because the test surface
/// (`BaseEnv::home` is a tempdir) is the same on every host. Production
/// hits on non-Linux are essentially no-ops because the directory tree
/// won't exist.
pub fn check(env: &BaseEnv<'_>) -> Option<LinuxUserScopeRedirect> {
    let home = env.home?;
    let cert_path = home
        .join(".local")
        .join("share")
        .join("ca-certificates")
        .join("spendguard.crt");
    if std::fs::metadata(&cert_path).is_ok() {
        Some(LinuxUserScopeRedirect { cert_path })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `check` returns `Some(_)` when the canonical user-scope cert path
    /// is populated under the injected HOME.
    #[test]
    fn check_returns_some_when_user_scope_cert_present() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cert_dir = tmp
            .path()
            .join(".local")
            .join("share")
            .join("ca-certificates");
        std::fs::create_dir_all(&cert_dir).expect("mkdir");
        let cert_path = cert_dir.join("spendguard.crt");
        std::fs::write(
            &cert_path,
            b"-----BEGIN CERTIFICATE-----\nBODY\n-----END CERTIFICATE-----\n",
        )
        .expect("seed cert");
        let env = BaseEnv {
            home: Some(tmp.path()),
            gemini_api_key: None,
            google_application_credentials: None,
        };
        let redirect = check(&env).expect("must surface redirect");
        assert_eq!(redirect.cert_path, cert_path);
    }

    /// `check` returns `None` when the file is absent.
    #[test]
    fn check_returns_none_when_user_scope_cert_absent() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let env = BaseEnv {
            home: Some(tmp.path()),
            gemini_api_key: None,
            google_application_credentials: None,
        };
        assert!(check(&env).is_none());
    }

    /// `check` returns `None` when HOME is unset (no path to probe).
    #[test]
    fn check_returns_none_when_home_unset() {
        let env = BaseEnv {
            home: None,
            gemini_api_key: None,
            google_application_credentials: None,
        };
        assert!(check(&env).is_none());
    }

    /// Renderer emits all four env-var redirect commands so the
    /// operator can pick whichever fits their CLI.
    #[test]
    fn render_emits_all_four_redirect_commands() {
        let redirect = LinuxUserScopeRedirect {
            cert_path: PathBuf::from("/home/u/.local/share/ca-certificates/spendguard.crt"),
        };
        let plain = redirect.render(false);
        for var in [
            "CURL_CA_BUNDLE",
            "SSL_CERT_FILE",
            "REQUESTS_CA_BUNDLE",
            "NODE_EXTRA_CA_CERTS",
        ] {
            assert!(
                plain.contains(var),
                "redirect must surface {var}, got: {plain}"
            );
        }
        // Header marker present.
        assert!(plain.contains("REDIRECT"));
        // Cert path embedded in every export line — surface once.
        let occurrences = plain
            .matches("/home/u/.local/share/ca-certificates/spendguard.crt")
            .count();
        assert_eq!(
            occurrences, 5,
            "cert path should appear once in the prose + 4 export lines"
        );
    }
}
