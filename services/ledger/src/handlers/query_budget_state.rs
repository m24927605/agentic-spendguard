//! `Ledger::QueryBudgetState` handler (Phase 2B Step 7).
//!
//! Reads canonical truth from `ledger_entries` JOIN `ledger_accounts`
//! (account_kind lives on accounts; per Codex round 2 M1.1). For the
//! POC, projection table `spending_window_projections` lag is bypassed
//! by reading entries directly. Production reads from projection.
//!
//! Snapshot semantics per Ledger §8: `effective_at <= snapshot_at` upper
//! bound; lower bound from `budget_window_instances.boundary_start`
//! (inclusive) and upper bound `boundary_end` (exclusive, when set).
//! NULL boundaries treated as +/- infinity (Codex round 4 K2.2 fix).

use sqlx::{PgPool, Row};
use tonic::Status;
use tracing::instrument;
use uuid::Uuid;

use crate::{
    domain::error::DomainError,
    proto::{
        common::v1::UnitRef,
        ledger::v1::{QueryBudgetStateRequest, QueryBudgetStateResponse},
    },
};

#[instrument(skip(pool, req), fields(
    tenant = %req.tenant_id,
    budget_id = %req.budget_id,
    window = %req.window_instance_id,
    unit = %req.unit_id,
))]
pub async fn handle(
    pool: &PgPool,
    req: QueryBudgetStateRequest,
) -> Result<QueryBudgetStateResponse, Status> {
    handle_inner(pool, req).await.map_err(|e| match e {
        DomainError::Internal(err) => Status::internal(err.to_string()),
        DomainError::Db(err) => Status::unavailable(format!("db: {err}")),
        DomainError::InvalidRequest(d) => Status::invalid_argument(d),
        other => other.to_status(),
    })
}

