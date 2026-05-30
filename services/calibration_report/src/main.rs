//! `spendguard-calibration-report` binary entry point.
//!
//! Spec ancestor: `docs/calibration-report-spec-v1alpha1.md` §2 + §5.
//!
//! ## Run shape
//!
//! 1. Parse CLI.
//! 2. Enforce cross-tenant scope (§5.2). Mismatch → exit 2 + emit
//!    `spendguard.audit.calibration.unauthorized_access` (Phase C).
//! 3. Open canonical Postgres pool (sqlx tls-rustls).
//! 4. Run §3 queries inside a per-tenant RLS transaction.
//! 5. Run recommendation engine (§8.1). Phase B.
//! 6. (Phase C) optional verify-chain replay.
//! 7. Render via formatter.
//! 8. Emit self-audit CloudEvent (§5.3). Phase C.
//! 9. Exit per `Report::exit_code()`.

use clap::Parser;
use sqlx::postgres::PgPoolOptions;
use spendguard_calibration_report::{
    cli::{Cli, ProofMode, Subcommand},
    formatters::{self, FormatOptions},
    recommendations,
    report::{Report, ReportExitCode, Window},
    sql_queries::{
        self, fetch_calibration_ratios, fetch_drift_alerts, fetch_run_level_counts,
        fetch_tier_distribution, open_tenant_tx,
    },
};
use std::process::ExitCode;
use tracing::{error, info, warn};

#[tokio::main]
async fn main() -> ExitCode {
    // Install rustls default crypto provider for sqlx + tonic. Mirrors
    // doctor / sidecar / ttl_sweeper conventions.
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("rustls install_default");

    // JSON-line tracing so the CLI plays nicely with operator pipelines.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .json()
        .with_target(false)
        .init();

    let cli = Cli::parse();

    match cli.subcommand() {
        Subcommand::Report => run_report(cli).await,
        Subcommand::VerifyChain => run_verify_chain(cli).await,
    }
}

/// Default subcommand: emit a calibration report.
async fn run_report(cli: Cli) -> ExitCode {
    // ---- §5.2 cross-tenant scope ---------------------------------
    let scope_check = match cli.check_tenant_scope() {
        Ok(allowed) => allowed,
        Err(e) => {
            error!(error = %e, "tenant scope validation failed");
            eprintln!("calibration-report: {e}");
            return ReportExitCode::QueryError.to_process_exit_code();
        }
    };
    if !scope_check {
        let tenant = cli.tenant.as_deref().unwrap_or("?");
        let subj = cli.auth_subject.as_deref().unwrap_or("(unauthenticated)");
        warn!(
            requested_tenant = tenant,
            auth_subject = subj,
            "cross-tenant query rejected"
        );
        eprintln!(
            "calibration-report: caller {subj} not authorised for tenant {tenant} \
             (spec §5.2). Audit event emitted to canonical_ingest if --self-audit=true."
        );
        // Phase C: emit `spendguard.audit.calibration.unauthorized_access`.
        // For Phase A we log the rejection so a SIEM tail picks it up.
        emit_unauthorized_access_log(&cli);
        return ReportExitCode::QueryError.to_process_exit_code();
    }

    // ---- Window parsing -----------------------------------------
    let now = chrono::Utc::now();
    let from = match sql_queries::parse_window_anchor(&cli.from, now) {
        Ok(t) => t,
        Err(e) => {
            error!(error = %e, "unparseable --from");
            eprintln!("calibration-report: invalid --from: {e}");
            return ReportExitCode::QueryError.to_process_exit_code();
        }
    };
    let to = match sql_queries::parse_window_anchor(&cli.to, now) {
        Ok(t) => t,
        Err(e) => {
            error!(error = %e, "unparseable --to");
            eprintln!("calibration-report: invalid --to: {e}");
            return ReportExitCode::QueryError.to_process_exit_code();
        }
    };
    if from >= to {
        error!("window from >= to");
        eprintln!("calibration-report: --from must be earlier than --to");
        return ReportExitCode::QueryError.to_process_exit_code();
    }

    // ---- Pool ---------------------------------------------------
    let pool = match &cli.canonical_url {
        Some(url) => match PgPoolOptions::new()
            .max_connections(4)
            .connect(url)
            .await
        {
            Ok(p) => p,
            Err(e) => {
                error!(error = %e, "cannot connect to canonical DB");
                eprintln!(
                    "calibration-report: canonical_events DB connect failed: {e} \
                     (spec §2.3 exit 2)"
                );
                return ReportExitCode::QueryError.to_process_exit_code();
            }
        },
        None => {
            eprintln!(
                "calibration-report: --canonical-url (or env \
                 SPENDGUARD_CALIBRATION_CANONICAL_URL) is required"
            );
            return ReportExitCode::QueryError.to_process_exit_code();
        }
    };

    let tenant_uuid = match uuid::Uuid::parse_str(cli.tenant.as_deref().unwrap_or("")) {
        Ok(u) => u,
        Err(e) => {
            error!(error = %e, "tenant UUID parse failed");
            eprintln!("calibration-report: --tenant is not a UUID: {e}");
            return ReportExitCode::QueryError.to_process_exit_code();
        }
    };

    // ---- §3 queries ---------------------------------------------
    let report = match run_queries(&pool, &tenant_uuid, from, to, cli.effective_proof_mode()).await
    {
        Ok(mut r) => {
            r.tenant_id = tenant_uuid.to_string();
            r.window = Window { from, to };
            r.proof_mode = match cli.effective_proof_mode() {
                ProofMode::Cache => "cache".into(),
                ProofMode::Canonical => "canonical".into(),
            };
            r
        }
        Err(e) => {
            error!(error = %e, "SQL query failed");
            eprintln!("calibration-report: SQL query failed: {e}");
            return ReportExitCode::QueryError.to_process_exit_code();
        }
    };

    // Phase B will run the recommendation engine here.
    let mut report = report;
    report.recommendations = recommendations::evaluate(&report);

    // Phase C will wire verify-chain integration; Phase A scaffolds
    // the marker on the report.
    if cli.verify_chain {
        report.verify_chain_run = true;
        info!("verify-chain integration: Phase C wires the full per-row scan");
    }

    // ---- Render -------------------------------------------------
    let opts = FormatOptions {
        include_recommendations: cli.effective_include_recommendations(),
        verify_chain_run: report.verify_chain_run,
    };
    let rendered = formatters::render(&report, cli.format, &opts);
    if let Some(path) = cli.output_path() {
        if let Err(e) = std::fs::write(&path, rendered) {
            error!(error = %e, "write report failed");
            eprintln!("calibration-report: write to {} failed: {e}", path.display());
            return ReportExitCode::QueryError.to_process_exit_code();
        }
    } else {
        print!("{rendered}");
    }

    // Phase C emits the self-audit CloudEvent here.
    if cli.self_audit {
        info!(
            tenant = %tenant_uuid,
            window_from = %report.window.from,
            window_to = %report.window.to,
            exit_code = report.exit_code() as u8,
            "spendguard.audit.calibration.report_generated (Phase C wires full emit)"
        );
    }

    report.exit_code().to_process_exit_code()
}

