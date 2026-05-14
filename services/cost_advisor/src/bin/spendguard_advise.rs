//! `spendguard-advise` — CLI entrypoint for the Cost Advisor P1
//! runtime.
//!
//! Single-shot evaluation against one (tenant, date) bucket. Outputs
//! JSON findings to stdout. Used by:
//!   * Operators inspecting recent waste during incident review.
//!   * The e2e benchmark in deploy/demo/verify_p1_cost_advisor.sql.
//!   * (Future) the long-running daemon that polls tenants on a
//!     schedule.
//!
//! Usage:
//!   spendguard-advise --tenant <UUID> --date YYYY-MM-DD
//!                     [--show-proposed-patches] [--write-proposals]
//!                     [--ledger-db URL] [--canonical-db URL]
//!
//! Env vars supersede the CLI defaults so docker compose / k8s can
//! inject DSNs without flag plumbing.

use anyhow::{Context, Result};
use chrono::NaiveDate;
use clap::Parser;
use sqlx::postgres::PgPoolOptions;
use spendguard_cost_advisor::runtime;
use uuid::Uuid;

#[derive(Parser, Debug)]
#[command(
    name = "spendguard-advise",
    version,
    about = "Cost Advisor CLI: run rules + emit JSON findings for one (tenant, day) bucket"
)]
struct Cli {
    /// Tenant UUID to evaluate. Required.
    #[arg(long, value_name = "UUID")]
    tenant: Uuid,

    /// Date bucket (UTC) to evaluate. Defaults to today.
    #[arg(long, value_name = "YYYY-MM-DD")]
    date: Option<NaiveDate>,

    /// Also include RFC-6902 contract DSL patch SUGGESTIONS in the
    /// output JSON. This does NOT write to approval_requests — pass
    /// --write-proposals for that. The flag is named explicitly to
    /// avoid the impression that running with it submits proposals
    /// for operator review (codex CA-P1 r1 P3 caught the earlier
    /// `--propose-patches` name as misleading).
    #[arg(long)]
    show_proposed_patches: bool,

    /// CA-P3 + CA-P3.1 + owner-ack #55: actually INSERT into
    /// approval_requests for every finding with a non-None proposed
    /// patch. Patches are validated against the allowlist (5 replace
    /// paths + 1 test path under /spec/, see `patch_validator`
    /// module + migrations 0043 + 0044). decision_id is derived
    /// deterministically from finding_id so re-runs are idempotent.
    ///
    /// v0.1 note (CA-P3.1): `idle_reservation_rate_v1` emits a
    /// budget-identity-pinned 2-op patch (test on /spec/budgets/0/id
    /// + replace on /spec/budgets/0/reservation_ttl_seconds). The
    /// other two P1.5 rules don't emit patches yet (operator
    /// judgment needed on which rule to tighten). A stderr summary
    /// line reports the count of proposals written.
    #[arg(long)]
    write_proposals: bool,

    /// ledger DB connection string (spendguard_ledger).
    #[arg(
        long,
        env = "SPENDGUARD_COST_ADVISOR_LEDGER_DATABASE_URL",
        default_value = "postgres://spendguard:spendguard_demo@localhost:5433/spendguard_ledger?sslmode=disable"
    )]
    ledger_db: String,

    /// canonical DB connection string (spendguard_canonical).
    #[arg(
        long,
        env = "SPENDGUARD_COST_ADVISOR_CANONICAL_DATABASE_URL",
        default_value = "postgres://spendguard:spendguard_demo@localhost:5433/spendguard_canonical?sslmode=disable"
    )]
    canonical_db: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    let bucket_date = cli
        .date
        .unwrap_or_else(|| chrono::Utc::now().date_naive());

    let ledger = PgPoolOptions::new()
        .max_connections(2)
        .connect(&cli.ledger_db)
        .await
        .context("connect ledger DB")?;
    let canonical = PgPoolOptions::new()
        .max_connections(2)
        .connect(&cli.canonical_db)
        .await
        .context("connect canonical DB")?;

    let emitted = runtime::evaluate_tenant_day(
        &ledger,
        &canonical,
        cli.tenant,
        bucket_date,
        cli.show_proposed_patches,
        cli.write_proposals,
    )
    .await?;

    if cli.write_proposals {
        let proposals_written = emitted
            .iter()
            .filter(|f| f.proposal_outcome.is_some())
            .count();
        eprintln!(
            "write-proposals: {} proposals written ({} findings emitted).",
            proposals_written,
            emitted.len()
        );
    }

    let out = serde_json::json!({
        "tenant_id": cli.tenant.to_string(),
        "bucket_date": bucket_date.to_string(),
        "findings": emitted,
    });
    println!("{}", serde_json::to_string_pretty(&out)?);

    Ok(())
}
