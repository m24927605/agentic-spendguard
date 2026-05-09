//! `spendguard doctor` CLI binary.
//!
//! Usage:
//!   spendguard-doctor [--json] [--ledger-url URL] [--canonical-url URL]
//!                     [--bundle-dir PATH] [--uds-path PATH]
//!                     [--tenant-id UUID] [--max-staleness-seconds N]
//!                     [--profile demo|production]
//!
//! Output: human-readable text by default; `--json` for SIEM /
//! dashboard consumption.
//!
//! Exit codes:
//!   0 — all checks pass.
//!   1 — at least one fail.
//!   2 — invalid args / startup error.

use clap::Parser;
use sqlx::postgres::PgPoolOptions;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "spendguard-doctor")]
#[command(about = "Phase 5 GA hardening S21: diagnose deployment readiness")]
struct Cli {
    /// Render JSON instead of human-readable text.
    #[arg(long, default_value_t = false)]
    json: bool,

    /// Ledger Postgres URL. Skipped if unset.
    #[arg(long, env = "SPENDGUARD_DOCTOR_LEDGER_URL")]
    ledger_url: Option<String>,

    /// Canonical Postgres URL. Skipped if unset.
    #[arg(long, env = "SPENDGUARD_DOCTOR_CANONICAL_URL")]
    canonical_url: Option<String>,

    /// Contract bundle dir. Default /var/lib/spendguard/bundles.
    #[arg(long, default_value = "/var/lib/spendguard/bundles")]
    bundle_dir: PathBuf,

    /// Sidecar UDS path. Default /var/run/spendguard/adapter.sock.
    #[arg(long, default_value = "/var/run/spendguard/adapter.sock")]
    uds_path: PathBuf,

    /// Tenant id to verify provisioning for.
    #[arg(long, env = "SPENDGUARD_DOCTOR_TENANT_ID")]
    tenant_id: Option<uuid::Uuid>,

    /// Max staleness for pricing snapshots (seconds).
    #[arg(long, default_value_t = 86400)]
    max_staleness_seconds: u64,

    /// Profile (demo|production). Affects signing-mode check.
    #[arg(long, env = "SPENDGUARD_PROFILE", default_value = "production")]
    profile: String,

    /// Service prefix to introspect for SIGNING_MODE env.
    #[arg(long, default_value = "SPENDGUARD_SIDECAR")]
    signing_env_prefix: String,
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let cli = Cli::parse();

    let mut checks = Vec::new();

    // 1. UDS reachability.
    checks.push(spendguard_doctor::check_sidecar_uds(&cli.uds_path));

    // 2. Contract bundle.
    checks.push(spendguard_doctor::check_contract_bundle(&cli.bundle_dir));

    // 3. Signing mode.
    checks.push(spendguard_doctor::check_signing_mode(
        &cli.signing_env_prefix,
        &cli.profile,
    ));

    // 4. Ledger DB.
    let ledger_pool = match &cli.ledger_url {
        Some(url) => match PgPoolOptions::new()
            .max_connections(2)
            .connect(url)
            .await
        {
            Ok(p) => Some(p),
            Err(e) => {
                checks.push(spendguard_doctor::CheckResult::fail(
                    "ledger.db_reachable",
                    "LEDGER_DB_CONNECT_FAILED",
                    format!("connect failed: {e}"),
                    "verify ledger DB URL + network policy",
                ));
                None
            }
        },
        None => None,
    };
    if cli.ledger_url.is_some() {
        checks.push(spendguard_doctor::check_ledger_db(ledger_pool.as_ref()).await);
    }

    // 5. Canonical DB + pricing freshness.
    let canonical_pool = match &cli.canonical_url {
        Some(url) => match PgPoolOptions::new()
            .max_connections(2)
            .connect(url)
            .await
        {
            Ok(p) => Some(p),
            Err(e) => {
                checks.push(spendguard_doctor::CheckResult::fail(
                    "canonical.db_reachable",
                    "CANONICAL_DB_CONNECT_FAILED",
                    format!("connect failed: {e}"),
                    "verify canonical DB URL + network policy",
                ));
                None
            }
        },
        None => None,
    };
    checks.push(
        spendguard_doctor::check_pricing_freshness(
            canonical_pool.as_ref(),
            cli.max_staleness_seconds,
        )
        .await,
    );

    // 6. Tenant provisioned.
    if let Some(tid) = cli.tenant_id {
        checks.push(
            spendguard_doctor::check_tenant_provisioned(ledger_pool.as_ref(), tid).await,
        );
    } else {
        checks.push(spendguard_doctor::CheckResult::skipped(
            "tenant.provisioned",
            "no --tenant-id supplied",
        ));
    }

    let report = spendguard_doctor::Report::from_checks(checks);

    let env_map: std::collections::HashMap<String, String> = std::env::vars().collect();
    if cli.json {
        let json =
            serde_json::to_string_pretty(&report).expect("report serializes to JSON");
        let redacted = spendguard_doctor::redact_secrets(&json, &env_map);
        println!("{redacted}");
    } else {
        let human = report.render_human();
        let redacted = spendguard_doctor::redact_secrets(&human, &env_map);
        print!("{redacted}");
    }

    match report.overall {
        spendguard_doctor::CheckStatus::Pass => std::process::ExitCode::from(0),
        spendguard_doctor::CheckStatus::Fail => std::process::ExitCode::from(1),
        spendguard_doctor::CheckStatus::Skipped => std::process::ExitCode::from(0),
    }
}
