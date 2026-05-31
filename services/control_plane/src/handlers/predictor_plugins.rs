//! Strategy C plugin endpoint registry REST API.
//!
//! Spec refs:
//!   - `output-predictor-plugin-contract-v1alpha1.md` §8 (control plane
//!     API surface)
//!   - `output-predictor-plugin-contract-v1alpha1.md` §3.2 (cert
//!     pinning — `server_cert_fingerprint` validated at register time)
//!   - `output-predictor-plugin-contract-v1alpha1.md` §6.3 (force-reset
//!     circuit breaker)
//!
//! ## Routes
//!
//!   * `POST   /v1/predictor/plugins`         — register new endpoint
//!   * `PUT    /v1/predictor/plugins/{tenant_id}` — update endpoint
//!   * `DELETE /v1/predictor/plugins/{tenant_id}` — delete endpoint
//!   * `GET    /v1/predictor/plugins/{tenant_id}` — read current row
//!   * `POST   /v1/predictor/plugins/{tenant_id}/force-reset` — operator
//!     force-reset circuit breaker (audit event only; the breaker itself
//!     lives in output_predictor process memory and reads the event via
//!     the audit chain or a future SLICE_07-extra side-channel)
//!
//! ## Auth
//!
//! Per Phase 5 GA hardening S17/S18: all routes require Permission::TenantWrite
//! (admin role). Tenant-scoping is enforced via `principal.assert_tenant`
//! against the request body's tenant_id (for POST) or the URL path
//! parameter (for PUT / DELETE / GET / force-reset).
//!
//! ## Audit chain
//!
//! Per spec §8 every mutation emits a signed CloudEvent
//! `spendguard.audit.plugin_{registered, updated, deleted, force_reset}`
//! via the canonical_ingest AppendEvents RPC. Handlers use the
//! transactional outbox pattern; HARDEN_06's control-plane audit
//! forwarder signs each pending row and relays it to canonical_ingest
//! with the same `AppendEventsRequest` envelope required from other
//! audit producers.
//!
//! ## RLS
//!
//! Per the Phase A migration, predictor_plugin_endpoints has
//! `ENABLE / FORCE ROW LEVEL SECURITY` with a FOR ALL policy keyed on
//! `app.current_tenant_id`. The handlers below ALL issue
//! `SELECT set_config('app.current_tenant_id', tenant, true)` inside
//! their transactions before the DML — failing to do so produces a
//! WITH CHECK violation (defense in depth; the auth gate above is the
//! primary tenant guard).

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use tracing::{info, warn};
use uuid::Uuid;

use spendguard_auth::{Permission, Principal};

