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
use tracing::{debug, info, warn};
use tracing_subscriber::EnvFilter;

mod decision;
mod forward;
mod predictor_client;
mod proto;
mod providers;
mod redacted_auth;
mod routing;
mod sidecar_client;

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

    // Slice 4a: connect to sidecar UDS + handshake with retry-with-backoff.
    // Spec §9: fail-fast if 30s elapses without successful handshake.
    let sidecar_cfg = sidecar_client::SidecarConfig::from_env()
        .context("loading sidecar client config")?;
    info!(
        uds = %sidecar_cfg.uds_path.display(),
        tenant = %sidecar_cfg.tenant_id,
        workload = %sidecar_cfg.workload_instance_id,
        "connecting to sidecar UDS"
    );
    let sidecar = sidecar_client::connect_with_retry(&sidecar_cfg)
        .await
        .context("sidecar UDS handshake at startup")?;
    info!(session_id = %sidecar.session_id, "sidecar handshake complete");

    let forward_state =
        Arc::new(forward::ForwardState::new().context("build reqwest client")?);

    // SLICE_10 Phase A: boot Tokenizer (library form; in-process, p99 ≤ 1ms)
    // + optional output_predictor + run_cost_projector gRPC clients. Both
    // gRPC clients are optional in demo deployments — when their endpoint
    // env var is unset, decision::estimate_call_cost falls back per spec
    // §11 (Strategy A only; no RUN_* code emitted; audit row carries -1
    // sentinel for run_predicted_remaining_steps per audit-chain-extension
    // §3.3).
    //
    // Tokenizer.new_with_embedded_assets() is fail-closed: any embedded
    // asset sha256 mismatch panics at boot (Tier 2 panic invariant per
    // tokenizer-service-spec-v1alpha1.md §7.4 + §3.6). This is the only
    // place in egress_proxy startup that fails the binary outright.
    let tokenizer = Arc::new(
        spendguard_tokenizer::Tokenizer::new_with_embedded_assets()
            .context("tokenizer library init failed (Tier 2 invariant fail-closed)")?,
    );
    info!("tokenizer library booted (embedded BPE assets verified)");

    let predictor_cfg = predictor_client::PredictorClientConfig::from_env();
    let output_predictor = if let Some(url) = predictor_cfg.output_predictor_endpoint.clone() {
        info!(endpoint = %url, "dialing output_predictor gRPC");
        match predictor_client::OutputPredictorClient::connect(url.clone()).await {
            Ok(c) => {
                info!("output_predictor client ready");
                Some(c)
            }
            Err(e) => {
                // Per SLICE_05 R1: fall-through behaviour on boot failure is
                // acceptable when the downstream service is explicitly
                // optional. We log + continue with None so demo mode still
                // boots; audit rows simply carry Strategy A only.
                warn!(err = %e, endpoint = %url, "output_predictor connect failed; falling back to local Strategy A");
                None
            }
        }
    } else {
        debug!("SPENDGUARD_PROXY_OUTPUT_PREDICTOR_ENDPOINT unset; Strategy A only");
        None
    };

    let run_cost_projector = if let Some(url) = predictor_cfg.run_cost_projector_endpoint.clone() {
        info!(endpoint = %url, "dialing run_cost_projector gRPC");
        match predictor_client::RunCostProjectorClient::connect(url.clone()).await {
            Ok(c) => {
                info!("run_cost_projector client ready");
                Some(c)
            }
            Err(e) => {
                warn!(err = %e, endpoint = %url, "run_cost_projector connect failed; pass-through (no RUN_* codes)");
                None
            }
        }
    } else {
        debug!("SPENDGUARD_PROXY_RUN_COST_PROJECTOR_ENDPOINT unset; pass-through");
        None
    };

    let app = build_app(
        forward_state,
        sidecar,
        tokenizer,
        output_predictor,
        run_cost_projector,
    );
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

/// Combined app state: HTTP client for upstream forward + sidecar
/// handle for decision gating + pricing cache (slice 4b) + SLICE_10
/// prediction services (tokenizer library + output_predictor + run_cost_projector
/// gRPC clients). All cheaply-clonable; axum requires `Clone` on the state
/// extractor's bound type.
#[derive(Clone)]
pub struct AppState {
    pub forward: Arc<forward::ForwardState>,
    pub sidecar: sidecar_client::SidecarHandle,
    pub pricing_cache: Option<decision::PricingCache>,
    /// SLICE_10 Phase A — in-process Tier 2 BPE tokenizer (p99 ≤ 1ms).
    /// Boot-time integrity check guarantees the embedded encoder assets
    /// match `asset_sha256` constants per tokenizer spec §7.4 fail-fast.
    pub tokenizer: Arc<spendguard_tokenizer::Tokenizer>,
    /// SLICE_10 Phase A — optional output_predictor gRPC client. When
    /// `None`, decision::estimate_call_cost falls back to local Strategy A
    /// (input_tokens × max_tokens_default) per spec §11 failure mode.
    pub output_predictor: Option<predictor_client::OutputPredictorClient>,
    /// SLICE_10 Phase A — optional run_cost_projector gRPC client. When
    /// `None`, no RUN_* codes are emitted and run_predicted_remaining_steps
    /// gets the -1 sentinel per audit-chain-prediction-extension §3.3.
    pub run_cost_projector: Option<predictor_client::RunCostProjectorClient>,
}