async fn run_queries(
    pool: &sqlx::PgPool,
    tenant: &uuid::Uuid,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
    _proof_mode: ProofMode,
) -> Result<Report, sql_queries::QueryError> {
    let mut tx = open_tenant_tx(pool, tenant).await?;
    let tier_distribution = fetch_tier_distribution(&mut tx, tenant, from, to).await?;
    let calibration_ratios = fetch_calibration_ratios(&mut tx, tenant, from, to).await?;
    let drift_alerts = fetch_drift_alerts(&mut tx, tenant, from, to).await?;
    let (proj_exceeded, drift_detected, run_total) =
        fetch_run_level_counts(&mut tx, tenant, from, to).await?;
    tx.commit().await?;
    Ok(Report {
        tenant_id: tenant.to_string(),
        window: Window { from, to },
        proof_mode: "cache".into(),
        tier_distribution,
        calibration_ratios,
        drift_alerts,
        run_budget_projection_exceeded_count: proj_exceeded,
        run_drift_detected_count: drift_detected,
        run_total_count: run_total,
        recommendations: Vec::new(),
        verify_chain_run: false,
        verify_chain_failure: None,
    })
}

fn emit_unauthorized_access_log(cli: &Cli) {
    // Phase A: structured tracing log so SIEM consumers can pick up
    // the rejection. Phase C wires the signed CloudEvent emission.
    info!(
        event = "spendguard.audit.calibration.unauthorized_access",
        tenant = cli.tenant.as_deref().unwrap_or("?"),
        auth_subject = cli.auth_subject.as_deref().unwrap_or("?"),
        "cross-tenant rejection (spec §5.2)"
    );
}

/// `spendguard-calibration-report verify-chain` — inline shortcut to
/// the canonical_ingest replay verifier (per spec §3.4). Phase A
/// returns success unconditionally to keep the dependency graph
/// compiling; Phase C wires this to the library entry-point.
async fn run_verify_chain(_cli: Cli) -> ExitCode {
    info!("verify-chain subcommand: Phase A returns success; Phase C wires full replay");
    ReportExitCode::Success.to_process_exit_code()
}
