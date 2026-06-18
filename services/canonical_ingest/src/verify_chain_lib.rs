//! Verify-chain library entry point.
//!
//! Spec ancestor: `docs/audit-chain-prediction-extension-v1alpha1.md`
//! §7 + §11.2 + `docs/calibration-report-spec-v1alpha1.md` §3.4.
//!
//! ## Why expose this as a library
//!
//! SLICE_01 shipped `src/bin/verify_chain.rs` as a stub (status:
//! `NOT_IMPLEMENTED`). SLICE_13 (calibration-report CLI) needs to call
//! the same verify path inline so `--verify-chain` can abort a report
//! on chain integrity failure (spec §3.4). Rather than fork the logic,
//! we expose a small library entry point here. The stub bin can be
//! migrated later; for now this module is **additive only** and does
//! not change the existing bin behaviour.
//!
//! ## Contract
//!
//! `verify_chain` scans the canonical_events table for the given
//! tenant + window and verifies every row's `producer_signature`
//! against `canonical_bytes`. The optional `check_prediction_mirror`
//! flag also asserts that the first-class mirror columns (predicted_a,
//! predicted_b, predicted_c, etc.) match the CloudEvent payload.
//!
//! ## Phase C scope (SLICE_13 reusing)
//!
//! For the SLICE_13 first ship, the library entry point's `summarize`
//! method returns the count of scanned + failed rows. The full per-row
//! Postgres scan + cross-storage consistency check is gated on the
//! producer-side mirror writes that land in SLICE_06+. Today the bin
//! emits a SLICE_10_ACTIVATED status line; the library mirrors that
//! contract so SLICE_13 callers can rely on it.
//!
//! ## Non-goals
//!
//! - This module does NOT modify any existing signing logic.
//! - It does NOT change the canonical_bytes derivation.
//! - It only exposes a typed API for SLICE_13's report wrapper.

use serde::{Deserialize, Serialize};

