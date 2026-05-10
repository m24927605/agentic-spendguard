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

mod metrics;

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
use spendguard_auth::{AuthConfig, Authenticator, Permission, Principal};

use crate::metrics::DashboardMetrics;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tracing::info;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
struct Config {
    bind_addr: String,
    database_url: String,
    /// Phase 5 GA hardening S9: optional canonical DB connection for
    /// audit export endpoint. Empty/missing disables /api/audit/export.
    #[serde(default)]
    canonical_database_url: Option<String>,
    tenant_id: String,
    /// Round-2 #11: Prometheus /metrics endpoint bind addr. Defaults
    /// to `0.0.0.0:9095` (dashboard gets 9095 per the round-2 port
    /// table; ledger=9092, sidecar=9093, control_plane=9094).
    #[serde(default = "default_metrics_addr")]
    metrics_addr: String,
}

fn default_metrics_addr() -> String {
    "0.0.0.0:9095".to_string()
}

struct AppState {
    pg: PgPool,
    /// Phase 5 GA hardening S9: separate pool for canonical_events
    /// (lives in spendguard_canonical DB). None when not configured.
    canonical_pg: Option<PgPool>,
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

    // Phase 5 GA hardening S9: optional canonical DB pool for audit
    // export. Skipped if SPENDGUARD_DASHBOARD_CANONICAL_DATABASE_URL
    // is unset; the export endpoint then returns 503.
    let canonical_pg = if let Some(url) = &cfg.canonical_database_url {
        if !url.trim().is_empty() {
            info!(target = "audit_export", "S9: connecting to canonical DB for audit export");
            Some(
                PgPoolOptions::new()
                    .max_connections(5)
                    .connect(url)
                    .await?,
            )
        } else {
            None
        }
    } else {
        None
    };

    let state = Arc::new(AppState { pg, canonical_pg, tenant_id });

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

    // Round-2 #11: shared metrics counter store + middleware applied
    // to the outer Router (so /healthz and /api/* are both covered).
    let metrics = DashboardMetrics::new();
    if !cfg.metrics_addr.is_empty() {
        let metrics_addr = cfg.metrics_addr.clone();
        let metrics_handle = metrics.clone();
        tokio::spawn(async move {
            if let Err(e) = serve_metrics(metrics_addr, metrics_handle).await {
                tracing::warn!(err = %e, "metrics server terminated");
            }
        });
        info!(addr = %cfg.metrics_addr, "metrics server bound");
    }

    let api_routes = Router::new()
        .route("/api/budgets", get(api_budgets))
        .route("/api/decisions", get(api_decisions))
        .route("/api/deny-stats", get(api_deny_stats))
        .route("/api/outbox-health", get(api_outbox_health))
        // Phase 5 GA hardening S9: audit export endpoint.
        .route("/api/audit/export", get(api_audit_export))
        .layer(from_fn_with_state(auth.clone(), spendguard_auth::require_auth));

    let app = Router::new()
        .route("/", get(index))
        .route("/healthz", get(healthz))
        .merge(api_routes)
        .layer(from_fn_with_state(metrics.clone(), metrics::record_metrics))
        .with_state(state);

