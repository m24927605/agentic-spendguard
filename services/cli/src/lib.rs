//! Public library surface for `spendguard-cli`.
//!
//! Per design §8, the CLI exposes three top-level operations:
//!
//! - [`install`] — CA issuance, leaf issuance, trust-store install, per-tool
//!   env emitter. SLICE 1 (COV_05) implemented CA + leaf gen + on-disk PEM
//!   persistence. **SLICE 2 (COV_06, this slice)** wires the macOS trust
//!   store (`MacosTrustStore`) into `install` / `uninstall` / `doctor` and
//!   populates `InstallReport.trust_store_locations`. Linux trust install
//!   lands in SLICE 3 (COV_07), Windows in SLICE 4 (COV_08).
//! - [`uninstall`] — symmetric removal of trust-store entries +
//!   on-disk PEMs. This slice implements the trust-store half on macOS;
//!   the shell-rc / per-tool overrides half lands in SLICE 5 / SLICE 7.
//! - [`doctor`] — CA-in-store check. This slice implements the trust-store
//!   query on macOS; HTTPS_PROXY reachability + TLS handshake land in
//!   SLICE 7.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub mod ca;
pub mod doctor;
pub mod paths;
pub mod preflight;
pub mod shell;
pub mod tools;
pub mod trust;

/// Per-user vs admin trust scope. Parsed in this slice; honoured by SLICE
/// 2/3/4 trust-store backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrustScope {
    #[default]
    User,
    System,
}

/// Shell rc backend selector. Parsed here; honoured by SLICE 5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShellKind {
    Bash,
    Zsh,
    Fish,
    Pwsh,
}

/// `spendguard install` options. Field set is the v1 lock per design §2;
/// later slices populate more sub-reports but the option struct does not
/// grow.
#[derive(Debug, Clone, clap::Args)]
pub struct InstallOpts {
    /// `user` (no sudo) or `system` (admin). Trust-store install honours
    /// this in SLICE 2/3/4.
    #[arg(long, value_enum, default_value_t = TrustScope::User)]
    pub scope: TrustScope,

    /// Directory the four PEM files are written to. If omitted, resolved
    /// from `paths::ca_root_dir()` (OS-specific XDG path).
    #[arg(long, value_name = "DIR")]
    pub ca_out: Option<PathBuf>,

    /// Override shell detection (parsed; rc emission in SLICE 5).
    #[arg(long, value_enum)]
    pub shell: Option<ShellKind>,

    /// SLICE 6 (COV_10) — bypass the Gemini OAuth free-tier refusal
    /// preflight gate. The operator accepts that their Gemini CLI may
    /// stop working until they sign out and re-authenticate, because
    /// SpendGuard's self-signed CA breaks Gemini's OAuth refresh
    /// handshake against `accounts.google.com`. Default `false` so the
    /// safe path is the default.
    #[arg(long, default_value_t = false)]
    pub force_allow_gemini_oauth: bool,
}

impl InstallOpts {
    /// Project the install opts down to the preflight override flags. A
    /// helper so future overrides (additional gates) can be carried in
    /// `InstallOpts` and surfaced here without touching the install seam.
    pub fn preflight_overrides(&self) -> preflight::PreflightOverrides {
        preflight::PreflightOverrides {
            allow_gemini_oauth: self.force_allow_gemini_oauth,
        }
    }
}

/// `spendguard uninstall` options.
///
/// SLICE 2 (COV_06) honours `--scope` and `--ca-fingerprint` to drive the
/// macOS trust-store removal. SLICE 8 (COV_12) closes the loop with
/// on-disk PEM cleanup (auto-resolving the fingerprint from the PEM when
/// `--ca-fingerprint` is omitted), shell-rc strip, and per-tool config
/// cleanup. Two opt-out flags let the operator preserve specific
/// artefacts (e.g. when they want to keep the rc block live while
/// rotating the CA).
#[derive(Debug, Clone, clap::Args)]
pub struct UninstallOpts {
    /// Match the `--scope` used at install time so the symmetric removal
    /// targets the same trust store.
    #[arg(long, value_enum, default_value_t = TrustScope::User)]
    pub scope: TrustScope,

    /// Lower-case SHA-256 hex of the root CA to remove. When omitted
    /// (SLICE 8) the fingerprint is re-derived from the on-disk
    /// `root_ca.pem` at the canonical XDG path. Supply this explicitly
    /// only when the PEM has already been deleted and the operator has
    /// the fingerprint from a saved install report JSON.
    #[arg(long, value_name = "HEX")]
    pub ca_fingerprint: Option<String>,

    /// SLICE 8 (COV_12): preserve the SpendGuard marker block in the
    /// shell rc file. Default `false` so the rc strip runs symmetric to
    /// install. Use this when the operator wants to keep the rc-side
    /// breadcrumb (e.g. they're rotating the CA but want their tooling
    /// to keep reading `HTTPS_PROXY`).
    #[arg(long, default_value_t = false)]
    pub keep_shell_rc: bool,

    /// SLICE 8 (COV_12): preserve the on-disk CA + leaf PEM blobs.
    /// Default `false` so the four PEM files are deleted symmetric to
    /// install. Use this when the operator wants the trust-store entry
    /// gone but the PEM kept (e.g. for forensic / audit retention).
    #[arg(long, default_value_t = false)]
    pub keep_ca_files: bool,
}

/// `spendguard doctor` options.
///
/// SLICE 2 (COV_06) reads the supplied CA fingerprint and reports whether
/// it's trusted in the configured keychain scope. SLICE 7 expands this with
/// HTTPS_PROXY reachability + TLS handshake checks.
#[derive(Debug, Clone, clap::Args)]
pub struct DoctorOpts {
    /// Trust-store scope to probe.
    #[arg(long, value_enum, default_value_t = TrustScope::User)]
    pub scope: TrustScope,

    /// CA fingerprint to look for in the trust store. Optional — when
    /// omitted, the report flags `ca_present_in_store = false` and adds a
    /// warning. SLICE 7 will derive from the on-disk PEM.
    #[arg(long, value_name = "HEX")]
    pub ca_fingerprint: Option<String>,
}

/// What `install` produced. Public so SLICE 5 can extend it additively
/// (tools, shell rc paths) without breaking JSON consumers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallReport {
    pub ca_fingerprint_sha256: String,
    pub ca_pem_path: PathBuf,
    pub ca_key_path: PathBuf,
    pub leaf_pem_path: PathBuf,
    pub leaf_key_path: PathBuf,
    /// Populated by SLICE 2/3/4 OS trust backends; lets `doctor` reason
    /// about "issued but not trusted" when the list is empty.
    pub trust_store_locations: Vec<PathBuf>,
    /// **SLICE 5 (COV_09)**: rc file paths the writer touched. Empty when
    /// detection returned `Cmd` (operator runs `setx` from the breadcrumb
    /// in [`Self::shell_env_vars`]) or `--shell` was not supplied AND no
    /// shell could be detected.
    pub shell_rc_paths: Vec<PathBuf>,
    /// **SLICE 5 (COV_09)**: the `(name, value)` pairs the rc writer
    /// emitted (HTTPS_PROXY + per-tool overrides). Carried in the report
    /// JSON so `doctor` (SLICE 7) can verify each entry landed in the
    /// caller's environment, and so the `cmd.exe` breadcrumb path can
    /// surface the literal `setx HTTPS_PROXY …` lines for the operator.
    #[serde(default)]
    pub shell_env_vars: Vec<(String, String)>,
    /// **SLICE 5 (COV_09)**: per-tool stanza covering the full 14-row
    /// matrix from `design.md` §5 — id, display, env var (or empty for
    /// `ConfigFile` / `OsTrustOnly`), and optional notes. Lets the
    /// install report show the operator every tool that was considered,
    /// not just the ones that contributed env vars.
    #[serde(default)]
    pub tools: Vec<ToolReport>,
}