/// R2 M1 — write a `spendguard.audit.plugin_*.v1alpha1` CloudEvent
/// into `control_plane_audit_outbox` inside the caller's transaction.
/// The audit row + the registry-row mutation commit together so an
/// audit chain consumer never sees a row mutation without the
/// corresponding audit event (atomic single-tx outbox pattern,
/// mirror of ledger's post_ledger_transaction SP).
///
/// `payload` is the event-specific `data` body — register/update
/// include `endpoint_url` + `server_cert_fingerprint` (NOT the full
/// PEM); delete includes only the tenant_id; force_reset includes the
/// operator's reason. The caller MUST have run
/// `SELECT set_config('app.current_tenant_id', ...)` inside the same
/// transaction so the RLS WITH CHECK clause passes.
///
/// `producer_sequence` is allocated per-tenant by the helper itself —
/// it issues `SELECT COALESCE(MAX(producer_sequence), 0) + 1 FROM
/// control_plane_audit_outbox WHERE tenant_id = $1` inside the same
/// tx so concurrent INSERTs in different transactions hit the
/// UNIQUE(tenant_id, producer_sequence) constraint and one rolls
/// back (the handler's HTTP semantics then surface as 409 / 500).
/// SLICE-extra will replace this with a dedicated sequence allocator
/// SP per the ledger pattern.
async fn emit_plugin_audit_event(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    event_kind: &str, // "registered" | "updated" | "deleted" | "force_reset"
    actor_subject: &str,
    payload_data: serde_json::Value,
) -> Result<(), sqlx::Error> {
    let audit_outbox_id = Uuid::now_v7();
    let event_id = Uuid::now_v7();
    let event_type = format!("spendguard.audit.plugin_{event_kind}.v1alpha1");

    // Allocate the next per-tenant producer_sequence inside the same tx.
    let next_seq: (i64,) = sqlx::query_as(
        r#"
        SELECT COALESCE(MAX(producer_sequence), 0) + 1
          FROM control_plane_audit_outbox
         WHERE tenant_id = $1
        "#,
    )
    .bind(tenant_id)
    .fetch_one(&mut **tx)
    .await?;
    let producer_sequence = next_seq.0;

    let now = Utc::now();
    let cloudevent = serde_json::json!({
        "specversion": "1.0",
        "type": event_type,
        "id": event_id.to_string(),
        "source": "spendguard-control-plane",
        "tenantid": tenant_id.to_string(),
        "subject": format!("plugin/{tenant_id}"),
        "time": now.to_rfc3339(),
        "actor_subject": actor_subject,
        "producer_sequence": producer_sequence,
        "data": payload_data,
    });

    sqlx::query(
        r#"
        INSERT INTO control_plane_audit_outbox
            (audit_outbox_id, tenant_id, event_type, cloudevent_payload,
             cloudevent_payload_signature_hex, producer_sequence)
        VALUES ($1, $2, $3, $4::JSONB, '', $5)
        "#,
    )
    .bind(audit_outbox_id)
    .bind(tenant_id)
    .bind(&event_type)
    .bind(&cloudevent)
    .bind(producer_sequence)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Shared app state — for SLICE_07 we only need the pg pool. The
/// existing `AppState` in main.rs holds the same shape so we accept
/// `Arc<dyn PluginAppState>` via the `State` extractor to remain
/// loosely coupled.
pub trait PluginAppState: Send + Sync {
    fn pg(&self) -> &PgPool;
}

// ============================================================================
// POST /v1/predictor/plugins — register new endpoint
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct RegisterReq {
    pub tenant_id: String,
    pub endpoint_url: String,
    /// SHA-256 fingerprint of the plugin's TLS server cert (lowercase
    /// hex, 64 chars). Spec §3.2 pinning value.
    pub server_cert_fingerprint: String,
    /// SpendGuard-issued client cert identifier. v1alpha1 may pass a
    /// placeholder value ("spendguard-default") until the SLICE_14
    /// cert_issuer pipeline lands.
    pub client_cert_id: String,
}

#[derive(Debug, Serialize)]
pub struct RegisterResp {
    pub plugin_endpoint_id: Uuid,
    pub tenant_id: Uuid,
    pub endpoint_url: String,
    pub server_cert_fingerprint: String,
    pub client_cert_id: String,
    pub enabled: bool,
    pub registered_at: DateTime<Utc>,
}

/// Validate the request body shape BEFORE touching the database so
/// malformed input never reaches a transaction. CHECK constraints in
/// the migration enforce the same shape at the row level; these are
/// the user-friendly version.
///
/// R2 M4 (Security F8): under `SPENDGUARD_PROFILE=production` the
/// endpoint_url MUST start with `https://` — plaintext http to a
/// customer plugin endpoint violates spec §3.1 mTLS-only auth.
/// Helm chart.profile=production blocks the input gate at chart
/// render time; this handler-level enforcement defends against
/// non-Helm production deployments (raw kubectl apply, custom
/// orchestrators).
fn validate_register_input(req: &RegisterReq) -> Result<Uuid, (StatusCode, String)> {
    let tenant_uuid = Uuid::parse_str(&req.tenant_id)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("invalid tenant_id: {e}")))?;
    if req.endpoint_url.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "endpoint_url is required".into()));
    }
    if !(req.endpoint_url.starts_with("http://") || req.endpoint_url.starts_with("https://")) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "endpoint_url must start with http:// or https:// (got `{}`)",
                req.endpoint_url
            ),
        ));
    }
    enforce_https_under_production(&req.endpoint_url)?;
    if req.endpoint_url.len() > 2048 {
        return Err((
            StatusCode::BAD_REQUEST,
            "endpoint_url exceeds 2048 byte cap".into(),
        ));
    }
    if !is_lowercase_hex_64(&req.server_cert_fingerprint) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "server_cert_fingerprint must be 64 lowercase hex chars (got {} chars)",
                req.server_cert_fingerprint.len()
            ),
        ));
    }
    validate_client_cert_id(&req.client_cert_id)?;
    Ok(tenant_uuid)
}

fn validate_client_cert_id(client_cert_id: &str) -> Result<(), (StatusCode, String)> {
    if client_cert_id.is_empty() || client_cert_id.len() > 256 {
        return Err((
            StatusCode::BAD_REQUEST,
            "client_cert_id must be 1-256 bytes".into(),
        ));
    }
    if !client_cert_id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "client_cert_id may contain only [A-Za-z0-9_-]".into(),
        ));
    }
    Ok(())
}

