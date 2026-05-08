use thiserror::Error;
use tonic::Status;

use crate::proto::common::v1::{error::Code as ProtoCode, Error as ProtoError};

#[derive(Debug, Error)]
pub enum DomainError {
    #[error("trust bootstrap failed: {0}")]
    TrustBootstrap(String),

    #[error("manifest signature invalid: {0}")]
    ManifestSignatureInvalid(String),

    #[error("manifest critical_max_stale exceeded: {0}")]
    ManifestStale(String),

    #[error("bundle signature invalid: {0}")]
    BundleSignatureInvalid(String),

    #[error("fencing acquire failed: {0}")]
    FencingAcquire(String),

    #[error("fencing epoch stale: {0}")]
    FencingEpochStale(String),

    #[error("reservation state conflict: {0}")]
    ReservationStateConflict(String),

    #[error("reservation TTL expired: {0}")]
    ReservationTtlExpired(String),

    #[error("pricing freeze mismatch: {0}")]
    PricingFreezeMismatch(String),

    #[error("overrun reservation: {0}")]
    OverrunReservation(String),

    #[error("multi-reservation commit deferred: {0}")]
    MultiReservationCommitDeferred(String),

    #[error("reservation not found: {0}")]
    ReservationNotFound(String),

    #[error("ledger client: {0}")]
    LedgerClient(String),

    #[error("canonical ingest client: {0}")]
    CanonicalIngestClient(String),

    #[error("decision transaction stage failed: {0}")]
    DecisionStage(String),

    #[error("audit invariant violated: {0}")]
    AuditInvariantViolated(String),

    #[error("draining; refusing new decisions")]
    Draining,

    #[error("invalid request: {0}")]
    InvalidRequest(String),

    #[error("internal: {0}")]
    Internal(#[from] anyhow::Error),
}

impl DomainError {
    /// Map to wire `Error` payload for TraceEventAck / oneof Outcome::Error.
    pub fn to_proto(&self) -> ProtoError {
        let (code, summary) = match self {
            DomainError::FencingEpochStale(d) => (ProtoCode::FencingEpochStale, d.clone()),
            DomainError::ReservationStateConflict(d) => {
                (ProtoCode::ReservationStateConflict, d.clone())
            }
            DomainError::ReservationTtlExpired(d) => (ProtoCode::ReservationTtlExpired, d.clone()),
            DomainError::PricingFreezeMismatch(d) => (ProtoCode::PricingFreezeMismatch, d.clone()),
            DomainError::OverrunReservation(d) => (ProtoCode::OverrunReservation, d.clone()),
            DomainError::MultiReservationCommitDeferred(d) => {
                (ProtoCode::MultiReservationCommitDeferred, d.clone())
            }
            DomainError::ReservationNotFound(d) => (ProtoCode::Unspecified, d.clone()),
            DomainError::InvalidRequest(d) => (ProtoCode::Unspecified, d.clone()),
            DomainError::TrustBootstrap(d)
            | DomainError::ManifestSignatureInvalid(d)
            | DomainError::BundleSignatureInvalid(d) => (ProtoCode::SignatureInvalid, d.clone()),
            DomainError::ManifestStale(d) => (ProtoCode::Unspecified, d.clone()),
            DomainError::FencingAcquire(d) => (ProtoCode::FencingEpochStale, d.clone()),
            DomainError::LedgerClient(d) | DomainError::CanonicalIngestClient(d) => {
                (ProtoCode::Unspecified, d.clone())
            }
            DomainError::DecisionStage(d) | DomainError::AuditInvariantViolated(d) => {
                (ProtoCode::AuditInvariantViolated, d.clone())
            }
            DomainError::Draining => (ProtoCode::Unspecified, "draining".to_string()),
            DomainError::Internal(e) => (ProtoCode::Unspecified, e.to_string()),
        };
        ProtoError {
            code: code as i32,
            message: self.to_string(),
            details: std::collections::HashMap::from([("summary".to_string(), summary)]),
        }
    }

    pub fn to_status(&self) -> Status {
        match self {
            DomainError::TrustBootstrap(_)
            | DomainError::ManifestSignatureInvalid(_)
            | DomainError::BundleSignatureInvalid(_) => Status::unauthenticated(self.to_string()),
            DomainError::ManifestStale(_) => Status::failed_precondition(self.to_string()),
            DomainError::FencingAcquire(_) | DomainError::FencingEpochStale(_) => {
                Status::failed_precondition(self.to_string())
            }
            DomainError::ReservationStateConflict(_)
            | DomainError::ReservationTtlExpired(_)
            | DomainError::PricingFreezeMismatch(_)
            | DomainError::OverrunReservation(_)
            | DomainError::MultiReservationCommitDeferred(_) => {
                Status::failed_precondition(self.to_string())
            }
            DomainError::ReservationNotFound(_) => Status::not_found(self.to_string()),
            DomainError::LedgerClient(_) | DomainError::CanonicalIngestClient(_) => {
                Status::unavailable(self.to_string())
            }
            DomainError::Draining => Status::unavailable(self.to_string()),
            DomainError::InvalidRequest(_) => Status::invalid_argument(self.to_string()),
            DomainError::DecisionStage(_)
            | DomainError::AuditInvariantViolated(_)
            | DomainError::Internal(_) => Status::internal(self.to_string()),
        }
    }
}
