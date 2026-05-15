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
    routing::{get, post},
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
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!("failed to install rustls aws_lc_rs default provider"))?;

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
        // CA-P3.6: cost_advisor approval rendering + operator resolve.
        .route("/api/approvals", get(api_approvals_list))
        .route("/api/approvals/:id", get(api_approval_detail))
        .route("/api/approvals/:id/resolve", post(api_approval_resolve))
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

  <h2>5. Cost Advisor approvals (pending)</h2>
  <section>
    <div id="approvals-list" class="stale">loading…</div>
    <div id="approval-detail" style="display:none; margin-top:1rem;
         padding-top:1rem; border-top:1px solid #ddd;">
      <h3 style="font-size:0.9rem; margin:0 0 0.5rem 0;">
        Selected approval <code id="approval-detail-id"></code>
      </h3>
      <pre id="approval-detail-body"></pre>
      <div style="margin-top:0.75rem;">
        <label style="display:block; font-size:0.85rem; margin-bottom:0.25rem;">
          Reason (required, max 1024 chars):
        </label>
        <input id="approval-reason" type="text"
               style="width:100%; padding:0.4rem; font-size:0.85rem;
                      box-sizing:border-box; border:1px solid #ccc;
                      border-radius:3px;">
      </div>
      <div style="margin-top:0.5rem;">
        <button id="btn-approve" type="button"
                style="background:#2a7; color:white; border:0; padding:0.4rem 1rem;
                       border-radius:3px; cursor:pointer; margin-right:0.5rem;">
          Approve
        </button>
        <button id="btn-deny" type="button"
                style="background:#c33; color:white; border:0; padding:0.4rem 1rem;
                       border-radius:3px; cursor:pointer;">
          Deny
        </button>
        <span id="approval-action-result" style="margin-left:1rem;
              font-size:0.85rem;"></span>
      </div>
    </div>
  </section>

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

    // ---- CA-P3.6: approvals listing + resolve ----------------------
    let selectedApproval = null;

    async function loadApprovals() {
      const el = document.getElementById("approvals-list");
      try {
        const r = await fetch("/api/approvals", { headers });
        if (!r.ok) throw new Error("HTTP " + r.status);
        const j = await r.json();
        if (!Array.isArray(j) || j.length === 0) {
          el.textContent = "(no pending cost_advisor approvals)";
          el.classList.remove("stale", "fail");
          return;
        }
        const rows = j.map(a =>
          `<div style="margin-bottom:0.4rem; font-family:ui-monospace,monospace;
                       font-size:0.8rem;">
             <button type="button" data-id="${a.approval_id}"
                     class="approval-view-btn"
                     style="background:#fff; border:1px solid #aaa;
                            padding:0.2rem 0.6rem; cursor:pointer;
                            margin-right:0.5rem;">View</button>
             <code>${a.approval_id}</code> — finding
             <code>${a.proposing_finding_id || "(none)"}</code> —
             created ${a.created_at}
           </div>`
        ).join("");
        el.innerHTML = rows;
        el.classList.remove("stale", "fail");
        document.querySelectorAll(".approval-view-btn").forEach(b => {
          b.addEventListener("click", () => viewApproval(b.dataset.id));
        });
      } catch (e) {
        el.textContent = "error: " + e;
        el.classList.add("fail");
      }
    }

    async function viewApproval(id) {
      const wrap = document.getElementById("approval-detail");
      const body = document.getElementById("approval-detail-body");
      const idEl = document.getElementById("approval-detail-id");
      const resultEl = document.getElementById("approval-action-result");
      idEl.textContent = id;
      body.textContent = "loading…";
      wrap.style.display = "block";
      resultEl.textContent = "";
      selectedApproval = id;
      try {
        const r = await fetch("/api/approvals/" + encodeURIComponent(id),
                              { headers });
        if (!r.ok) throw new Error("HTTP " + r.status);
        const j = await r.json();
        body.textContent = JSON.stringify(j, null, 2);
      } catch (e) {
        body.textContent = "error: " + e;
      }
    }

    async function resolveApproval(targetState) {
      if (!selectedApproval) return;
      const reason = document.getElementById("approval-reason").value.trim();
      if (!reason) {
        document.getElementById("approval-action-result").textContent =
          "reason required";
        return;
      }
      try {
        const r = await fetch(
          "/api/approvals/" + encodeURIComponent(selectedApproval) + "/resolve",
          {
            method: "POST",
            headers: { ...headers, "Content-Type": "application/json" },
            body: JSON.stringify({ target_state: targetState, reason }),
          }
        );
        const j = await r.json();
        document.getElementById("approval-action-result").textContent =
          r.ok ? `→ ${j.final_state} (transitioned=${j.transitioned})`
               : `error: HTTP ${r.status}`;
        if (r.ok) {
          await loadApprovals();
          if (j.transitioned) {
            document.getElementById("approval-detail").style.display = "none";
            selectedApproval = null;
          }
        }
      } catch (e) {
        document.getElementById("approval-action-result").textContent =
          "error: " + e;
      }
    }

    document.getElementById("btn-approve")
      .addEventListener("click", () => resolveApproval("approved"));
    document.getElementById("btn-deny")
      .addEventListener("click", () => resolveApproval("denied"));

    loadApprovals();
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

