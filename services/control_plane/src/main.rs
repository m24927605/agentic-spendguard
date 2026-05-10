//! SpendGuard SaaS Control Plane MVP — Phase 4 O8.
//!
//! REST API for tenant + budget provisioning. POC scope:
//!   POST /v1/tenants                 — create tenant + budget + window
//!                                      + fencing scope + opening deposit
//!   GET  /v1/tenants/:id             — tenant overview
//!   POST /v1/tenants/:id/budgets     — add another budget under tenant
//!                                      (token-denominated; USD is O4)
//!   DELETE /v1/tenants/:id           — soft-delete (mark tombstoned;
//!                                      audit chain stays immutable)
//!
//! Auth (Phase 5 GA hardening S17): OIDC JWT or static_token (demo
//! profile only) via the shared `spendguard-auth` crate. Subject +
//! tenant scope come from the authenticated Principal in axum
//! extensions. S18 will wire role-aware tenant scoping and per-route
//! authorization checks.
//!
//! NOT YET wired:
//!   - Per-route RBAC (S18)
//!   - Contract YAML upload + bundle build pipeline
//!   - Stripe / billing integration
//!   - Per-tenant rate limiting
//!   - Multi-region routing
//!
//! Ledger writes go through the existing post_ledger_transaction SP
//! so audit chain + fencing CAS + per-unit balance invariants are
//! exercised on every provisioning step.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    middleware::from_fn_with_state,
    response::{IntoResponse, Response},
    routing::{get, post},
    Extension, Json, Router,
};
use serde::{Deserialize, Serialize};
use spendguard_auth::{AuthConfig, Authenticator, Permission, Principal};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tracing::info;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
struct Config {
    bind_addr: String,
    database_url: String,
}

struct AppState {
    pg: PgPool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let envfilter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,spendguard_control_plane=debug"));
    tracing_subscriber::registry()
        .with(envfilter)
        .with(fmt::layer().json())
        .init();

    let cfg: Config = envy::prefixed("SPENDGUARD_CONTROL_PLANE_").from_env()?;
    let pg = PgPoolOptions::new()
        .max_connections(5)
        .connect(&cfg.database_url)
        .await?;

    let state = Arc::new(AppState { pg });

    // Phase 5 GA hardening S17: build Authenticator before binding
    // listener so misconfig (missing OIDC issuer, static_token outside
    // demo profile) crashes startup rather than admitting unauthed
    // requests.
    let profile = std::env::var("SPENDGUARD_PROFILE").unwrap_or_default();
    let auth_cfg = AuthConfig::from_env("SPENDGUARD_CONTROL_PLANE", &profile)
        .map_err(|e| anyhow::anyhow!("S17: build control_plane auth config: {e}"))?;
    let auth = Arc::new(
        Authenticator::from_config(auth_cfg)
            .await
            .map_err(|e| anyhow::anyhow!("S17: init authenticator: {e}"))?,
    );

    // Auth-required routes go behind the middleware; /healthz stays open.
    let v1_routes = Router::new()
        .route("/v1/tenants", post(create_tenant))
        .route("/v1/tenants/:id", get(get_tenant).delete(tombstone_tenant))
        // Phase 5 GA hardening S15: approval REST API. List, detail,
        // resolve. RBAC + tenant scope checks live in each handler.
        .route("/v1/approvals", get(list_approvals))
        .route("/v1/approvals/:id", get(get_approval))
        .route("/v1/approvals/:id/resolve", post(resolve_approval))
        .layer(from_fn_with_state(auth.clone(), spendguard_auth::require_auth));

    let app = Router::new()
        .route("/healthz", get(healthz))
        .merge(v1_routes)
        .with_state(state);

