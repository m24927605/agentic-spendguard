//! GET /healthz handler.

use axum::{extract::State, http::StatusCode};
use std::sync::Arc;

use crate::server::AppState;

pub async fn healthz(State(state): State<Arc<AppState>>) -> (StatusCode, &'static str) {
    if sqlx::query("SELECT 1").fetch_one(&state.pg).await.is_err() {
        return (StatusCode::SERVICE_UNAVAILABLE, "db unreachable");
    }
    (StatusCode::OK, "ok")
}