async fn handle_inner(
    pool: &PgPool,
    req: QueryBudgetStateRequest,
) -> Result<QueryBudgetStateResponse, DomainError> {
    if req.tenant_id.is_empty()
        || req.budget_id.is_empty()
        || req.window_instance_id.is_empty()
        || req.unit_id.is_empty()
    {
        return Err(DomainError::InvalidRequest(
            "tenant_id + budget_id + window_instance_id + unit_id required".into(),
        ));
    }
    let tenant_id = Uuid::parse_str(&req.tenant_id)
        .map_err(|e| DomainError::InvalidRequest(format!("tenant_id: {e}")))?;
    let budget_id = Uuid::parse_str(&req.budget_id)
        .map_err(|e| DomainError::InvalidRequest(format!("budget_id: {e}")))?;
    let window_id = Uuid::parse_str(&req.window_instance_id)
        .map_err(|e| DomainError::InvalidRequest(format!("window_instance_id: {e}")))?;
    let unit_id = Uuid::parse_str(&req.unit_id)
        .map_err(|e| DomainError::InvalidRequest(format!("unit_id: {e}")))?;

    let snapshot_at = req
        .snapshot_at
        .as_ref()
        .ok_or_else(|| DomainError::InvalidRequest("snapshot_at required (Ledger §8)".into()))?;
    let snapshot_ts = chrono::DateTime::<chrono::Utc>::from_timestamp(
        snapshot_at.seconds,
        snapshot_at.nanos as u32,
    )
    .ok_or_else(|| DomainError::InvalidRequest("snapshot_at out of range".into()))?;

    // Sum entries grouped by account_kind. Inclusive boundary_start,
    // exclusive boundary_end; NULL boundaries treated as -inf/+inf.
    let rows = sqlx::query(
        "SELECT la.account_kind,
                COALESCE(SUM(CASE WHEN le.direction='debit'  THEN le.amount_atomic
                                  WHEN le.direction='credit' THEN -le.amount_atomic
                             END), 0)::TEXT AS net_debit
           FROM ledger_entries le
           JOIN ledger_accounts la
             ON le.ledger_account_id = la.ledger_account_id
           JOIN budget_window_instances bwi
             ON bwi.window_instance_id = le.window_instance_id
          WHERE le.tenant_id = $1
            AND la.budget_id = $2
            AND le.window_instance_id = $3
            AND la.unit_id = $4
            AND le.effective_at <= $5
            AND (bwi.boundary_start IS NULL OR le.effective_at >= bwi.boundary_start)
            AND (bwi.boundary_end IS NULL OR le.effective_at <  bwi.boundary_end)
          GROUP BY la.account_kind",
    )
    .bind(tenant_id)
    .bind(budget_id)
    .bind(window_id)
    .bind(unit_id)
    .bind(snapshot_ts)
    .fetch_all(pool)
    .await
    .map_err(|e| DomainError::Internal(anyhow::anyhow!("query budget state: {e}")))?;

    let mut available = "0".to_string();
    let mut reserved_hold = "0".to_string();
    let mut committed_spend = "0".to_string();
    let mut debt = "0".to_string();
    let mut adjustment = "0".to_string();
    let mut refund_credit = "0".to_string();

    // Account orientation per Ledger §3 / §10:
    //   credit-positive accounts (balance = -net_debit):
    //     available_budget, reserved_hold, committed_spend, refund_credit
    //   debit-positive accounts (balance = +net_debit):
    //     debt, adjustment, dispute_adjustment
    use num_bigint::BigInt;
    for row in rows {
        let kind: String = row.get("account_kind");
        let net_str: String = row.get("net_debit");
        let net = net_str.parse::<BigInt>().unwrap_or_else(|_| BigInt::from(0));
        let balance = match kind.as_str() {
            "available_budget" | "reserved_hold" | "committed_spend" | "refund_credit" => -&net,
            _ => net,
        };
        let s = balance.to_string();
        match kind.as_str() {
            "available_budget" => available = s,
            "reserved_hold" => reserved_hold = s,
            "committed_spend" => committed_spend = s,
            "debt" => debt = s,
            "adjustment" => adjustment = s,
            "refund_credit" => refund_credit = s,
            _ => {}
        }
    }

    // Counts: reservations active in this (budget, window, unit); commits
    // estimated for matching scope.
    let res_count: i64 = sqlx::query(
        "SELECT COUNT(*)::BIGINT AS n FROM reservations r \
          WHERE r.tenant_id = $1 AND r.budget_id = $2 \
            AND r.window_instance_id = $3 AND r.current_state = 'reserved' \
            AND EXISTS (\
              SELECT 1 FROM ledger_entries le \
              JOIN ledger_accounts la ON le.ledger_account_id = la.ledger_account_id \
              WHERE le.reservation_id = r.reservation_id AND la.unit_id = $4)",
    )
    .bind(tenant_id)
    .bind(budget_id)
    .bind(window_id)
    .bind(unit_id)
    .fetch_one(pool)
    .await
    .map(|r| r.get::<i64, _>("n"))
    .map_err(|e| DomainError::Internal(anyhow::anyhow!("reservation_count: {e}")))?;

    let commit_count: i64 = sqlx::query(
        "SELECT COUNT(*)::BIGINT AS n FROM commits c \
           JOIN reservations r ON c.reservation_id = r.reservation_id \
          WHERE c.tenant_id = $1 AND c.budget_id = $2 \
            AND r.window_instance_id = $3 AND c.unit_id = $4 \
            AND c.latest_state = 'estimated'",
    )
    .bind(tenant_id)
    .bind(budget_id)
    .bind(window_id)
    .bind(unit_id)
    .fetch_one(pool)
    .await
    .map(|r| r.get::<i64, _>("n"))
    .map_err(|e| DomainError::Internal(anyhow::anyhow!("commit_count: {e}")))?;

    Ok(QueryBudgetStateResponse {
        unit: Some(UnitRef {
            unit_id: unit_id.to_string(),
            ..Default::default()
        }),
        available_atomic: available,
        reserved_hold_atomic: reserved_hold,
        committed_spend_atomic: committed_spend,
        debt_atomic: debt,
        adjustment_atomic: adjustment,
        refund_credit_atomic: refund_credit,
        reservation_count: res_count as u64,
        commit_count: commit_count as u64,
        as_of: Some(prost_types::Timestamp {
            seconds: snapshot_ts.timestamp(),
            nanos: snapshot_ts.timestamp_subsec_nanos() as i32,
        }),
    })
}
