//! `Ledger::GetApprovalForResume` handler (Round-2 #9 part 2 PR 9b).
//!
//! Sidecar's `ResumeAfterApproval` flow calls this RPC to read an
//! `approval_requests` row's resume context. The response carries:
//!
//!   * the current `state` (pending / approved / denied / expired /
//!     cancelled) — sidecar branches based on this;
//!   * `decision_context_json` — the matched_rule_ids + reason_codes +
//!     ContractBundleRef + PricingFreeze tuple captured at
//!     REQUIRE_APPROVAL time;
//!   * `requested_effect_json` — the proposed budget effect envelope;
//!   * `bundled_ledger_transaction_id` (optional) — set when the
//!     resume path has already landed (idempotent).
//!
//! Tenant scope is enforced server-side: the response is `Error` when
//! the caller's `tenant_id` doesn't match the row's tenant.

use sqlx::{PgPool, Row};
use tonic::Status;
use tracing::instrument;
use uuid::Uuid;

use crate::proto::{
    common::v1::{error::Code as ErrorCode, Error as ProtoError},
    ledger::v1::{
        get_approval_for_resume_response::Outcome, ApprovalResumeContext,
        GetApprovalForResumeRequest, GetApprovalForResumeResponse,
    },
};

#[instrument(skip(pool, req), fields(
    tenant = %req.tenant_id,
    approval_id = %req.approval_id,
))]
pub async fn handle(
    pool: &PgPool,
    req: GetApprovalForResumeRequest,
) -> Result<GetApprovalForResumeResponse, Status> {
    let approval_id = match Uuid::parse_str(&req.approval_id) {
        Ok(u) => u,
        Err(e) => {
            return Ok(invalid(format!("[INVALID_REQUEST] approval_id parse: {e}")));
        }
    };
    let tenant_id = match Uuid::parse_str(&req.tenant_id) {
        Ok(u) => u,
        Err(e) => {
            return Ok(invalid(format!("[INVALID_REQUEST] tenant_id parse: {e}")));
        }
    };

    let row = sqlx::query(
        r#"
        SELECT approval_id, tenant_id, decision_id, state,
               decision_context, requested_effect,
               bundled_ledger_transaction_id
        FROM approval_requests
        WHERE approval_id = $1
        "#,
    )
    .bind(approval_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| Status::unavailable(format!("db: {e}")))?;

    let row = match row {
        Some(r) => r,
        None => {
            return Ok(GetApprovalForResumeResponse {
                outcome: Some(Outcome::Error(ProtoError {
                    code: ErrorCode::Unspecified as i32,
                    message: format!("[NOT_FOUND] approval {approval_id} not found"),
                    details: Default::default(),
                })),
            });
        }
    };

    // Tenant scope check.
    let row_tenant: Uuid = row
        .try_get("tenant_id")
        .map_err(|e| Status::internal(format!("decode tenant_id: {e}")))?;
    if row_tenant != tenant_id {
        return Ok(GetApprovalForResumeResponse {
            outcome: Some(Outcome::Error(ProtoError {
                code: ErrorCode::Unspecified as i32,
                message: "[PERMISSION_DENIED] tenant_id mismatch".into(),
                details: Default::default(),
            })),
        });
    }

    let state: String = row
        .try_get("state")
        .map_err(|e| Status::internal(format!("decode state: {e}")))?;
    let decision_id: Uuid = row
        .try_get("decision_id")
        .map_err(|e| Status::internal(format!("decode decision_id: {e}")))?;
    let decision_context_json: serde_json::Value = row
        .try_get("decision_context")
        .map_err(|e| Status::internal(format!("decode decision_context: {e}")))?;
    let requested_effect_json: serde_json::Value = row
        .try_get("requested_effect")
        .map_err(|e| Status::internal(format!("decode requested_effect: {e}")))?;
    let bundled_tx: Option<Uuid> = row
        .try_get("bundled_ledger_transaction_id")
        .map_err(|e| {
            Status::internal(format!("decode bundled_ledger_transaction_id: {e}"))
        })?;

    Ok(GetApprovalForResumeResponse {
        outcome: Some(Outcome::Context(ApprovalResumeContext {
            approval_id: approval_id.to_string(),
            tenant_id: tenant_id.to_string(),
            decision_id: decision_id.to_string(),
            state,
            decision_context_json: serde_json::to_vec(&decision_context_json)
                .unwrap_or_default()
                .into(),
            requested_effect_json: serde_json::to_vec(&requested_effect_json)
                .unwrap_or_default()
                .into(),
            bundled_ledger_transaction_id: bundled_tx
                .map(|u| u.to_string())
                .unwrap_or_default(),
        })),
    })
}

fn invalid(msg: String) -> GetApprovalForResumeResponse {
    GetApprovalForResumeResponse {
        outcome: Some(Outcome::Error(ProtoError {
            code: ErrorCode::Unspecified as i32,
            message: msg,
            details: Default::default(),
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_request_returns_typed_error() {
        let resp = invalid("approval_id parse: nope".into());
        match resp.outcome.unwrap() {
            Outcome::Error(e) => {
                assert_eq!(e.code, ErrorCode::Unspecified as i32);
                assert!(e.message.contains("approval_id parse"));
            }
            other => panic!("expected Error; got {other:?}"),
        }
    }
}
