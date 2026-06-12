//! D41 session reservation ledger entry points.
//!
//! Postgres stored procedures in `0062_session_reservations.sql` are the
//! authority for row locks, idempotency, over-budget denial, release, and TTL
//! expiry. This module provides typed request builders for Rust callers and
//! focused integration tests.

use chrono::{DateTime, Utc};
use num_bigint::{BigInt, Sign};
use serde::Serialize;
use serde_json::Value;
use sqlx::{types::Json, PgPool};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum SessionReservationError {
    #[error("invalid session reservation request: {0}")]
    InvalidRequest(String),
    #[error("db: {0}")]
    Db(#[from] sqlx::Error),
}

#[derive(Clone, Debug, Serialize)]
pub struct PricingFreezeRef {
    pub pricing_version: String,
    pub price_snapshot_hash_hex: String,
    pub fx_rate_version: String,
    pub unit_conversion_version: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ReserveSessionLedgerRequest {
    pub tenant_id: Uuid,
    pub budget_id: Uuid,
    pub window_instance_id: Uuid,
    pub unit_id: Uuid,
    #[serde(flatten)]
    pub pricing: PricingFreezeRef,
    pub session_id: String,
    pub route: String,
    pub estimated_amount_atomic: String,
    pub ttl_seconds: i64,
    pub idempotency_key: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct CommitSessionDeltaLedgerRequest {
    pub session_reservation_id: Uuid,
    pub streaming_commit_id: String,
    pub amount_atomic_delta: String,
    pub outcome: String,
    pub event_time: DateTime<Utc>,
    pub idempotency_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_instance_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pricing_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price_snapshot_hash_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fx_rate_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_conversion_version: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ReleaseSessionLedgerRequest {
    pub session_reservation_id: Uuid,
    pub reason_code: String,
    pub event_time: DateTime<Utc>,
    pub idempotency_key: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ExpireSessionLedgerRequest {
    pub session_reservation_id: Uuid,
    pub event_time: DateTime<Utc>,
    pub idempotency_key: String,
}

pub async fn reserve_session(
    pool: &PgPool,
    req: &ReserveSessionLedgerRequest,
) -> Result<Value, SessionReservationError> {
    validate_non_empty("session_id", &req.session_id)?;
    validate_non_empty("route", &req.route)?;
    validate_non_empty("idempotency_key", &req.idempotency_key)?;
    validate_positive_atomic("estimated_amount_atomic", &req.estimated_amount_atomic)?;
    if req.ttl_seconds <= 0 {
        return Err(SessionReservationError::InvalidRequest(
            "ttl_seconds must be > 0".into(),
        ));
    }
    validate_pricing(&req.pricing)?;
    call_json_function(pool, "post_session_reserve", serde_json::to_value(req)?).await
}

pub async fn commit_session_delta(
    pool: &PgPool,
    req: &CommitSessionDeltaLedgerRequest,
) -> Result<Value, SessionReservationError> {
    validate_non_empty("streaming_commit_id", &req.streaming_commit_id)?;
    validate_non_empty("idempotency_key", &req.idempotency_key)?;
    validate_positive_atomic("amount_atomic_delta", &req.amount_atomic_delta)?;
    call_json_function(
        pool,
        "post_session_commit_delta",
        serde_json::to_value(req)?,
    )
    .await
}

pub async fn release_session(
    pool: &PgPool,
    req: &ReleaseSessionLedgerRequest,
) -> Result<Value, SessionReservationError> {
    validate_non_empty("reason_code", &req.reason_code)?;
    validate_non_empty("idempotency_key", &req.idempotency_key)?;
    call_json_function(pool, "post_session_release", serde_json::to_value(req)?).await
}

pub async fn expire_session(
    pool: &PgPool,
    req: &ExpireSessionLedgerRequest,
) -> Result<Value, SessionReservationError> {
    validate_non_empty("idempotency_key", &req.idempotency_key)?;
    call_json_function(pool, "post_session_expire", serde_json::to_value(req)?).await
}

async fn call_json_function(
    pool: &PgPool,
    function_name: &str,
    payload: Value,
) -> Result<Value, SessionReservationError> {
    let sql = format!("SELECT {function_name}($1) AS outcome");
    let (Json(outcome),): (Json<Value>,) = sqlx::query_as(&sql)
        .bind(Json(payload))
        .fetch_one(pool)
        .await?;
    Ok(outcome)
}

fn validate_pricing(pricing: &PricingFreezeRef) -> Result<(), SessionReservationError> {
    validate_non_empty("pricing_version", &pricing.pricing_version)?;
    validate_non_empty("price_snapshot_hash_hex", &pricing.price_snapshot_hash_hex)?;
    validate_non_empty("fx_rate_version", &pricing.fx_rate_version)?;
    validate_non_empty("unit_conversion_version", &pricing.unit_conversion_version)?;
    Ok(())
}

fn validate_non_empty(field: &str, value: &str) -> Result<(), SessionReservationError> {
    if value.is_empty() {
        return Err(SessionReservationError::InvalidRequest(format!(
            "{field} must be non-empty"
        )));
    }
    Ok(())
}

fn validate_positive_atomic(field: &str, value: &str) -> Result<(), SessionReservationError> {
    let parsed = value.parse::<BigInt>().map_err(|e| {
        SessionReservationError::InvalidRequest(format!("{field} must be an integer: {e}"))
    })?;
    if parsed.sign() != Sign::Plus {
        return Err(SessionReservationError::InvalidRequest(format!(
            "{field} must be > 0"
        )));
    }
    Ok(())
}

impl From<serde_json::Error> for SessionReservationError {
    fn from(err: serde_json::Error) -> Self {
        Self::InvalidRequest(format!("json serialization failed: {err}"))
    }
}