/// R2 M4 — refuse `http://` plugin endpoints when running under
/// `SPENDGUARD_PROFILE=production`. The env var name matches the one
/// the Helm chart exports (see chart.profile=production); a non-Helm
/// production deployment MUST set this env to opt into the gate.
fn enforce_https_under_production(endpoint_url: &str) -> Result<(), (StatusCode, String)> {
    let profile = std::env::var("SPENDGUARD_PROFILE").unwrap_or_default();
    if profile == "production" && !endpoint_url.starts_with("https://") {
        return Err((
            StatusCode::BAD_REQUEST,
            "endpoint_url must use https:// under SPENDGUARD_PROFILE=production \
             (spec §3.1 mTLS-only auth)"
                .into(),
        ));
    }
    Ok(())
}

fn is_lowercase_hex_64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

pub async fn register_plugin<S: PluginAppState>(
    Extension(principal): Extension<Principal>,
    State(state): State<Arc<S>>,
    Json(req): Json<RegisterReq>,
) -> Result<Response, Response> {
    if principal.require(Permission::TenantWrite).is_err() {
        return Err((StatusCode::FORBIDDEN, "TenantWrite required").into_response());
    }
    let tenant_uuid = validate_register_input(&req).map_err(|(c, m)| (c, m).into_response())?;
    if principal.assert_tenant(&tenant_uuid.to_string()).is_err() {
        warn!(
            subject = %principal.subject,
            requested_tenant = %tenant_uuid,
            scope = ?principal.tenant_ids,
            "register_plugin rejected — cross-tenant"
        );
        return Err((StatusCode::FORBIDDEN, "cross-tenant").into_response());
    }
    let plugin_endpoint_id = Uuid::now_v7();

    // Set the RLS session variable inside a tx so the INSERT's
    // WITH CHECK clause sees `app.current_tenant_id`. Per spec §7.3
    // + SLICE_06 R2 B1 convention.
    let mut tx = state
        .pg()
        .begin()
        .await
        .map_err(|e| internal_err(e).into_response())?;
    sqlx::query("SELECT set_config('app.current_tenant_id', $1, true)")
        .bind(tenant_uuid.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|e| internal_err(e).into_response())?;

    // INSERT ... ON CONFLICT — POST is strictly create; conflict on
    // tenant_id (UNIQUE) returns 409 so callers can choose to PUT
    // instead of overwriting.
    let row: Result<(Uuid, Uuid, String, String, String, bool, DateTime<Utc>), sqlx::Error> =
        sqlx::query_as(
            r#"
            INSERT INTO predictor_plugin_endpoints
                (plugin_endpoint_id, tenant_id, endpoint_url,
                 server_cert_fingerprint, client_cert_id)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING plugin_endpoint_id, tenant_id, endpoint_url,
                      server_cert_fingerprint, client_cert_id, enabled,
                      registered_at
            "#,
        )
        .bind(plugin_endpoint_id)
        .bind(tenant_uuid)
        .bind(&req.endpoint_url)
        .bind(&req.server_cert_fingerprint)
        .bind(&req.client_cert_id)
        .fetch_one(&mut *tx)
        .await;

    let row = match row {
        Ok(r) => r,
        Err(sqlx::Error::Database(db))
            if db.constraint() == Some("predictor_plugin_endpoints_pkey")
                || db
                    .message()
                    .contains("predictor_plugin_endpoints_tenant_id_key") =>
        {
            // UNIQUE violation on tenant_id — operator must PUT to update.
            return Err((
                StatusCode::CONFLICT,
                "plugin endpoint already registered for this tenant; use PUT to update".to_string(),
            )
                .into_response());
        }
        Err(e) => return Err(internal_err(e).into_response()),
    };

    // R2 M1: signed CloudEvent emission inside the same tx so
    // outbox + registry-row commit atomically. payload includes the
    // endpoint URL + cert fingerprint (NOT the full PEM) so audit
    // consumers can reconstruct the wire identity.
    if let Err(e) = emit_plugin_audit_event(
        &mut tx,
        tenant_uuid,
        "registered",
        &principal.subject,
        serde_json::json!({
            "plugin_endpoint_id": row.0.to_string(),
            "endpoint_url": &req.endpoint_url,
            "server_cert_fingerprint": &req.server_cert_fingerprint,
            "client_cert_id": &req.client_cert_id,
        }),
    )
    .await
    {
        return Err(internal_err(e).into_response());
    }

    tx.commit()
        .await
        .map_err(|e| internal_err(e).into_response())?;

    info!(
        subject = %principal.subject,
        tenant = %tenant_uuid,
        plugin_endpoint_id = %plugin_endpoint_id,
        endpoint_url = %req.endpoint_url,
        "register_plugin success"
    );

    Ok(Json(RegisterResp {
        plugin_endpoint_id: row.0,
        tenant_id: row.1,
        endpoint_url: row.2,
        server_cert_fingerprint: row.3,
        client_cert_id: row.4,
        enabled: row.5,
        registered_at: row.6,
    })
    .into_response())
}