fn build_app(
    forward_state: Arc<forward::ForwardState>,
    sidecar: sidecar_client::SidecarHandle,
    tokenizer: Arc<spendguard_tokenizer::Tokenizer>,
    output_predictor: Option<predictor_client::OutputPredictorClient>,
    run_cost_projector: Option<predictor_client::RunCostProjectorClient>,
) -> Router {
    let pricing_cache = std::env::var("SPENDGUARD_PROXY_RUNTIME_ENV_PATH")
        .ok()
        .or_else(|| Some("/var/lib/spendguard/bundles/runtime.env".to_string()))
        .and_then(|path| {
            let pb = std::path::PathBuf::from(path);
            match decision::PricingCache::load(pb.clone()) {
                Ok(c) => Some(c),
                Err(e) => {
                    tracing::warn!(err = %e, path = %pb.display(),
                        "pricing cache disabled (runtime.env unreadable); commits will send empty pricing");
                    None
                }
            }
        });

    let state = AppState {
        forward: forward_state.clone(),
        sidecar: sidecar.clone(),
        pricing_cache,
        tokenizer,
        output_predictor,
        run_cost_projector,
    };
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/v1/chat/completions", post(forward::chat_completions))
        // v0.3: openai-agents SDK shorthand Agent(model="...") hits this.
        // Spec: docs/specs/egress-proxy-v0.3-responses-api.md.
        .route("/v1/responses", post(forward::responses))
        .with_state(state)
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

/// Slice 4a: /readyz reflects sidecar handshake state.
/// Returns 200 only after `connect_with_retry` has handed us a ready
/// `SidecarHandle`. Pre-handshake (during startup race) this would
/// 503; but since main() awaits the handshake before binding the
/// listener, /readyz is effectively always 200 once the listener is
/// up. The `ready` AtomicBool still guards against a future "set
/// false on disconnect" extension.
async fn readyz(axum::extract::State(state): axum::extract::State<AppState>) -> impl IntoResponse {
    if state.sidecar.is_ready() {
        (axum::http::StatusCode::OK, Json(serde_json::json!({
            "ready": true,
            "sidecar_session_id": state.sidecar.session_id,
        })))
    } else {
        (axum::http::StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({
            "ready": false,
            "reason": "sidecar handshake not complete",
        })))
    }
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
        let forward = Arc::new(forward::ForwardState::new().expect("reqwest client"));
        // Test build: skip real sidecar handshake. Provide a stub
        // handle with `ready=true` so unit tests that only hit
        // /healthz / /readyz don't need a real UDS server.
        let sidecar = make_stub_sidecar_handle();
        // SLICE_10 Phase A: tokenizer library + None for the two
        // optional gRPC clients. Tokenizer init is fast (~50ms) and
        // tests /healthz / /readyz routes don't go near the predictor
        // path, so the None branch is unreached.
        let tokenizer = Arc::new(
            spendguard_tokenizer::Tokenizer::new_with_embedded_assets()
                .expect("tokenizer init"),
        );
        build_app(forward, sidecar, tokenizer, None, None)
    }

    /// Helper: produce a SidecarHandle without a real UDS server.
    /// The client field uses a tonic channel that points at a
    /// never-reachable URI; this works for unit tests that don't
    /// actually call gRPC methods.
    fn make_stub_sidecar_handle() -> sidecar_client::SidecarHandle {
        use tonic::transport::Endpoint;
        let channel = Endpoint::try_from("http://[::1]:1")
            .unwrap()
            .connect_lazy();
        let client =
            crate::proto::sidecar_adapter::v1::sidecar_adapter_client::SidecarAdapterClient::new(
                channel,
            );
        sidecar_client::SidecarHandle {
            client,
            ready: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
            session_id: "test-session".to_string(),
            tenant_id: "00000000-0000-4000-8000-000000000001".to_string(),
        }
    }

    // Allow the test_app helper to compile without a real pricing
    // cache. The pricing_cache field is Option<PricingCache>; tests
    // that don't go through chat_completions don't exercise it.

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
