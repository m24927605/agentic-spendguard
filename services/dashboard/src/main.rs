//! SpendGuard Operator Dashboard MVP — Phase 4 O7.
//!
//! Read-only HTTP service that renders 4 views:
//!   GET /                      — single-page dashboard (HTML + fetch())
//!   GET /api/budgets           — current available / reserved / committed
//!                                per (budget_id, unit) for the active tenant
//!   GET /api/decisions         — last 50 decisions (operation_kind +
//!                                decision_id + posting_state + recorded_at)
//!   GET /api/deny-stats        — counts of denied_decision rows by hour
//!                                for the last 24h
//!   GET /api/outbox-health     — pending_forward / forwarded counts +
//!                                oldest pending row age
//!
//! Auth (Phase 5 GA hardening S17): OIDC JWT or static-token (demo
//! profile only) via `spendguard-auth` middleware. Bearer token in the
//! `Authorization` header. Subject + tenant scope come from the
//! authenticated Principal in axum extensions.
//!
//! Tenant resolution is still env-based at the dashboard level
//! (`SPENDGUARD_DASHBOARD_TENANT_ID`); S18 will switch to deriving
//! tenant scope from the Principal's `tenant_ids` claim.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    middleware::from_fn_with_state,
    response::{Html, IntoResponse, Response},
    routing::get,
    Extension, Json, Router,
};
use serde::{Deserialize, Serialize};
use spendguard_auth::{AuthConfig, Authenticator, Principal};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tracing::info;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
struct Config {
    bind_addr: String,
    database_url: String,
    tenant_id: String,
}

struct AppState {
    pg: PgPool,
    tenant_id: Uuid,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let envfilter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,spendguard_dashboard=debug"));
    tracing_subscriber::registry()
        .with(envfilter)
        .with(fmt::layer().json())
        .init();

    let cfg: Config = envy::prefixed("SPENDGUARD_DASHBOARD_").from_env()?;
    let tenant_id = Uuid::parse_str(&cfg.tenant_id)?;

    let pg = PgPoolOptions::new()
        .max_connections(5)
        .connect(&cfg.database_url)
        .await?;

    let state = Arc::new(AppState { pg, tenant_id });

    // Phase 5 GA hardening S17: build Authenticator (jwt or
    // static_token-with-demo-profile) before binding the listener.
    let profile = std::env::var("SPENDGUARD_PROFILE").unwrap_or_default();
    let auth_cfg = AuthConfig::from_env("SPENDGUARD_DASHBOARD", &profile)
        .map_err(|e| anyhow::anyhow!("S17: build dashboard auth config: {e}"))?;
    let auth = Arc::new(
        Authenticator::from_config(auth_cfg)
            .await
            .map_err(|e| anyhow::anyhow!("S17: init authenticator: {e}"))?,
    );

    let api_routes = Router::new()
        .route("/api/budgets", get(api_budgets))
        .route("/api/decisions", get(api_decisions))
        .route("/api/deny-stats", get(api_deny_stats))
        .route("/api/outbox-health", get(api_outbox_health))
        .layer(from_fn_with_state(auth.clone(), spendguard_auth::require_auth));

    let app = Router::new()
        .route("/", get(index))
        .route("/healthz", get(healthz))
        .merge(api_routes)
        .with_state(state);

