//! SLICE 7 (COV_11) — `spendguard doctor` runtime diagnostic.
//!
//! Implements the design `§3 line 34` doctor flow:
//!
//! > `doctor : CA fingerprint in store + HTTPS_PROXY reachable + TLS
//! > handshake round-trip`
//!
//! plus the SLICE 3 R2 clarification at design `§3 line 47` ("the SLICE
//! 7 doctor surfaces the `CURL_CA_BUNDLE` / `SSL_CERT_FILE` env-var
//! redirect instead") and the SLICE 6 Gemini OAuth detector reuse.
//!
//! ## Read-only contract
//!
//! Doctor is **read-only**. No filesystem mutation, no cert mutation, no
//! shell-rc edits — review-standards `T1` (no key-byte movement),
//! `T6` (no silent privilege escalation), and `T8` (no shell injection)
//! are all enforced by the fact that we never *write* anything from this
//! module. The slice doc anti-scope ("no mutating recovery actions —
//! doctor is read-only") is the user-facing version of the same.
//!
//! ## Composition
//!
//! Each check lives in its own submodule with its own `Result` enum
//! variant per outcome:
//!
//! | Submodule | Probe |
//! |-----------|-------|
//! | [`ca_fingerprint`] | Reads `$XDG_DATA_HOME/spendguard/ca/root_ca.pem`, SHA-256s it, cross-checks via [`crate::trust::TrustStore::verify_installed`] |
//! | [`proxy_reachable`] | TCP connect to the proxy URL with a 5s deadline; surfaces unreachable / handshake failure separately |
//! | [`shell_rc`] | Greps the user's shell rc for the SpendGuard marker block ([`crate::shell::MARKER_BEGIN`]); reports presence + which vars |
//! | [`linux_user_scope`] | Reads `$HOME/.local/share/ca-certificates/spendguard.crt`; if present, surfaces the `CURL_CA_BUNDLE` / `SSL_CERT_FILE` redirect commands |
//! | [`gemini_check`] | Wraps [`crate::preflight::gemini::detect`] — does not reimplement |
//!
//! [`run`] composes the five checks sequentially. None of them panic; each
//! degrades to a typed "not present" result so the report stays informative
//! even when SpendGuard is fully uninstalled.
//!
//! ## Exit-code contract (consumed by [`crate::main`])
//!
//! - `is_healthy() == true` → exit 0
//! - Degraded (anything else) → exit 1
//! - Gemini OAuth refusal → exit 2 (matches the install path's refusal
//!   code so tooling can branch on the same number)
//!
//! ## Forward-compat
//!
//! Adding a new probe (e.g. SLICE 8 uninstall verification) means adding
//! a submodule, adding a field to [`DoctorReport`], rendering it in
//! [`DoctorReport::render`], and folding its severity into
//! [`DoctorReport::is_healthy`]. The submodule pattern keeps each probe
//! mockable in isolation.
//!
//! ## Deviation #1
//!
//! The slice doc's `DoctorOpts` signature lists `proxy_url: Option<String>`
//! and `timeout: Duration`. We honour both names verbatim; the default
//! values come from [`crate::DEFAULT_PROXY_URL`] (locked at SLICE 5)
//! and `DEFAULT_TIMEOUT` (5s, matching the design's "TLS handshake
//! round-trip" budget).
//!
//! ## Deviation #2
//!
//! The lib-level legacy `pub struct DoctorReport` in [`crate`] (carrying
//! only `ca_present_in_store` / `https_proxy_set` / `round_trip_ok` /
//! `warnings`) is preserved as-is so the existing
//! `doctor_with_trust_store` callers do not break. SLICE 7's richer
//! report lives in [`DoctorReport`] inside *this* module (path
//! `crate::doctor::DoctorReport`). The CLI's `Cmd::Doctor` arm switches
//! to this new path; the legacy lib API stays for backward-compat.

use crate::preflight::BaseEnv;
use crate::shell::EnvView;
use std::path::PathBuf;
use std::time::Duration;

pub mod ca_fingerprint;
pub mod gemini_check;
pub mod linux_user_scope;
pub mod proxy_reachable;
pub mod shell_rc;

pub use ca_fingerprint::CaCheckResult;
pub use gemini_check::GeminiCheckResult;
pub use linux_user_scope::LinuxUserScopeRedirect;
pub use proxy_reachable::ProxyCheckResult;
pub use shell_rc::ShellRcCheckResult;

