//! Domain error model and Postgres error -> proto Error.Code mapping.

use thiserror::Error;
use tonic::Status;

use crate::proto::common::v1::{error::Code as ProtoCode, Error as ProtoError};

#[derive(Debug, Error)]
pub enum DomainError {
    #[error("FENCING_EPOCH_STALE: {0}")]
    FencingEpochStale(String),

    #[error("LOCK_ORDER_TOKEN_MISMATCH: {0}")]
    LockOrderTokenMismatch(String),

    #[error("PRICING_VERSION_UNKNOWN: {0}")]
    PricingVersionUnknown(String),

    #[error("UNIT_NORMALIZATION_REQUIRED: {0}")]
    UnitNormalizationRequired(String),

    #[error("BUDGET_EXHAUSTED: {0}")]
    BudgetExhausted(String),

    #[error("DEADLOCK_TIMEOUT")]
    DeadlockTimeout,

    #[error("SERIALIZATION_FAILURE")]
    SerializationFailure,

    #[error("SYNC_REPLICA_UNAVAILABLE")]
    SyncReplicaUnavailable,

    #[error("TENANT_DISABLED: {0}")]
    TenantDisabled(String),

    #[error("SCHEMA_BUNDLE_UNKNOWN: {0}")]
    SchemaBundleUnknown(String),

    #[error("SIGNATURE_INVALID: {0}")]
    SignatureInvalid(String),

    #[error("AUDIT_INVARIANT_VIOLATED: {0}")]
    AuditInvariantViolated(String),

    #[error("DUPLICATE_DECISION_EVENT: {0}")]
    DuplicateDecisionEvent(String),

    #[error("RESERVATION_STATE_CONFLICT: {0}")]
    ReservationStateConflict(String),

    #[error("PRICING_FREEZE_MISMATCH: {0}")]
    PricingFreezeMismatch(String),

    #[error("OVERRUN_RESERVATION: {0}")]
    OverrunReservation(String),

    #[error("RESERVATION_TTL_EXPIRED: {0}")]
    ReservationTtlExpired(String),

    #[error("MULTI_RESERVATION_COMMIT_DEFERRED: {0}")]
    MultiReservationCommitDeferred(String),

    #[error("RESERVATION_NOT_FOUND: {0}")]
    ReservationNotFound(String),

    #[error("idempotency_key reused with different request")]
    IdempotencyConflict,

    #[error("invalid request: {0}")]
    InvalidRequest(String),

