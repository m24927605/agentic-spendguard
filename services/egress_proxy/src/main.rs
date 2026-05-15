//! `spendguard-egress-proxy` — auto-instrument HTTP proxy for SpendGuard.
//!
//! Slices 2-3 shipped:
//! - Slice 2: crate skeleton + /healthz + RedactedAuth newtype +
//!   tracing config + spendguard-ids shared crate
//! - Slice 3: POST /v1/chat/completions forwarder (byte-identical body,
//!   stream=true → 501, upstream SSE response → 502, header allowlist).
//!   NO SpendGuard gating yet — slice 4 wires the sidecar UDS client +
//!   429-on-STOP fail-closed routing.
//!
//! See `docs/specs/auto-instrument-egress-proxy-spec.md` v7 for the
//! full design.
//!
//! Acceptance criteria invariants enforced here:
//! - `rustls::crypto::aws_lc_rs::default_provider().install_default()`
//!   invoked before any TLS construction (F1 backport pattern)
//! - tracing layer drops Authorization header from spans
//!   (`DefaultMakeSpan::new().include_headers(false)`)
//! - RedactedAuth newtype prevents Display/Debug/Serialize leak
//!   (structural compile-time guarantee — see redacted_auth.rs)
//! - `expose_secret()` called exactly once (audit grep target;
//!   forward.rs upstream HTTP request construction)

use anyhow::{Context, Result};
use axum::{
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::trace::{DefaultMakeSpan, TraceLayer};
use tracing::info;
use tracing_subscriber::EnvFilter;

mod forward;
mod redacted_auth;

#[derive(Debug, Deserialize, Clone)]
struct Config {
    /// Bind address. Spec §8 v0.1 trust model: localhost only by
    /// default. Operators exposing to other interfaces accept the
    /// trust-boundary widening.
    #[serde(default = "default_bind_addr")]
    bind_addr: String,
}

fn default_bind_addr() -> String {
    "127.0.0.1:9000".to_string()
}

#[tokio::main]
async fn main() -> Result<()> {
    // F1 backport pattern: install rustls aws_lc_rs default provider
    // before any TLS use. This service doesn't terminate TLS in slice 2
    // but slice 3 will use reqwest with rustls for the upstream OpenAI
    // call, so we wire the provider here to fail-fast if it can't init.
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!(
            "rustls aws_lc_rs default provider already installed by another crate; \
             cannot continue startup. Check for duplicate rustls initialization paths."
        ))?;

    init_tracing();

    let cfg: Config = envy::prefixed("SPENDGUARD_EGRESS_PROXY_")
        .from_env()
        .context("loading egress-proxy config")?;

    info!(bind_addr = %cfg.bind_addr, "spendguard-egress-proxy starting (slice 2 skeleton)");

    let forward_state =
        Arc::new(forward::ForwardState::new().context("build reqwest client")?);
    let app = build_app(forward_state);
    let addr: SocketAddr = cfg.bind_addr.parse().context("parse bind_addr")?;

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("bind to {}", addr))?;
    info!(addr = %addr, "egress-proxy bound");

    axum::serve(listener, app)
        .await
        .context("axum serve terminated")?;

    Ok(())
}

/// Initialize structured logging. Codex r1 P1.6 + r2 P2-r2.C fix
/// (defense-in-depth #1): tracing-subscriber JSON output; TraceLayer
/// configured with `include_headers(false)` so the default span DOES
/// NOT record the Authorization header. The RedactedAuth newtype is
/// the second layer (structural compile-time guarantee).
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .json()
        .init();
}

fn build_app(forward_state: Arc<forward::ForwardState>) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/chat/completions", post(forward::chat_completions))
        .with_state(forward_state)
        .layer(
            // Defense layer 1 per spec §8: do NOT include headers in
            // request spans. RedactedAuth (defense layer 2) is the
            // structural backstop.
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().include_headers(false)),
        )
}

async fn healthz() -> impl IntoResponse {
    Json(serde_json::json!({ "ok": true, "service": "egress-proxy", "version": env!("CARGO_PKG_VERSION") }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Method, Request, StatusCode},
    };
    use tower::ServiceExt;

    fn test_app() -> Router {
        let state = Arc::new(forward::ForwardState::new().expect("reqwest client"));
        build_app(state)
    }

    #[tokio::test]
    async fn healthz_returns_ok() {
        let app = test_app();
        let req = Request::builder()
            .method(Method::GET)
            .uri("/healthz")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["ok"], true);
        assert_eq!(json["service"], "egress-proxy");
    }

    #[tokio::test]
    async fn unknown_route_returns_404() {
        let app = test_app();
        let req = Request::builder()
            .method(Method::GET)
            .uri("/nonexistent")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