// =====================================================================
// CA-P3.6: cost_advisor approval rendering — list / detail / resolve.
// =====================================================================
//
// The dashboard reads approval_requests + cost_findings directly from
// the ledger DB (same pattern as the other /api/* endpoints). Auth:
// `Permission::ApprovalResolve` required for ALL three endpoints
// (codex round-13 P2 tightened these on control_plane; dashboard
// follows the same boundary).
//
// The resolve endpoint invokes the `resolve_approval_request` SP —
// the canonical state-transition path that fires the
// `approval_requests_state_change_notify` trigger, which
// bundle_registry picks up via PgListener.

#[derive(Serialize)]
struct ApprovalSummary {
    approval_id: Uuid,
    decision_id: Uuid,
    state: String,
    proposing_finding_id: Option<Uuid>,
    ttl_expires_at: chrono::DateTime<chrono::Utc>,
    created_at: chrono::DateTime<chrono::Utc>,
}

async fn api_approvals_list(
    Extension(principal): Extension<Principal>,
    State(state): State<Arc<AppState>>,
) -> Result<Response, StatusCode> {
    if principal.require(Permission::ApprovalResolve).is_err() {
        return Err(StatusCode::FORBIDDEN);
    }
    if principal.assert_tenant(&state.tenant_id.to_string()).is_err() {
        return Err(StatusCode::FORBIDDEN);
    }

    let rows = sqlx::query_as::<
        _,
        (
            Uuid,
            Uuid,
            String,
            Option<Uuid>,
            chrono::DateTime<chrono::Utc>,
            chrono::DateTime<chrono::Utc>,
        ),
    >(
        r#"
        SELECT approval_id, decision_id, state, proposing_finding_id,
               ttl_expires_at, created_at
          FROM approval_requests
         WHERE tenant_id = $1
           AND proposal_source = 'cost_advisor'
           AND state = 'pending'
         ORDER BY created_at DESC
         LIMIT 50
        "#,
    )
    .bind(state.tenant_id)
    .fetch_all(&state.pg)
    .await
    .map_err(|e| {
        info!(err = %e, "approvals_list query failed");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let out: Vec<ApprovalSummary> = rows
        .into_iter()
        .map(
            |(aid, did, st, finding, ttl, ct)| ApprovalSummary {
                approval_id: aid,
                decision_id: did,
                state: st,
                proposing_finding_id: finding,
                ttl_expires_at: ttl,
                created_at: ct,
            },
        )
        .collect();
    Ok(Json(out).into_response())
}

#[derive(Serialize)]
struct ApprovalDetail {
    approval_id: Uuid,
    tenant_id: Uuid,
    decision_id: Uuid,
    state: String,
    proposal_source: String,
    proposed_dsl_patch: Option<serde_json::Value>,
    proposing_finding_id: Option<Uuid>,
    /// Finding evidence joined from cost_findings (NULL when the
    /// approval is sidecar_decision-sourced or the finding has been
    /// retention-swept).
    finding_evidence: Option<serde_json::Value>,
    ttl_expires_at: chrono::DateTime<chrono::Utc>,
    created_at: chrono::DateTime<chrono::Utc>,
}

async fn api_approval_detail(
    Extension(principal): Extension<Principal>,
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Response, StatusCode> {
    if principal.require(Permission::ApprovalResolve).is_err() {
        return Err(StatusCode::FORBIDDEN);
    }
    let approval_uuid = Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?;

    let row: Option<(
        Uuid,
        Uuid,
        Uuid,
        String,
        String,
        Option<serde_json::Value>,
        Option<Uuid>,
        chrono::DateTime<chrono::Utc>,
        chrono::DateTime<chrono::Utc>,
    )> = sqlx::query_as(
        r#"
        SELECT approval_id, tenant_id, decision_id, state, proposal_source,
               proposed_dsl_patch, proposing_finding_id,
               ttl_expires_at, created_at
          FROM approval_requests
         WHERE approval_id = $1
        "#,
    )
    .bind(approval_uuid)
    .fetch_optional(&state.pg)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let Some(detail) = row else {
        // Information-leak avoidance: 403 rather than 404 so an
        // attacker can't probe approval_id existence (mirrors
        // control_plane's `get_approval` handler).
        return Err(StatusCode::FORBIDDEN);
    };

    // Tenant scope check uses the row's tenant_id (NOT a query param).
    if principal.assert_tenant(&detail.1.to_string()).is_err() {
        return Err(StatusCode::FORBIDDEN);
    }

    // Join with cost_findings for the evidence JSON (cost_advisor rows
    // carry this; sidecar_decision rows don't).
    let finding_evidence: Option<serde_json::Value> = if let Some(fid) = detail.6 {
        sqlx::query_scalar::<_, serde_json::Value>(
            "SELECT evidence FROM cost_findings WHERE finding_id = $1",
        )
        .bind(fid)
        .fetch_optional(&state.pg)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    } else {
        None
    };

    Ok(Json(ApprovalDetail {
        approval_id: detail.0,
        tenant_id: detail.1,
        decision_id: detail.2,
        state: detail.3,
        proposal_source: detail.4,
        proposed_dsl_patch: detail.5,
        proposing_finding_id: detail.6,
        finding_evidence,
        ttl_expires_at: detail.7,
        created_at: detail.8,
    })
    .into_response())
}

#[derive(Deserialize)]
struct ResolveBody {
    target_state: String,
    reason: String,
}

#[derive(Serialize)]
struct ResolveResp {
    approval_id: Uuid,
    final_state: String,
    transitioned: bool,
    event_id: Option<Uuid>,
}

/// Resolve a cost_advisor approval via the dashboard UI.
///
/// **Scope (codex CA-P3.6 r1 P1)**: this endpoint is restricted to
/// `proposal_source='cost_advisor'` rows. sidecar_decision approvals
/// have non-trivial `approver_policy` (per-rule approver_role from
/// the contract DSL) and tighter TTLs that control_plane's resolve
/// handler enforces. cost_advisor proposals are inserted by
/// `cost_advisor_create_proposal` SP with `approver_policy='{}'`
/// (no per-policy restriction) so the policy-check gap that would
/// exist for sidecar_decision doesn't apply here. Any sidecar_decision
/// row will return 404 from this endpoint; operators resolve those
/// via `control_plane.POST /v1/approvals/:id/resolve`.
///
/// TTL guard: mirrors control_plane's r3 P1 — reject 409 if
/// state=pending and ttl_expires_at has passed.
///
/// Error mapping: SP transition errors → 409 Conflict (not 500),
/// matching control_plane semantics.
async fn api_approval_resolve(
    Extension(principal): Extension<Principal>,
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(body): Json<ResolveBody>,
) -> Result<Response, StatusCode> {
    if principal.require(Permission::ApprovalResolve).is_err() {
        return Err(StatusCode::FORBIDDEN);
    }
    let approval_uuid = Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?;

    if body.target_state != "approved" && body.target_state != "denied" {
        return Err(StatusCode::BAD_REQUEST);
    }
    let reason_trimmed = body.reason.trim();
    if reason_trimmed.is_empty() || reason_trimmed.len() > 1024 {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Pre-check: cost_advisor scope + tenant + state + TTL.
    let row: Option<(
        Uuid,
        String,
        String,
        chrono::DateTime<chrono::Utc>,
    )> = sqlx::query_as(
        "SELECT tenant_id, state, proposal_source, ttl_expires_at \
           FROM approval_requests WHERE approval_id = $1",
    )
    .bind(approval_uuid)
    .fetch_optional(&state.pg)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let Some((row_tenant, row_state, row_source, ttl_expires_at)) = row else {
        return Err(StatusCode::FORBIDDEN);
    };
    if principal.assert_tenant(&row_tenant.to_string()).is_err() {
        return Err(StatusCode::FORBIDDEN);
    }
    if row_source != "cost_advisor" {
        // Out of scope for this endpoint. Return 403 (same as for
        // missing rows + cross-tenant) to avoid leaking existence
        // info to a probe.
        info!(
            subject = %principal.subject,
            approval_id = %approval_uuid,
            row_source = %row_source,
            "dashboard resolve rejected — non-cost_advisor proposal must resolve via control_plane"
        );
        return Err(StatusCode::FORBIDDEN);
    }
    // Idempotency: if already terminal AND target matches, return
    // 200 transitioned=false. If target differs from current terminal
    // state, that's a conflict (409).
    if row_state != "pending" {
        if row_state == body.target_state {
            return Ok(Json(ResolveResp {
                approval_id: approval_uuid,
                final_state: row_state,
                transitioned: false,
                event_id: None,
            })
            .into_response());
        }
        return Err(StatusCode::CONFLICT);
    }
    // TTL guard for pending rows (mirrors control_plane r3 P1).
    if ttl_expires_at <= chrono::Utc::now() {
        info!(
            subject = %principal.subject,
            approval_id = %approval_uuid,
            ttl_expires_at = %ttl_expires_at,
            "dashboard resolve rejected — approval expired (TTL passed)"
        );
        return Err(StatusCode::CONFLICT);
    }

    let result: Result<
        (String, bool, Option<Uuid>),
        sqlx::Error,
    > = sqlx::query_as(
        r#"
        SELECT final_state, transitioned, event_id
          FROM resolve_approval_request($1::uuid, $2::text, $3::text, $4::text, $5::text)
        "#,
    )
    .bind(approval_uuid)
    .bind(&body.target_state)
    .bind(&principal.subject)
    .bind(&principal.issuer)
    .bind(reason_trimmed)
    .fetch_one(&state.pg)
    .await;

    let (final_state, transitioned, event_id) = match result {
        Ok(row) => row,
        Err(e) => {
            // SP signals invalid-state-transition via a typed error.
            // Map to 409 (not 500) so the client knows the row state
            // changed under them. control_plane uses the same pattern.
            info!(err = %e, approval_id = %approval_uuid, "resolve_approval_request SP error");
            return Err(StatusCode::CONFLICT);
        }
    };

    info!(
        subject = %principal.subject,
        approval_id = %approval_uuid,
        target_state = %body.target_state,
        final_state = %final_state,
        transitioned,
        "CA-P3.6: cost_advisor approval resolved via dashboard"
    );

    Ok(Json(ResolveResp {
        approval_id: approval_uuid,
        final_state,
        transitioned,
        event_id,
    })
    .into_response())
}