    let addr: SocketAddr = cfg.bind_addr.parse()?;
    info!("dashboard listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

/// Round-2 #11: minimal HTTP /metrics endpoint that renders the
/// DashboardMetrics Prometheus text. Same hyper-based pattern as
/// canonical_ingest / ledger / sidecar / control_plane.
async fn serve_metrics(addr: String, metrics: DashboardMetrics) -> anyhow::Result<()> {
    use std::convert::Infallible;
    use hyper::body::Bytes;
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper::{Request, Response};
    use hyper_util::rt::TokioIo;
    use http_body_util::Full;
    use tokio::net::TcpListener;

    let listener = TcpListener::bind(&addr).await?;
    info!(addr = %addr, "dashboard metrics listening");

    loop {
        let (stream, _peer) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let metrics = metrics.clone();
        tokio::task::spawn(async move {
            let svc = service_fn(move |req: Request<hyper::body::Incoming>| {
                let metrics = metrics.clone();
                async move {
                    let body = if req.uri().path() == "/metrics" {
                        metrics.render()
                    } else {
                        "".to_string()
                    };
                    Ok::<_, Infallible>(
                        Response::builder()
                            .header(
                                "content-type",
                                "text/plain; version=0.0.4; charset=utf-8",
                            )
                            .body(Full::new(Bytes::from(body)))
                            .unwrap(),
                    )
                }
            });
            if let Err(e) = http1::Builder::new().serve_connection(io, svc).await {
                tracing::debug!(err = %e, "metrics conn closed");
            }
        });
    }
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
    // S18: viewer / operator / admin / auditor all have ReadView.
    if principal.require(Permission::ReadView).is_err() {
        return Err(StatusCode::FORBIDDEN);
    }
    // Codex round-12 P2: cross-tenant guard. The dashboard is
    // currently env-pinned to state.tenant_id, but a JWT principal
    // scoped to a different tenant (or with no tenant scope at all)
    // could otherwise read these aggregate views for the env tenant.
    // Mirror the api_audit_export pattern: assert_tenant against the
    // env-pinned tenant_id; reject 403 on mismatch / no scope.
    if principal.assert_tenant(&state.tenant_id.to_string()).is_err() {
        info!(
            subject = %principal.subject,
            requested = %state.tenant_id,
            scope = ?principal.tenant_ids,
            "dashboard read rejected — cross-tenant"
        );
        return Err(StatusCode::FORBIDDEN);
    }

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
    // S18: viewer / operator / admin / auditor all have ReadView.
    if principal.require(Permission::ReadView).is_err() {
        return Err(StatusCode::FORBIDDEN);
    }
    // Codex round-12 P2: cross-tenant guard. The dashboard is
    // currently env-pinned to state.tenant_id, but a JWT principal
    // scoped to a different tenant (or with no tenant scope at all)
    // could otherwise read these aggregate views for the env tenant.
    // Mirror the api_audit_export pattern: assert_tenant against the
    // env-pinned tenant_id; reject 403 on mismatch / no scope.
    if principal.assert_tenant(&state.tenant_id.to_string()).is_err() {
        info!(
            subject = %principal.subject,
            requested = %state.tenant_id,
            scope = ?principal.tenant_ids,
            "dashboard read rejected — cross-tenant"
        );
        return Err(StatusCode::FORBIDDEN);
    }

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
    // S18: viewer / operator / admin / auditor all have ReadView.
    if principal.require(Permission::ReadView).is_err() {
        return Err(StatusCode::FORBIDDEN);
    }
    // Codex round-12 P2: cross-tenant guard. The dashboard is
    // currently env-pinned to state.tenant_id, but a JWT principal
    // scoped to a different tenant (or with no tenant scope at all)
    // could otherwise read these aggregate views for the env tenant.
    // Mirror the api_audit_export pattern: assert_tenant against the
    // env-pinned tenant_id; reject 403 on mismatch / no scope.
    if principal.assert_tenant(&state.tenant_id.to_string()).is_err() {
        info!(
            subject = %principal.subject,
            requested = %state.tenant_id,
            scope = ?principal.tenant_ids,
            "dashboard read rejected — cross-tenant"
        );
        return Err(StatusCode::FORBIDDEN);
    }

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
    // S18: viewer / operator / admin / auditor all have ReadView.
    if principal.require(Permission::ReadView).is_err() {
        return Err(StatusCode::FORBIDDEN);
    }
    // Codex round-12 P2: cross-tenant guard. The dashboard is
    // currently env-pinned to state.tenant_id, but a JWT principal
    // scoped to a different tenant (or with no tenant scope at all)
    // could otherwise read these aggregate views for the env tenant.
    // Mirror the api_audit_export pattern: assert_tenant against the
    // env-pinned tenant_id; reject 403 on mismatch / no scope.
    if principal.assert_tenant(&state.tenant_id.to_string()).is_err() {
        info!(
            subject = %principal.subject,
            requested = %state.tenant_id,
            scope = ?principal.tenant_ids,
            "dashboard read rejected — cross-tenant"
        );
        return Err(StatusCode::FORBIDDEN);
    }

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

// ============================================================================
// Phase 5 GA hardening S9: audit export
// ============================================================================
//
// Read-only export of canonical_events scoped to a tenant + time range.
// Operators stream the JSONL output to object storage / SIEM. Cursor
// is the last (recorded_month, ingest_log_offset) tuple — clients
// resume by passing it back as `cursor` query param.
//
// RBAC: AuditExport permission (granted to Admin and Auditor roles
// in S18 policy). Tenant scope: principal.assert_tenant rejects
// cross-tenant exports.
//
// Output format: each line is a JSON object with stable fields:
//   {
//     "event_id": "...",
//     "event_type": "spendguard.audit.decision",
//     "tenant_id": "...",
//     "decision_id": "...",
//     "recorded_at": "2026-05-09T20:00:00Z",
//     "cloudevent_payload": { ... },
//     "producer_signature_hex": "...",
//     "signing_key_id": "...",
//     "ingest_log_offset": 12345,
//     "verification_status": "verified" | "pre_s6" | "disabled" | "quarantined"
//   }
//
// Final line is a manifest JSON object:
//   { "_manifest": { "batch_sha256": "...", "row_count": N,
//                    "from": "...", "to": "...",
//                    "next_cursor": "<recorded_month>:<offset>" | null } }
//
// The hash is sha256 over the concatenated JSONL lines (excluding the
// manifest line). Operators can verify integrity by re-streaming and
// recomputing.

#[derive(Debug, Deserialize)]
struct ExportQuery {
    tenant_id: String,
    /// Inclusive ISO 8601 start time (UTC).
    from: String,
    /// Exclusive ISO 8601 end time (UTC).
    to: String,
    /// Optional cursor from a previous batch — `<recorded_month>:<offset>`.
    /// Stable across exports of the same tenant + range.
    cursor: Option<String>,
    /// Page size. Defaults to 1000; capped at 10000.
    #[serde(default)]
    page_size: Option<usize>,
}

#[derive(Debug, Serialize)]
struct ExportRow {
    event_id: String,
    event_type: String,
    tenant_id: String,
    decision_id: Option<String>,
    recorded_at: chrono::DateTime<chrono::Utc>,
    cloudevent_payload: serde_json::Value,
    producer_signature_hex: String,
    signing_key_id: String,
    signing_algorithm: String,
    ingest_log_offset: i64,
    recorded_month: chrono::NaiveDate,
}

#[derive(Debug, Serialize)]
struct ExportManifest {
    batch_sha256: String,
    row_count: usize,
    from: String,
    to: String,
    /// Next cursor when more rows exist; null when the export is
    /// complete. Format: `<recorded_month_yyyy_mm_dd>:<offset>`.
    next_cursor: Option<String>,
    tenant_id: String,
}

async fn api_audit_export(
    Extension(principal): Extension<Principal>,
    State(state): State<Arc<AppState>>,
    axum::extract::Query(q): axum::extract::Query<ExportQuery>,
) -> Result<Response, StatusCode> {
    // S18 RBAC: AuditExport granted to Admin + Auditor.
    if principal.require(Permission::AuditExport).is_err() {
        info!(
            subject = %principal.subject,
            roles = ?principal.roles,
            "audit export rejected — missing AuditExport permission"
        );
        return Err(StatusCode::FORBIDDEN);
    }
    // S18 cross-tenant guard: principal must be scoped to the
    // requested tenant. Spec: "tenant A cannot export tenant B".
    if principal.assert_tenant(&q.tenant_id).is_err() {
        info!(
            subject = %principal.subject,
            requested = %q.tenant_id,
            scope = ?principal.tenant_ids,
            "audit export rejected — cross-tenant"
        );
        return Err(StatusCode::FORBIDDEN);
    }

    let canonical_pg = match &state.canonical_pg {
        Some(p) => p,
        None => {
            // 503: operator hasn't configured the canonical DB url.
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
    };

    let tenant_uuid = match Uuid::parse_str(&q.tenant_id) {
        Ok(u) => u,
        Err(_) => return Err(StatusCode::BAD_REQUEST),
    };
    let from_ts = chrono::DateTime::parse_from_rfc3339(&q.from)
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .with_timezone(&chrono::Utc);
    let to_ts = chrono::DateTime::parse_from_rfc3339(&q.to)
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .with_timezone(&chrono::Utc);
    if to_ts <= from_ts {
        return Err(StatusCode::BAD_REQUEST);
    }
    let page_size = q.page_size.unwrap_or(1000).min(10_000);

    // Cursor parse.
    let (cursor_month, cursor_offset) = match q.cursor.as_deref() {
        None => (None::<chrono::NaiveDate>, 0i64),
        Some(s) => match s.split_once(':') {
            Some((m, o)) => {
                let month = chrono::NaiveDate::parse_from_str(m, "%Y-%m-%d")
                    .map_err(|_| StatusCode::BAD_REQUEST)?;
                let off: i64 = o.parse().map_err(|_| StatusCode::BAD_REQUEST)?;
                (Some(month), off)
            }
            None => return Err(StatusCode::BAD_REQUEST),
        },
    };

    // Query: ordered by (recorded_month, ingest_log_offset) for cursor
    // stability; bind cursor as a >= predicate.
    let rows: Vec<ExportRow> = sqlx::query_as::<
        _,
        (
            Uuid,                           // event_id
            String,                         // event_type
            Uuid,                           // tenant_id
            Option<Uuid>,                   // decision_id
            chrono::DateTime<chrono::Utc>,  // recorded_at
            serde_json::Value,              // cloudevent_payload
            Vec<u8>,                        // producer_signature
            String,                         // signing_key_id
            String,                         // signing_algorithm
            i64,                            // ingest_log_offset
            chrono::NaiveDate,              // recorded_month
        ),
    >(
        r#"
        SELECT event_id, event_type, tenant_id, decision_id, recorded_at,
               cloudevent_payload, producer_signature, signing_key_id,
               signing_algorithm, ingest_log_offset, recorded_month
          FROM canonical_events
         WHERE tenant_id = $1
           AND recorded_at >= $2
           AND recorded_at < $3
           AND (
                 $4::DATE IS NULL
              OR recorded_month > $4
              OR (recorded_month = $4 AND ingest_log_offset > $5)
           )
         ORDER BY recorded_month, ingest_log_offset
         LIMIT $6
        "#,
    )
    .bind(tenant_uuid)
    .bind(from_ts)
    .bind(to_ts)
    .bind(cursor_month)
    .bind(cursor_offset)
    .bind(page_size as i64)
    .fetch_all(canonical_pg)
    .await
    .map_err(|e| {
        info!(err = %e, "S9: canonical_events query failed");
        StatusCode::INTERNAL_SERVER_ERROR
    })?
    .into_iter()
    .map(|(eid, et, tid, did, rat, payload, sig, kid, algo, off, mon)| ExportRow {
        event_id: eid.to_string(),
        event_type: et,
        tenant_id: tid.to_string(),
        decision_id: did.map(|d| d.to_string()),
        recorded_at: rat,
        cloudevent_payload: payload,
        producer_signature_hex: hex::encode(&sig),
        signing_key_id: kid,
        signing_algorithm: algo,
        ingest_log_offset: off,
        recorded_month: mon,
    })
    .collect();

    // Build JSONL body + sha256 over the lines.
    use sha2::{Digest, Sha256};
    let mut body = String::with_capacity(rows.len() * 256);
    let mut hasher = Sha256::new();
    for row in &rows {
        let line =
            serde_json::to_string(row).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        hasher.update(line.as_bytes());
        hasher.update(b"\n");
        body.push_str(&line);
        body.push('\n');
    }
    let batch_sha256 = format!("{:x}", hasher.finalize());

    // Cursor for next batch.
    let next_cursor = rows.last().and_then(|r| {
        if rows.len() == page_size {
            Some(format!(
                "{}:{}",
                r.recorded_month.format("%Y-%m-%d"),
                r.ingest_log_offset
            ))
        } else {
            None
        }
    });

    let manifest = ExportManifest {
        batch_sha256,
        row_count: rows.len(),
        from: q.from.clone(),
        to: q.to.clone(),
        next_cursor,
        tenant_id: q.tenant_id.clone(),
    };
    body.push_str("{\"_manifest\":");
    body.push_str(
        &serde_json::to_string(&manifest).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    );
    body.push_str("}\n");

    info!(
        subject = %principal.subject,
        tenant = %q.tenant_id,
        row_count = rows.len(),
        "S9: audit export served"
    );

    Ok(([
        ("content-type", "application/x-ndjson"),
        ("x-spendguard-audit-export", "v1"),
    ], body)
        .into_response())
}

