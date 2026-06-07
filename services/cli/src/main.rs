//! `spendguard` — closed CLI installer entry point.
//!
//! ```text
//! spendguard install   --ca-out <DIR> [--scope user|system] [--shell bash|zsh|fish|pwsh]
//! spendguard uninstall [--scope user|system]
//! spendguard doctor
//! ```
//!
//! SLICE 1 (COV_05) implements `install` end-to-end for the CA + leaf
//! material; `uninstall` and `doctor` parse their args but currently bail
//! with a "see SLICE 7" message so the surface area is stable from day one.

use clap::{Args, Parser, Subcommand};
use spendguard_cli::{install, InstallOpts, PreflightRefusal, UninstallOpts};

#[derive(Parser, Debug)]
#[command(
    name = "spendguard",
    version,
    about = "SpendGuard closed-CLI installer (root CA + leaf bootstrap)",
    long_about = None,
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Issue a root CA, issue a localhost leaf, write four PEM files, and
    /// install the CA into the OS trust store. macOS-only in SLICE 2;
    /// Linux/Windows in SLICE 3/4.
    Install(InstallOpts),

    /// Symmetric inverse of `install` — strips the shell rc marker block,
    /// removes the CA from the OS trust store, and deletes the four PEM
    /// files. SLICE 8 (COV_12) closes the D02 deliverable. Pass
    /// `--keep-shell-rc` or `--keep-ca-files` to preserve a specific
    /// artefact (e.g. for CA rotation without disabling tooling).
    Uninstall(UninstallOpts),

    /// Healthcheck: CA fingerprint in trust store + HTTPS_PROXY
    /// reachable + shell rc carries the SpendGuard block + Gemini CLI
    /// auth state. SLICE 7 (COV_11) wires the full diagnostic surface;
    /// the legacy [`spendguard_cli::DoctorOpts`] /
    /// [`spendguard_cli::doctor`] symbols stay available for callers
    /// that want the trust-store-only report.
    Doctor(DoctorCliOpts),
}

/// `spendguard doctor` CLI args. Mirrors the slice doc's `DoctorOpts`
/// + the three operator-visible flags (`--proxy-url`, `--no-color`,
///   `--json`).
#[derive(Args, Debug)]
struct DoctorCliOpts {
    /// HTTPS proxy URL to probe (default: `https://localhost:8443`).
    #[arg(long, value_name = "URL")]
    proxy_url: Option<String>,

    /// Suppress ANSI colour escape sequences in the rendered report.
    /// Default `false` — emit colour when stdout is a TTY.
    #[arg(long, default_value_t = false)]
    no_color: bool,

    /// Emit the report as JSON instead of human-readable text. Useful
    /// for tooling integration; rendering / colour flags are ignored.
    #[arg(long, default_value_t = false)]
    json: bool,
}

fn main() -> anyhow::Result<()> {
    // Subscriber on by default at INFO; `RUST_LOG=spendguard_cli=debug` for
    // CA-issuance traces during dev.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();

    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Install(opts) => {
            // SLICE 6 (COV_10): the preflight refusal carries a multi-line
            // user-facing message that we want printed verbatim to stderr,
            // not collapsed into anyhow's `error:` chain. Match on the
            // typed refusal first; everything else flows through `?`.
            let report = match install(&opts) {
                Ok(r) => r,
                Err(e) => {
                    if let Some(refusal) = e.downcast_ref::<PreflightRefusal>() {
                        eprintln!("{refusal}");
                        std::process::exit(2);
                    }
                    return Err(e);
                }
            };
            // `serde_json` keeps the surface JSON-greppable for the
            // forthcoming demo target without extra plumbing.
            let json = serde_json::to_string_pretty(&report)?;
            println!("{json}");
            tracing::info!(
                fingerprint = %report.ca_fingerprint_sha256,
                ca_pem = %report.ca_pem_path.display(),
                trust_store_locations = ?report.trust_store_locations,
                "spendguard install complete (SLICE 2: CA + leaf + macOS keychain)"
            );
            Ok(())
        }
        Cmd::Uninstall(opts) => {
            // SLICE 8 (COV_12): uninstall is best-effort symmetric inverse.
            // Per implementation.md §9 exit codes:
            //   - 0 on full cleanup (`warnings.is_empty()`)
            //   - 75 (EX_TEMPFAIL) when partial cleanup succeeded — at
            //     least one step landed but at least one warned. The
            //     report JSON carries the per-step `warnings` so tooling
            //     can decide whether to retry.
            //   - non-zero anyhow path only when the dispatch itself
            //     failed (e.g. unsupported OS, no trust backend).
            let report = spendguard_cli::uninstall(&opts)?;
            let json = serde_json::to_string_pretty(&report)?;
            println!("{json}");
            for warning in &report.warnings {
                eprintln!("warning: {warning}");
            }
            if !report.warnings.is_empty() {
                // EX_TEMPFAIL — per BSD sysexits.h, "temporary failure;
                // user is invited to retry". Matches design §9.
                std::process::exit(75);
            }
            Ok(())
        }
        Cmd::Doctor(cli_opts) => {
            use spendguard_cli::doctor::{run, DoctorOpts, ExitCode, DEFAULT_TIMEOUT};
            use spendguard_cli::preflight::BaseEnv;

            // Build the doctor BaseEnv from process env. We do this
            // here in main rather than via a helper because every
            // other caller (lib tests, slice 8 uninstall) supplies a
            // custom BaseEnv literal; only the production CLI seam
            // needs the env-snapshot wiring.
            let env = BaseEnv::from_process();
            let doctor_opts = DoctorOpts {
                proxy_url: cli_opts.proxy_url.clone(),
                timeout: DEFAULT_TIMEOUT,
            };
            let report = run(&env, doctor_opts);

            if cli_opts.json {
                // We serialize a flat structural projection so the
                // JSON consumer doesn't depend on Rust's enum
                // discriminant shape. The structural form is stable
                // for tooling integration.
                let json = serde_json::to_string_pretty(&report_to_json(&report))?;
                println!("{json}");
            } else {
                let use_color = !cli_opts.no_color;
                print!("{}", report.render(use_color));
            }

            match report.exit_code() {
                ExitCode::Healthy => Ok(()),
                ExitCode::Degraded => std::process::exit(1),
                ExitCode::GeminiRefusal => std::process::exit(2),
            }
        }
    }
}