    let addr: SocketAddr = cfg.bind_addr.parse()?;
    info!("dashboard listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn healthz() -> &'static str {
    "ok"
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

const INDEX_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>SpendGuard Dashboard</title>
  <style>
    body { font-family: -apple-system, BlinkMacSystemFont, sans-serif;
           margin: 2rem; max-width: 1200px; color: #222; }
    h1 { font-size: 1.5rem; margin-bottom: 0.5rem; }
    h2 { font-size: 1rem; margin-top: 1.5rem; color: #555; }
    section { margin-bottom: 1rem; padding: 0.75rem 1rem;
              background: #f9f9f9; border-radius: 4px;
              border: 1px solid #e0e0e0; }
    pre { font-family: ui-monospace, monospace; font-size: 0.8rem;
          white-space: pre-wrap; margin: 0; }
    .stale { color: #999; font-style: italic; }
    .pass { color: #2a7; }
    .fail { color: #c33; }
  </style>
</head>
<body>
  <h1>SpendGuard Dashboard <span class="stale">(read-only POC)</span></h1>
  <p class="stale">
    Phase 4 O7 MVP. Auth: <code>Authorization: Bearer ${TOKEN}</code>
    on /api/* endpoints.
  </p>

  <h2>1. Budgets</h2>
  <section><pre id="budgets" class="stale">loading…</pre></section>

  <h2>2. Recent decisions (last 50)</h2>
  <section><pre id="decisions" class="stale">loading…</pre></section>

  <h2>3. DENY histogram (24h)</h2>
  <section><pre id="deny-stats" class="stale">loading…</pre></section>

  <h2>4. Outbox forwarder health</h2>
  <section><pre id="outbox-health" class="stale">loading…</pre></section>

  <script>
    const token = window.prompt("Bearer token (one-time, not stored):");
    const headers = token
      ? { "Authorization": "Bearer " + token }
      : {};
    async function load(path, target) {
      try {
        const r = await fetch(path, { headers });
        const j = await r.json();
        document.getElementById(target).textContent =
          JSON.stringify(j, null, 2);
        document.getElementById(target).classList.remove("stale");
      } catch (e) {
        document.getElementById(target).textContent = "error: " + e;
        document.getElementById(target).classList.add("fail");
      }
    }
    load("/api/budgets",      "budgets");
    load("/api/decisions",    "decisions");
    load("/api/deny-stats",   "deny-stats");
    load("/api/outbox-health","outbox-health");
  </script>
</body>
</html>"#;

#[derive(Serialize)]
struct BudgetRow {
    budget_id: Uuid,
    window_instance_id: Uuid,
    unit_kind: String,
    currency: Option<String>,
    available_atomic: String,
    reserved_atomic: String,
    committed_atomic: String,
}

async fn api_budgets(
    Extension(principal): Extension<Principal>,
    State(state): State<Arc<AppState>>,
) -> Result<Response, StatusCode> {
    let _ = principal; // S18 will scope queries to principal.tenant_ids

    let rows = sqlx::query_as::<_, (Uuid, Uuid, String, Option<String>, Option<sqlx::types::BigDecimal>, Option<sqlx::types::BigDecimal>, Option<sqlx::types::BigDecimal>)>(
        r#"
        SELECT
            la.budget_id,
            la.window_instance_id,
            lu.unit_kind,
            lu.currency,
            COALESCE(SUM(CASE WHEN la.account_kind='available_budget' AND le.direction='credit' THEN le.amount_atomic
                              WHEN la.account_kind='available_budget' AND le.direction='debit'  THEN -le.amount_atomic ELSE 0 END), 0) AS available_atomic,
            COALESCE(SUM(CASE WHEN la.account_kind='reserved_hold' AND le.direction='credit' THEN le.amount_atomic
                              WHEN la.account_kind='reserved_hold' AND le.direction='debit'  THEN -le.amount_atomic ELSE 0 END), 0) AS reserved_atomic,
            COALESCE(SUM(CASE WHEN la.account_kind='committed_spend' AND le.direction='credit' THEN le.amount_atomic
                              WHEN la.account_kind='committed_spend' AND le.direction='debit'  THEN -le.amount_atomic ELSE 0 END), 0) AS committed_atomic
        FROM ledger_accounts la
        JOIN ledger_units lu ON lu.unit_id = la.unit_id
        LEFT JOIN ledger_entries le ON le.ledger_account_id = la.ledger_account_id
        WHERE la.tenant_id = $1
        GROUP BY la.budget_id, la.window_instance_id, lu.unit_kind, lu.currency
        ORDER BY la.budget_id, lu.unit_kind
        "#,
    )
    .bind(state.tenant_id)
    .fetch_all(&state.pg)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let out: Vec<BudgetRow> = rows
        .into_iter()
        .map(|(bid, wid, uk, cur, avail, res, comm)| BudgetRow {
            budget_id: bid,
            window_instance_id: wid,
            unit_kind: uk,
            currency: cur,
            available_atomic: avail.map(|d| d.to_string()).unwrap_or_else(|| "0".into()),
            reserved_atomic: res.map(|d| d.to_string()).unwrap_or_else(|| "0".into()),
            committed_atomic: comm.map(|d| d.to_string()).unwrap_or_else(|| "0".into()),
        })
        .collect();

    Ok(Json(out).into_response())
}

#[derive(Serialize)]
struct DecisionRow {
    ledger_transaction_id: Uuid,
    operation_kind: String,
    posting_state: String,
    decision_id: Option<Uuid>,
    recorded_at: chrono::DateTime<chrono::Utc>,
}

async fn api_decisions(
    Extension(principal): Extension<Principal>,
    State(state): State<Arc<AppState>>,
) -> Result<Response, StatusCode> {
    let _ = principal; // S18 will scope queries to principal.tenant_ids

    let rows = sqlx::query_as::<_, (Uuid, String, String, Option<Uuid>, chrono::DateTime<chrono::Utc>)>(
        r#"
        SELECT ledger_transaction_id, operation_kind, posting_state,
               decision_id, recorded_at
          FROM ledger_transactions
         WHERE tenant_id = $1
         ORDER BY recorded_at DESC
         LIMIT 50
        "#,
    )
    .bind(state.tenant_id)
    .fetch_all(&state.pg)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let out: Vec<DecisionRow> = rows
        .into_iter()
        .map(|(tx, op, st, did, ts)| DecisionRow {
            ledger_transaction_id: tx,
            operation_kind: op,
            posting_state: st,
            decision_id: did,
            recorded_at: ts,
        })
        .collect();

    Ok(Json(out).into_response())
}

#[derive(Serialize)]
struct DenyStatRow {
    bucket_hour: chrono::DateTime<chrono::Utc>,
    count: i64,
}

async fn api_deny_stats(
    Extension(principal): Extension<Principal>,
    State(state): State<Arc<AppState>>,
) -> Result<Response, StatusCode> {
    let _ = principal; // S18 will scope queries to principal.tenant_ids

    let rows = sqlx::query_as::<_, (chrono::DateTime<chrono::Utc>, i64)>(
        r#"
        SELECT date_trunc('hour', recorded_at) AS bucket_hour,
               COUNT(*)::bigint AS count
          FROM ledger_transactions
         WHERE tenant_id = $1
           AND operation_kind = 'denied_decision'
           AND recorded_at > now() - interval '24 hours'
         GROUP BY bucket_hour
         ORDER BY bucket_hour
        "#,
    )
    .bind(state.tenant_id)
    .fetch_all(&state.pg)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let out: Vec<DenyStatRow> = rows
        .into_iter()
        .map(|(b, c)| DenyStatRow { bucket_hour: b, count: c })
        .collect();
    Ok(Json(out).into_response())
}

#[derive(Serialize)]
struct OutboxHealth {
    pending: i64,
    forwarded: i64,
    oldest_pending_age_seconds: Option<f64>,
}

async fn api_outbox_health(
    Extension(principal): Extension<Principal>,
    State(state): State<Arc<AppState>>,
) -> Result<Response, StatusCode> {
    let _ = principal; // S18 will scope queries to principal.tenant_ids

    let (pending, forwarded): (i64, i64) = sqlx::query_as(
        r#"
        SELECT
            COUNT(*) FILTER (WHERE pending_forward) AS pending,
            COUNT(*) FILTER (WHERE NOT pending_forward) AS forwarded
          FROM audit_outbox
         WHERE tenant_id = $1
        "#,
    )
    .bind(state.tenant_id)
    .fetch_one(&state.pg)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let oldest_pending_age: Option<f64> = sqlx::query_scalar(
        r#"
        SELECT EXTRACT(EPOCH FROM (now() - MIN(recorded_at)))::float8
          FROM audit_outbox
         WHERE tenant_id = $1
           AND pending_forward
        "#,
    )
    .bind(state.tenant_id)
    .fetch_one(&state.pg)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(OutboxHealth {
        pending,
        forwarded,
        oldest_pending_age_seconds: oldest_pending_age,
    })
    .into_response())
}
