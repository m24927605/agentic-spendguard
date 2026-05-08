use thiserror::Error;
use tonic::Status;

#[derive(Debug, Error)]
pub enum DomainError {
    #[error("schema bundle unknown: {0}")]
    SchemaBundleUnknown(String),

    #[error("schema bundle hash mismatch: {0}")]
    SchemaBundleHashMismatch(String),

    #[error("signature invalid: {0}")]
    SignatureInvalid(String),

    #[error("AWAITING_PRECEDING_DECISION: {0}")]
    AwaitingPrecedingDecision(String),

    #[error("orphan outcome (timeout): {0}")]
    OrphanOutcome(String),

    #[error("DUPLICATE: {0}")]
    Duplicate(String),

    #[error("backpressure")]
    Backpressure,

    #[error("invalid request: {0}")]
    InvalidRequest(String),

    #[error("internal: {0}")]
    Internal(#[from] anyhow::Error),

    #[error("postgres: {0}")]
    Db(#[from] sqlx::Error),
}

impl DomainError {
    pub fn to_status(&self) -> Status {
        match self {
            DomainError::Backpressure => Status::resource_exhausted(self.to_string()),
            DomainError::SignatureInvalid(_) => Status::unauthenticated(self.to_string()),
            DomainError::SchemaBundleUnknown(_)
            | DomainError::SchemaBundleHashMismatch(_)
            | DomainError::InvalidRequest(_) => Status::invalid_argument(self.to_string()),
            DomainError::Db(_) | DomainError::Internal(_) => Status::internal(self.to_string()),
            // Per-event statuses flow through EventResult instead of gRPC
            // Status; if they bubble here it's a programming error.
            DomainError::AwaitingPrecedingDecision(_)
            | DomainError::OrphanOutcome(_)
            | DomainError::Duplicate(_) => {
                Status::internal(format!("BUG: per-event status escaped: {}", self))
            }
        }
    }
}

pub fn map_pg_error(err: sqlx::Error) -> DomainError {
    if let Some(db_err) = err.as_database_error() {
        if let Some(code) = db_err.code() {
            let msg = db_err.message();
            let constraint = db_err.constraint().unwrap_or("").to_string();
            return match code.as_ref() {
                "P0002" if msg.contains("AWAITING_PRECEDING_DECISION") => {
                    DomainError::AwaitingPrecedingDecision(msg.to_string())
                }
                "23505" => {
                    // Per-decision uniqueness (audit chain).
                    if constraint.contains("canonical_global_one_decision")
                        || constraint.contains("canonical_global_one_outcome")
                    {
                        DomainError::Duplicate(format!("{}: {}", constraint, msg))
                    } else if constraint.contains("canonical_ingest_positions_pkey") {
                        DomainError::Internal(anyhow::anyhow!(
                            "ingest position collision: {}: {}",
                            constraint, msg
                        ))
                    } else if constraint.is_empty() {
                        // event_id PK on global_keys — handled by ON CONFLICT,
                        // shouldn't bubble here; if it does, treat as duplicate.
                        DomainError::Duplicate(msg.to_string())
                    } else {
                        DomainError::Internal(anyhow::anyhow!(
                            "unique violation {}: {}",
                            constraint, msg
                        ))
                    }
                }
                "42P10" => DomainError::Internal(anyhow::anyhow!(
                    "immutability trigger: {}",
                    msg
                )),
                _ => DomainError::Db(err),
            };
        }
    }
    DomainError::Db(err)
}
