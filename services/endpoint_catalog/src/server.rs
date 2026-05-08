use std::sync::Arc;

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use tokio::sync::broadcast;
use tower_http::trace::TraceLayer;

use crate::{config::ServerConfig, handlers, persistence::store::Store};

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<dyn Store>,
    pub cfg: ServerConfig,
    pub invalidation_tx: broadcast::Sender<String>,
}

pub fn router(store: Arc<dyn Store>, cfg: ServerConfig) -> Router {
    let (tx, _) = broadcast::channel::<String>(64);
    let state = AppState {
        store,
        cfg,
        invalidation_tx: tx,
    };

    Router::new()
        .route("/v1/catalog/manifest", get(handlers::manifest::get_manifest))
        .route("/v1/catalog/events", get(handlers::sse::sse_events))
        // axum 0.7 path syntax: {param}, not :param.
        .route("/v1/catalog/{version_id}", get(handlers::catalog::get_catalog))
        .route("/v1/internal/notify-catalog-change", post(notify_catalog_change))
        .route("/healthz", get(|| async { "ok" }))
        .with_state(state)
        .layer(TraceLayer::new_for_http())
}

/// Internal: publisher pings this after a successful publish to fan out an
/// SSE invalidation hint. Authenticated by a bearer token from
/// `internal_notify_token`. Sidecars MUST NOT call this; mTLS or VPC ACL
/// also restricts to control-plane callers.
async fn notify_catalog_change(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let expected = match state.cfg.internal_notify_token.as_deref() {
        Some(t) if !t.is_empty() => t,
        _ => return (StatusCode::FORBIDDEN, "internal notify disabled").into_response(),
    };
    let auth = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let token = auth.strip_prefix("Bearer ").unwrap_or("");
    if !constant_time_eq::constant_time_eq(token.as_bytes(), expected.as_bytes()) {
        return (StatusCode::UNAUTHORIZED, "bad token").into_response();
    }

    // Best-effort fan-out; ignore lagged subscribers.
    let _ = state.invalidation_tx.send(body);
    (StatusCode::ACCEPTED, "").into_response()
}

mod constant_time_eq {
    /// Constant-time byte slice comparison.
    pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
        if a.len() != b.len() {
            return false;
        }
        let mut diff: u8 = 0;
        for (x, y) in a.iter().zip(b.iter()) {
            diff |= x ^ y;
        }
        diff == 0
    }
}