    #[error("internal: {0}")]
    Internal(#[from] anyhow::Error),

    #[error("postgres: {0}")]
    Db(#[from] sqlx::Error),
}

impl DomainError {
    /// Map to wire `Error` payload.
    pub fn to_proto(&self) -> ProtoError {
        let (code, details) = match self {
            DomainError::FencingEpochStale(d) => (ProtoCode::FencingEpochStale, d.clone()),
            DomainError::LockOrderTokenMismatch(d) => {
                (ProtoCode::LockOrderTokenMismatch, d.clone())
            }
            DomainError::PricingVersionUnknown(d) => (ProtoCode::PricingVersionUnknown, d.clone()),
            DomainError::UnitNormalizationRequired(d) => {
                (ProtoCode::UnitNormalizationRequired, d.clone())
            }
            DomainError::BudgetExhausted(d) => (ProtoCode::BudgetExhausted, d.clone()),
            DomainError::DeadlockTimeout => (ProtoCode::DeadlockTimeout, String::new()),
            DomainError::SerializationFailure => (ProtoCode::DeadlockTimeout, "serialization_failure".to_string()),
            DomainError::SyncReplicaUnavailable => (ProtoCode::SyncReplicaUnavailable, String::new()),
            DomainError::TenantDisabled(d) => (ProtoCode::TenantDisabled, d.clone()),
            DomainError::SchemaBundleUnknown(d) => (ProtoCode::SchemaBundleUnknown, d.clone()),
            DomainError::SignatureInvalid(d) => (ProtoCode::SignatureInvalid, d.clone()),
            DomainError::AuditInvariantViolated(d) => (ProtoCode::AuditInvariantViolated, d.clone()),
            DomainError::DuplicateDecisionEvent(d) => (ProtoCode::DuplicateDecisionEvent, d.clone()),
            DomainError::ReservationStateConflict(d) => (ProtoCode::ReservationStateConflict, d.clone()),
            DomainError::PricingFreezeMismatch(d) => (ProtoCode::PricingFreezeMismatch, d.clone()),
            DomainError::OverrunReservation(d) => (ProtoCode::OverrunReservation, d.clone()),
            DomainError::ReservationTtlExpired(d) => (ProtoCode::ReservationTtlExpired, d.clone()),
            DomainError::MultiReservationCommitDeferred(d) => (ProtoCode::MultiReservationCommitDeferred, d.clone()),
            DomainError::ReservationNotFound(d) => (ProtoCode::Unspecified, d.clone()),
            DomainError::IdempotencyConflict => (
                ProtoCode::Unspecified,
                "idempotency_key reused with different request_hash".to_string(),
            ),
            DomainError::InvalidRequest(d) => (ProtoCode::Unspecified, d.clone()),
            DomainError::Internal(e) => (ProtoCode::Unspecified, e.to_string()),
            DomainError::Db(e) => (ProtoCode::Unspecified, e.to_string()),
        };

        ProtoError {
            code: code as i32,
            message: self.to_string(),
            details: std::collections::HashMap::from([(
                "summary".to_string(),
                details,
            )]),
        }
    }

    /// Convert to gRPC Status (for streaming RPCs that cannot return Error in body).
    pub fn to_status(&self) -> Status {
        match self {
            DomainError::IdempotencyConflict => Status::failed_precondition(self.to_string()),
            DomainError::FencingEpochStale(d) => {
                Status::failed_precondition(format!("{}: {}", self, d))
            }
            DomainError::LockOrderTokenMismatch(d) => {
                Status::failed_precondition(format!("{}: {}", self, d))
            }
            DomainError::DuplicateDecisionEvent(d) => {
                Status::failed_precondition(format!("{}: {}", self, d))
            }
            DomainError::ReservationStateConflict(d)
            | DomainError::PricingFreezeMismatch(d)
            | DomainError::OverrunReservation(d)
            | DomainError::ReservationTtlExpired(d)
            | DomainError::MultiReservationCommitDeferred(d) => {
                Status::failed_precondition(format!("{}: {}", self, d))
            }
            DomainError::ReservationNotFound(d) => Status::not_found(d.clone()),
            DomainError::TenantDisabled(_) | DomainError::SignatureInvalid(_) => {
                Status::unauthenticated(self.to_string())
            }
            DomainError::InvalidRequest(_)
            | DomainError::SchemaBundleUnknown(_)
            | DomainError::PricingVersionUnknown(_)
            | DomainError::UnitNormalizationRequired(_) => {
                Status::invalid_argument(self.to_string())
            }
            DomainError::BudgetExhausted(_) => Status::resource_exhausted(self.to_string()),
            DomainError::DeadlockTimeout
            | DomainError::SerializationFailure
            | DomainError::SyncReplicaUnavailable
            | DomainError::Db(_) => Status::unavailable(self.to_string()),
            DomainError::AuditInvariantViolated(_) | DomainError::Internal(_) => {
                Status::internal(self.to_string())
            }
        }
    }
}

/// Map Postgres error code (SQLSTATE) to DomainError.
///
/// Reference: PostgreSQL Error Codes (Appendix A).
pub fn map_pg_error(err: sqlx::Error) -> DomainError {
    if let Some(db_err) = err.as_database_error() {
        if let Some(code) = db_err.code() {
            let msg = db_err.message();
            return match code.as_ref() {
                // 40P01 = deadlock_detected
                "40P01" => DomainError::DeadlockTimeout,
                // 40001 = serialization_failure (SERIALIZABLE conflict)
                "40001" => DomainError::SerializationFailure,
                // Custom user-thrown SQLSTATE for fencing.
                "40P02" => DomainError::FencingEpochStale(msg.to_string()),
                // Custom user-thrown SQLSTATE for caller-supplied checks.
                "40P03" if msg.contains("LOCK_ORDER_TOKEN_MISMATCH") => {
                    DomainError::LockOrderTokenMismatch(msg.to_string())
                }
                "40P03" if msg.contains("idempotency_key reused") => {
                    DomainError::IdempotencyConflict
                }
                "40P03" => DomainError::InvalidRequest(msg.to_string()),
                // P0001 = raise_exception (default for plpgsql RAISE without
                // explicit code). Disambiguate by message text.
                "P0001" if msg.contains("PRICING_VERSION_UNKNOWN") => {
                    DomainError::PricingVersionUnknown(msg.to_string())
                }
                "P0001" if msg.contains("FENCING_EPOCH_STALE") => {
                    DomainError::FencingEpochStale(msg.to_string())
                }
                "P0001" if msg.contains("RESERVATION_STATE_CONFLICT") => {
                    DomainError::ReservationStateConflict(msg.to_string())
                }
                "P0001" if msg.contains("RESERVATION_TTL_EXPIRED") => {
                    DomainError::ReservationTtlExpired(msg.to_string())
                }
                "P0001" if msg.contains("PRICING_FREEZE_MISMATCH") => {
                    DomainError::PricingFreezeMismatch(msg.to_string())
                }
                "P0001" if msg.contains("OVERRUN_RESERVATION") => {
                    DomainError::OverrunReservation(msg.to_string())
                }
                "P0001" if msg.contains("COMMIT_ROW_DIVERGENT") => {
                    DomainError::IdempotencyConflict
                }
                // Step 8 SP raises COMMIT_NOT_FOUND when ProviderReport
                // is called before CommitEstimated; commit_lifecycle_race
                // when CAS UPDATE affects 0 rows; UNIT_MISMATCH when
                // caller unit_id differs from original reserve.
                "P0001" if msg.contains("COMMIT_NOT_FOUND") => {
                    DomainError::ReservationStateConflict(msg.to_string())
                }
                "P0001" if msg.contains("commit_lifecycle_race") => {
                    DomainError::ReservationStateConflict(msg.to_string())
                }
                "P0001" if msg.contains("UNIT_MISMATCH") => {
                    DomainError::InvalidRequest(msg.to_string())
                }
                // Step 7.5 release SP raises:
                "P0001" if msg.contains("RESERVE_NOT_FOUND") => {
                    DomainError::ReservationNotFound(msg.to_string())
                }
                "P0001" if msg.contains("RESERVATION_SET_EMPTY") => {
                    DomainError::ReservationNotFound(msg.to_string())
                }
                "P0001" if msg.contains("MULTI_RESERVATION_SET_DEFERRED") => {
                    DomainError::MultiReservationCommitDeferred(msg.to_string())
                }
                "P0001" => DomainError::Internal(anyhow::anyhow!(msg.to_string())),
                // 23514 = check_violation (per_unit balance trigger).
                "23514" => DomainError::AuditInvariantViolated(format!(
                    "balance violation: {}",
                    msg
                )),
                // 23505 = unique_violation. Disambiguate by constraint name when present.
                "23505" => map_unique_violation(db_err),
                // 22023 = invalid_parameter_value (used for input shape errors).
                "22023" => DomainError::InvalidRequest(msg.to_string()),
                // 42P10 = invalid_column_reference; also used by our immutability triggers.
                "42P10" => DomainError::AuditInvariantViolated(format!(
                    "immutability trigger fired: {}",
                    msg
                )),
                // 23P01 = exclusion_violation; treat as deadlock-like.
                "23P01" => DomainError::DeadlockTimeout,
                _ => DomainError::Db(err),
            };
        }
    }
    DomainError::Db(err)
}

fn map_unique_violation(db_err: &dyn sqlx::error::DatabaseError) -> DomainError {
    let msg = db_err.message().to_string();
    let constraint = db_err
        .constraint()
        .map(str::to_string)
        .unwrap_or_default();

    if constraint.contains("audit_outbox_global_per_decision")
        || constraint.contains("audit_outbox_decision_per_decision")
        || constraint.contains("audit_outbox_outcome_per_decision")
        || constraint.contains("audit_outbox_decision_event")
        || constraint.contains("audit_outbox_global_keys_pkey")
    {
        DomainError::DuplicateDecisionEvent(format!("{}: {}", constraint, msg))
    } else if constraint.contains("audit_outbox_global_idempotency")
        || constraint.contains("audit_outbox_idempotency")
        || constraint.contains("ledger_transactions_tenant_id_operation_kind_idempotency_key")
    {
        DomainError::IdempotencyConflict
    } else if constraint.contains("audit_outbox_global_producer_seq")
        || constraint.contains("audit_outbox_producer_seq")
    {
        DomainError::AuditInvariantViolated(format!(
            "producer_sequence reused: {}: {}",
            constraint, msg
        ))
    } else if constraint.contains("ledger_entries_partition_sequence") {
        // Should never collide given monotonic per-shard allocator.
        DomainError::Internal(anyhow::anyhow!(
            "ledger sequence collision: {}: {}",
            constraint,
            msg
        ))
    } else {
        DomainError::Db(sqlx::Error::Protocol(format!(
            "unique violation: {}: {}",
            constraint, msg
        )))
    }
}