// ============================================================================
// PUT /v1/predictor/plugins/{tenant_id} — update endpoint
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct UpdateReq {
    pub endpoint_url: Option<String>,
    pub server_cert_fingerprint: Option<String>,
    pub client_cert_id: Option<String>,
    pub enabled: Option<bool>,
}

pub async fn update_plugin<S: PluginAppState>(
    Extension(principal): Extension<Principal>,
    State(state): State<Arc<S>>,
    Path(tenant_id): Path<String>,
    Json(req): Json<UpdateReq>,
) -> Result<Response, Response> {
    if principal.require(Permission::TenantWrite).is_err() {
        return Err((StatusCode::FORBIDDEN, "TenantWrite required").into_response());
    }
    let tenant_uuid = Uuid::parse_str(&tenant_id).map_err(|e| {
        (StatusCode::BAD_REQUEST, format!("invalid tenant_id: {e}")).into_response()
    })?;
    if principal.assert_tenant(&tenant_uuid.to_string()).is_err() {
        return Err((StatusCode::FORBIDDEN, "cross-tenant").into_response());
    }

    // Validate any supplied non-None field with the same checks as
    // register.
    if let Some(url) = &req.endpoint_url {
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err((
                StatusCode::BAD_REQUEST,
                "endpoint_url must start with http:// or https://",
            )
                .into_response());
        }
        // R2 M4 (Security F8): same https://-under-production gate as
        // register_plugin. Without this, an operator could register
        // with https:// (passes the chart gate), then PUT to flip the
        // URL to http:// (which bypassed any gate before this fix).
        if let Err((code, msg)) = enforce_https_under_production(url) {
            return Err((code, msg).into_response());
        }
        if url.len() > 2048 {
            return Err((
                StatusCode::BAD_REQUEST,
                "endpoint_url exceeds 2048 byte cap",
            )
                .into_response());
        }
    }
    if let Some(fp) = &req.server_cert_fingerprint {
        if !is_lowercase_hex_64(fp) {
            return Err((
                StatusCode::BAD_REQUEST,
                "server_cert_fingerprint must be 64 lowercase hex chars",
            )
                .into_response());
        }
    }
    if let Some(cid) = &req.client_cert_id {
        if let Err((code, msg)) = validate_client_cert_id(cid) {
            return Err((code, msg).into_response());
        }
    }

    let mut tx = state
        .pg()
        .begin()
        .await
        .map_err(|e| internal_err(e).into_response())?;
    sqlx::query("SELECT set_config('app.current_tenant_id', $1, true)")
        .bind(tenant_uuid.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|e| internal_err(e).into_response())?;

    // COALESCE pattern so unset fields fall back to existing value.
    let row: Option<(Uuid, Uuid, String, String, String, bool, DateTime<Utc>)> = sqlx::query_as(
        r#"
        UPDATE predictor_plugin_endpoints
           SET endpoint_url            = COALESCE($2, endpoint_url),
               server_cert_fingerprint = COALESCE($3, server_cert_fingerprint),
               client_cert_id          = COALESCE($4, client_cert_id),
               enabled                 = COALESCE($5, enabled)
         WHERE tenant_id = $1
        RETURNING plugin_endpoint_id, tenant_id, endpoint_url,
                  server_cert_fingerprint, client_cert_id, enabled,
                  registered_at
        "#,
    )
    .bind(tenant_uuid)
    .bind(req.endpoint_url.as_deref())
    .bind(req.server_cert_fingerprint.as_deref())
    .bind(req.client_cert_id.as_deref())
    .bind(req.enabled)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| internal_err(e).into_response())?;

    let row = match row {
        Some(r) => r,
        None => {
            // No row → return 404. Auth + tenant guard already passed;
            // safe to surface "not found" without leaking tenant
            // existence.
            return Err((
                StatusCode::NOT_FOUND,
                "plugin endpoint not registered for tenant",
            )
                .into_response());
        }
    };

    // R2 M1: signed CloudEvent emission. Payload includes the RESULTING
    // endpoint_url + server_cert_fingerprint (post-COALESCE) so audit
    // consumers see the effective state. The "fields requested" list
    // captures which knobs the operator touched even if COALESCE
    // resolved them to the same value.
    let fields_changed: Vec<&str> = [
        ("endpoint_url", req.endpoint_url.is_some()),
        (
            "server_cert_fingerprint",
            req.server_cert_fingerprint.is_some(),
        ),
        ("client_cert_id", req.client_cert_id.is_some()),
        ("enabled", req.enabled.is_some()),
    ]
    .into_iter()
    .filter_map(|(k, set)| if set { Some(k) } else { None })
    .collect();
    if let Err(e) = emit_plugin_audit_event(
        &mut tx,
        tenant_uuid,
        "updated",
        &principal.subject,
        serde_json::json!({
            "plugin_endpoint_id": row.0.to_string(),
            "endpoint_url": &row.2,
            "server_cert_fingerprint": &row.3,
            "client_cert_id": &row.4,
            "enabled": row.5,
            "fields_changed": fields_changed,
        }),
    )
    .await
    {
        return Err(internal_err(e).into_response());
    }

    tx.commit()
        .await
        .map_err(|e| internal_err(e).into_response())?;

    info!(
        subject = %principal.subject,
        tenant = %tenant_uuid,
        plugin_endpoint_id = %row.0,
        "update_plugin success"
    );

    Ok(Json(RegisterResp {
        plugin_endpoint_id: row.0,
        tenant_id: row.1,
        endpoint_url: row.2,
        server_cert_fingerprint: row.3,
        client_cert_id: row.4,
        enabled: row.5,
        registered_at: row.6,
    })
    .into_response())
}