/// Project a [`spendguard_cli::doctor::DoctorReport`] into a stable JSON
/// surface — the doctor module's enums carry rust-internal shape, but
/// tooling consumers want a flat record with string discriminants.
fn report_to_json(report: &spendguard_cli::doctor::DoctorReport) -> serde_json::Value {
    use spendguard_cli::doctor::{
        CaCheckResult, GeminiCheckResult, ProxyCheckResult, ShellRcCheckResult,
    };

    let ca = match &report.ca {
        CaCheckResult::Healthy {
            fingerprint,
            trust_store_locations,
        } => serde_json::json!({
            "status": "healthy",
            "fingerprint_sha256": fingerprint,
            "trust_store_locations": trust_store_locations
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>(),
        }),
        CaCheckResult::NotInTrustStore {
            fingerprint,
            ca_pem_path,
        } => serde_json::json!({
            "status": "not_in_trust_store",
            "fingerprint_sha256": fingerprint,
            "ca_pem_path": ca_pem_path.display().to_string(),
        }),
        CaCheckResult::NotInstalled => serde_json::json!({"status": "not_installed"}),
        CaCheckResult::FileNotReadable { ca_pem_path, error } => serde_json::json!({
            "status": "file_not_readable",
            "ca_pem_path": ca_pem_path.display().to_string(),
            "error": error,
        }),
    };
    let proxy = match &report.proxy {
        ProxyCheckResult::Reachable { addr } => {
            serde_json::json!({"status": "reachable", "addr": addr})
        }
        ProxyCheckResult::ProxyUnreachable { addr, error } => serde_json::json!({
            "status": "unreachable",
            "addr": addr,
            "error": error,
        }),
        ProxyCheckResult::TlsHandshakeFailed { addr, error } => serde_json::json!({
            "status": "tls_handshake_failed",
            "addr": addr,
            "error": error,
        }),
    };
    let shell = match &report.shell {
        ShellRcCheckResult::Present { rc_path, env_vars } => serde_json::json!({
            "status": "present",
            "rc_path": rc_path.display().to_string(),
            "env_vars": env_vars
                .iter()
                .map(|(n, v)| serde_json::json!({"name": n, "value": v}))
                .collect::<Vec<_>>(),
        }),
        ShellRcCheckResult::Absent { reason } => {
            serde_json::json!({"status": "absent", "reason": format!("{reason:?}")})
        }
    };
    let linux_user_scope = report.linux_user_scope.as_ref().map(|r| {
        serde_json::json!({
            "cert_path": r.cert_path.display().to_string(),
        })
    });
    let gemini = match &report.gemini {
        GeminiCheckResult::Healthy { mode } => {
            serde_json::json!({"status": "healthy", "mode": format!("{mode:?}")})
        }
        GeminiCheckResult::OauthFreetierWarning => {
            serde_json::json!({"status": "oauth_freetier_warning"})
        }
        GeminiCheckResult::NotInstalled => serde_json::json!({"status": "not_installed"}),
    };
    serde_json::json!({
        "healthy": report.is_healthy(),
        "exit_code": report.exit_code() as i32,
        "ca": ca,
        "proxy": proxy,
        "shell": shell,
        "linux_user_scope": linux_user_scope,
        "gemini": gemini,
    })
}