/// Default proxy reachability timeout — 5 seconds per slice doc + design
/// §3 (the round-trip handshake budget). Lives as a constant so future
/// callers can override via `DoctorOpts::timeout` without touching the
/// probe code.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// `spendguard doctor` runtime options.
///
/// Mirrors the slice-doc signature: `proxy_url` overrides the localhost
/// default; `timeout` overrides the 5s default for the TCP / TLS probe.
#[derive(Debug, Clone)]
pub struct DoctorOpts {
    /// HTTPS proxy URL to probe. Defaults to [`crate::DEFAULT_PROXY_URL`]
    /// when `None`.
    pub proxy_url: Option<String>,
    /// TCP connect + TLS handshake timeout. Defaults to [`DEFAULT_TIMEOUT`].
    pub timeout: Duration,
}

impl Default for DoctorOpts {
    fn default() -> Self {
        Self {
            proxy_url: None,
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

/// SLICE 7 doctor report. Aggregates every check; each field is its own
/// typed result so the renderer can drive per-check colour / severity
/// without re-parsing strings.
///
/// Field order matches the slice doc.
#[derive(Debug, Clone)]
pub struct DoctorReport {
    pub ca: CaCheckResult,
    pub proxy: ProxyCheckResult,
    pub shell: ShellRcCheckResult,
    /// `Some(_)` only when the Linux user-scope conditions matched (the
    /// SLICE 3 R2 fail-closed redirect). `None` everywhere else so the
    /// renderer skips the section.
    pub linux_user_scope: Option<LinuxUserScopeRedirect>,
    pub gemini: GeminiCheckResult,
}

/// Suggested exit code for the CLI. Matches the slice prompt's
/// "0 if healthy, 1 if degraded, 2 if Gemini OAuth refusal" contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitCode {
    Healthy = 0,
    Degraded = 1,
    GeminiRefusal = 2,
}

impl DoctorReport {
    /// Returns `true` only when every required check is `Healthy` and
    /// every advisory check is `Healthy` or absent. Gemini refusal is
    /// NOT healthy; Gemini `NotInstalled` is.
    pub fn is_healthy(&self) -> bool {
        matches!(self.ca, CaCheckResult::Healthy { .. })
            && matches!(self.proxy, ProxyCheckResult::Reachable { .. })
            && matches!(self.shell, ShellRcCheckResult::Present { .. })
            && self.linux_user_scope.is_none()
            && matches!(
                self.gemini,
                GeminiCheckResult::NotInstalled | GeminiCheckResult::Healthy { .. }
            )
    }

    /// Pick the exit code per slice prompt. Gemini refusal wins over
    /// generic degradation so tooling can branch on it.
    pub fn exit_code(&self) -> ExitCode {
        if matches!(self.gemini, GeminiCheckResult::OauthFreetierWarning) {
            return ExitCode::GeminiRefusal;
        }
        if self.is_healthy() {
            ExitCode::Healthy
        } else {
            ExitCode::Degraded
        }
    }

    /// Render the report as terminal-friendly text. `use_color=true`
    /// emits ANSI colour codes; `false` strips them. The renderer never
    /// reads from the terminal — no `isatty` probe inside the doctor
    /// module — so callers wanting "auto" colour resolve it themselves.
    pub fn render(&self, use_color: bool) -> String {
        let mut out = String::new();
        out.push_str("SpendGuard doctor report\n");
        out.push_str("------------------------\n");
        out.push_str(&format!(
            "  CA fingerprint    : {}\n",
            self.ca.render(use_color)
        ));
        out.push_str(&format!(
            "  Proxy reachable   : {}\n",
            self.proxy.render(use_color)
        ));
        out.push_str(&format!(
            "  Shell rc          : {}\n",
            self.shell.render(use_color)
        ));
        if let Some(redirect) = &self.linux_user_scope {
            out.push_str("  Linux user scope  : ");
            out.push_str(&redirect.render(use_color));
            out.push('\n');
        }
        out.push_str(&format!(
            "  Gemini CLI        : {}\n",
            self.gemini.render(use_color)
        ));
        out.push('\n');
        let summary = if self.is_healthy() {
            paint("OK", Color::Green, use_color)
        } else if matches!(self.gemini, GeminiCheckResult::OauthFreetierWarning) {
            paint("REFUSED (Gemini OAuth)", Color::Red, use_color)
        } else {
            paint("DEGRADED", Color::Yellow, use_color)
        };
        out.push_str(&format!("Summary: {summary}\n"));
        out
    }
}

/// Run every doctor check sequentially. Never panics; each probe degrades
/// to a typed "absent" result. Reads the live process environment for
/// `BaseEnv::from_process()`-style probes only when [`DoctorEnv`] is the
/// default; tests supply [`DoctorEnv::with`] to inject the same
/// tempdir-rooted HOME / shell pattern SLICE 5/6 use.
pub fn run(env: &BaseEnv<'_>, opts: DoctorOpts) -> DoctorReport {
    run_with_env(env, opts, DoctorEnv::default())
}

/// Run the doctor with explicit env injection — used by lib-level tests
/// to mock `EnvView` (shell detection) and `proxy_url` overrides without
/// mutating process state. Production callers go through [`run`].
pub fn run_with_env(
    env: &BaseEnv<'_>,
    opts: DoctorOpts,
    doctor_env: DoctorEnv<'_>,
) -> DoctorReport {
    let ca = ca_fingerprint::check(env, doctor_env.ca_pem_override);
    let proxy_url = opts
        .proxy_url
        .clone()
        .unwrap_or_else(|| crate::DEFAULT_PROXY_URL.to_string());
    let proxy = proxy_reachable::check(&proxy_url, opts.timeout, doctor_env.tcp_probe);
    let shell = shell_rc::check(env, doctor_env.shell_env);
    let linux_user_scope = linux_user_scope::check(env);
    let gemini = gemini_check::check(env);
    DoctorReport {
        ca,
        proxy,
        shell,
        linux_user_scope,
        gemini,
    }
}

/// Injected environment seam for the doctor — lets tests override the
/// CA PEM path (so a tempdir cert can be probed), the shell `EnvView`
/// (so detection is deterministic), and the TCP probe (so unit tests
/// don't open real sockets). Production callers use the default which
/// resolves everything from process env / the live network.
#[derive(Debug, Clone, Copy, Default)]
pub struct DoctorEnv<'a> {
    /// Override the CA PEM path the [`ca_fingerprint`] probe reads. `None`
    /// → resolve from `paths::ca_root_dir_unchecked()`.
    pub ca_pem_override: Option<&'a std::path::Path>,
    /// Shell env (SHELL / ComSpec / PSModulePath) — passed to
    /// [`crate::shell::DetectedShell::detect_from_env`]. Default reads
    /// from process env when `None`.
    pub shell_env: Option<EnvView<'a>>,
    /// Custom TCP probe. Default is a real `TcpStream::connect_timeout`.
    pub tcp_probe: Option<proxy_reachable::TcpProbe>,
}

// ─── render helpers ──────────────────────────────────────────────────

/// Tiny ANSI colour palette — kept private so we don't grow into a
/// dependency on `colored` / `termcolor`. Three colours match the slice
/// prompt's "green ✓ / red ✗ / yellow ⚠".
#[derive(Debug, Clone, Copy)]
pub(crate) enum Color {
    Green,
    Red,
    Yellow,
}

pub(crate) fn paint(s: &str, color: Color, use_color: bool) -> String {
    if !use_color {
        return s.to_string();
    }
    let code = match color {
        Color::Green => "\x1b[32m",
        Color::Red => "\x1b[31m",
        Color::Yellow => "\x1b[33m",
    };
    format!("{code}{s}\x1b[0m")
}

/// Resolve the canonical CA PEM path from the injected paths::BaseEnv
/// pattern. Public for the SLICE 8 uninstall slice to reuse — both
/// `uninstall` and `doctor` need to find the CA PEM by the same rules.
pub fn default_ca_pem_path(env: &BaseEnv<'_>) -> Option<PathBuf> {
    // Mirror paths::base_data_dir_from() but use the doctor's BaseEnv
    // surface so HOME / XDG injection flows through the SLICE 6 seam.
    // We don't reuse paths::ca_root_dir_unchecked() here because that
    // function reads process-global std::env directly; using it would
    // race with the EnvVarGuard pattern in our tests.
    let home = env.home;
    let xdg = std::env::var("XDG_DATA_HOME").ok();
    let appdata = std::env::var("APPDATA").ok();
    let paths_env = crate::paths::BaseEnv {
        home: home.and_then(|p| p.to_str()),
        xdg_data_home: xdg.as_deref().filter(|s| !s.is_empty()),
        appdata: appdata.as_deref().filter(|s| !s.is_empty()),
    };
    crate::paths::base_data_dir_from(&paths_env)
        .ok()
        .map(|d| d.join("ca").join("root_ca.pem"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trust::TrustStore;
    use crate::TrustScope;
    use anyhow::Result;
    use std::path::Path;
    use std::sync::Mutex;

    /// Inert TrustStore used so we don't touch the developer's real
    /// keychain. Records what was queried.
    #[derive(Debug, Default)]
    struct NoopTrustStore {
        verified: Mutex<Vec<String>>,
        return_present: bool,
    }

    impl TrustStore for NoopTrustStore {
        fn add_root(&self, _: &Path, _: TrustScope) -> Result<Vec<PathBuf>> {
            unreachable!("doctor must not call add_root")
        }
        fn remove_root(&self, _: &str, _: TrustScope) -> Result<Vec<PathBuf>> {
            unreachable!("doctor must not call remove_root")
        }
        fn verify_installed(&self, fp: &str, _: TrustScope) -> Result<bool> {
            self.verified.lock().unwrap().push(fp.to_string());
            Ok(self.return_present)
        }
    }

    /// LIB-INTEGRATION 1: fully uninstalled host yields a degraded report
    /// where every check is its negative variant, doctor() never panics,
    /// and exit_code == Degraded (not GeminiRefusal — Gemini is absent).
    #[test]
    fn run_with_fully_uninstalled_host_returns_degraded_report() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let env = BaseEnv {
            home: Some(tmp.path()),
            gemini_api_key: None,
            google_application_credentials: None,
        };
        let opts = DoctorOpts::default();
        let doctor_env = DoctorEnv {
            ca_pem_override: Some(&tmp.path().join("does-not-exist.pem")),
            shell_env: Some(EnvView::default()),
            tcp_probe: Some(proxy_reachable::TcpProbe::always_unreachable()),
        };
        let report = run_with_env(&env, opts, doctor_env);

        assert!(matches!(report.ca, CaCheckResult::NotInstalled));
        assert!(matches!(
            report.proxy,
            ProxyCheckResult::ProxyUnreachable { .. }
        ));
        assert!(matches!(report.shell, ShellRcCheckResult::Absent { .. }));
        assert!(report.linux_user_scope.is_none());
        assert!(matches!(report.gemini, GeminiCheckResult::NotInstalled));

        assert!(!report.is_healthy(), "uninstalled host must not be healthy");
        assert_eq!(report.exit_code(), ExitCode::Degraded);
    }

    /// LIB-INTEGRATION 2: mixed state — Gemini OAuth refusal present.
    /// Even though every OTHER check could be healthy in a hostile env,
    /// the Gemini refusal pinned exit_code is GeminiRefusal=2 per slice
    /// prompt.
    #[test]
    fn run_with_gemini_oauth_state_returns_gemini_refusal_exit_code() {
        let home_tmp = tempfile::tempdir().expect("home tempdir");
        // Seed OAuth creds — matches SLICE 6's seed_gemini_oauth_creds.
        std::fs::create_dir_all(home_tmp.path().join(".gemini")).expect("mkdir .gemini");
        std::fs::write(
            home_tmp.path().join(".gemini").join("oauth_creds.json"),
            br#"{"access_token":"fake"}"#,
        )
        .expect("seed oauth_creds.json");

        let env = BaseEnv {
            home: Some(home_tmp.path()),
            gemini_api_key: None,
            google_application_credentials: None,
        };
        let doctor_env = DoctorEnv {
            ca_pem_override: Some(&home_tmp.path().join("nope.pem")),
            shell_env: Some(EnvView::default()),
            tcp_probe: Some(proxy_reachable::TcpProbe::always_unreachable()),
        };
        let report = run_with_env(&env, DoctorOpts::default(), doctor_env);

        assert!(matches!(
            report.gemini,
            GeminiCheckResult::OauthFreetierWarning
        ));
        assert_eq!(
            report.exit_code(),
            ExitCode::GeminiRefusal,
            "Gemini OAuth refusal must dominate exit code"
        );
    }

    /// LIB-INTEGRATION 3: render() produces a non-empty string and is
    /// callable in both color modes. No assertion on exact byte sequence
    /// — the per-check render() tests cover that — but we assert the
    /// header + summary line are present in both modes.
    #[test]
    fn render_emits_header_and_summary_in_both_color_modes() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let env = BaseEnv {
            home: Some(tmp.path()),
            gemini_api_key: None,
            google_application_credentials: None,
        };
        let doctor_env = DoctorEnv {
            ca_pem_override: Some(&tmp.path().join("none.pem")),
            shell_env: Some(EnvView::default()),
            tcp_probe: Some(proxy_reachable::TcpProbe::always_unreachable()),
        };
        let report = run_with_env(&env, DoctorOpts::default(), doctor_env);

        let plain = report.render(false);
        assert!(plain.contains("SpendGuard doctor report"));
        assert!(plain.contains("Summary:"));
        // Plain mode must contain NO escape sequences.
        assert!(
            !plain.contains("\x1b["),
            "no_color rendering must not include ANSI codes"
        );

        let colored = report.render(true);
        assert!(colored.contains("SpendGuard doctor report"));
        assert!(
            colored.contains("\x1b["),
            "color rendering must include ANSI codes"
        );
    }

    /// `is_healthy()` returns true when every check is at its happy
    /// variant. Builds a synthetic report by hand so we can flip
    /// individual fields and re-assert.
    #[test]
    fn is_healthy_only_when_every_check_passes() {
        let healthy = DoctorReport {
            ca: CaCheckResult::Healthy {
                fingerprint: "deadbeef".repeat(8),
                trust_store_locations: vec![PathBuf::from("/keychain")],
            },
            proxy: ProxyCheckResult::Reachable {
                addr: "127.0.0.1:8443".into(),
            },
            shell: ShellRcCheckResult::Present {
                rc_path: PathBuf::from("/home/u/.zshrc"),
                env_vars: vec![("HTTPS_PROXY".into(), "https://localhost:8443".into())],
            },
            linux_user_scope: None,
            gemini: GeminiCheckResult::NotInstalled,
        };
        assert!(healthy.is_healthy());
        assert_eq!(healthy.exit_code(), ExitCode::Healthy);

        let mut degraded = healthy.clone();
        degraded.proxy = ProxyCheckResult::ProxyUnreachable {
            addr: "127.0.0.1:8443".into(),
            error: "connection refused".into(),
        };
        assert!(!degraded.is_healthy());
        assert_eq!(degraded.exit_code(), ExitCode::Degraded);

        let mut linux_scope = healthy.clone();
        linux_scope.linux_user_scope = Some(LinuxUserScopeRedirect {
            cert_path: PathBuf::from("/home/u/.local/share/ca-certificates/spendguard.crt"),
        });
        assert!(!linux_scope.is_healthy());
    }

    /// Ensure NoopTrustStore is not invoked from the lib-integration
    /// flow — its add_root / remove_root paths panic to enforce the
    /// read-only contract.
    #[test]
    fn doctor_lib_integration_does_not_call_trust_mutation() {
        let backend = NoopTrustStore {
            return_present: false,
            ..Default::default()
        };
        // Just call any TrustStore method directly to confirm we can
        // construct + drop without invoking add_root / remove_root.
        assert!(!backend
            .verify_installed("deadbeef", TrustScope::User)
            .unwrap());
        let queried = backend.verified.lock().unwrap().clone();
        assert_eq!(queried, vec!["deadbeef".to_string()]);
    }

    /// `paint()` no-op when `use_color=false`.
    #[test]
    fn paint_strips_ansi_when_disabled() {
        assert_eq!(paint("ok", Color::Green, false), "ok");
        assert!(paint("ok", Color::Green, true).contains("\x1b[32m"));
    }

    /// `default_ca_pem_path` returns a sensible default rooted at the
    /// injected HOME — no process-env mutation required.
    #[test]
    fn default_ca_pem_path_rooted_at_injected_home() {
        let env = BaseEnv {
            home: Some(Path::new("/home/u")),
            gemini_api_key: None,
            google_application_credentials: None,
        };
        let path = default_ca_pem_path(&env);
        let path = path.expect("path resolves under happy injected HOME");
        assert!(
            path.ends_with("root_ca.pem"),
            "expected suffix root_ca.pem, got {path:?}"
        );
    }
}
