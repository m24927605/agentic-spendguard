//! Phase 5 S3: `Ledger::AcquireFencingLease` handler.
//!
//! Wraps the `acquire_fencing_lease` SP (migration 0023). The SP runs
//! all CAS logic atomically inside a Postgres transaction; the handler
//! is responsible for input validation + protobuf <-> Rust marshaling.

use prost_types::Timestamp;
use sqlx::PgPool;
use tracing::{debug, instrument};
use uuid::Uuid;

use crate::{
    domain::error::{map_pg_error, DomainError},
    proto::ledger::v1::{
        acquire_fencing_lease_response::Outcome, AcquireFencingLeaseDenied,
        AcquireFencingLeaseRequest, AcquireFencingLeaseResponse,
        AcquireFencingLeaseSuccess,
    },
};

#[instrument(skip(pool, req), fields(
    tenant = %req.tenant_id,
    scope_id = %req.fencing_scope_id,
    workload = %req.workload_instance_id,
    ttl_secs = req.ttl_seconds,
    force = req.force
))]
pub async fn handle(
    pool: &PgPool,
    req: AcquireFencingLeaseRequest,
) -> Result<AcquireFencingLeaseResponse, tonic::Status> {
    match handle_inner(pool, req).await {
        Ok(resp) => Ok(resp),
        Err(DomainError::Internal(e)) => Err(tonic::Status::internal(e.to_string())),
        Err(DomainError::Db(e)) => Err(tonic::Status::unavailable(format!("db: {}", e))),
        Err(other) => Ok(AcquireFencingLeaseResponse {
            outcome: Some(Outcome::Error(other.to_proto())),
        }),
    }
}

async fn handle_inner(
    pool: &PgPool,
    req: AcquireFencingLeaseRequest,
) -> Result<AcquireFencingLeaseResponse, DomainError> {
    // Validate inputs at the handler boundary so the SP only sees
    // well-formed values.
    if req.fencing_scope_id.is_empty() {
        return Err(DomainError::InvalidRequest(
            "fencing_scope_id required".into(),
        ));
    }
    if req.tenant_id.is_empty() {
        return Err(DomainError::InvalidRequest("tenant_id required".into()));
    }
    if req.workload_instance_id.is_empty() {
        return Err(DomainError::InvalidRequest(
            "workload_instance_id required".into(),
        ));
    }
    if req.ttl_seconds == 0 {
        return Err(DomainError::InvalidRequest("ttl_seconds must be > 0".into()));
    }
    // Cap absurdly long TTLs to avoid operator footgun (e.g. 10 years).
    // 1 hour is generous for a sidecar lease; CIRCLE BACK in S5 if a
    // legitimate use case needs longer.
    if req.ttl_seconds > 3600 {
        return Err(DomainError::InvalidRequest(
            "ttl_seconds capped at 3600 (1 hour); pass shorter TTL + renew loop".into(),
        ));
    }

    let scope_id = Uuid::parse_str(&req.fencing_scope_id)
        .map_err(|e| DomainError::InvalidRequest(format!("fencing_scope_id: {e}")))?;
    let tenant_id = Uuid::parse_str(&req.tenant_id)
        .map_err(|e| DomainError::InvalidRequest(format!("tenant_id: {e}")))?;
    let audit_event_id = if req.audit_event_id.is_empty() {
        None
    } else {
        Some(
            Uuid::parse_str(&req.audit_event_id)
                .map_err(|e| DomainError::InvalidRequest(format!("audit_event_id: {e}")))?,
        )
    };

    // Call SP. Returns one row.
    let row: (
        bool,
        i64,
        chrono::DateTime<chrono::Utc>,
        String,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT granted, new_epoch, expires_at, action, holder_instance_id \
           FROM acquire_fencing_lease($1, $2, $3, $4::INT, $5, $6)",
    )
    .bind(scope_id)
    .bind(tenant_id)
    .bind(&req.workload_instance_id)
    .bind(req.ttl_seconds as i32)
    .bind(req.force)
    .bind(audit_event_id)
    .fetch_one(pool)
    .await
    .map_err(map_pg_error)?;

    let (granted, epoch, ttl_expires, action, holder) = row;

    if granted {
        debug!(action = %action, epoch, "fencing lease granted");
        Ok(AcquireFencingLeaseResponse {
            outcome: Some(Outcome::Success(AcquireFencingLeaseSuccess {
                epoch: epoch as u64,
                ttl_expires_at: Some(Timestamp {
                    seconds: ttl_expires.timestamp(),
                    nanos: ttl_expires.timestamp_subsec_nanos() as i32,
                }),
                action,
            })),
        })
    } else {
        debug!(holder = ?holder, "fencing lease denied");
        Ok(AcquireFencingLeaseResponse {
            outcome: Some(Outcome::Denied(AcquireFencingLeaseDenied {
                current_holder_instance_id: holder.unwrap_or_default(),
                current_epoch: epoch as u64,
                current_ttl_expires_at: Some(Timestamp {
                    seconds: ttl_expires.timestamp(),
                    nanos: ttl_expires.timestamp_subsec_nanos() as i32,
                }),
            })),
        })
    }
}
