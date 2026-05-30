//! verify-chain integration wrapper.
//!
//! Spec ancestor: `docs/calibration-report-spec-v1alpha1.md` §3.4 +
//! `docs/audit-chain-prediction-extension-v1alpha1.md` §7.
//!
//! ## Architecture
//!
//! The calibration-report CLI does NOT re-implement the verifier. It
//! delegates to `spendguard_canonical_ingest::verify_chain_lib`
//! (exposed by Phase C as a library entry point) so we only have one
//! source of truth for "is this audit chain intact". The wrapper:
//!
//!   1. Translates `Cli` flags to `VerifyChainArgs`.
//!   2. Calls the library function.
//!   3. Maps the typed summary into a `Report::verify_chain_failure`.
//!
//! ## Failure mapping (spec §3.4)
//!
//! Any non-zero `rows_failed` aborts the report with exit code 3 and
//! the `first_failure` is surfaced to the operator. Legacy NULL-mirror
//! rows are counted as skipped (per the SLICE_01 + SLICE_10 conventions)
//! and do not abort.

use crate::report::VerifyChainFailure;
use chrono::{DateTime, Utc};
use spendguard_canonical_ingest::verify_chain_lib::{
    verify_chain as upstream_verify_chain, VerifyChainArgs, VerifyChainSummary,
};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum VerifyChainError {
    #[error("verify-chain SQL error: {0}")]
    Db(#[from] sqlx::Error),
}

/// Run the upstream verify-chain replay scoped to the calibration
/// window. Returns a `VerifyChainFailure` iff any row failed; returns
/// `Ok(None)` for clean runs (including the "scan complete; producer
/// mirror writes are live but no rows in window" case).
pub async fn run_verify_chain(
    pool: &PgPool,
    tenant_id: Uuid,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Option<VerifyChainFailure>, VerifyChainError> {
    let args = VerifyChainArgs {
        tenant_id: Some(tenant_id),
        check_prediction_mirror: true,
        from: Some(from),
        to: Some(to),
    };
    let summary: VerifyChainSummary = upstream_verify_chain(pool, &args).await?;
    if summary.rows_failed > 0 {
        if let Some(f) = summary.first_failure {
            return Ok(Some(VerifyChainFailure {
                event_id: f.event_id,
                reason: f.reason,
            }));
        }
        // No first_failure detail (defence-in-depth): synthesize one
        // so the report still exits 3.
        return Ok(Some(VerifyChainFailure {
            event_id: "(unknown)".to_string(),
            reason: format!("verify-chain reported {} failed rows", summary.rows_failed),
        }));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The verify-chain library function requires a live PgPool; full
    // integration coverage lives in `tests/verify_chain_integration.rs`.
    // The unit test here just exercises the type wiring.

    #[test]
    fn verify_chain_error_displays() {
        let e = VerifyChainError::Db(sqlx::Error::PoolClosed);
        let s = format!("{e}");
        assert!(s.contains("verify-chain SQL error"));
    }
}
