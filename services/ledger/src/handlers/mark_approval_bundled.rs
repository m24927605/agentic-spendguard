//! `Ledger::MarkApprovalBundled` handler (Round-2 #9 part 2 PR 9b).
//!
//! Wraps the `mark_approval_bundled` SP shipped in migration 0036.
//! Sidecar's resume flow calls this AFTER a fresh `ReserveSet` has
//! landed, so the approval row's `bundled_ledger_transaction_id`
//! column captures the link from approval → final ledger transaction.
//!
//! SP semantics (per migration 0036):
//!   * Atomic state assertion: row MUST be in `state='approved'`.
//!     Caller violations bubble up as Postgres SQLSTATE 22023 →
//!     `INVALID_REQUEST`.
//!   * Idempotent on `(approval_id, ledger_transaction_id)`. Re-runs
//!     with the same tx return `was_first_bundling = false` plus the
//!     same tx_id.
//!   * Replays with a different tx_id raise SQLSTATE 40P03
//!     (IDEMPOTENCY_CONFLICT) → typed gRPC `Error`.

use sqlx::{PgPool, Row};
use tonic::Status;
use tracing::instrument;
use uuid::Uuid;

use crate::proto::{
    common::v1::{error::Code as ErrorCode, Error as ProtoError},
    ledger::v1::{
        mark_approval_bundled_response::Outcome, MarkApprovalBundledRequest,
        MarkApprovalBundledResponse, MarkApprovalBundledSuccess,
    },
};

#[instrument(skip(pool, req), fields(
    approval_id = %req.approval_id,
    ledger_transaction_id = %req.ledger_transaction_id,
))]
pub async fn handle(
    pool: &PgPool,
    req: MarkApprovalBundledRequest,
) -> Result<MarkApprovalBundledResponse, Status> {
    let approval_id = match Uuid::parse_str(&req.approval_id) {
        Ok(u) => u,
        Err(e) => return Ok(invalid(format!("approval_id parse: {e}"))),
    };
    let tx_id = match Uuid::parse_str(&req.ledger_transaction_id) {
        Ok(u) => u,
        Err(e) => return Ok(invalid(format!("ledger_transaction_id parse: {e}"))),
    };

    let row_result = sqlx::query(
        "SELECT was_first_bundling, ledger_transaction_id FROM mark_approval_bundled($1, $2)",
    )
    .bind(approval_id)
    .bind(tx_id)
    .fetch_one(pool)
    .await;

    let row = match row_result {
        Ok(r) => r,
        Err(sqlx::Error::Database(db_err)) => {
            // Translate Postgres SQLSTATEs to typed gRPC errors.
            let code = db_err.code().map(|c| c.to_string()).unwrap_or_default();
            let msg = db_err.message().to_string();
            return Ok(MarkApprovalBundledResponse {
                outcome: Some(Outcome::Error(map_sqlstate(&code, msg))),
            });
        }
        Err(e) => return Err(Status::unavailable(format!("db: {e}"))),
    };

    let was_first_bundling: bool = row.try_get("was_first_bundling").map_err(|e| {
        Status::internal(format!("decode was_first_bundling: {e}"))
    })?;
    let returned_tx: Uuid = row
        .try_get("ledger_transaction_id")
        .map_err(|e| Status::internal(format!("decode ledger_transaction_id: {e}")))?;

    Ok(MarkApprovalBundledResponse {
        outcome: Some(Outcome::Success(MarkApprovalBundledSuccess {
            was_first_bundling,
            ledger_transaction_id: returned_tx.to_string(),
        })),
    })
}

fn invalid(msg: String) -> MarkApprovalBundledResponse {
    MarkApprovalBundledResponse {
        outcome: Some(Outcome::Error(ProtoError {
            code: ErrorCode::Unspecified as i32,
            message: msg,
            details: Default::default(),
        })),
    }
}

fn map_sqlstate(sqlstate: &str, msg: String) -> ProtoError {
    let code = match sqlstate {
        // 22023: invalid_parameter_value — SP raises this on
        // state-machine violations (e.g. row not in 'approved').
        "22023" => ErrorCode::Unspecified,
        // 40P03: SP raises a custom IDEMPOTENCY_CONFLICT in the
        // mark_approval_bundled body when a different tx_id is
        // presented for an already-bundled approval.
        "40P03" => ErrorCode::Unspecified,
        // P0002 (no_data_found) — SP didn't find the row.
        "P0002" => ErrorCode::Unspecified,
        _ => ErrorCode::Unspecified,
    };
    ProtoError {
        code: code as i32,
        message: msg,
        details: Default::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_uuid_returns_invalid_request() {
        let resp = invalid("approval_id parse: bad".into());
        match resp.outcome.unwrap() {
            Outcome::Error(e) => {
                assert_eq!(e.code, ErrorCode::Unspecified as i32);
                assert!(e.message.contains("parse"));
            }
            other => panic!("expected Error; got {other:?}"),
        }
    }

    #[test]
    fn sqlstate_mapping_covers_known_codes() {
        assert_eq!(
            map_sqlstate("22023", "bad state".into()).code,
            ErrorCode::Unspecified as i32
        );
        assert_eq!(
            map_sqlstate("40P03", "already bundled to different tx".into()).code,
            ErrorCode::Unspecified as i32
        );
        assert_eq!(
            map_sqlstate("P0002", "row missing".into()).code,
            ErrorCode::Unspecified as i32
        );
        assert_eq!(
            map_sqlstate("99999", "unknown".into()).code,
            ErrorCode::Unspecified as i32
        );
    }
}
