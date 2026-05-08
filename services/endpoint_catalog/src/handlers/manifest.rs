//! Manifest pull endpoint.
//!
//! GET /v1/catalog/manifest
//!   Cache-Control: no-store, max-age=0  (Stage 2 §8.2.4)
//!   Returns the signed manifest from storage.
//!
//! Sidecars MUST poll this at most every `manifest_validity_seconds`
//! (default 300 = matches Sidecar §8 critical_revocation_max_stale).

use axum::{
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};

use crate::server::AppState;

pub async fn get_manifest(State(state): State<AppState>) -> Response {
    match state.store.get("manifest.json").await {
        Ok(Some(bytes)) => {
            let mut resp = (StatusCode::OK, bytes).into_response();
            resp.headers_mut().insert(
                header::CONTENT_TYPE,
                "application/json".parse().unwrap(),
            );
            resp.headers_mut().insert(
                header::CACHE_CONTROL,
                "no-store, max-age=0".parse().unwrap(),
            );
            resp
        }
        Ok(None) => (StatusCode::NOT_FOUND, "no manifest published yet").into_response(),
        Err(e) => {
            tracing::error!(?e, "manifest fetch failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "manifest fetch failed").into_response()
        }
    }
}