/// Per-tool install-report stanza — one row of `design.md` §5 plus the
/// resolved value that landed (when applicable).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolReport {
    pub id: String,
    pub display: String,
    /// Env var the tool reads, or empty for `ConfigFile` / `OsTrustOnly`.
    pub env_var: String,
    /// Resolved value (CA PEM path or proxy URL). Empty when the tool is
    /// `OsTrustOnly` or `ConfigFile` — the operator's checklist surfaces
    /// the alternative mechanism via [`Self::notes`].
    pub value: String,
    /// Free-form note (matches `ToolOverride::notes`).
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UninstallReport {
    /// Files the uninstall path actually touched — rc paths whose marker
    /// blocks were stripped AND CA / leaf PEM blobs that were deleted.
    /// SLICE 8 (COV_12) extends this to include the four PEM files when
    /// `--keep-ca-files` is not set.
    pub removed_files: Vec<PathBuf>,
    pub trust_store_locations_cleared: Vec<PathBuf>,
    /// SLICE 8 (COV_12): non-empty when one or more uninstall steps
    /// failed best-effort (rc strip failed but trust removal succeeded,
    /// or vice versa). The CLI maps `!warnings.is_empty()` →
    /// `EX_TEMPFAIL` (exit code 75) per implementation.md §9 so tooling
    /// can branch on partial cleanup without parsing strings.
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorReport {
    pub ca_present_in_store: bool,
    pub https_proxy_set: Option<String>,
    pub round_trip_ok: bool,
    pub warnings: Vec<String>,
}

/// `spendguard install` entry point.
///
/// SLICE 2 (COV_06) wires the macOS trust store on top of SLICE 1's CA
/// material:
///
/// 1. Issue a fresh root CA + localhost leaf (SLICE 1).
/// 2. Write all four PEM files via the atomic temp-file + 0o600 rename
///    pipeline (SLICE 1, R2 fix B1).
/// 3. **NEW**: Hand the CA PEM to the OS trust-store backend (macOS only
///    in this slice). Populates `InstallReport.trust_store_locations` so
///    the operator + `doctor` know exactly which keychain was touched.
///
/// Non-macOS callers fall through with an empty `trust_store_locations`
/// for now — SLICE 3 / SLICE 4 fill those in without an API break.
///
/// Internally this is a thin shim over [`install_with_trust_store`], which
/// the unit tests use with a no-op trust backend to avoid mutating the
/// developer's real keychain.
pub fn install(opts: &InstallOpts) -> Result<InstallReport> {
    let backend = trust_backend()?;
    install_with_trust_store(opts, backend.as_ref())
}

/// `spendguard install` with an injected trust backend. Public so the
/// integration test (`tests/trust_macos.rs`) and the eventual SLICE 7
/// driver can compose this without re-implementing the four-PEM emit.
pub fn install_with_trust_store(
    opts: &InstallOpts,
    trust_backend: &dyn trust::TrustStore,
) -> Result<InstallReport> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    let shell_env = shell::EnvView {
        shell: std::env::var("SHELL").ok().as_deref().map(leak_static),
        psmodulepath: std::env::var("PSModulePath")
            .ok()
            .as_deref()
            .map(leak_static),
        comspec: std::env::var("ComSpec").ok().as_deref().map(leak_static),
    };
    install_with_backends(
        opts,
        trust_backend,
        home.as_deref(),
        shell_env,
        DEFAULT_PROXY_URL,
    )
}

/// `spendguard install` with full backend injection — trust + HOME + shell
/// env detection + proxy URL. The thin shim above ([`install_with_trust_store`])
/// reads from process env; this seam is what the SLICE 5 lib-tests use to
/// stay hermetic on the developer's real `~/.zshrc`.
pub fn install_with_backends(
    opts: &InstallOpts,
    trust_backend: &dyn trust::TrustStore,
    home: Option<&std::path::Path>,
    shell_env: shell::EnvView<'_>,
    proxy_url: &str,
) -> Result<InstallReport> {
    // SLICE 6 (COV_10) — gate before any side effects per design §3.5 +
    // review-standards T10 ("Gemini OAuth gate is enforced before any
    // trust-store mutation … exits before issuing or installing any
    // cert"). The slice-prompt's suggested placement (between env-vars
    // build and resolve_shell, L264–L267) would already have written 4
    // PEM files and called trust_backend.add_root — that violates T10.
    // We refuse cleanly here, before generate_root_ca, so a `Ctrl-C`-
    // resistant install reaches the keychain only after the gate clears.
    // Deviation #2: placement is BEFORE CA issuance, not BEFORE
    // resolve_shell.
    install_with_preflight_env(opts, home)?;

    let out_dir = match &opts.ca_out {
        Some(dir) => {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("create --ca-out dir {}", dir.display()))?;
            dir.clone()
        }
        None => paths::ca_root_dir().context("resolve default CA root dir")?,
    };

    let root = ca::generate_root_ca().context("generate root CA")?;
    let leaf = ca::issue_leaf_cert(&root, &["localhost", "127.0.0.1", "::1"])
        .context("issue localhost leaf")?;

    let ca_pem_path = out_dir.join("root_ca.pem");
    let ca_key_path = out_dir.join("root_ca.key.pem");
    let leaf_pem_path = out_dir.join("leaf.pem");
    let leaf_key_path = out_dir.join("leaf.key.pem");

    write_secret(&ca_pem_path, root.cert_pem.as_bytes(), false)?;
    write_secret(&ca_key_path, root.key_pem.as_bytes(), true)?;
    write_secret(&leaf_pem_path, leaf.cert_pem.as_bytes(), false)?;
    write_secret(&leaf_key_path, leaf.key_pem.as_bytes(), true)?;

    let ca_fingerprint_sha256 = ca::fingerprint_hex(&root.fingerprint_sha256);

    // SLICE 2/3/4: install the CA into the OS trust store. Cfg-gated
    // backends (review-standards X1) — runtime selection happens in
    // `trust::dispatch`, not here.
    let trust_store_locations = trust_backend
        .add_root(&ca_pem_path, opts.scope)
        .context("install CA into OS trust store")?;

    // SLICE 5: emit the per-shell rc block + per-tool report.
    let ca_pem_str = ca_pem_path.to_string_lossy().to_string();
    let shell_env_vars = tools::env_vars_for_install(proxy_url, &ca_pem_str);
    let tool_reports = build_tool_reports(proxy_url, &ca_pem_str);

    let shell_rc_paths = match resolve_shell(opts, shell_env) {
        ResolvedShell::Writer { kind, rc_path } => {
            let writer = shell::dispatch_writer(kind);
            if let Some(path) = rc_path {
                writer
                    .write_rc(&path, &shell_env_vars)
                    .with_context(|| format!("emit shell rc at {}", path.display()))?;
                vec![path]
            } else {
                // Cmd detected — no file mutation; breadcrumb lives in
                // shell_env_vars / tools fields.
                Vec::new()
            }
        }
        ResolvedShell::DetectedNoHome => {
            // Detected a shell but couldn't resolve HOME; surface a
            // warning via the empty rc-paths return and let the install
            // report carry the env vars so the operator can paste them
            // manually.
            Vec::new()
        }
        ResolvedShell::None => {
            // No `--shell` override and detection returned None. Per
            // SLICE 5 contract we record the env vars in the report so
            // the operator can route them manually; we do NOT bail out
            // because the trust store install (SLICE 2-4) already
            // succeeded.
            Vec::new()
        }
    };
    let _ = home; // retained for future use; pwsh on non-Windows
                  // resolution already runs inside resolve_shell

    Ok(InstallReport {
        ca_fingerprint_sha256,
        ca_pem_path,
        ca_key_path,
        leaf_pem_path,
        leaf_key_path,
        trust_store_locations,
        shell_rc_paths,
        shell_env_vars,
        tools: tool_reports,
    })
}

/// `https://localhost:8443` — the locked default proxy listen URL from
/// design §4 + slice doc §3. Lives as a constant so the SLICE 6 / SLICE 7
/// follow-ups have one knob to flip when the operator wants a custom port.
pub const DEFAULT_PROXY_URL: &str = "https://localhost:8443";

/// SLICE 6 (COV_10): construct the preflight env from the install-call
/// home arg + process-env env vars, then run the gate. Pulled into its own
/// function so the install path stays linear. The `home` arg lets the
/// SLICE 5 hermetic-test pattern (tempdir-rooted HOME via
/// `install_with_backends(home=Some(tmp))`) flow through to the preflight
/// detector — without it, the lib-test `install_refuses_when_gemini_oauth_detected`
/// would need to mutate process-global HOME, racing other tests.
///
/// Env-var resolution still goes through `std::env::var` for
/// `GEMINI_API_KEY` / `GOOGLE_APPLICATION_CREDENTIALS` because:
///   1. The slice doc doesn't ask for env-var injection (only HOME).
///   2. Tests can use `HomeGuard`-style mutexes if they ever need to
///      assert behaviour under specific env-var states — out of scope
///      for the v1 gate which trusts the operator's shell.
fn install_with_preflight_env(
    opts: &InstallOpts,
    home: Option<&std::path::Path>,
) -> Result<(), PreflightRefusal> {
    fn leak_str(s: String) -> &'static str {
        Box::leak(s.into_boxed_str())
    }
    let gemini_api_key = std::env::var("GEMINI_API_KEY").ok().map(leak_str);
    let google_application_credentials = std::env::var("GOOGLE_APPLICATION_CREDENTIALS")
        .ok()
        .map(leak_str);
    let env = preflight::BaseEnv {
        home,
        gemini_api_key,
        google_application_credentials,
    };
    preflight::run_preflight(&env, opts.preflight_overrides())
}

/// Re-export so the install seam can propagate the refusal up via `?`
/// (`anyhow::Result` accepts `Box<dyn Error>` via `From`).
pub use preflight::PreflightRefusal;

/// Build the full 14-row `ToolReport` list from `tools::TOOL_OVERRIDES`,
/// resolving `EnvVar` rows against `(ca_pem_path, proxy_url)` and leaving
/// `ConfigFile` / `OsTrustOnly` rows with an empty `value`.
fn build_tool_reports(proxy_url: &str, ca_pem_path: &str) -> Vec<ToolReport> {
    tools::TOOL_OVERRIDES
        .iter()
        .map(|t| {
            let value = match t.kind {
                tools::OverrideKind::EnvVar if t.env_var == "SRC_HTTPS_PROXY" => {
                    proxy_url.to_string()
                }
                tools::OverrideKind::EnvVar => ca_pem_path.to_string(),
                _ => String::new(),
            };
            ToolReport {
                id: t.id.to_string(),
                display: t.display.to_string(),
                env_var: t.env_var.to_string(),
                value,
                notes: t.notes.map(|n| n.to_string()),
            }
        })
        .collect()
}

enum ResolvedShell {
    Writer {
        kind: shell::DetectedShell,
        rc_path: Option<PathBuf>,
    },
    DetectedNoHome,
    None,
}

