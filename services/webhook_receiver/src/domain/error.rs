//! Receiver-internal error types + HTTP mapping.

use axum::http::StatusCode;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ReceiverError {
    #[error("invalid request: {0}")]
    InvalidRequest(String),

    #[error("unknown provider")]
    UnknownProvider,

    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("conflict: canonical hash mismatch")]
    Conflict,

    #[error("ledger business rejection: {0}")]
    LedgerBusiness(String),

    #[error("ledger transient: {0}")]
    LedgerTransient(String),

    #[error("ledger unspecified: {0}")]
    LedgerUnspecified(String),

    /// Carry the exact StatusCode through (Codex challenge P2.1).
    /// Used when the ledger Error.Code maps to a status that doesn't
    /// fit the coarse Business/Transient/Unspecified buckets (e.g. 500
    /// AUDIT_INVARIANT_VIOLATED, 501 MULTI_RESERVATION_COMMIT_DEFERRED,
    /// 401 UNAUTHORIZED).
    #[error("ledger error: {message}")]
    LedgerExact { status: StatusCode, message: String },

    #[error("internal: {0}")]
    Internal(#[from] anyhow::Error),

    #[error("postgres: {0}")]
    Db(#[from] sqlx::Error),
}

impl ReceiverError {
    pub fn status(&self) -> StatusCode {
        match self {
            ReceiverError::InvalidRequest(_) => StatusCode::BAD_REQUEST,
            ReceiverError::UnknownProvider => StatusCode::NOT_FOUND,
            ReceiverError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            ReceiverError::Conflict => StatusCode::CONFLICT,
            ReceiverError::LedgerBusiness(_) => StatusCode::UNPROCESSABLE_ENTITY,
            ReceiverError::LedgerTransient(_) => StatusCode::SERVICE_UNAVAILABLE,
            ReceiverError::LedgerUnspecified(_) => StatusCode::BAD_GATEWAY,
            ReceiverError::LedgerExact { status, .. } => *status,
            ReceiverError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            ReceiverError::Db(_) => StatusCode::SERVICE_UNAVAILABLE,
        }
    }
}

/// Map ledger Error.Code (proto enum) → HTTP status (v3 §Δ4).
pub fn ledger_code_to_http(code: i32, message: &str) -> StatusCode {
    use crate::proto::common::v1::error::Code as PC;
    let parsed = PC::try_from(code).unwrap_or(PC::Unspecified);
    match parsed {
        PC::FencingEpochStale | PC::LockOrderTokenMismatch => StatusCode::SERVICE_UNAVAILABLE,
        PC::DeadlockTimeout | PC::SyncReplicaUnavailable => StatusCode::SERVICE_UNAVAILABLE,
        PC::PricingVersionUnknown
        | PC::UnitNormalizationRequired
        | PC::BudgetExhausted
        | PC::SchemaBundleUnknown
        | PC::ReservationStateConflict
        | PC::PricingFreezeMismatch
        | PC::OverrunReservation
        | PC::ReservationTtlExpired => StatusCode::UNPROCESSABLE_ENTITY,
        PC::TenantDisabled | PC::SignatureInvalid => StatusCode::UNAUTHORIZED,
        PC::AuditInvariantViolated | PC::DuplicateDecisionEvent => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
        PC::MultiReservationCommitDeferred => StatusCode::NOT_IMPLEMENTED,
        PC::Unspecified => {
            // Receiver guardrail v3 V3.1: ledger maps "idempotency_key reused"
            // into Code::Unspecified. Detect by substring in either Error.message
            // or details["summary"]. Webhook handler resolves this BEFORE
            // calling this mapper for the body-conflict case (returns 409
            // explicitly via Conflict). If we reach here with that text,
            // something upstream mis-routed; return 409 defensively.
            if message.contains("idempotency_key reused") {
                StatusCode::CONFLICT
            } else {
                StatusCode::BAD_GATEWAY
            }
        }
    }
}