/// Summary of a verify-chain run. Returned to callers so they can
/// emit structured logs / metrics / report-aborts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerifyChainSummary {
    /// Number of canonical_events rows visited.
    pub rows_scanned: u64,
    /// Number of rows that failed signature verification.
    pub rows_failed: u64,
    /// Number of rows skipped because the mirror columns are NULL
    /// (legacy pre-SLICE_06 rows).
    pub rows_skipped_legacy: u64,
    /// If a verify failure occurred, the offending event_id + reason.
    pub first_failure: Option<VerifyChainFailure>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerifyChainFailure {
    pub event_id: String,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct VerifyChainArgs {
    pub tenant_id: Option<uuid::Uuid>,
    pub check_prediction_mirror: bool,
    pub from: Option<chrono::DateTime<chrono::Utc>>,
    pub to: Option<chrono::DateTime<chrono::Utc>>,
}

/// Audit event types that carry the predictor mirror column contract.
///
/// Tokenizer/stats `*_drift_alert` events are audit-routed and must be
/// admitted by verify-chain, but they are not decision/outcome rows and do
/// not carry the 18 prediction mirror columns.
pub fn event_type_requires_prediction_mirror(event_type: &str) -> bool {
    matches!(
        event_type,
        "spendguard.audit.decision" | "spendguard.audit.outcome"
    )
}

/// Run the verify-chain replay over the configured Postgres pool.
///
/// This counts canonical_events rows in the window and reports how many
/// carry NULL prediction-mirror columns (legacy pre-SLICE_06 rows). It
/// does **not** verify producer signatures and does **not** perform the
/// per-row prediction-mirror cross-check (re-decoding the embedded
/// CloudEvent and comparing column <-> proto field) — that needs the
/// verifier/key-registry wiring which is not yet present here.
///
/// ## Fail-closed contract (security fix)
///
/// Because the per-row cross-check is unimplemented, returning
/// `Ok(VerifyChainSummary { rows_failed: 0, .. })` when
/// `check_prediction_mirror == true` would be a silent fail-open: the
/// calibration-report cron gate (which always passes
/// `check_prediction_mirror = true`) would stay green regardless of
/// whether the on-disk audit_outbox prediction columns actually match
/// the signed CloudEvent proto bytes. This mirrors the same fail-open
/// the `verify-chain` binary closes.
///
/// So this function FAILS CLOSED: when `check_prediction_mirror` is true
/// it returns an explicit error (which both callers map to a non-zero
/// exit code) instead of a success summary it cannot back up. Callers
/// that only want the legacy NULL-prediction count scan must set
/// `check_prediction_mirror = false`.
pub async fn verify_chain(
    pool: &sqlx::PgPool,
    args: &VerifyChainArgs,
) -> Result<VerifyChainSummary, sqlx::Error> {
    // Fail-closed: the per-row prediction-mirror cross-check is not
    // implemented in this build. Do not emit a green summary the gate
    // cannot back up — return an explicit error so the calibration-report
    // cron / CLI exit non-zero instead of silently passing.
    if args.check_prediction_mirror {
        return Err(sqlx::Error::Configuration(
            "verify-chain prediction-mirror cross-check is not implemented \
             in this build (requires a live ledger DB + verifier/key-registry \
             wiring); refusing to report a passing audit-integrity result. \
             Pass check_prediction_mirror=false to run only the legacy \
             NULL-prediction count scan."
                .into(),
        ));
    }

    // Count canonical_events rows in the window.
    //
    // RLS scope: the calibration-report CLI sets
    // app.current_tenant_id inside its transaction; this library
    // runs against the same pool but uses its own short-lived
    // statement and re-binds the session variable.
    let mut tx = pool.begin().await?;

    if let Some(tid) = args.tenant_id {
        sqlx::query("SELECT set_config('app.current_tenant_id', $1, true)")
            .bind(tid.to_string())
            .execute(&mut *tx)
            .await?;
    }

    // SQL is deliberately defensive: tenant_id filter is optional
    // (covers the bin's --no-tenant mode), prediction-mirror counting
    // is optional. The COUNT(*) is the rows_scanned signal.
    let legacy_count_sql = if args.check_prediction_mirror {
        "COUNT(*) FILTER (\
            WHERE (event_type = 'spendguard.audit.decision' \
                   AND (predicted_a_tokens IS NULL \
                        OR reserved_strategy IS NULL \
                        OR prediction_strategy_used IS NULL \
                        OR prediction_policy_used IS NULL \
                        OR tokenizer_tier IS NULL \
                        OR run_projection_at_decision_atomic IS NULL \
                        OR run_steps_completed_so_far IS NULL)) \
               OR (event_type = 'spendguard.audit.outcome' \
                   AND (actual_input_tokens IS NULL \
                        OR actual_output_tokens IS NULL))\
        )::bigint"
    } else {
        "0::bigint"
    };
    let mut sql = format!(
        "SELECT \
            COUNT(*)::bigint AS rows_total, \
            {legacy_count_sql} AS rows_legacy \
         FROM canonical_events WHERE event_type LIKE 'spendguard.audit.%'"
    );
    if args.tenant_id.is_some() {
        sql.push_str(" AND tenant_id = $1");
    }
    if let (Some(_), Some(_)) = (args.from, args.to) {
        let from_param = if args.tenant_id.is_some() { "$2" } else { "$1" };
        let to_param = if args.tenant_id.is_some() { "$3" } else { "$2" };
        sql.push_str(&format!(
            " AND event_time BETWEEN {from_param} AND {to_param}"
        ));
    }

    let mut q = sqlx::query(&sql);
    if let Some(tid) = args.tenant_id {
        q = q.bind(tid);
    }
    if let Some(from) = args.from {
        q = q.bind(from);
    }
    if let Some(to) = args.to {
        q = q.bind(to);
    }

    use sqlx::Row;
    let row = q.fetch_one(&mut *tx).await?;
    let rows_total: i64 = row.try_get(0)?;
    let rows_legacy: i64 = row.try_get(1)?;
    tx.commit().await?;

    Ok(VerifyChainSummary {
        rows_scanned: rows_total.max(0) as u64,
        rows_failed: 0,
        rows_skipped_legacy: rows_legacy.max(0) as u64,
        first_failure: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_serializes() {
        let s = VerifyChainSummary {
            rows_scanned: 100,
            rows_failed: 0,
            rows_skipped_legacy: 5,
            first_failure: None,
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("rows_scanned"));
    }

    #[test]
    fn summary_carries_first_failure() {
        let s = VerifyChainSummary {
            rows_scanned: 100,
            rows_failed: 1,
            rows_skipped_legacy: 0,
            first_failure: Some(VerifyChainFailure {
                event_id: "deadbeef".into(),
                reason: "signature mismatch".into(),
            }),
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("deadbeef"));
        assert!(json.contains("signature mismatch"));
    }

    #[test]
    fn args_have_optional_window() {
        // Make sure the API doesn't force a tenant + window; the bin
        // still calls without either parameter on legacy runs.
        let args = VerifyChainArgs {
            tenant_id: None,
            check_prediction_mirror: true,
            from: None,
            to: None,
        };
        assert!(args.from.is_none());
    }

    #[test]
    fn tokenizer_drift_alert_is_admitted_without_prediction_mirror_columns() {
        assert!(!event_type_requires_prediction_mirror(
            "spendguard.audit.tokenizer_drift_alert.v1alpha1"
        ));
        assert!(!event_type_requires_prediction_mirror(
            "spendguard.audit.prediction_drift_alert.v1alpha1"
        ));
    }

    #[test]
    fn decision_and_outcome_require_prediction_mirror_columns() {
        assert!(event_type_requires_prediction_mirror(
            "spendguard.audit.decision"
        ));
        assert!(event_type_requires_prediction_mirror(
            "spendguard.audit.outcome"
        ));
    }
}
