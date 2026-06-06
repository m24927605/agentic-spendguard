//! `spendguard-calibration-report` binary entry point.
//!
//! Spec ancestor: `docs/calibration-report-spec-v1alpha1.md` §2 + §5.
//!
//! ## Run shape
//!
//! 1. Parse CLI.
//! 2. Enforce cross-tenant scope (§5.2). Mismatch → exit 2 + emit
//!    `spendguard.audit.calibration.unauthorized_access`.
//! 3. Open canonical Postgres pool (sqlx tls-rustls).
//! 4. Run §3 queries inside a per-tenant RLS transaction. Routes
//!    cache vs canonical per `--proof-mode`.
//! 5. Run §8 recommendation engine.
//! 6. If `--verify-chain` set: invoke `spendguard_canonical_ingest::
//!    verify_chain_lib::verify_chain` and stop on failure with exit 3.
//! 7. Render via formatter.
//! 8. Emit `spendguard.audit.calibration.report_generated` (§5.3).
//! 9. Exit per `Report::exit_code()`.

use clap::Parser;
use spendguard_calibration_report::{
    cli::{Cli, ProofMode, Subcommand},
    formatters::{self, FormatOptions},
    recommendations,
    report::{Report, ReportExitCode, Window},
    self_audit,
    sql_queries::{
        self, fetch_calibration_ratios, fetch_calibration_ratios_cache_mode, fetch_drift_alerts,
        fetch_run_level_counts, fetch_tier_distribution, open_tenant_tx,
    },
    verify_chain_wrapper,
};
use sqlx::postgres::PgPoolOptions;
use std::process::ExitCode;
use tracing::{error, info, warn};

#[tokio::main]
async fn main() -> ExitCode {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("rustls install_default");

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
        Subcommand::VerifyChain => run_verify_chain_subcommand(cli).await,
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
             (spec §5.2). Audit event emitted."
        );
        if cli.self_audit {
            self_audit::emit_unauthorized_access(&cli);
        }
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
        Some(url) => match PgPoolOptions::new().max_connections(4).connect(url).await {
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
    let effective_mode = cli.effective_proof_mode();
    let mut report = match run_queries(&pool, &tenant_uuid, from, to, effective_mode).await {
        Ok(mut r) => {
            r.tenant_id = tenant_uuid.to_string();
            r.window = Window { from, to };
            r.proof_mode = match effective_mode {
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

    // ---- §8 recommendation engine -------------------------------
    report.recommendations = recommendations::evaluate(&report);

    // ---- §3.4 verify-chain integration --------------------------
    if cli.verify_chain {
        report.verify_chain_run = true;
        match verify_chain_wrapper::run_verify_chain(&pool, tenant_uuid, from, to).await {
            Ok(Some(failure)) => {
                error!(
                    event_id = %failure.event_id,
                    reason = %failure.reason,
                    "verify-chain failure"
                );
                report.verify_chain_failure = Some(failure);
            }
            Ok(None) => {
                info!("verify-chain replay clean");
            }
            Err(e) => {
                error!(error = %e, "verify-chain wrapper error");
                eprintln!("calibration-report: verify-chain wrapper failed: {e}");
                return ReportExitCode::QueryError.to_process_exit_code();
            }
        }
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
            eprintln!(
                "calibration-report: write to {} failed: {e}",
                path.display()
            );
            return ReportExitCode::QueryError.to_process_exit_code();
        }
    } else {
        print!("{rendered}");
    }

    // ---- Self-audit (§5.3) --------------------------------------
    let exit_code = report.exit_code();
    if cli.self_audit {
        self_audit::emit_report_generated(&cli, from, to, exit_code as u8);
    }

    exit_code.to_process_exit_code()
}

async fn run_queries(
    pool: &sqlx::PgPool,
    tenant: &uuid::Uuid,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
    proof_mode: ProofMode,
) -> Result<Report, sql_queries::QueryError> {
    let mut tx = open_tenant_tx(pool, tenant).await?;
    // Tier distribution + drift alerts always read from canonical_events
    // (cache has no tier breakdown; drift_alert is a canonical event type).
    let tier_distribution = fetch_tier_distribution(&mut tx, tenant, from, to).await?;
    let drift_alerts = fetch_drift_alerts(&mut tx, tenant, from, to).await?;
    let (proj_exceeded, drift_detected, run_total) =
        fetch_run_level_counts(&mut tx, tenant, from, to).await?;
    // Calibration ratios depend on proof mode.
    let calibration_ratios = match proof_mode {
        ProofMode::Cache => fetch_calibration_ratios_cache_mode(&mut tx, tenant, from, to).await?,
        ProofMode::Canonical => fetch_calibration_ratios(&mut tx, tenant, from, to).await?,
    };
    tx.commit().await?;
    Ok(Report {
        tenant_id: tenant.to_string(),
        window: Window { from, to },
        proof_mode: match proof_mode {
            ProofMode::Cache => "cache".into(),
            ProofMode::Canonical => "canonical".into(),
        },
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

/// `spendguard-calibration-report verify-chain` — inline shortcut to
/// the canonical_ingest replay verifier (per spec §3.4). Re-uses the
/// library entry point.
async fn run_verify_chain_subcommand(cli: Cli) -> ExitCode {
    info!("verify-chain subcommand invoked");
    let pool = match &cli.canonical_url {
        Some(url) => match PgPoolOptions::new().max_connections(2).connect(url).await {
            Ok(p) => p,
            Err(e) => {
                eprintln!("calibration-report verify-chain: DB connect failed: {e}");
                return ReportExitCode::QueryError.to_process_exit_code();
            }
        },
        None => {
            eprintln!(
                "calibration-report verify-chain: --canonical-url (or env \
                 SPENDGUARD_CALIBRATION_CANONICAL_URL) required"
            );
            return ReportExitCode::QueryError.to_process_exit_code();
        }
    };
    let tenant_uuid = cli
        .tenant
        .as_deref()
        .and_then(|s| uuid::Uuid::parse_str(s).ok());
    if tenant_uuid.is_none() && cli.tenant.is_some() {
        eprintln!("calibration-report verify-chain: --tenant is not a UUID");
        return ReportExitCode::QueryError.to_process_exit_code();
    }

    // Use library API directly so the verify-chain subcommand is a
    // thin pass-through.
    let now = chrono::Utc::now();
    let from =
        sql_queries::parse_window_anchor(&cli.from, now).unwrap_or(now - chrono::Duration::days(7));
    let to = sql_queries::parse_window_anchor(&cli.to, now).unwrap_or(now);

    let args = spendguard_canonical_ingest::verify_chain_lib::VerifyChainArgs {
        tenant_id: tenant_uuid,
        check_prediction_mirror: true,
        from: Some(from),
        to: Some(to),
    };
    match spendguard_canonical_ingest::verify_chain_lib::verify_chain(&pool, &args).await {
        Ok(summary) => {
            let json = serde_json::to_string(&summary).unwrap_or_default();
            println!("{json}");
            if summary.rows_failed > 0 {
                return ReportExitCode::VerifyChainFailed.to_process_exit_code();
            }
            ReportExitCode::Success.to_process_exit_code()
        }
        Err(e) => {
            eprintln!("calibration-report verify-chain: {e}");
            ReportExitCode::QueryError.to_process_exit_code()
        }
    }
}