fn resolve_shell(opts: &InstallOpts, env: shell::EnvView<'_>) -> ResolvedShell {
    let kind = match opts.shell {
        Some(explicit) => shell::DetectedShell::from(explicit),
        None => match shell::DetectedShell::detect_from_env(env) {
            Some(k) => k,
            None => return ResolvedShell::None,
        },
    };
    // HOME is needed for every rc path EXCEPT Cmd (which has none).
    if kind == shell::DetectedShell::Cmd {
        return ResolvedShell::Writer {
            kind,
            rc_path: None,
        };
    }
    let home = match std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        Some(h) => PathBuf::from(h),
        None => return ResolvedShell::DetectedNoHome,
    };
    let rc_path = kind.rc_path(&home);
    ResolvedShell::Writer { kind, rc_path }
}

/// Leak a `&str` into `'static` — only used to bridge `std::env::var(...).ok()`
/// into the `EnvView<'static>` the SLICE 5 detector takes. Bounded to the
/// number of `install` calls in a process (one).
fn leak_static(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

/// `spendguard uninstall` — SLICE 2 lands the trust-store half (macOS only).
///
/// Per design §9 (Uninstall guarantees) the full inverse is:
///   1. Remove rc markers (SLICE 5 / SLICE 7).
///   2. Clear per-tool config-file overrides (SLICE 5 / SLICE 7).
///   3. **NEW (this slice)**: Remove CA from trust store(s) by fingerprint
///      — macOS only.
///   4. Delete on-disk CA / leaf PEM and key blobs (SLICE 7).
///
/// In this slice we accept the SHA-256 fingerprint via `opts.ca_fingerprint`
/// — the caller is the operator with their install-report JSON to hand. The
/// SLICE 7 implementation will resolve the fingerprint from the on-disk PEM
/// when the operator doesn't have one handy.
pub fn uninstall(opts: &UninstallOpts) -> Result<UninstallReport> {
    let backend = trust_backend()?;
    uninstall_with_trust_store(opts, backend.as_ref())
}

/// `spendguard uninstall` with injected trust backend — see
/// [`install_with_trust_store`] for the rationale.
///
/// SLICE 5 (COV_09) addition: per slice doc §3 the rc strip MUST run
/// BEFORE the trust-store removal — that way a `Ctrl-C` mid-uninstall
/// can never leave the operator with an active `HTTPS_PROXY` export
/// pointed at a now-untrusted local proxy. Trust removal failure after
/// successful rc strip is logged via the `UninstallReport`; the caller
/// can re-run uninstall to retry the trust step.
pub fn uninstall_with_trust_store(
    opts: &UninstallOpts,
    trust_backend: &dyn trust::TrustStore,
) -> Result<UninstallReport> {
    let shell_env = shell::EnvView {
        shell: std::env::var("SHELL").ok().as_deref().map(leak_static),
        psmodulepath: std::env::var("PSModulePath")
            .ok()
            .as_deref()
            .map(leak_static),
        comspec: std::env::var("ComSpec").ok().as_deref().map(leak_static),
    };
    uninstall_with_backends(opts, trust_backend, shell_env)
}

/// `spendguard uninstall` with full backend injection — trust + shell env.
/// Used by SLICE 5/8 lib tests so the developer's `~/.zshrc` and on-disk
/// CA blobs stay untouched.
///
/// **SLICE 8 (COV_12)** wires the design §3 line 32 four-step inverse of
/// install in reverse order:
///
/// 1. `shell::strip_rc` — strip the marker block from the rc file
///    (unless `--keep-shell-rc`). Symmetric inverse of SLICE 5 install
///    rc emission; also clears the per-tool env-var overrides that
///    live INSIDE the marker block (claude_code, codex, etc).
/// 2. `trust::TrustStore::remove_root` — remove the CA from the OS
///    trust store (SLICE 2-4 backends). Idempotent per review-standards
///    `X3` — absent cert returns `Ok(vec![])`.
/// 3. Delete the four PEM files (CA + leaf + keys) from the XDG
///    `ca/` directory (unless `--keep-ca-files`). The default location
///    is resolved via [`doctor::default_ca_pem_path`] (SLICE 7 helper
///    made `pub` for SLICE 8 reuse).
///
/// **Best-effort semantics**: each step is independent. A failure in
/// step 1 doesn't block step 2; failure in step 2 doesn't block step 3.
/// Per-step failures are recorded in [`UninstallReport::warnings`] and
/// the CLI surfaces them as `EX_TEMPFAIL` (exit code 75) per
/// implementation.md §9. The function itself only `bail!`s when no step
/// could even start (e.g. the trust dispatch failed AND no fingerprint
/// could be resolved AND no PEM files exist) — at that point there is
/// no useful uninstall work to perform.
///
/// Backward compat: the legacy 3-arg signature is preserved by routing
/// the `home: None` case to the same PEM auto-resolution path. The
/// existing trust_macos integration tests at lines 306/349/379 still
/// compile because `UninstallOpts` field additions are `#[derive(Default)]`-
/// equivalent via the clap defaults.
pub fn uninstall_with_backends(
    opts: &UninstallOpts,
    trust_backend: &dyn trust::TrustStore,
    shell_env: shell::EnvView<'_>,
) -> Result<UninstallReport> {
    uninstall_with_backends_full(opts, trust_backend, None, shell_env)
}

/// `spendguard uninstall` with HOME injection for hermetic SLICE 8 tests.
/// Production callers go through [`uninstall_with_backends`]; tests that
/// need to assert "the PEM files in this tempdir were deleted" use this
/// seam so they don't race the developer's real XDG state.
pub fn uninstall_with_backends_full(
    opts: &UninstallOpts,
    trust_backend: &dyn trust::TrustStore,
    home: Option<&std::path::Path>,
    shell_env: shell::EnvView<'_>,
) -> Result<UninstallReport> {
    let mut warnings: Vec<String> = Vec::new();
    let mut removed_files: Vec<PathBuf> = Vec::new();

    // ─── Step 1: strip the shell rc marker block ──────────────────────
    // The marker block contains BOTH the global HTTPS_PROXY export AND
    // every per-tool env var override (NODE_EXTRA_CA_CERTS, CODEX_CA_CERTIFICATE,
    // etc) — stripping the block clears them all at once. ConfigFile
    // tools (Tabnine, see tools.rs OverrideKind::ConfigFile) are out of
    // scope per slice doc anti-scope ("uninstall does NOT touch
    // oauth_creds.json or any user data outside SpendGuard's own files");
    // operator removes those manually.
    if !opts.keep_shell_rc {
        match strip_shell_rc(opts, shell_env) {
            Ok(paths) => removed_files.extend(paths),
            Err(e) => warnings.push(format!("strip shell rc failed: {e:#}")),
        }
    }

    // ─── Step 2: remove the CA from the OS trust store ────────────────
    // The fingerprint comes either from `--ca-fingerprint` (explicit) or
    // from re-hashing the on-disk root_ca.pem (default). We compute the
    // PEM path EARLY so step 3 can reuse it; a missing PEM at this point
    // is non-fatal — the trust store entry might still be present from
    // a prior install whose PEM was deleted manually.
    let ca_pem_path = resolve_ca_pem_path(home);
    let fingerprint_outcome = resolve_fingerprint(opts, ca_pem_path.as_deref());

    let trust_store_locations_cleared = match &fingerprint_outcome {
        Ok(fp) => match trust_backend.remove_root(fp, opts.scope) {
            Ok(paths) => paths,
            Err(e) => {
                warnings.push(format!("remove CA from OS trust store failed: {e:#}"));
                Vec::new()
            }
        },
        Err(reason) => {
            warnings.push(format!(
                "trust-store removal skipped: {reason} \
                 (pass --ca-fingerprint to override)"
            ));
            Vec::new()
        }
    };

    // ─── Step 3: delete the four PEM files ────────────────────────────
    // Best-effort: an already-absent file is no-op success. We delete
    // the four canonical names rather than `read_dir + remove_file *`
    // to avoid wiping unrelated files an operator may have parked in
    // the `ca/` directory (e.g. backup PEMs).
    if !opts.keep_ca_files {
        if let Some(pem_path) = &ca_pem_path {
            if let Some(parent) = pem_path.parent() {
                for name in ["root_ca.pem", "root_ca.key.pem", "leaf.pem", "leaf.key.pem"] {
                    let path = parent.join(name);
                    match std::fs::remove_file(&path) {
                        Ok(()) => removed_files.push(path),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                            // Already absent — symmetric idempotency,
                            // not a warning.
                        }
                        Err(e) => {
                            warnings.push(format!("delete {} failed: {e}", path.display()));
                        }
                    }
                }
            }
        }
    }

    Ok(UninstallReport {
        removed_files,
        trust_store_locations_cleared,
        warnings,
    })
}

/// Resolve the canonical CA PEM path using the doctor helper. Falls back
/// to None when neither HOME nor XDG resolves — in that case both the
/// fingerprint re-derivation and the PEM delete skip cleanly.
fn resolve_ca_pem_path(home: Option<&std::path::Path>) -> Option<PathBuf> {
    let env = preflight::BaseEnv {
        home,
        gemini_api_key: None,
        google_application_credentials: None,
    };
    let env_with_home = if env.home.is_some() {
        env
    } else {
        preflight::BaseEnv::from_process()
    };
    doctor::default_ca_pem_path(&env_with_home)
}

