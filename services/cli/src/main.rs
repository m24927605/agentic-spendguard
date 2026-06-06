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

use clap::{Parser, Subcommand};
use spendguard_cli::{install, DoctorOpts, InstallOpts, UninstallOpts};

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

    /// Symmetric removal of the trust-store entry. Full on-disk + rc
    /// cleanup lands in SLICE 7.
    Uninstall(UninstallOpts),

    /// Healthcheck: CA in store (this slice) + HTTPS_PROXY reachable
    /// + TLS handshake (SLICE 7).
    Doctor(DoctorOpts),
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
            let report = install(&opts)?;
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
            let report = spendguard_cli::uninstall(&opts)?;
            let json = serde_json::to_string_pretty(&report)?;
            println!("{json}");
            Ok(())
        }
        Cmd::Doctor(opts) => {
            let report = spendguard_cli::doctor(&opts)?;
            let json = serde_json::to_string_pretty(&report)?;
            println!("{json}");
            Ok(())
        }
    }
}