// ============================================================================
// DELETE /v1/predictor/plugins/{tenant_id}
// ============================================================================

pub async fn delete_plugin<S: PluginAppState>(
    Extension(principal): Extension<Principal>,
    State(state): State<Arc<S>>,
    Path(tenant_id): Path<String>,
) -> Result<Response, Response> {
    if principal.require(Permission::TenantWrite).is_err() {
        return Err((StatusCode::FORBIDDEN, "TenantWrite required").into_response());
    }
    let tenant_uuid = Uuid::parse_str(&tenant_id).map_err(|e| {
        (StatusCode::BAD_REQUEST, format!("invalid tenant_id: {e}")).into_response()
    })?;
    if principal.assert_tenant(&tenant_uuid.to_string()).is_err() {
        return Err((StatusCode::FORBIDDEN, "cross-tenant").into_response());
    }

    let mut tx = state
        .pg()
        .begin()
        .await
        .map_err(|e| internal_err(e).into_response())?;
    sqlx::query("SELECT set_config('app.current_tenant_id', $1, true)")
        .bind(tenant_uuid.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|e| internal_err(e).into_response())?;

    let res = sqlx::query("DELETE FROM predictor_plugin_endpoints WHERE tenant_id = $1")
        .bind(tenant_uuid)
        .execute(&mut *tx)
        .await
        .map_err(|e| internal_err(e).into_response())?;

    if res.rows_affected() == 0 {
        // No row to delete — DO NOT emit an audit event; surface 404.
        // tx.commit() is still safe (no changes).
        let _ = tx.commit().await;
        return Err((
            StatusCode::NOT_FOUND,
            "plugin endpoint not registered for tenant",
        )
            .into_response());
    }

    // R2 M1: signed CloudEvent emission inside the same tx as the DELETE.
    if let Err(e) = emit_plugin_audit_event(
        &mut tx,
        tenant_uuid,
        "deleted",
        &principal.subject,
        serde_json::json!({
            "tenant_id": tenant_uuid.to_string(),
        }),
    )
    .await
    {
        return Err(internal_err(e).into_response());
    }

    tx.commit()
        .await
        .map_err(|e| internal_err(e).into_response())?;

    info!(
        subject = %principal.subject,
        tenant = %tenant_uuid,
        "delete_plugin success"
    );
    Ok(StatusCode::NO_CONTENT.into_response())
}

// ============================================================================
// GET /v1/predictor/plugins/{tenant_id}
// ============================================================================