/// Resolve the fingerprint either from explicit opts or by re-hashing
/// the on-disk PEM. Returns a human-readable reason string when neither
/// path produces a fingerprint so the warning surface is useful.
fn resolve_fingerprint(
    opts: &UninstallOpts,
    ca_pem_path: Option<&std::path::Path>,
) -> std::result::Result<String, String> {
    if let Some(fp) = &opts.ca_fingerprint {
        return Ok(fp.clone());
    }
    let Some(path) = ca_pem_path else {
        return Err("no --ca-fingerprint supplied and no canonical CA PEM path resolved".into());
    };
    let pem_bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!(
                "no --ca-fingerprint supplied and CA PEM at {} is absent",
                path.display()
            ));
        }
        Err(e) => return Err(format!("read CA PEM {}: {e}", path.display())),
    };
    doctor::ca_fingerprint::cert_der_sha256_hex(&pem_bytes)
        .map(|s| s.to_string())
        .map_err(|e| format!("decode CA PEM {}: {e}", path.display()))
}

fn strip_shell_rc(opts: &UninstallOpts, env: shell::EnvView<'_>) -> Result<Vec<PathBuf>> {
    // Honour `--shell` if the operator stuffed it in; otherwise detect.
    let kind = match shell::DetectedShell::detect_from_env(env) {
        Some(k) => k,
        None => return Ok(Vec::new()),
    };
    if kind == shell::DetectedShell::Cmd {
        // Symmetric with install: nothing to strip for cmd.exe.
        return Ok(Vec::new());
    }
    let _ = opts; // reserved for a future --include / --exclude tool flag.
    let home = match std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        Some(h) => PathBuf::from(h),
        None => return Ok(Vec::new()),
    };
    let Some(path) = kind.rc_path(&home) else {
        return Ok(Vec::new());
    };
    // SLICE 8 (COV_12): only report the rc path in `removed_files` when
    // the strip actually changed something. The writer's `strip_rc` is
    // already idempotent for absent files / absent marker blocks (it
    // returns Ok without rewriting), but we want a second uninstall to
    // surface an empty `removed_files` so the operator can tell "this
    // call was a no-op". We diff the file's contents around the strip.
    let before = std::fs::read_to_string(&path).ok();
    let writer = shell::dispatch_writer(kind);
    writer
        .strip_rc(&path)
        .with_context(|| format!("strip shell rc at {}", path.display()))?;
    let after = std::fs::read_to_string(&path).ok();
    if before == after {
        // No change → don't list the path (SLICE 8 idempotency).
        Ok(Vec::new())
    } else {
        Ok(vec![path])
    }
}

/// `spendguard doctor` — SLICE 2 reports whether the CA is trusted in the
/// configured keychain.
///
/// SLICE 7 expands this to also probe `HTTPS_PROXY` and to drive a TLS
/// handshake; for now we provide the deterministic, no-network half: was
/// the CA fingerprint successfully landed in the trust store?
pub fn doctor(opts: &DoctorOpts) -> Result<DoctorReport> {
    let backend = trust_backend()?;
    doctor_with_trust_store(opts, backend.as_ref())
}

/// `spendguard doctor` with injected trust backend.
pub fn doctor_with_trust_store(
    opts: &DoctorOpts,
    trust_backend: &dyn trust::TrustStore,
) -> Result<DoctorReport> {
    let mut warnings = Vec::new();
    let ca_present_in_store = match &opts.ca_fingerprint {
        Some(fp) => trust_backend
            .verify_installed(fp, opts.scope)
            .unwrap_or_else(|e| {
                warnings.push(format!("trust-store probe failed: {e:#}"));
                false
            }),
        None => {
            warnings.push(
                "doctor: --ca-fingerprint not supplied; trust-store probe skipped".to_string(),
            );
            false
        }
    };

    Ok(DoctorReport {
        ca_present_in_store,
        https_proxy_set: std::env::var("HTTPS_PROXY").ok(),
        round_trip_ok: false, // SLICE 7
        warnings,
    })
}

/// Resolve the OS trust-store backend for the production entry points.
/// Pulled out so the `#[cfg]` gate lives in one place — see
/// [`trust::dispatch`] for the per-OS routing.
fn trust_backend() -> Result<Box<dyn trust::TrustStore>> {
    trust::dispatch()
}

/// Write a PEM blob to disk. On POSIX, `secret=true` clamps mode to `0o600`
/// (review-standards.md `T2`); on Windows the OS handles ACLs and DPAPI
/// sealing is added in SLICE 4.
///
/// R2 fix per security review (B1): on POSIX the write is staged through a
/// `.tmp-<uuid>` sibling created with the target mode and *then* atomically
/// renamed over the destination. This:
///   1. Hard-clamps mode at create-time AND verifies it via
///      `set_permissions` immediately before rename, so even if the
///      pre-existing destination file is `0o644` / `0o666` from a prior
///      botched install it is replaced (inode-swap) with one at `0o600`.
///   2. Closes the partial-write window — a crash mid-`write_all` leaves the
///      old file intact, never a half-written secret at the final path.
fn write_secret(path: &std::path::Path, bytes: &[u8], secret: bool) -> Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

        let mode = if secret { 0o600 } else { 0o644 };
        let tmp_path = tmp_sibling_for(path);

        let mut opts = std::fs::OpenOptions::new();
        opts.create_new(true).write(true).mode(mode);
        let mut f = opts
            .open(&tmp_path)
            .with_context(|| format!("open temp {} for atomic write", tmp_path.display()))?;
        f.write_all(bytes)
            .with_context(|| format!("write temp {}", tmp_path.display()))?;
        f.sync_all()
            .with_context(|| format!("sync temp {}", tmp_path.display()))?;
        // Hard-clamp again: defends against umask, ACL inheritance, or a
        // hostile pre-existing file (B1). `set_permissions` is the only call
        // documented to apply mode unconditionally on the inode.
        std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(mode))
            .with_context(|| format!("clamp mode on {}", tmp_path.display()))?;
        // Atomic rename: replaces dest inode, dropping any prior mode bits.
        if let Err(e) = std::fs::rename(&tmp_path, path) {
            // Best-effort cleanup so we don't leak the temp file on rename
            // failure. The original `Err` is what the caller sees.
            let _ = std::fs::remove_file(&tmp_path);
            return Err(e)
                .with_context(|| format!("rename {} -> {}", tmp_path.display(), path.display()));
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = secret;
        std::fs::write(path, bytes).with_context(|| format!("write {}", path.display()))?;
        Ok(())
    }
}

