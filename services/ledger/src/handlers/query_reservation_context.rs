//! `Ledger::QueryReservationContext` handler (Phase 2B Step 7).
//!
//! Sidecar uses this RPC to recover reservation context after a
//! reservation_cache miss (e.g., process restart). Returns canonical
//! truth derived from `reservations` + `ledger_transactions` +
//! `ledger_entries` JOIN `ledger_accounts`.
//!
//! Single-reservation constraint: server returns
//! `MULTI_RESERVATION_COMMIT_DEFERRED` Error when the originating
//! decision (and source ledger_transaction) created more than one
//! reservation. CommitEstimatedSet for batches is a future slice.

use sqlx::{PgPool, Row};
use tonic::Status;
use tracing::instrument;
use uuid::Uuid;

use crate::{
    domain::error::DomainError,
    proto::{
        common::v1::{PricingFreeze, UnitRef},
        ledger::v1::{
            query_reservation_context_response::Outcome, QueryReservationContextRequest,
            QueryReservationContextResponse, ReservationContext,
        },
    },
};

#[instrument(skip(pool, req), fields(
    tenant = %req.tenant_id,
    reservation_id = %req.reservation_id,
))]
pub async fn handle(
    pool: &PgPool,
    req: QueryReservationContextRequest,
) -> Result<QueryReservationContextResponse, Status> {
    match handle_inner(pool, req).await {
        Ok(resp) => Ok(resp),
        Err(DomainError::Internal(e)) => Err(Status::internal(e.to_string())),
        Err(DomainError::Db(e)) => Err(Status::unavailable(format!("db: {}", e))),
        Err(other) => {
            let err = other.to_proto();
            Ok(QueryReservationContextResponse {
                outcome: Some(Outcome::Error(err)),
            })
        }
    }
}

async fn handle_inner(
    pool: &PgPool,
    req: QueryReservationContextRequest,
) -> Result<QueryReservationContextResponse, DomainError> {
    if req.tenant_id.is_empty() || req.reservation_id.is_empty() {
        return Err(DomainError::InvalidRequest(
            "tenant_id + reservation_id required".into(),
        ));
    }
    let tenant_id = Uuid::parse_str(&req.tenant_id)
        .map_err(|e| DomainError::InvalidRequest(format!("tenant_id: {e}")))?;
    let reservation_id = Uuid::parse_str(&req.reservation_id)
        .map_err(|e| DomainError::InvalidRequest(format!("reservation_id: {e}")))?;

    // 1) Look up reservation + source tx fencing
    let row = sqlx::query(
        "SELECT r.reservation_id, r.tenant_id, r.budget_id, r.window_instance_id,
                r.current_state, r.ttl_expires_at,
                r.source_ledger_transaction_id,
                lt.fencing_scope_id, lt.fencing_epoch_at_post,
                lt.decision_id
           FROM reservations r
           JOIN ledger_transactions lt
             ON lt.ledger_transaction_id = r.source_ledger_transaction_id
          WHERE r.tenant_id = $1
            AND r.reservation_id = $2",
    )
    .bind(tenant_id)
    .bind(reservation_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| DomainError::Internal(anyhow::anyhow!("reservations lookup: {e}")))?;
    let row = row.ok_or_else(|| {
        DomainError::ReservationNotFound(format!(
            "reservation_id {} not found for tenant {}",
            reservation_id, tenant_id
        ))
    })?;

    let source_ltx: Uuid = row.get("source_ledger_transaction_id");

    // 2) Reject multi-reservation decision (Step 7 limitation)
    let count_row = sqlx::query(
        "SELECT COUNT(*) AS n FROM reservations \
          WHERE tenant_id = $1 AND source_ledger_transaction_id = $2",
    )
    .bind(tenant_id)
    .bind(source_ltx)
    .fetch_one(pool)
    .await
    .map_err(|e| DomainError::Internal(anyhow::anyhow!("multi-reservation count: {e}")))?;
    let n: i64 = count_row.get("n");
    if n > 1 {
        return Err(DomainError::MultiReservationCommitDeferred(format!(
            "source_ledger_transaction {} created {} reservations; CommitEstimatedSet RPC is a future slice",
            source_ltx, n
        )));
    }

    // 3) Look up the original reserve credit entry (account_kind via JOIN
    //    ledger_accounts; M1.1 fix from Codex round 2).
    let entry_row = sqlx::query(
        "SELECT le.amount_atomic::TEXT AS amount,
                la.unit_id,
                le.pricing_version,
                le.price_snapshot_hash,
                le.fx_rate_version,
                le.unit_conversion_version
           FROM ledger_entries le
           JOIN ledger_accounts la
             ON le.ledger_account_id = la.ledger_account_id
          WHERE le.tenant_id = $1
            AND le.reservation_id = $2
            AND la.account_kind = 'reserved_hold'
            AND le.direction = 'credit'
          LIMIT 1",
    )
    .bind(tenant_id)
    .bind(reservation_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| DomainError::Internal(anyhow::anyhow!("ledger_entries lookup: {e}")))?
    .ok_or_else(|| {
        DomainError::Internal(anyhow::anyhow!(
            "no reserved_hold credit entry for reservation {}",
            reservation_id
        ))
    })?;

    let unit_id: Uuid = entry_row.get("unit_id");
    let amount_str: String = entry_row.get("amount");
    let pricing_version: String = entry_row.get("pricing_version");
    let price_snapshot_hash: Vec<u8> = entry_row.get("price_snapshot_hash");
    let fx_rate_version: String = entry_row.get("fx_rate_version");
    let unit_conversion_version: String = entry_row.get("unit_conversion_version");

    let budget_id: Uuid = row.get("budget_id");
    let window_instance_id: Uuid = row.get("window_instance_id");
    let current_state: String = row.get("current_state");
    let ttl_ts: chrono::DateTime<chrono::Utc> = row.get("ttl_expires_at");
    let fencing_scope_id: Uuid = row.get("fencing_scope_id");
    let fencing_epoch_at_post: i64 = row.get("fencing_epoch_at_post");
    let decision_id: Uuid = row.get("decision_id");

    let context = ReservationContext {
        reservation_id: reservation_id.to_string(),
        budget_id: budget_id.to_string(),
        window_instance_id: window_instance_id.to_string(),
        unit: Some(UnitRef {
            unit_id: unit_id.to_string(),
            ..Default::default()
        }),
        original_reserved_amount_atomic: amount_str,
        pricing: Some(PricingFreeze {
            pricing_version,
            price_snapshot_hash: price_snapshot_hash.into(),
            fx_rate_version,
            unit_conversion_version,
        }),
        fencing_scope_id: fencing_scope_id.to_string(),
        fencing_epoch_at_post: fencing_epoch_at_post as u64,
        decision_id: decision_id.to_string(),
        ttl_expires_at: Some(prost_types::Timestamp {
            seconds: ttl_ts.timestamp(),
            nanos: ttl_ts.timestamp_subsec_nanos() as i32,
        }),
        current_state,
        source_ledger_transaction_id: source_ltx.to_string(),
        tenant_id: tenant_id.to_string(),
    };

    Ok(QueryReservationContextResponse {
        outcome: Some(Outcome::Context(context)),
    })
}