    let addr: SocketAddr = cfg.bind_addr.parse()?;
    info!("control-plane listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn healthz() -> &'static str {
    "ok"
}


#[derive(Deserialize)]
struct CreateTenantReq {
    name: String,
    /// Initial deposit amount in atomic units of the budget unit.
    /// For token unit: tokens. For USD unit: micro-USD.
    opening_deposit_atomic: String,
    /// One of: "token" (default) or "usd_micros"
    #[serde(default = "default_unit_kind")]
    budget_unit_kind: String,
}

fn default_unit_kind() -> String {
    "token".into()
}

#[derive(Serialize)]
struct CreateTenantResp {
    tenant_id: Uuid,
    budget_id: Uuid,
    window_instance_id: Uuid,
    fencing_scope_id: Uuid,
    unit_id: Uuid,
    opening_deposit_atomic: String,
    workload_instance_id: String,
    /// Bootstrap config the operator passes to their sidecar.
    sidecar_config_env: serde_json::Value,
}

async fn create_tenant(
    Extension(principal): Extension<Principal>,
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateTenantReq>,
) -> Result<Response, StatusCode> {
    // S18: only Admin role grants TenantWrite.
    if principal.require(Permission::TenantWrite).is_err() {
        info!(
            subject = %principal.subject,
            roles = ?principal.roles,
            "create_tenant rejected — missing TenantWrite permission"
        );
        return Err(StatusCode::FORBIDDEN);
    }
    info!(
        subject = %principal.subject,
        mode = %principal.mode,
        roles = ?principal.roles,
        "create_tenant invoked"
    );

    if req.name.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let tenant_id = Uuid::now_v7();
    let budget_id = Uuid::now_v7();
    let window_instance_id = Uuid::now_v7();
    let fencing_scope_id = Uuid::now_v7();
    let unit_id = Uuid::now_v7();
    let workload_instance_id = format!("sidecar-{}", &tenant_id.to_string()[..8]);

    // POC: create rows directly via single transaction. Production
    // routes through a saga so partial failures roll back cleanly.
    let mut tx = state
        .pg
        .begin()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // 1) ledger_units row.
    let (unit_kind_db, currency, scale, rounding_mode, token_kind) = match req
        .budget_unit_kind
        .as_str()
    {
        "usd_micros" => ("monetary", Some("USD"), 6, "half_up", None::<&str>),
        "token" | _ => ("token", None, 0, "truncate", Some("output_token")),
    };

    sqlx::query(
        r#"INSERT INTO ledger_units
            (unit_id, tenant_id, unit_kind, currency, scale, rounding_mode,
             token_kind, model_family)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT DO NOTHING"#,
    )
    .bind(unit_id)
    .bind(tenant_id)
    .bind(unit_kind_db)
    .bind(currency)
    .bind(scale)
    .bind(rounding_mode)
    .bind(token_kind)
    .bind(token_kind.map(|_| "gpt-4"))
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!(error = ?e, "ledger_units insert");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // 2) budget_window_instances row.
    sqlx::query(
        r#"INSERT INTO budget_window_instances
            (window_instance_id, tenant_id, budget_id, window_type, tzdb_version,
             boundary_start, boundary_end, computed_from_snapshot_at)
            VALUES ($1, $2, $3, 'rolling', '2025c',
                    now() - interval '1 hour',
                    now() + interval '24 hours',
                    now())"#,
    )
    .bind(window_instance_id)
    .bind(tenant_id)
    .bind(budget_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!(error = ?e, "budget_window_instances insert");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // 3) fencing_scopes row (current_epoch=1; SP rejects 0).
    sqlx::query(
        r#"INSERT INTO fencing_scopes
            (fencing_scope_id, scope_type, tenant_id, budget_id,
             window_instance_id, current_epoch, active_owner_instance_id,
             ttl_expires_at, epoch_source_authority)
            VALUES ($1, 'budget_window', $2, $3, $4,
                    1, $5, 'infinity'::timestamptz, 'control_plane')"#,
    )
    .bind(fencing_scope_id)
    .bind(tenant_id)
    .bind(budget_id)
    .bind(window_instance_id)
    .bind(&workload_instance_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!(error = ?e, "fencing_scopes insert");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // 4) ledger_accounts (4 kinds × this unit).
    sqlx::query(
        r#"INSERT INTO ledger_accounts
            (ledger_account_id, tenant_id, budget_id, window_instance_id,
             account_kind, unit_id)
            SELECT gen_random_uuid(), $1, $2, $3, kind, $4
              FROM (VALUES ('available_budget'), ('reserved_hold'),
                          ('committed_spend'), ('adjustment')) AS k(kind)"#,
    )
    .bind(tenant_id)
    .bind(budget_id)
    .bind(window_instance_id)
    .bind(unit_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!(error = ?e, "ledger_accounts insert");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // 5) Opening deposit via SP — exercises full audit chain.
    // POC: re-uses sidecar fencing scope (control_plane_writer would
    // be cleaner but requires a separate scope; deferred).
    let cp_fencing_scope_id = Uuid::now_v7();
    sqlx::query(
        r#"INSERT INTO fencing_scopes
            (fencing_scope_id, scope_type, tenant_id, workload_kind,
             current_epoch, active_owner_instance_id,
             ttl_expires_at, epoch_source_authority)
            VALUES ($1, 'control_plane_writer', $2, 'control_plane_seed',
                    1, 'control-plane-runner',
                    'infinity'::timestamptz, 'control_plane')"#,
    )
    .bind(cp_fencing_scope_id)
    .bind(tenant_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!(error = ?e, "control_plane_writer fencing_scopes insert");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let decision_id = Uuid::now_v7();
    let audit_event_id = Uuid::now_v7();
    let outbox_id = Uuid::now_v7();
    let ledger_tx_id = Uuid::now_v7();
    let entry_a = Uuid::now_v7();
    let entry_b = Uuid::now_v7();

    use sha2::{Digest, Sha256};
    let request_hash = Sha256::digest(format!("control-plane-deposit-{tenant_id}").as_bytes());
    let request_hash_hex = hex::encode(request_hash);

    // Query the latest pricing_snapshots tuple so the SP's
    // PRICING_VERSION_UNKNOWN check passes. POC: pick the most-recent
    // row; production resolves per-tenant active bundle.
    let (pricing_version, price_hash_bytes, fx_version, unit_conv_version): (
        String,
        Vec<u8>,
        String,
        String,
    ) = sqlx::query_as(
        r#"SELECT pricing_version, price_snapshot_hash, fx_rate_version,
                  unit_conversion_version
             FROM pricing_snapshots
            ORDER BY deployed_at DESC
            LIMIT 1"#,
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!(error = ?e, "pricing_snapshots lookup");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let price_hash_hex = hex::encode(&price_hash_bytes);

    let payload_b64 = base64_encode(
        format!(
            r#"{{"kind":"operator_adjustment","reason":"control_plane_opening_deposit","amount_atomic":"{}"}}"#,
            req.opening_deposit_atomic
        )
        .as_bytes(),
    );

    let tx_json = serde_json::json!({
        "tenant_id":               tenant_id.to_string(),
        "operation_kind":          "adjustment",
        "idempotency_key":         format!("control-plane-deposit-{tenant_id}"),
        "request_hash_hex":        request_hash_hex,
        "decision_id":             decision_id.to_string(),
        "audit_decision_event_id": audit_event_id.to_string(),
        "fencing_scope_id":        cp_fencing_scope_id.to_string(),
        "fencing_epoch":           1,
        "workload_instance_id":    "control-plane-runner",
        "effective_at":            chrono::Utc::now().to_rfc3339(),
        "ledger_transaction_id":   ledger_tx_id.to_string(),
        "minimal_replay_response": {},
    });
    let entries_json = serde_json::json!([
        {
            "budget_id":               budget_id.to_string(),
            "window_instance_id":      window_instance_id.to_string(),
            "unit_id":                 unit_id.to_string(),
            "account_kind":            "available_budget",
            "direction":               "credit",
            "amount_atomic":           req.opening_deposit_atomic,
            "pricing_version":         pricing_version,
            "price_snapshot_hash_hex": price_hash_hex,
            "fx_rate_version":         fx_version,
            "unit_conversion_version": unit_conv_version,
            "ledger_entry_id":         entry_a.to_string(),
            "ledger_shard_id":         1,
        },
        {
            "budget_id":               budget_id.to_string(),
            "window_instance_id":      window_instance_id.to_string(),
            "unit_id":                 unit_id.to_string(),
            "account_kind":            "adjustment",
            "direction":               "debit",
            "amount_atomic":           req.opening_deposit_atomic,
            "pricing_version":         pricing_version,
            "price_snapshot_hash_hex": price_hash_hex,
            "fx_rate_version":         fx_version,
            "unit_conversion_version": unit_conv_version,
            "ledger_entry_id":         entry_b.to_string(),
            "ledger_shard_id":         1,
        },
    ]);
    let outbox_json = serde_json::json!({
        "audit_outbox_id":                  outbox_id.to_string(),
        "event_type":                       "spendguard.audit.decision",
        "cloudevent_payload":               {
            "specversion":  "1.0",
            "type":         "spendguard.audit.decision",
            "id":           audit_event_id.to_string(),
            "source":       "control-plane-runner",
            "tenantid":     tenant_id.to_string(),
            "data_b64":     payload_b64,
            "producer_sequence": 1,
        },
        "cloudevent_payload_signature_hex": "",
        "producer_sequence":                1,
    });

    sqlx::query("SELECT post_ledger_transaction($1::JSONB, $2::JSONB, NULL::JSONB, $3::JSONB, NULL)")
        .bind(tx_json)
        .bind(entries_json)
        .bind(outbox_json)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            tracing::error!(error = ?e, "post_ledger_transaction (opening deposit)");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    tx.commit()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(CreateTenantResp {
        tenant_id,
        budget_id,
        window_instance_id,
        fencing_scope_id,
        unit_id,
        opening_deposit_atomic: req.opening_deposit_atomic,
        workload_instance_id: workload_instance_id.clone(),
        sidecar_config_env: serde_json::json!({
            "SPENDGUARD_SIDECAR_TENANT_ID":                tenant_id.to_string(),
            "SPENDGUARD_SIDECAR_FENCING_SCOPE_ID":         fencing_scope_id.to_string(),
            "SPENDGUARD_SIDECAR_WORKLOAD_INSTANCE_ID":     workload_instance_id,
            "SPENDGUARD_SIDECAR_BUDGET_ID":                budget_id.to_string(),
            "SPENDGUARD_SIDECAR_WINDOW_INSTANCE_ID":       window_instance_id.to_string(),
            "SPENDGUARD_SIDECAR_UNIT_ID":                  unit_id.to_string(),
        }),
    })
    .into_response())
}

#[derive(Serialize)]
struct GetTenantResp {
    tenant_id: Uuid,
    budgets: Vec<BudgetSummary>,
}

#[derive(Serialize)]
struct BudgetSummary {
    budget_id: Uuid,
    unit_id: Uuid,
    available_atomic: String,
    reserved_atomic: String,
    committed_atomic: String,
}

async fn get_tenant(
    Extension(principal): Extension<Principal>,
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Response, StatusCode> {
    // S18: any role may read a tenant they're scoped to.
    if principal.require(Permission::ReadView).is_err() {
        return Err(StatusCode::FORBIDDEN);
    }
    // S18: cross-tenant guard — principal MUST have id in their
    // tenant_ids claim. Returns generic 403 (no tenant-existence
    // leak per S17 spec).
    if principal.assert_tenant(&id).is_err() {
        info!(
            subject = %principal.subject,
            requested_tenant = %id,
            scope = ?principal.tenant_ids,
            "get_tenant rejected — cross-tenant"
        );
        return Err(StatusCode::FORBIDDEN);
    }

    let tenant_id =
        Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?;

    let rows = sqlx::query_as::<
        _,
        (Uuid, Uuid, Option<String>, Option<String>, Option<String>),
    >(
        r#"
        SELECT
            la.budget_id,
            la.unit_id,
            COALESCE(SUM(CASE WHEN la.account_kind='available_budget' AND le.direction='credit' THEN le.amount_atomic
                              WHEN la.account_kind='available_budget' AND le.direction='debit'  THEN -le.amount_atomic ELSE 0 END), 0)::TEXT,
            COALESCE(SUM(CASE WHEN la.account_kind='reserved_hold' AND le.direction='credit' THEN le.amount_atomic
                              WHEN la.account_kind='reserved_hold' AND le.direction='debit'  THEN -le.amount_atomic ELSE 0 END), 0)::TEXT,
            COALESCE(SUM(CASE WHEN la.account_kind='committed_spend' AND le.direction='credit' THEN le.amount_atomic
                              WHEN la.account_kind='committed_spend' AND le.direction='debit'  THEN -le.amount_atomic ELSE 0 END), 0)::TEXT
          FROM ledger_accounts la
     LEFT JOIN ledger_entries le ON le.ledger_account_id = la.ledger_account_id
         WHERE la.tenant_id = $1
         GROUP BY la.budget_id, la.unit_id
        "#,
    )
    .bind(tenant_id)
    .fetch_all(&state.pg)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if rows.is_empty() {
        return Err(StatusCode::NOT_FOUND);
    }

    let budgets = rows
        .into_iter()
        .map(|(b, u, a, r, c)| BudgetSummary {
            budget_id: b,
            unit_id: u,
            available_atomic: a.unwrap_or_else(|| "0".into()),
            reserved_atomic: r.unwrap_or_else(|| "0".into()),
            committed_atomic: c.unwrap_or_else(|| "0".into()),
        })
        .collect();

    Ok(Json(GetTenantResp { tenant_id, budgets }).into_response())
}

#[derive(Serialize)]
struct TombstoneResp {
    tenant_id: Uuid,
    tombstoned: bool,
    note: String,
}

async fn tombstone_tenant(
    Extension(principal): Extension<Principal>,
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Response, StatusCode> {
    // S18: tombstone is admin-only AND tenant-scoped.
    if principal.require(Permission::TenantWrite).is_err() {
        info!(
            subject = %principal.subject,
            roles = ?principal.roles,
            "tombstone_tenant rejected — missing TenantWrite permission"
        );
        return Err(StatusCode::FORBIDDEN);
    }
    if principal.assert_tenant(&id).is_err() {
        info!(
            subject = %principal.subject,
            requested_tenant = %id,
            scope = ?principal.tenant_ids,
            "tombstone_tenant rejected — cross-tenant"
        );
        return Err(StatusCode::FORBIDDEN);
    }
    info!(
        subject = %principal.subject,
        mode = %principal.mode,
        target_tenant = %id,
        "tombstone_tenant invoked"
    );

    let tenant_id =
        Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?;

    // POC: tombstone via fencing_scope expiry. Audit chain is
    // immutable so we don't delete rows — just expire the writer's
    // lease so no further reservations can be issued.
    sqlx::query(
        r#"UPDATE fencing_scopes
              SET ttl_expires_at = now() - interval '1 second'
            WHERE tenant_id = $1
              AND scope_type = 'budget_window'"#,
    )
    .bind(tenant_id)
    .execute(&state.pg)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(TombstoneResp {
        tenant_id,
        tombstoned: true,
        note: "fencing scope TTL expired; new ReserveSet will fail closed. Audit chain immutable.".into(),
    })
    .into_response())
}

fn base64_encode(bytes: &[u8]) -> String {
    // Lightweight Standard b64. Avoid a full base64 crate dep; output
    // is a fresh String of ASCII bytes built char-by-char.
    const TABLE: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::with_capacity(((bytes.len() + 2) / 3) * 4);
    let mut i = 0;
    while i < bytes.len() {
        let b0 = bytes[i];
        let b1 = if i + 1 < bytes.len() { bytes[i + 1] } else { 0 };
        let b2 = if i + 2 < bytes.len() { bytes[i + 2] } else { 0 };
        out.push(TABLE[(b0 >> 2) as usize]);
        out.push(TABLE[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize]);
        out.push(if i + 1 < bytes.len() {
            TABLE[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize]
        } else {
            b'='
        });
        out.push(if i + 2 < bytes.len() {
            TABLE[(b2 & 0x3f) as usize]
        } else {
            b'='
        });
        i += 3;
    }
    String::from_utf8(out).expect("base64 table is ASCII")
}

// ============================================================================
// Phase 5 GA hardening S15: approval REST API
// ============================================================================
//
// GET  /v1/approvals?tenant_id=...&state=...  — list pending or resolved
// GET  /v1/approvals/:id                      — full record + recent events
// POST /v1/approvals/:id/resolve              — { target_state, reason }
//
// Every handler enforces:
//   * Permission::ApprovalResolve via principal.require()
//   * principal.assert_tenant() against the row's tenant_id (after lookup)
//   * Cross-tenant attempts return 403, never 404 (tenant existence not leaked).

#[derive(Debug, Deserialize)]
struct ListApprovalsQuery {
    tenant_id: String,
    /// Optional state filter. Default = pending.
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Debug, Serialize)]
struct ApprovalSummary {
    approval_id: Uuid,
    tenant_id: Uuid,
    decision_id: Uuid,
    state: String,
    ttl_expires_at: chrono::DateTime<chrono::Utc>,
    created_at: chrono::DateTime<chrono::Utc>,
}

async fn list_approvals(
    Extension(principal): Extension<Principal>,
    State(state): State<Arc<AppState>>,
    axum::extract::Query(q): axum::extract::Query<ListApprovalsQuery>,
) -> Result<Response, StatusCode> {
    if principal.require(Permission::ApprovalResolve).is_err() {
        return Err(StatusCode::FORBIDDEN);
    }
    if principal.assert_tenant(&q.tenant_id).is_err() {
        return Err(StatusCode::FORBIDDEN);
    }
    let tenant_uuid = Uuid::parse_str(&q.tenant_id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let state_filter = q.state.unwrap_or_else(|| "pending".into());
    if !["pending", "approved", "denied", "expired", "cancelled"].contains(&state_filter.as_str())
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let limit = q.limit.unwrap_or(50).clamp(1, 200);

    let rows = sqlx::query_as::<
        _,
        (Uuid, Uuid, Uuid, String, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>),
    >(
        r#"
        SELECT approval_id, tenant_id, decision_id, state,
               ttl_expires_at, created_at
          FROM approval_requests
         WHERE tenant_id = $1 AND state = $2
         ORDER BY created_at DESC
         LIMIT $3
        "#,
    )
    .bind(tenant_uuid)
    .bind(&state_filter)
    .bind(limit)
    .fetch_all(&state.pg)
    .await
    .map_err(|e| {
        info!(err = %e, "S15: list_approvals query failed");
        StatusCode::INTERNAL_SERVER_ERROR
    })?
    .into_iter()
    .map(|(aid, tid, did, st, ttl, ct)| ApprovalSummary {
        approval_id: aid,
        tenant_id: tid,
        decision_id: did,
        state: st,
        ttl_expires_at: ttl,
        created_at: ct,
    })
    .collect::<Vec<_>>();
    Ok(Json(rows).into_response())
}

#[derive(Debug, Serialize)]
struct ApprovalDetail {
    approval_id: Uuid,
    tenant_id: Uuid,
    decision_id: Uuid,
    audit_decision_event_id: Uuid,
    state: String,
    ttl_expires_at: chrono::DateTime<chrono::Utc>,
    approver_policy: serde_json::Value,
    requested_effect: serde_json::Value,
    decision_context: serde_json::Value,
    resolved_at: Option<chrono::DateTime<chrono::Utc>>,
    resolved_by_subject: Option<String>,
    resolution_reason: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    recent_events: Vec<ApprovalEventOut>,
}

#[derive(Debug, Serialize)]
struct ApprovalEventOut {
    event_id: Uuid,
    from_state: String,
    to_state: String,
    actor_subject: Option<String>,
    resolution_reason: Option<String>,
    occurred_at: chrono::DateTime<chrono::Utc>,
}

#[allow(clippy::type_complexity)]
async fn get_approval(
    Extension(principal): Extension<Principal>,
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Response, StatusCode> {
    if principal.require(Permission::ApprovalResolve).is_err()
        && principal.require(Permission::ReadView).is_err()
    {
        return Err(StatusCode::FORBIDDEN);
    }
    let approval_uuid = Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?;

    let detail: Option<(
        Uuid,
        Uuid,
        Uuid,
        Uuid,
        String,
        chrono::DateTime<chrono::Utc>,
        serde_json::Value,
        serde_json::Value,
        serde_json::Value,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<String>,
        Option<String>,
        chrono::DateTime<chrono::Utc>,
    )> = sqlx::query_as(
        r#"
        SELECT approval_id, tenant_id, decision_id, audit_decision_event_id,
               state, ttl_expires_at, approver_policy, requested_effect,
               decision_context, resolved_at, resolved_by_subject,
               resolution_reason, created_at
          FROM approval_requests
         WHERE approval_id = $1
        "#,
    )
    .bind(approval_uuid)
    .fetch_optional(&state.pg)
    .await
    .map_err(|e| {
        info!(err = %e, "S15: get_approval query failed");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let detail = match detail {
        Some(d) => d,
        // Information leak avoidance: return 403 not 404 so an
        // attacker can't probe approval_id existence.
        None => return Err(StatusCode::FORBIDDEN),
    };

    // Tenant scope check uses the row's tenant_id (NOT a query param)
    // so an attacker can't claim cross-tenant access.
    let row_tenant = detail.1.to_string();
    if principal.assert_tenant(&row_tenant).is_err() {
        return Err(StatusCode::FORBIDDEN);
    }

    let events: Vec<ApprovalEventOut> = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            String,
            Option<String>,
            Option<String>,
            chrono::DateTime<chrono::Utc>,
        ),
    >(
        r#"
        SELECT event_id, from_state, to_state, actor_subject,
               resolution_reason, occurred_at
          FROM approval_events
         WHERE approval_id = $1
         ORDER BY occurred_at DESC
         LIMIT 20
        "#,
    )
    .bind(approval_uuid)
    .fetch_all(&state.pg)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .into_iter()
    .map(|(eid, fs, ts, sub, rsn, ts_at)| ApprovalEventOut {
        event_id: eid,
        from_state: fs,
        to_state: ts,
        actor_subject: sub,
        resolution_reason: rsn,
        occurred_at: ts_at,
    })
    .collect();

    Ok(Json(ApprovalDetail {
        approval_id: detail.0,
        tenant_id: detail.1,
        decision_id: detail.2,
        audit_decision_event_id: detail.3,
        state: detail.4,
        ttl_expires_at: detail.5,
        approver_policy: detail.6,
        requested_effect: detail.7,
        decision_context: detail.8,
        resolved_at: detail.9,
        resolved_by_subject: detail.10,
        resolution_reason: detail.11,
        created_at: detail.12,
        recent_events: events,
    })
    .into_response())
}

#[derive(Debug, Deserialize)]
struct ResolveApprovalReq {
    target_state: String,
    reason: String,
}

#[derive(Debug, Serialize)]
struct ResolveApprovalResp {
    final_state: String,
    transitioned: bool,
    event_id: Option<Uuid>,
}

/// Three-way result for `parse_approver_policy`. The third arm is what
/// makes the gate a real security boundary: a policy that *looks
/// restrictive* but is malformed (wrong types, empty array, etc.)
/// is treated as fail-closed — Codex round-2 P1.
#[derive(Debug, PartialEq, Eq)]
enum ApproverPolicyParse {
    /// Empty `{}`, JSON null, or an object that carries only
    /// non-restrictive metadata (e.g. `{"description": "..."}`).
    /// Permission gate is the only check.
    NoRestriction,
    /// Restrictive policy with at least one valid role name. Caller
    /// intersects against `principal.roles`.
    Restrict(Vec<String>),
    /// One or more restrictive keys are present but the value is
    /// malformed (non-array where array expected, wrong element type,
    /// empty list, empty string). Treat as fail-closed: the operator
    /// *intended* to restrict but the data is unusable, so widening
    /// access silently is unsafe.
    Malformed,
}

/// Parse `approval_requests.approver_policy` JSONB into a typed
/// outcome. The schema only enforces `JSONB NOT NULL DEFAULT '{}'`,
/// so the parser is the security boundary.
///
/// Accepted restrictive keys:
///   * `roles` / `required_roles`     — array of role-name strings
///   * `role` / `approver_role`       — single role-name string OR
///                                       array of role-name strings
///
/// `approver_role` matches the canonical contract.yaml /
/// `ApprovalDecision.approver_role` field name (Codex round-2 P1).
fn parse_approver_policy(policy: &serde_json::Value) -> ApproverPolicyParse {
    if policy.is_null() {
        return ApproverPolicyParse::NoRestriction;
    }
    let Some(obj) = policy.as_object() else {
        // Non-object, non-null shape (array, scalar) — operator likely
        // intended *something*; fail closed.
        return ApproverPolicyParse::Malformed;
    };
    if obj.is_empty() {
        return ApproverPolicyParse::NoRestriction;
    }

    const ARRAY_KEYS: &[&str] = &["roles", "required_roles"];
    const STRING_OR_ARRAY_KEYS: &[&str] = &["role", "approver_role"];

    let any_restrictive = ARRAY_KEYS
        .iter()
        .chain(STRING_OR_ARRAY_KEYS.iter())
        .any(|k| obj.contains_key(*k));
    if !any_restrictive {
        // Object has only metadata-style keys. No restriction.
        return ApproverPolicyParse::NoRestriction;
    }

    let mut roles: Vec<String> = Vec::new();

    for key in ARRAY_KEYS {
        let Some(v) = obj.get(*key) else { continue };
        let Some(arr) = v.as_array() else {
            return ApproverPolicyParse::Malformed;
        };
        if arr.is_empty() {
            return ApproverPolicyParse::Malformed;
        }
        for item in arr {
            match item.as_str() {
                Some(s) if !s.is_empty() => roles.push(s.to_string()),
                _ => return ApproverPolicyParse::Malformed,
            }
        }
    }

    for key in STRING_OR_ARRAY_KEYS {
        let Some(v) = obj.get(*key) else { continue };
        if let Some(s) = v.as_str() {
            if s.is_empty() {
                return ApproverPolicyParse::Malformed;
            }
            roles.push(s.to_string());
        } else if let Some(arr) = v.as_array() {
            if arr.is_empty() {
                return ApproverPolicyParse::Malformed;
            }
            for item in arr {
                match item.as_str() {
                    Some(s) if !s.is_empty() => roles.push(s.to_string()),
                    _ => return ApproverPolicyParse::Malformed,
                }
            }
        } else {
            return ApproverPolicyParse::Malformed;
        }
    }

    if roles.is_empty() {
        // Restrictive keys present but we somehow extracted no roles.
        // Defensive: fail closed.
        return ApproverPolicyParse::Malformed;
    }
    ApproverPolicyParse::Restrict(roles)
}

async fn resolve_approval(
    Extension(principal): Extension<Principal>,
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<ResolveApprovalReq>,
) -> Result<Response, StatusCode> {
    if principal.require(Permission::ApprovalResolve).is_err() {
        info!(
            subject = %principal.subject,
            roles = ?principal.roles,
            "resolve_approval rejected — missing ApprovalResolve permission"
        );
        return Err(StatusCode::FORBIDDEN);
    }

    let approval_uuid = Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?;
    if !["approved", "denied", "cancelled"].contains(&req.target_state.as_str()) {
        // 'expired' is system-only; not exposed via API.
        return Err(StatusCode::BAD_REQUEST);
    }
    if req.reason.trim().is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Fetch tenant_id + approver_policy (read-only) to enforce
    // cross-tenant + per-approval policy guard BEFORE issuing the SP.
    // Same return-403-not-404 semantics.
    //
    // Codex P1#2: a permission-only check would let any approver in
    // the tenant resolve any approval — bypassing the contract's
    // per-rule approver scoping (`approver_role` in contract.yaml).
    let row: Option<(Uuid, serde_json::Value)> = sqlx::query_as(
        "SELECT tenant_id, approver_policy FROM approval_requests WHERE approval_id = $1",
    )
    .bind(approval_uuid)
    .fetch_optional(&state.pg)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let Some((row_tenant, approver_policy)) = row else {
        return Err(StatusCode::FORBIDDEN);
    };
    if principal.assert_tenant(&row_tenant.to_string()).is_err() {
        info!(
            subject = %principal.subject,
            requested = %row_tenant,
            scope = ?principal.tenant_ids,
            "resolve_approval rejected — cross-tenant"
        );
        return Err(StatusCode::FORBIDDEN);
    }
    match parse_approver_policy(&approver_policy) {
        ApproverPolicyParse::NoRestriction => {
            // Permission + tenant gates already passed; admit.
        }
        ApproverPolicyParse::Restrict(required) => {
            let satisfied = required
                .iter()
                .any(|r| principal.roles.iter().any(|p| p == r));
            if !satisfied {
                info!(
                    subject = %principal.subject,
                    approval_id = %approval_uuid,
                    required_roles = ?required,
                    principal_roles = ?principal.roles,
                    "resolve_approval rejected — approver_policy role mismatch"
                );
                return Err(StatusCode::FORBIDDEN);
            }
        }
        ApproverPolicyParse::Malformed => {
            // Codex round-2 P1: a policy that looks restrictive but
            // can't be parsed cleanly (wrong types / empty array) is
            // a security boundary failure. Fail closed.
            info!(
                subject = %principal.subject,
                approval_id = %approval_uuid,
                approver_policy = %approver_policy,
                "resolve_approval rejected — approver_policy malformed (fail-closed)"
            );
            return Err(StatusCode::FORBIDDEN);
        }
    }

    info!(
        subject = %principal.subject,
        approval_id = %approval_uuid,
        target_state = %req.target_state,
        "S15: resolve_approval invoked"
    );

    let row: (String, bool, Option<Uuid>) = sqlx::query_as(
        "SELECT * FROM resolve_approval_request($1, $2, $3, $4, $5)",
    )
    .bind(approval_uuid)
    .bind(&req.target_state)
    .bind(&principal.subject)
    .bind(&principal.issuer)
    .bind(&req.reason)
    .fetch_one(&state.pg)
    .await
    .map_err(|e| {
        info!(err = %e, "S15: resolve_approval SP failed");
        // SP raises 22023 / P0002 for invalid transitions / missing
        // approval. Both surface here as 422-ish; keep the public
        // mapping conservative.
        StatusCode::CONFLICT
    })?;

    Ok(Json(ResolveApprovalResp {
        final_state: row.0,
        transitioned: row.1,
        event_id: row.2,
    })
    .into_response())
}

#[cfg(test)]
mod approver_policy_tests {
    use super::{parse_approver_policy, ApproverPolicyParse};
    use serde_json::json;

    fn restrict(items: &[&str]) -> ApproverPolicyParse {
        ApproverPolicyParse::Restrict(items.iter().map(|s| s.to_string()).collect())
    }

    // ---- NoRestriction ----------------------------------------------

    #[test]
    fn empty_object_no_restriction() {
        assert_eq!(
            parse_approver_policy(&json!({})),
            ApproverPolicyParse::NoRestriction
        );
    }

    #[test]
    fn json_null_no_restriction() {
        assert_eq!(
            parse_approver_policy(&serde_json::Value::Null),
            ApproverPolicyParse::NoRestriction
        );
    }

    #[test]
    fn metadata_only_object_no_restriction() {
        // Object carries non-restrictive metadata only.
        let p = json!({"description": "billing-team approval flow"});
        assert_eq!(parse_approver_policy(&p), ApproverPolicyParse::NoRestriction);
    }

    // ---- Restrict ---------------------------------------------------

    #[test]
    fn roles_array_extracts() {
        let p = json!({"roles": ["admin", "approver"]});
        assert_eq!(parse_approver_policy(&p), restrict(&["admin", "approver"]));
    }

    #[test]
    fn required_roles_alias_extracts() {
        let p = json!({"required_roles": ["operator"]});
        assert_eq!(parse_approver_policy(&p), restrict(&["operator"]));
    }

    #[test]
    fn role_singleton_string_extracts() {
        let p = json!({"role": "admin"});
        assert_eq!(parse_approver_policy(&p), restrict(&["admin"]));
    }

    /// Codex round-2 P1: contract.yaml / proto's canonical shape is
    /// `approver_role: <name>`. Helper MUST recognize it.
    #[test]
    fn contract_canonical_approver_role_extracts() {
        let p = json!({"approver_role": "approver"});
        assert_eq!(parse_approver_policy(&p), restrict(&["approver"]));
    }

    #[test]
    fn approver_role_array_extracts() {
        let p = json!({"approver_role": ["admin", "approver"]});
        assert_eq!(parse_approver_policy(&p), restrict(&["admin", "approver"]));
    }

    #[test]
    fn multiple_restrictive_keys_unioned() {
        let p = json!({"roles": ["admin"], "approver_role": "approver"});
        assert_eq!(parse_approver_policy(&p), restrict(&["admin", "approver"]));
    }

    // ---- Malformed (fail-closed) -----------------------------------

    /// Codex round-2 P1: malformed restrictive shapes must NOT widen.
    #[test]
    fn roles_with_non_string_element_is_malformed() {
        let p = json!({"roles": ["admin", 42]});
        assert_eq!(parse_approver_policy(&p), ApproverPolicyParse::Malformed);
    }

    #[test]
    fn roles_string_instead_of_array_is_malformed() {
        let p = json!({"roles": "approver"});
        assert_eq!(parse_approver_policy(&p), ApproverPolicyParse::Malformed);
    }

    #[test]
    fn empty_roles_array_is_malformed() {
        // Operator declared a restrictive key but with an empty list —
        // fail closed; do not silently downgrade to "no restriction".
        let p = json!({"roles": []});
        assert_eq!(parse_approver_policy(&p), ApproverPolicyParse::Malformed);
    }

    #[test]
    fn empty_string_role_is_malformed() {
        let p = json!({"role": ""});
        assert_eq!(parse_approver_policy(&p), ApproverPolicyParse::Malformed);
    }

    #[test]
    fn empty_string_approver_role_is_malformed() {
        let p = json!({"approver_role": ""});
        assert_eq!(parse_approver_policy(&p), ApproverPolicyParse::Malformed);
    }

    #[test]
    fn role_as_number_is_malformed() {
        let p = json!({"role": 42});
        assert_eq!(parse_approver_policy(&p), ApproverPolicyParse::Malformed);
    }

    #[test]
    fn approver_role_array_with_null_is_malformed() {
        let p = json!({"approver_role": ["admin", null]});
        assert_eq!(parse_approver_policy(&p), ApproverPolicyParse::Malformed);
    }

    #[test]
    fn array_at_top_is_malformed() {
        // Non-object, non-null — operator likely intended something.
        let p = json!(["admin"]);
        assert_eq!(parse_approver_policy(&p), ApproverPolicyParse::Malformed);
    }

    #[test]
    fn scalar_string_at_top_is_malformed() {
        let p = json!("admin");
        assert_eq!(parse_approver_policy(&p), ApproverPolicyParse::Malformed);
    }

    #[test]
    fn empty_string_array_entry_is_malformed() {
        let p = json!({"roles": ["admin", ""]});
        assert_eq!(parse_approver_policy(&p), ApproverPolicyParse::Malformed);
    }
}