pub async fn get_plugin<S: PluginAppState>(
    Extension(principal): Extension<Principal>,
    State(state): State<Arc<S>>,
    Path(tenant_id): Path<String>,
) -> Result<Response, Response> {
    // Read uses TenantWrite to match the rest of the surface — these
    // rows carry mTLS pinning values that aren't intended for a
    // generic ReadView role to see.
    if principal.require(Permission::TenantWrite).is_err() {
        return Err((StatusCode::FORBIDDEN, "TenantWrite required").into_response());
    }
    let tenant_uuid = Uuid::parse_str(&tenant_id).map_err(|e| {
        (StatusCode::BAD_REQUEST, format!("invalid tenant_id: {e}")).into_response()
    })?;
    if principal.assert_tenant(&tenant_uuid.to_string()).is_err() {
        return Err((StatusCode::FORBIDDEN, "cross-tenant").into_response());
    }

    let mut tx = state
        .pg()
        .begin()
        .await
        .map_err(|e| internal_err(e).into_response())?;
    sqlx::query("SELECT set_config('app.current_tenant_id', $1, true)")
        .bind(tenant_uuid.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|e| internal_err(e).into_response())?;

    let row: Option<(Uuid, Uuid, String, String, String, bool, DateTime<Utc>)> = sqlx::query_as(
        r#"
        SELECT plugin_endpoint_id, tenant_id, endpoint_url,
               server_cert_fingerprint, client_cert_id, enabled,
               registered_at
          FROM predictor_plugin_endpoints
         WHERE tenant_id = $1
        "#,
    )
    .bind(tenant_uuid)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| internal_err(e).into_response())?;

    tx.commit()
        .await
        .map_err(|e| internal_err(e).into_response())?;

    let row = row.ok_or(
        (
            StatusCode::NOT_FOUND,
            "plugin endpoint not registered for tenant",
        )
            .into_response(),
    )?;

    Ok(Json(RegisterResp {
        plugin_endpoint_id: row.0,
        tenant_id: row.1,
        endpoint_url: row.2,
        server_cert_fingerprint: row.3,
        client_cert_id: row.4,
        enabled: row.5,
        registered_at: row.6,
    })
    .into_response())
}

// ============================================================================
// POST /v1/predictor/plugins/{tenant_id}/force-reset
// ============================================================================
//
// Spec §6.3 — operator-triggered circuit breaker reset. The breaker
// itself lives in the output_predictor process; this endpoint marks the
// row's `current_health_status = 'serving'` + bumps the registered_at
// to current wallclock so output_predictor's next cache reload (within
// `plugin_endpoint_cache_ttl_seconds`) picks up the change. v1beta1
// will wire a control plane → output_predictor signed CloudEvent
// fanout for synchronous reset.

#[derive(Debug, Deserialize)]
pub struct ForceResetReq {
    /// Operator-supplied reason for the audit chain. Required.
    pub reason: String,
}

#[derive(Debug, Serialize)]
pub struct ForceResetResp {
    pub tenant_id: Uuid,
    pub reset_at: DateTime<Utc>,
    pub note: String,
}

pub async fn force_reset_plugin<S: PluginAppState>(
    Extension(principal): Extension<Principal>,
    State(state): State<Arc<S>>,
    Path(tenant_id): Path<String>,
    Json(req): Json<ForceResetReq>,
) -> Result<Response, Response> {
    if principal.require(Permission::TenantWrite).is_err() {
        return Err((StatusCode::FORBIDDEN, "TenantWrite required").into_response());
    }
    let tenant_uuid = Uuid::parse_str(&tenant_id).map_err(|e| {
        (StatusCode::BAD_REQUEST, format!("invalid tenant_id: {e}")).into_response()
    })?;
    if principal.assert_tenant(&tenant_uuid.to_string()).is_err() {
        return Err((StatusCode::FORBIDDEN, "cross-tenant").into_response());
    }
    if req.reason.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "reason is required").into_response());
    }

    let mut tx = state
        .pg()
        .begin()
        .await
        .map_err(|e| internal_err(e).into_response())?;
    sqlx::query("SELECT set_config('app.current_tenant_id', $1, true)")
        .bind(tenant_uuid.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|e| internal_err(e).into_response())?;

    let row: Option<(Uuid, DateTime<Utc>)> = sqlx::query_as(
        r#"
        UPDATE predictor_plugin_endpoints
           SET current_health_status = 'serving',
               last_health_check_at  = clock_timestamp()
         WHERE tenant_id = $1
        RETURNING tenant_id, last_health_check_at
        "#,
    )
    .bind(tenant_uuid)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| internal_err(e).into_response())?;

    let row = row.ok_or(
        (
            StatusCode::NOT_FOUND,
            "plugin endpoint not registered for tenant",
        )
            .into_response(),
    )?;

    // R2 M1: signed CloudEvent emission inside the same tx. The reason
    // is operator-supplied free text; we already enforce non-empty +
    // could grow length caps in a follow-up (tracked as GH issue per
    // R2 outputs).
    if let Err(e) = emit_plugin_audit_event(
        &mut tx,
        tenant_uuid,
        "force_reset",
        &principal.subject,
        serde_json::json!({
            "tenant_id": row.0.to_string(),
            "reset_at": row.1.to_rfc3339(),
            "reason": &req.reason,
        }),
    )
    .await
    {
        return Err(internal_err(e).into_response());
    }

    tx.commit()
        .await
        .map_err(|e| internal_err(e).into_response())?;

    info!(
        subject = %principal.subject,
        tenant = %tenant_uuid,
        reason = %req.reason,
        "force_reset_plugin success — output_predictor breaker will pick up via cache reload"
    );

    Ok(Json(ForceResetResp {
        tenant_id: row.0,
        reset_at: row.1,
        note: "Plugin endpoint marked SERVING. output_predictor's per-tenant circuit breaker will reset on next cache reload (≤ plugin_endpoint_cache_ttl_seconds).".to_string(),
    })
    .into_response())
}