/// Build a unique sibling temp path for atomic rename. Lives next to the
/// target so the rename is guaranteed to be on the same filesystem
/// (`std::fs::rename` is only atomic intra-filesystem).
#[cfg(unix)]
fn tmp_sibling_for(path: &std::path::Path) -> std::path::PathBuf {
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "secret".to_string());
    // Process id + uuid keeps the name unique across concurrent installers
    // sharing a directory; uuid alone would be sufficient but the pid aids
    // debugging.
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let tmp_name = format!(".{file_name}.tmp-{}-{suffix}", std::process::id());
    let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    parent.join(tmp_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::Mutex;

    /// Inert trust store used by lib-tests that exercise the install path
    /// without touching the developer's real keychain. SLICE 2 (COV_06) added
    /// this for the `install_with_trust_store` shim so unit tests can stay
    /// hermetic on macOS hosts — the real keychain integration test lives in
    /// `services/cli/tests/trust_macos.rs` and is `#[ignore]`-gated.
    #[derive(Debug, Default)]
    struct NoopTrustStore {
        added: Mutex<Vec<PathBuf>>,
        removed: Mutex<Vec<String>>,
    }

    impl trust::TrustStore for NoopTrustStore {
        fn add_root(&self, ca_pem_path: &Path, _scope: TrustScope) -> Result<Vec<PathBuf>> {
            self.added.lock().unwrap().push(ca_pem_path.to_path_buf());
            Ok(vec![PathBuf::from("/tmp/noop-trust-store")])
        }
        fn remove_root(
            &self,
            fingerprint_sha256_hex: &str,
            _scope: TrustScope,
        ) -> Result<Vec<PathBuf>> {
            self.removed
                .lock()
                .unwrap()
                .push(fingerprint_sha256_hex.to_string());
            Ok(vec![PathBuf::from("/tmp/noop-trust-store")])
        }
        fn verify_installed(
            &self,
            _fingerprint_sha256_hex: &str,
            _scope: TrustScope,
        ) -> Result<bool> {
            Ok(true)
        }
    }

    /// End-to-end: `install_with_backends --ca-out <tmp>` writes 4 PEM
    /// files, all parseable. Uses NoopTrustStore + empty EnvView so the
    /// developer's real keychain / rc files stay untouched (SLICE 5
    /// hermetic-test pattern).
    #[test]
    fn install_writes_four_pem_files() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let opts = InstallOpts {
            scope: TrustScope::User,
            ca_out: Some(tmp.path().to_path_buf()),
            shell: None,
            force_allow_gemini_oauth: false,
        };
        let backend = NoopTrustStore::default();
        let report = install_with_backends(
            &opts,
            &backend,
            None,
            shell::EnvView::default(),
            DEFAULT_PROXY_URL,
        )
        .expect("install");

        for p in [
            &report.ca_pem_path,
            &report.ca_key_path,
            &report.leaf_pem_path,
            &report.leaf_key_path,
        ] {
            assert!(p.exists(), "{p:?} not written");
            let content = std::fs::read_to_string(p).expect("read");
            assert!(content.contains("BEGIN"), "{p:?} not PEM");
        }

        assert_eq!(
            report.ca_fingerprint_sha256.len(),
            64,
            "sha256 hex must be 64 chars, got {}",
            report.ca_fingerprint_sha256
        );
        // SLICE 2: trust_store_locations is populated by the (noop here)
        // backend's add_root return value.
        assert_eq!(
            report.trust_store_locations,
            vec![PathBuf::from("/tmp/noop-trust-store")],
            "install_with_backends should surface the backend's return value"
        );
        // Empty EnvView → no shell detected → no rc written.
        assert!(report.shell_rc_paths.is_empty());
        // SLICE 5: tools + shell_env_vars are populated regardless of
        // whether a shell was detected (operator can read the JSON).
        assert!(
            !report.shell_env_vars.is_empty(),
            "shell_env_vars should carry HTTPS_PROXY + overrides"
        );
        assert_eq!(report.tools.len(), 14, "14-tool matrix in install report");
    }

    /// SLICE 2: `install_with_trust_store` calls `add_root` exactly once with
    /// the CA PEM path it wrote.
    #[test]
    fn install_invokes_trust_backend_add_root() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let opts = InstallOpts {
            scope: TrustScope::User,
            ca_out: Some(tmp.path().to_path_buf()),
            shell: None,
            force_allow_gemini_oauth: false,
        };
        let backend = NoopTrustStore::default();
        let report = install_with_backends(
            &opts,
            &backend,
            None,
            shell::EnvView::default(),
            DEFAULT_PROXY_URL,
        )
        .expect("install");

        let added = backend.added.lock().unwrap().clone();
        assert_eq!(added.len(), 1, "add_root called exactly once");
        assert_eq!(added[0], report.ca_pem_path);
    }

    /// SLICE 8 (COV_12): `uninstall` no longer hard-requires
    /// `--ca-fingerprint` — when omitted and the on-disk PEM is also
    /// absent the trust-store step is skipped with a warning rather
    /// than erroring. This preserves the partial-cleanup semantics:
    /// the rc strip + PEM delete still happen even when we can't probe
    /// the trust store. The SLICE 2 hard-bail behaviour was replaced
    /// here as part of D02 deliverable closure.
    #[test]
    fn uninstall_without_fingerprint_skips_trust_step_with_warning() {
        let backend = NoopTrustStore::default();
        // Use a HOME that resolves but has no PEM — auto-resolution
        // discovers "PEM absent" and surfaces a warning.
        let tmp = tempfile::tempdir().expect("tempdir");
        let opts = UninstallOpts {
            scope: TrustScope::User,
            ca_fingerprint: None,
            keep_shell_rc: false,
            keep_ca_files: false,
        };
        let report = uninstall_with_backends_full(
            &opts,
            &backend,
            Some(tmp.path()),
            shell::EnvView::default(),
        )
        .expect("uninstall must not bail on missing fingerprint in SLICE 8");
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.contains("trust-store removal skipped")),
            "missing fingerprint must surface trust-store-skipped warning, \
             got: {warnings:?}",
            warnings = report.warnings,
        );
        assert!(
            report.trust_store_locations_cleared.is_empty(),
            "trust step did not run, so no locations cleared"
        );
    }

    /// SLICE 2: `uninstall_with_backends` forwards the fingerprint to the
    /// backend and surfaces the cleared locations. SLICE 5 hermetic-test
    /// variant uses empty EnvView so the developer's `~/.zshrc` is not
    /// touched.
    #[test]
    fn uninstall_invokes_backend_remove_root() {
        let backend = NoopTrustStore::default();
        let opts = UninstallOpts {
            scope: TrustScope::User,
            ca_fingerprint: Some(
                "0000000000000000000000000000000000000000000000000000000000000000".into(),
            ),
            keep_shell_rc: false,
            keep_ca_files: false,
        };
        let report =
            uninstall_with_backends(&opts, &backend, shell::EnvView::default()).expect("uninstall");
        assert_eq!(
            report.trust_store_locations_cleared,
            vec![PathBuf::from("/tmp/noop-trust-store")]
        );
        let removed = backend.removed.lock().unwrap().clone();
        assert_eq!(removed.len(), 1);
        assert_eq!(
            removed[0],
            "0000000000000000000000000000000000000000000000000000000000000000"
        );
    }

    /// SLICE 2: `doctor_with_trust_store` reports the trust-store probe
    /// result and surfaces a warning when no fingerprint is supplied.
    #[test]
    fn doctor_without_fingerprint_warns_and_reports_false() {
        let backend = NoopTrustStore::default();
        let opts = DoctorOpts {
            scope: TrustScope::User,
            ca_fingerprint: None,
        };
        let report = doctor_with_trust_store(&opts, &backend).expect("doctor");
        assert!(!report.ca_present_in_store);
        assert!(report
            .warnings
            .iter()
            .any(|w| w.contains("ca-fingerprint not supplied")));
    }

    #[test]
    fn doctor_with_fingerprint_returns_backend_probe() {
        let backend = NoopTrustStore::default();
        let opts = DoctorOpts {
            scope: TrustScope::User,
            ca_fingerprint: Some(
                "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef".into(),
            ),
        };
        let report = doctor_with_trust_store(&opts, &backend).expect("doctor");
        assert!(
            report.ca_present_in_store,
            "NoopTrustStore.verify_installed returns true"
        );
        assert!(report
            .warnings
            .iter()
            .all(|w| !w.contains("ca-fingerprint not supplied")));
    }

    /// `T2` — private key files are mode 0o600.
    #[cfg(unix)]
    #[test]
    fn private_key_files_are_mode_0o600() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().expect("tempdir");
        let opts = InstallOpts {
            scope: TrustScope::User,
            ca_out: Some(tmp.path().to_path_buf()),
            shell: None,
            force_allow_gemini_oauth: false,
        };
        let backend = NoopTrustStore::default();
        let report = install_with_backends(
            &opts,
            &backend,
            None,
            shell::EnvView::default(),
            DEFAULT_PROXY_URL,
        )
        .expect("install");

        for key_path in [&report.ca_key_path, &report.leaf_key_path] {
            let meta = std::fs::metadata(key_path).expect("metadata");
            let mode = meta.permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{key_path:?} mode is {mode:o}, expected 0600");
        }
    }

    /// R2 regression for B1 — pre-existing key files at world-readable modes
    /// MUST be replaced with 0o600 across a re-install. The original failure
    /// mode was `OpenOptions::mode()` only applying at create-time, so a
    /// truncate-and-rewrite kept the old 0o644 / 0o666 mode bits.
    #[cfg(unix)]
    #[test]
    fn reinstall_clamps_world_readable_key_files_to_0o600() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().expect("tempdir");
        let ca_key = tmp.path().join("root_ca.key.pem");
        let leaf_key = tmp.path().join("leaf.key.pem");

        // Pre-seed both key paths at world-readable modes — the bug under
        // test is that the rewrite preserved these bits.
        std::fs::write(&ca_key, b"stale-root-key-from-prior-botched-install")
            .expect("seed stale ca key");
        std::fs::write(&leaf_key, b"stale-leaf-key-from-prior-botched-install")
            .expect("seed stale leaf key");
        std::fs::set_permissions(&ca_key, std::fs::Permissions::from_mode(0o644))
            .expect("seed ca key at 0o644");
        std::fs::set_permissions(&leaf_key, std::fs::Permissions::from_mode(0o666))
            .expect("seed leaf key at 0o666");

        let opts = InstallOpts {
            scope: TrustScope::User,
            ca_out: Some(tmp.path().to_path_buf()),
            shell: None,
            force_allow_gemini_oauth: false,
        };
        let backend = NoopTrustStore::default();
        let report = install_with_backends(
            &opts,
            &backend,
            None,
            shell::EnvView::default(),
            DEFAULT_PROXY_URL,
        )
        .expect("install over pre-existing key files");

        for key_path in [&report.ca_key_path, &report.leaf_key_path] {
            let meta = std::fs::metadata(key_path).expect("metadata");
            let mode = meta.permissions().mode() & 0o777;
            assert_eq!(
                mode, 0o600,
                "{key_path:?} re-install left mode {mode:o}, expected 0600"
            );
            // Sanity: content was replaced, not just permissioned.
            let content = std::fs::read_to_string(key_path).expect("read");
            assert!(
                content.contains("BEGIN") && content.contains("PRIVATE KEY"),
                "{key_path:?} content should be the freshly issued PEM"
            );
        }

        // The temp-sibling pattern leaves no residue on the happy path.
        let residue: Vec<_> = std::fs::read_dir(tmp.path())
            .expect("readdir")
            .filter_map(|e| e.ok())
            .filter(|e| {
                let name = e.file_name();
                let s = name.to_string_lossy();
                s.starts_with('.') && s.contains(".tmp-")
            })
            .collect();
        assert!(
            residue.is_empty(),
            "atomic-rename temp files leaked: {residue:?}"
        );
    }

    // ──────────── SLICE 5 (COV_09) install / uninstall integration ───────

    /// SLICE 5: `install_with_backends` with a `bash` EnvView writes the
    /// expected rc block into the tempdir-rooted ~/.bashrc. The developer's
    /// real home is never touched because we override HOME via a
    /// tempdir-set env var ONLY for the duration of this test (single
    /// threaded by virtue of `#[serial]`-style tempdir scoping — we run
    /// it as a stand-alone process via tempdir + atomic env).
    ///
    /// Note: we cannot mutate process-global HOME from a parallel test
    /// without racing (Rust 2024 unsafety, paths.rs B3 fix). Instead the
    /// test uses the explicit `home` arg already plumbed through
    /// `install_with_backends`, which is the entire reason that seam
    /// exists. The arg defaults to None so the production callers still
    /// resolve from process env.
    #[test]
    fn slice5_install_writes_rc_block_under_tempdir_home() {
        // Use TempDirs that survive the test's lifetime — one for HOME,
        // one for --ca-out.
        let home_tmp = tempfile::tempdir().expect("home tempdir");
        let ca_tmp = tempfile::tempdir().expect("ca tempdir");
        let opts = InstallOpts {
            scope: TrustScope::User,
            ca_out: Some(ca_tmp.path().to_path_buf()),
            // `--shell bash` makes the test deterministic regardless of
            // what `$SHELL` happens to be on the runner.
            shell: Some(ShellKind::Bash),
            force_allow_gemini_oauth: false,
        };
        let backend = NoopTrustStore::default();
        let env = shell::EnvView {
            shell: Some("/bin/bash"),
            psmodulepath: None,
            comspec: None,
        };

        // Run install with explicit HOME override. We use the env var
        // route because `install_with_backends` resolves HOME from
        // process env (so the production `install` shim Just Works) —
        // the test gates the env var via a setter helper that locks the
        // process for the duration.
        let _guard = HomeGuard::set(home_tmp.path());
        let report = install_with_backends(
            &opts,
            &backend,
            Some(home_tmp.path()),
            env,
            DEFAULT_PROXY_URL,
        )
        .expect("install");

        let bashrc = home_tmp.path().join(".bashrc");
        assert!(
            bashrc.exists(),
            "~/.bashrc must be written under tempdir HOME"
        );
        let content = std::fs::read_to_string(&bashrc).expect("read");
        assert!(content.contains("# >>> spendguard"));
        assert!(content.contains(r#"export HTTPS_PROXY="https://localhost:8443""#));
        assert_eq!(
            report.shell_rc_paths,
            vec![bashrc.clone()],
            "shell_rc_paths surfaces written rc"
        );
        assert_eq!(report.tools.len(), 14);
    }

    /// SLICE 5: install + uninstall round-trip — strip_rc removes the
    /// block, leaves the surrounding file intact.
    #[test]
    fn slice5_uninstall_strips_rc_block_round_trip() {
        let home_tmp = tempfile::tempdir().expect("home tempdir");
        let ca_tmp = tempfile::tempdir().expect("ca tempdir");
        let backend = NoopTrustStore::default();

        // Pre-seed ~/.zshrc with unrelated content the user typed.
        let zshrc = home_tmp.path().join(".zshrc");
        std::fs::write(&zshrc, "alias g='git'\n").expect("seed");

        let opts = InstallOpts {
            scope: TrustScope::User,
            ca_out: Some(ca_tmp.path().to_path_buf()),
            shell: Some(ShellKind::Zsh),
            force_allow_gemini_oauth: false,
        };
        let env = shell::EnvView {
            shell: Some("/bin/zsh"),
            ..Default::default()
        };
        let _guard = HomeGuard::set(home_tmp.path());
        let install_report = install_with_backends(
            &opts,
            &backend,
            Some(home_tmp.path()),
            env,
            DEFAULT_PROXY_URL,
        )
        .expect("install");

        // Block landed.
        let after_install = std::fs::read_to_string(&zshrc).expect("read");
        assert!(after_install.contains("HTTPS_PROXY"));
        assert!(after_install.contains("alias g='git'"));

        // Now uninstall.
        let uopts = UninstallOpts {
            scope: TrustScope::User,
            ca_fingerprint: Some(install_report.ca_fingerprint_sha256.clone()),
            keep_shell_rc: false,
            // Keep the SLICE 5 test focused on rc-strip semantics — the
            // ca_out tempdir is NOT the canonical XDG path so SLICE 8's
            // PEM deletion would no-op anyway. Setting keep_ca_files
            // makes the intent explicit.
            keep_ca_files: true,
        };
        let report = uninstall_with_backends(&uopts, &backend, env).expect("uninstall");

        let after_uninstall = std::fs::read_to_string(&zshrc).expect("read");
        assert!(!after_uninstall.contains("HTTPS_PROXY"));
        assert!(after_uninstall.contains("alias g='git'"));
        assert_eq!(report.removed_files, vec![zshrc]);
    }

    /// SLICE 5: install with `Cmd` detection leaves `shell_rc_paths` empty
    /// but still populates `shell_env_vars` (operator runs `setx` from
    /// the breadcrumb).
    #[test]
    fn slice5_install_cmd_detection_emits_no_rc_but_carries_breadcrumb_vars() {
        let ca_tmp = tempfile::tempdir().expect("ca tempdir");
        let backend = NoopTrustStore::default();
        let opts = InstallOpts {
            scope: TrustScope::User,
            ca_out: Some(ca_tmp.path().to_path_buf()),
            shell: None,
            force_allow_gemini_oauth: false,
        };
        let env = shell::EnvView {
            shell: Some("cmd.exe"),
            psmodulepath: None,
            comspec: None,
        };
        let report =
            install_with_backends(&opts, &backend, None, env, DEFAULT_PROXY_URL).expect("install");

        assert!(
            report.shell_rc_paths.is_empty(),
            "cmd detection MUST NOT write a file"
        );
        assert!(
            !report.shell_env_vars.is_empty(),
            "shell_env_vars must carry the setx breadcrumb"
        );
    }

    // ──────────── SLICE 6 (COV_10) Gemini OAuth refusal preflight ────────

    /// Helper for the SLICE 6 tests: seed `~/.gemini/oauth_creds.json` in
    /// the given home and return the tempdir for the caller to keep alive.
    fn seed_gemini_oauth_creds(home: &std::path::Path) {
        std::fs::create_dir_all(home.join(".gemini")).expect("create .gemini dir");
        std::fs::write(
            home.join(".gemini").join("oauth_creds.json"),
            br#"{"access_token":"fake","refresh_token":"fake"}"#,
        )
        .expect("seed oauth_creds.json");
    }

    /// SLICE 6: `install_with_backends` with Gemini OAuth state present
    /// AND no `--force-allow-gemini-oauth` flag must:
    ///   1. Return `Err` carrying the `PreflightRefusal::GeminiOauthFreetier`
    ///      message.
    ///   2. NOT write any of the 4 PEM files (CA issuance never ran).
    ///   3. NOT call `add_root` on the trust backend.
    ///   4. NOT write a shell rc block.
    ///
    /// This is the threat-model `T10` regression: the gate runs BEFORE any
    /// trust-store mutation.
    #[test]
    fn slice6_install_refuses_when_gemini_oauth_detected_without_force_flag() {
        let home_tmp = tempfile::tempdir().expect("home tempdir");
        let ca_tmp = tempfile::tempdir().expect("ca tempdir");

        seed_gemini_oauth_creds(home_tmp.path());

        let opts = InstallOpts {
            scope: TrustScope::User,
            ca_out: Some(ca_tmp.path().to_path_buf()),
            shell: Some(ShellKind::Bash),
            force_allow_gemini_oauth: false,
        };
        let backend = NoopTrustStore::default();
        let env = shell::EnvView {
            shell: Some("/bin/bash"),
            ..Default::default()
        };

        // Lock process env so GEMINI_API_KEY / GOOGLE_APPLICATION_CREDENTIALS
        // are reproducibly unset for the duration. The HomeGuard serialises
        // against the other slice5_/slice6_ tests in this binary; we use
        // it here too so the preflight env-var probes are deterministic
        // (a developer with GEMINI_API_KEY set in their shell would
        // otherwise see ApiKeyMode and the test would never refuse).
        let _guard = HomeGuard::set(home_tmp.path());
        let _api_guard = EnvVarGuard::unset("GEMINI_API_KEY");
        let _sa_guard = EnvVarGuard::unset("GOOGLE_APPLICATION_CREDENTIALS");

        let err = install_with_backends(
            &opts,
            &backend,
            Some(home_tmp.path()),
            env,
            DEFAULT_PROXY_URL,
        )
        .expect_err("install MUST refuse");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("Refusing to install"),
            "refusal message must surface in anyhow chain, got: {msg}"
        );
        assert!(
            msg.contains("--force-allow-gemini-oauth"),
            "refusal message must surface the override flag, got: {msg}"
        );

        // No CA file written — issuance never ran.
        let pem_files: Vec<_> = std::fs::read_dir(ca_tmp.path())
            .expect("read ca_tmp")
            .filter_map(|e| e.ok())
            .collect();
        assert!(
            pem_files.is_empty(),
            "no PEM files should have been written; found: {pem_files:?}"
        );

        // Trust backend `add_root` never called.
        let added = backend.added.lock().unwrap().clone();
        assert!(
            added.is_empty(),
            "add_root MUST NOT have been called; got: {added:?}"
        );

        // Shell rc never written — `.bashrc` in tempdir HOME is absent.
        let bashrc = home_tmp.path().join(".bashrc");
        assert!(
            !bashrc.exists(),
            "shell rc MUST NOT have been written; found: {bashrc:?}"
        );
    }

    /// SLICE 6: `install_with_backends` with Gemini OAuth state present
    /// AND `--force-allow-gemini-oauth=true` proceeds with the full
    /// install (CA + trust + rc). Operator has accepted the consequences.
    #[test]
    fn slice6_install_proceeds_when_force_allow_gemini_oauth_set() {
        let home_tmp = tempfile::tempdir().expect("home tempdir");
        let ca_tmp = tempfile::tempdir().expect("ca tempdir");

        seed_gemini_oauth_creds(home_tmp.path());

        let opts = InstallOpts {
            scope: TrustScope::User,
            ca_out: Some(ca_tmp.path().to_path_buf()),
            shell: Some(ShellKind::Bash),
            force_allow_gemini_oauth: true,
        };
        let backend = NoopTrustStore::default();
        let env = shell::EnvView {
            shell: Some("/bin/bash"),
            ..Default::default()
        };

        let _guard = HomeGuard::set(home_tmp.path());
        // Even with vars set, the gate is bypassed; we unset them so the
        // test asserts purely the bypass behaviour.
        let _api_guard = EnvVarGuard::unset("GEMINI_API_KEY");
        let _sa_guard = EnvVarGuard::unset("GOOGLE_APPLICATION_CREDENTIALS");

        let report = install_with_backends(
            &opts,
            &backend,
            Some(home_tmp.path()),
            env,
            DEFAULT_PROXY_URL,
        )
        .expect("install with force flag must proceed");

        // CA was issued + written.
        assert!(report.ca_pem_path.exists(), "CA PEM must be written");
        assert!(report.ca_key_path.exists(), "CA key must be written");

        // Trust backend `add_root` was called.
        let added = backend.added.lock().unwrap().clone();
        assert_eq!(added.len(), 1, "add_root must have been called once");

        // Shell rc was written.
        let bashrc = home_tmp.path().join(".bashrc");
        assert!(bashrc.exists(), ".bashrc must be written");
        let content = std::fs::read_to_string(&bashrc).expect("read .bashrc");
        assert!(content.contains("HTTPS_PROXY"));
    }

    /// SLICE 6: `uninstall_with_backends` bypasses preflight entirely.
    /// Per slice doc anti-scope: "uninstall is always safe to run
    /// regardless of Gemini state (the user is undoing the install)".
    /// We assert this by seeding the OAuth refusal condition and
    /// confirming uninstall succeeds without inspecting Gemini.
    #[test]
    fn slice6_uninstall_bypasses_preflight_under_gemini_oauth_state() {
        let home_tmp = tempfile::tempdir().expect("home tempdir");

        seed_gemini_oauth_creds(home_tmp.path());

        // Pre-seed a shell rc with a SpendGuard marker block so strip_rc
        // has something to remove. Use the literal markers from
        // `shell::MARKER_BEGIN` / `MARKER_END` so strip_rc actually
        // matches.
        let bashrc = home_tmp.path().join(".bashrc");
        std::fs::write(
            &bashrc,
            format!(
                "alias g='git'\n{}\nexport HTTPS_PROXY=\"https://localhost:8443\"\n{}\n",
                shell::MARKER_BEGIN,
                shell::MARKER_END,
            ),
        )
        .expect("seed bashrc");

        let backend = NoopTrustStore::default();
        let uopts = UninstallOpts {
            scope: TrustScope::User,
            ca_fingerprint: Some(
                "0000000000000000000000000000000000000000000000000000000000000000".into(),
            ),
            keep_shell_rc: false,
            // SLICE 6 bypass-preflight semantics — leave PEM step out so
            // the assertion stays focused on "uninstall ran the rc strip
            // + trust removal under Gemini OAuth state".
            keep_ca_files: true,
        };
        let env = shell::EnvView {
            shell: Some("/bin/bash"),
            ..Default::default()
        };

        let _guard = HomeGuard::set(home_tmp.path());
        let _api_guard = EnvVarGuard::unset("GEMINI_API_KEY");
        let _sa_guard = EnvVarGuard::unset("GOOGLE_APPLICATION_CREDENTIALS");

        let report =
            uninstall_with_backends(&uopts, &backend, env).expect("uninstall must succeed");

        // Trust backend `remove_root` was called — uninstall ran fully.
        let removed = backend.removed.lock().unwrap().clone();
        assert_eq!(removed.len(), 1, "remove_root must have been called once");

        // The marker block was stripped from .bashrc.
        let after = std::fs::read_to_string(&bashrc).expect("read");
        assert!(
            !after.contains("HTTPS_PROXY"),
            "marker block must be stripped"
        );
        assert!(
            after.contains("alias g='git'"),
            "non-marker lines must be preserved"
        );
        assert_eq!(report.removed_files, vec![bashrc]);
    }

    // ──────────── SLICE 8 (COV_12) symmetric uninstall — lib tests ───────

    /// SLICE 8: uninstall with `--keep-shell-rc` skips the rc strip even
    /// when an rc block is present. Hermetic: no real-shell sourcing,
    /// just the strip-or-skip decision logic.
    #[test]
    fn slice8_keep_shell_rc_skips_strip() {
        let home_tmp = tempfile::tempdir().expect("home tempdir");
        let backend = NoopTrustStore::default();

        // Pre-seed an rc with a marker block.
        let bashrc = home_tmp.path().join(".bashrc");
        std::fs::write(
            &bashrc,
            format!(
                "alias x='echo hi'\n{}\nexport HTTPS_PROXY=\"https://localhost:8443\"\n{}\n",
                shell::MARKER_BEGIN,
                shell::MARKER_END,
            ),
        )
        .expect("seed");

        let opts = UninstallOpts {
            scope: TrustScope::User,
            ca_fingerprint: Some(
                "0000000000000000000000000000000000000000000000000000000000000000".into(),
            ),
            keep_shell_rc: true,
            keep_ca_files: true,
        };
        let env = shell::EnvView {
            shell: Some("/bin/bash"),
            ..Default::default()
        };
        let _guard = HomeGuard::set(home_tmp.path());

        let report = uninstall_with_backends(&opts, &backend, env).expect("uninstall must succeed");

        // Marker block preserved.
        let after = std::fs::read_to_string(&bashrc).expect("read");
        assert!(
            after.contains(shell::MARKER_BEGIN),
            "--keep-shell-rc must preserve marker block"
        );
        assert!(
            !report.removed_files.iter().any(|p| p == &bashrc),
            "rc strip step did not run; removed_files should not list it"
        );
        // Trust step still ran.
        assert_eq!(backend.removed.lock().unwrap().len(), 1);
    }

    /// SLICE 8: uninstall with `--keep-ca-files` skips the 4-PEM delete
    /// even when canonical PEMs exist. Uses the hermetic `home: Some(...)`
    /// seam on `uninstall_with_backends_full` so we never touch the real
    /// XDG path.
    #[test]
    fn slice8_keep_ca_files_skips_pem_delete() {
        let home_tmp = tempfile::tempdir().expect("home tempdir");
        let backend = NoopTrustStore::default();

        // Pre-seed canonical-path PEMs under the tempdir HOME so the
        // would-be delete step has something to find.
        let ca_dir = canonical_ca_dir_under_for_test(home_tmp.path());
        std::fs::create_dir_all(&ca_dir).expect("mkdir ca dir");
        let pem_path = ca_dir.join("root_ca.pem");
        std::fs::write(
            &pem_path,
            b"-----BEGIN CERTIFICATE-----\nstub\n-----END CERTIFICATE-----\n",
        )
        .expect("seed pem");
        let key_path = ca_dir.join("root_ca.key.pem");
        std::fs::write(
            &key_path,
            b"-----BEGIN PRIVATE KEY-----\nstub\n-----END PRIVATE KEY-----\n",
        )
        .expect("seed key");

        let opts = UninstallOpts {
            scope: TrustScope::User,
            // Skip auto-resolve since the stub PEM has no real DER body
            // to hash; supply explicit fingerprint so the trust step runs.
            ca_fingerprint: Some(
                "0000000000000000000000000000000000000000000000000000000000000000".into(),
            ),
            keep_shell_rc: true,
            keep_ca_files: true,
        };
        let report = uninstall_with_backends_full(
            &opts,
            &backend,
            Some(home_tmp.path()),
            shell::EnvView::default(),
        )
        .expect("uninstall must succeed");

        // PEMs preserved.
        assert!(
            pem_path.exists(),
            "--keep-ca-files must preserve root_ca.pem"
        );
        assert!(
            key_path.exists(),
            "--keep-ca-files must preserve root_ca.key.pem"
        );
        assert!(
            report.warnings.is_empty(),
            "no warnings expected when both keep flags set with explicit fp"
        );
    }

    /// SLICE 8: uninstall auto-resolves the fingerprint from the on-disk
    /// PEM when `--ca-fingerprint` is omitted. We pre-seed a real PEM
    /// generated by ca::generate_root_ca so the SHA-256 over the DER body
    /// is well-defined.
    #[test]
    fn slice8_uninstall_auto_resolves_fingerprint_from_pem() {
        let home_tmp = tempfile::tempdir().expect("home tempdir");
        let ca_dir = canonical_ca_dir_under_for_test(home_tmp.path());
        std::fs::create_dir_all(&ca_dir).expect("mkdir ca dir");

        // Issue a real CA so the PEM hash matches the install-time
        // fingerprint shape.
        let root = ca::generate_root_ca().expect("gen root ca");
        let pem_path = ca_dir.join("root_ca.pem");
        std::fs::write(&pem_path, root.cert_pem.as_bytes()).expect("write pem");
        let expected_fp = ca::fingerprint_hex(&root.fingerprint_sha256);

        let backend = NoopTrustStore::default();
        let opts = UninstallOpts {
            scope: TrustScope::User,
            ca_fingerprint: None, // Auto-resolve.
            keep_shell_rc: true,  // Keep this test focused on fp resolution.
            keep_ca_files: false,
        };
        let report = uninstall_with_backends_full(
            &opts,
            &backend,
            Some(home_tmp.path()),
            shell::EnvView::default(),
        )
        .expect("uninstall must succeed");

        // Trust step ran with the auto-resolved fingerprint.
        let removed = backend.removed.lock().unwrap().clone();
        assert_eq!(
            removed.len(),
            1,
            "trust step should run with auto-resolved fp"
        );
        assert_eq!(
            removed[0], expected_fp,
            "auto-resolved fp must match install-time fingerprint"
        );
        // PEM was deleted (keep_ca_files=false).
        assert!(!pem_path.exists(), "PEM should be deleted after uninstall");
        assert!(
            report.warnings.is_empty(),
            "happy path produces no warnings"
        );
    }

    /// SLICE 8: when an absent canonical PEM AND no `--ca-fingerprint`
    /// combine, the trust step is skipped with a warning rather than
    /// erroring. The other steps still run (PEM cleanup is no-op for
    /// absent files; rc strip runs against the shell env).
    #[test]
    fn slice8_uninstall_warns_when_neither_fp_nor_pem_available() {
        let home_tmp = tempfile::tempdir().expect("home tempdir");
        let backend = NoopTrustStore::default();
        let opts = UninstallOpts {
            scope: TrustScope::User,
            ca_fingerprint: None,
            keep_shell_rc: true,
            keep_ca_files: false,
        };
        let report = uninstall_with_backends_full(
            &opts,
            &backend,
            Some(home_tmp.path()),
            shell::EnvView::default(),
        )
        .expect("uninstall returns Ok with warnings");

        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.contains("trust-store removal skipped")),
            "missing fp + missing PEM must surface a warning, got: {warnings:?}",
            warnings = report.warnings,
        );
        assert!(
            backend.removed.lock().unwrap().is_empty(),
            "trust backend remove_root must not be called"
        );
    }

    /// SLICE 8: PEM delete tolerates already-absent files (idempotent).
    /// We seed only `root_ca.pem` (no key, no leaf) and verify the
    /// uninstall succeeds without warnings.
    #[test]
    fn slice8_pem_delete_tolerates_partial_pem_set() {
        let home_tmp = tempfile::tempdir().expect("home tempdir");
        let ca_dir = canonical_ca_dir_under_for_test(home_tmp.path());
        std::fs::create_dir_all(&ca_dir).expect("mkdir ca dir");

        // Real CA so fingerprint auto-resolution succeeds.
        let root = ca::generate_root_ca().expect("gen root ca");
        let pem_path = ca_dir.join("root_ca.pem");
        std::fs::write(&pem_path, root.cert_pem.as_bytes()).expect("write pem");
        // Intentionally leave leaf.pem, leaf.key.pem, root_ca.key.pem
        // ABSENT to prove the delete loop tolerates each one's NotFound.

        let backend = NoopTrustStore::default();
        let opts = UninstallOpts {
            scope: TrustScope::User,
            ca_fingerprint: None,
            keep_shell_rc: true,
            keep_ca_files: false,
        };
        let report = uninstall_with_backends_full(
            &opts,
            &backend,
            Some(home_tmp.path()),
            shell::EnvView::default(),
        )
        .expect("uninstall must succeed");

        assert!(
            report.warnings.is_empty(),
            "absent siblings are idempotent (X3); no warnings expected"
        );
        // The one PEM that did exist was deleted.
        assert!(!pem_path.exists());
        // Only root_ca.pem appears in removed_files — the absent ones
        // are silently skipped.
        assert_eq!(report.removed_files, vec![pem_path]);
    }

    /// SLICE 8: warnings flag the partial-cleanup state — the CLI maps
    /// `!warnings.is_empty()` to EX_TEMPFAIL=75. Here we force a warning
    /// by leaving the PEM absent (no fingerprint source) and assert the
    /// report carries the partial-state signal.
    #[test]
    fn slice8_partial_cleanup_surfaces_warning_for_ex_tempfail_mapping() {
        let home_tmp = tempfile::tempdir().expect("home tempdir");
        let backend = NoopTrustStore::default();

        // Seed an rc block so the rc strip step lands.
        let bashrc = home_tmp.path().join(".bashrc");
        std::fs::write(
            &bashrc,
            format!(
                "{}\nexport HTTPS_PROXY=\"https://localhost:8443\"\n{}\n",
                shell::MARKER_BEGIN,
                shell::MARKER_END,
            ),
        )
        .expect("seed bashrc");

        let opts = UninstallOpts {
            scope: TrustScope::User,
            ca_fingerprint: None, // No PEM either → triggers the warning.
            keep_shell_rc: false,
            keep_ca_files: false,
        };
        let env = shell::EnvView {
            shell: Some("/bin/bash"),
            ..Default::default()
        };
        let _guard = HomeGuard::set(home_tmp.path());
        let report = uninstall_with_backends_full(&opts, &backend, Some(home_tmp.path()), env)
            .expect("uninstall must succeed");

        // Rc strip landed (partial cleanup).
        assert!(report.removed_files.contains(&bashrc));
        // Warning surfaced (the EX_TEMPFAIL signal).
        assert!(!report.warnings.is_empty(), "partial cleanup must warn");
    }

    /// Helper for SLICE 8 tests — resolve the canonical CA dir under a
    /// tempdir HOME. Mirrors paths::base_data_dir_from()/join("ca") for
    /// the current target OS so the install + uninstall both find the
    /// PEM at the same place.
    fn canonical_ca_dir_under_for_test(home: &std::path::Path) -> PathBuf {
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
            panic!("SLICE 8 lib tests not supported on Windows targets")
        }
    }

    /// HomeGuard — atomic process-global HOME setter for the SLICE 5
    /// install tests above. Uses a single shared `Mutex` so the three
    /// tests are serialised under cargo's intra-binary parallel test
    /// runner. The previous tests `install_writes_four_pem_files` etc.
    /// pass `home: None` and `EnvView::default()` so they don't need
    /// the guard — only the slice5_* tests that exercise the real
    /// `install` code path through `install_with_backends`'s HOME-reading
    /// shim do.
    ///
    /// Per `paths.rs` B3 doc: cargo parallelises WITHIN a binary, so we
    /// rely on this mutex to serialise tests that need to mutate the
    /// process env. The `restore` Drop impl always runs (Rust's panic
    /// safety contract) so even a panicking test leaves the env
    /// unchanged for subsequent tests.
    struct HomeGuard {
        prior: Option<std::ffi::OsString>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl HomeGuard {
        fn set(home: &std::path::Path) -> Self {
            static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
            let lock = HOME_LOCK.lock().unwrap_or_else(|p| p.into_inner());
            let prior = std::env::var_os("HOME");
            // SAFETY: Rust 2024 marks set_var unsafe due to multi-thread
            // races. We hold HOME_LOCK across the env mutation, and
            // Drop restores. The SLICE 5 install tests are the only
            // call sites and they all serialise on the same mutex.
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

    /// SLICE 6: atomic per-var env unsetter for the Gemini OAuth tests.
    /// The preflight reads `GEMINI_API_KEY` / `GOOGLE_APPLICATION_CREDENTIALS`
    /// from process env via `install_with_preflight_env`, so a developer
    /// with those set in their shell would see a different `GeminiPreflight`
    /// outcome and the test would race. The guard:
    ///   1. Acquires a per-var mutex serialising with itself.
    ///   2. Snapshots the prior value.
    ///   3. Removes the var.
    ///   4. Restores on Drop.
    ///
    /// **Per-var mutex** — each var has its own lock so a test can hold
    /// two guards simultaneously without self-deadlocking. SLICE 6 tests
    /// always grab `GEMINI_API_KEY` first then `GOOGLE_APPLICATION_CREDENTIALS`;
    /// a future test that needs the reverse order MUST acquire in the
    /// SAME order to avoid deadlock.
    struct EnvVarGuard {
        name: &'static str,
        prior: Option<std::ffi::OsString>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl EnvVarGuard {
        fn unset(name: &'static str) -> Self {
            // Per-var mutex; pick by name. Adding a new var to the slice
            // 6 test set requires extending this match.
            static GEMINI_API_KEY_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
            static GOOGLE_APPLICATION_CREDENTIALS_LOCK: std::sync::Mutex<()> =
                std::sync::Mutex::new(());
            let lock = match name {
                "GEMINI_API_KEY" => GEMINI_API_KEY_LOCK
                    .lock()
                    .unwrap_or_else(|p| p.into_inner()),
                "GOOGLE_APPLICATION_CREDENTIALS" => GOOGLE_APPLICATION_CREDENTIALS_LOCK
                    .lock()
                    .unwrap_or_else(|p| p.into_inner()),
                _ => panic!(
                    "EnvVarGuard: no mutex registered for env var `{name}` \
                     (add a new static lock above)"
                ),
            };
            let prior = std::env::var_os(name);
            // SAFETY: per-var lock serialises all mutations of this var.
            #[allow(unused_unsafe)]
            unsafe {
                std::env::remove_var(name);
            }
            Self {
                name,
                prior,
                _lock: lock,
            }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            #[allow(unused_unsafe)]
            unsafe {
                match self.prior.take() {
                    Some(v) => std::env::set_var(self.name, v),
                    None => std::env::remove_var(self.name),
                }
            }
        }
    }
}