// ============================================================================
// helpers
// ============================================================================

fn internal_err(e: sqlx::Error) -> (StatusCode, String) {
    warn!(error = ?e, "predictor_plugins handler SQL error");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal error".to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowercase_hex_64_accepts_valid() {
        assert!(is_lowercase_hex_64(&"a".repeat(64)));
        assert!(is_lowercase_hex_64(
            &"f0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde"
        ));
    }

    #[test]
    fn lowercase_hex_64_rejects_wrong_length() {
        assert!(!is_lowercase_hex_64(&"a".repeat(63)));
        assert!(!is_lowercase_hex_64(&"a".repeat(65)));
        assert!(!is_lowercase_hex_64(""));
    }

    #[test]
    fn lowercase_hex_64_rejects_uppercase() {
        // SHA-256 hex MUST be lowercase per spec.
        assert!(!is_lowercase_hex_64(&"A".repeat(64)));
        assert!(!is_lowercase_hex_64(
            "ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
        ));
    }

    #[test]
    fn lowercase_hex_64_rejects_non_hex() {
        assert!(!is_lowercase_hex_64(&"g".repeat(64)));
        assert!(!is_lowercase_hex_64(&"z".repeat(64)));
    }

    #[test]
    fn validate_register_accepts_valid_input() {
        let req = RegisterReq {
            tenant_id: Uuid::new_v4().to_string(),
            endpoint_url: "https://plugin.tenant-x.example/predict".into(),
            server_cert_fingerprint: "a".repeat(64),
            client_cert_id: "spendguard-default".into(),
        };
        let tenant_uuid = validate_register_input(&req).expect("ok");
        assert_eq!(tenant_uuid.to_string(), req.tenant_id);
    }

    #[test]
    fn validate_register_rejects_bad_tenant_uuid() {
        let req = RegisterReq {
            tenant_id: "not-a-uuid".into(),
            endpoint_url: "https://plugin.example/predict".into(),
            server_cert_fingerprint: "a".repeat(64),
            client_cert_id: "spendguard-default".into(),
        };
        let (code, msg) = validate_register_input(&req).expect_err("bad uuid");
        assert_eq!(code, StatusCode::BAD_REQUEST);
        assert!(msg.contains("tenant_id"));
    }

    #[test]
    fn validate_register_rejects_non_http_url() {
        let req = RegisterReq {
            tenant_id: Uuid::new_v4().to_string(),
            endpoint_url: "grpc://plugin.example/predict".into(),
            server_cert_fingerprint: "a".repeat(64),
            client_cert_id: "spendguard-default".into(),
        };
        let (code, msg) = validate_register_input(&req).expect_err("bad url");
        assert_eq!(code, StatusCode::BAD_REQUEST);
        assert!(msg.contains("endpoint_url"));
    }

    #[test]
    fn validate_register_rejects_bad_fingerprint() {
        let req = RegisterReq {
            tenant_id: Uuid::new_v4().to_string(),
            endpoint_url: "https://plugin.example/predict".into(),
            server_cert_fingerprint: "A".repeat(64),
            client_cert_id: "spendguard-default".into(),
        };
        let (code, msg) = validate_register_input(&req).expect_err("uppercase");
        assert_eq!(code, StatusCode::BAD_REQUEST);
        assert!(msg.contains("fingerprint"));
    }

    #[test]
    fn validate_register_rejects_empty_client_cert_id() {
        let req = RegisterReq {
            tenant_id: Uuid::new_v4().to_string(),
            endpoint_url: "https://plugin.example/predict".into(),
            server_cert_fingerprint: "a".repeat(64),
            client_cert_id: "".into(),
        };
        let (code, msg) = validate_register_input(&req).expect_err("empty");
        assert_eq!(code, StatusCode::BAD_REQUEST);
        assert!(msg.contains("client_cert_id"));
    }

    #[test]
    fn validate_register_rejects_huge_client_cert_id() {
        let req = RegisterReq {
            tenant_id: Uuid::new_v4().to_string(),
            endpoint_url: "https://plugin.example/predict".into(),
            server_cert_fingerprint: "a".repeat(64),
            client_cert_id: "x".repeat(257),
        };
        let (code, _msg) = validate_register_input(&req).expect_err("oversize");
        assert_eq!(code, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn validate_register_rejects_runtime_unsafe_client_cert_id() {
        for bad in ["tenant/a", "tenant.one", "tenant one", "../tenant"] {
            let req = RegisterReq {
                tenant_id: Uuid::new_v4().to_string(),
                endpoint_url: "https://plugin.example/predict".into(),
                server_cert_fingerprint: "a".repeat(64),
                client_cert_id: bad.into(),
            };
            let (code, msg) = validate_register_input(&req).expect_err("unsafe client_cert_id");
            assert_eq!(code, StatusCode::BAD_REQUEST);
            assert!(msg.contains("[A-Za-z0-9_-]"), "got: {msg}");
        }
    }

    // ─── R2 M4 — https://-under-production gate ──────────────────────
    //
    // These tests mutate SPENDGUARD_PROFILE — process-wide env var.
    // Each test SAVES the original value at entry + RESTORES at exit
    // so the test suite is order-independent. We hold a parking_lot
    // mutex (`PROFILE_LOCK`) across the env mutation so the four
    // tests don't race each other when run with `cargo test
    // -- --test-threads=8`.

    use std::sync::Mutex;
    static PROFILE_LOCK: Mutex<()> = Mutex::new(());

    /// Set SPENDGUARD_PROFILE to a known value, run the test body,
    /// restore. Returns the body's result so the caller can assert.
    fn with_profile<F: FnOnce() -> R, R>(profile: Option<&str>, f: F) -> R {
        let _g = PROFILE_LOCK.lock().expect("env mutex poisoned");
        let original = std::env::var("SPENDGUARD_PROFILE").ok();
        match profile {
            Some(p) => std::env::set_var("SPENDGUARD_PROFILE", p),
            None => std::env::remove_var("SPENDGUARD_PROFILE"),
        }
        let result = f();
        match original {
            Some(o) => std::env::set_var("SPENDGUARD_PROFILE", o),
            None => std::env::remove_var("SPENDGUARD_PROFILE"),
        }
        result
    }

    #[test]
    fn enforce_https_allows_http_under_demo() {
        with_profile(Some("demo"), || {
            assert!(enforce_https_under_production("http://plugin.example/predict").is_ok());
        });
    }

    #[test]
    fn enforce_https_allows_http_when_profile_unset() {
        with_profile(None, || {
            // Default (no env) treats as demo posture — http allowed.
            assert!(enforce_https_under_production("http://plugin.example/predict").is_ok());
        });
    }

    #[test]
    fn enforce_https_rejects_http_under_production() {
        with_profile(Some("production"), || {
            let err = enforce_https_under_production("http://plugin.example/predict")
                .expect_err("blocked");
            assert_eq!(err.0, StatusCode::BAD_REQUEST);
            assert!(err.1.contains("https://"));
            assert!(err.1.contains("SPENDGUARD_PROFILE=production"));
        });
    }

    #[test]
    fn enforce_https_accepts_https_under_production() {
        with_profile(Some("production"), || {
            assert!(enforce_https_under_production("https://plugin.example/predict").is_ok());
        });
    }

    #[test]
    fn validate_register_rejects_http_under_production() {
        with_profile(Some("production"), || {
            let req = RegisterReq {
                tenant_id: Uuid::new_v4().to_string(),
                endpoint_url: "http://plugin.example/predict".into(),
                server_cert_fingerprint: "a".repeat(64),
                client_cert_id: "spendguard-default".into(),
            };
            let (code, msg) = validate_register_input(&req).expect_err("blocked under production");
            assert_eq!(code, StatusCode::BAD_REQUEST);
            assert!(msg.contains("https://"));
        });
    }
}
