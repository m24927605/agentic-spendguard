//! Axum handlers + Kong-shaped JSON request/response shapes.
//!
//! Per `docs/specs/coverage/D09_kong_ai_gateway/review-standards.md`
//! §2.1 each handler body MUST be a thin wrapper (< 50 LOC) that
//! delegates straight to a [`super::DecisionService`] method and
//! translates the typed error back into an HTTP status code. Adding
//! business logic here is a BLOCK finding.

use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};

use super::service::{error_to_status, DecisionService, DecisionServiceError, WireError};

// --------------------------------------------------------------------
// Wire shapes
// --------------------------------------------------------------------

/// `POST /v1/tokenize` request body.
///
/// The Kong plugin sends the raw upstream request body alongside the
/// resolved `(provider, model)`. SLICE 3 wires the real tokenizer; in
/// SLICE 1 the stub returns a deterministic count.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TokenizeRequest {
    pub provider: String,
    pub model: String,
    /// Full prompt text. Plugin passes the OpenAI/Anthropic body
    /// flattened to a single string; the production tokenizer
    /// re-tokenizes per-message.
    pub prompt: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct TokenizeResponse {
    pub input_tokens: u32,
    /// One of `T1` / `T2` / `T3` per `docs/specs/tokenizer-spec-v1alpha1.md`
    /// §6.3. The Kong plugin echoes this into the audit row's
    /// `tokenizer_tier` column on commit.
    pub tokenizer_tier: String,
    pub tokenizer_version_id: String,
}

/// `POST /v1/decision` request body — Kong-shaped subset of the gRPC
/// `DecisionRequest`. SLICE 3 inflates this into a full proto request
/// inside `RealDecisionService::decision`.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DecisionRequest {
    pub tenant_id: String,
    /// Caller-supplied reserve amount in atomic units (cents-per-dollar
    /// for USD). String to dodge JS float precision per LiteLLM SDK
    /// convention; production impl re-parses into `BigInt`.
    pub claim_estimate_atomic: String,
    pub prompt_class: String,
    pub model_class: String,
    /// Required. Reuses the same fingerprint cache the gRPC adapter
    /// uses (Stage 2 §4.3 idempotency).
    pub idempotency_key: String,
    /// Optional. Overrides the bundle's default budget when the Kong
    /// plugin is routing across multiple budgets for the same tenant.
    pub budget_id: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct DecisionResponse {
    pub verdict: DecisionVerdict,
    /// Empty for DENY (no reservation is taken). Always present for
    /// ALLOW. DEGRADE returns the upstream reservation when one was
    /// minted, otherwise empty (degrade is fail-closed by default).
    pub reservation_id: String,
    pub decision_id: String,
    /// Audit reason codes — copied verbatim from the gRPC response so
    /// dashboard filters work across both transports.
    #[serde(default)]
    pub reason_codes: Vec<String>,
}

/// Kong-shaped decision verdict. Wire-stable strings so a future
/// migration to additional verdicts is additive.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum DecisionVerdict {
    Allow,
    Deny,
    Degrade,
}

/// `POST /v1/trace` request body — single LLM_CALL_POST event from the
/// Kong `body_filter` phase.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TraceRequest {
    pub reservation_id: String,
    /// SUCCESS → commit lane. RUN_ABORTED / PROVIDER_ERROR /
    /// CLIENT_TIMEOUT → release lane.
    pub outcome: TraceVerdict,
    /// Upstream provider event id (OpenAI response id, Anthropic
    /// message id). Used for dedup at the canonical ingest layer.
    pub provider_event_id: Option<String>,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    /// Atomic-unit cost actually realized by the provider. Empty +
    /// SUCCESS → sidecar falls through to estimated-amount commit
    /// (mirrors the gRPC adapter's `run_commit_estimated` path).
    pub actual_amount_atomic: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TraceVerdict {
    Accepted,
    Rejected,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct TraceAck {
    pub verdict: TraceVerdict,
    pub ledger_transaction_id: String,
}

// --------------------------------------------------------------------
// Handlers
// --------------------------------------------------------------------

/// `POST /v1/tokenize` handler. Thin wrapper per review-standards §2.1.
pub async fn tokenize_handler<S>(
    State(svc): State<Arc<S>>,
    Json(req): Json<TokenizeRequest>,
) -> Response
where
    S: DecisionService + 'static,
{
    match svc.tokenize(req).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => error_response(&e),
    }
}

/// `POST /v1/decision` handler. Thin wrapper per review-standards §2.1.
pub async fn decision_handler<S>(
    State(svc): State<Arc<S>>,
    Json(req): Json<DecisionRequest>,
) -> Response
where
    S: DecisionService + 'static,
{
    match svc.decision(req).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => error_response(&e),
    }
}

/// `POST /v1/trace` handler. Thin wrapper per review-standards §2.1.
pub async fn trace_handler<S>(State(svc): State<Arc<S>>, Json(req): Json<TraceRequest>) -> Response
where
    S: DecisionService + 'static,
{
    match svc.trace(req).await {
        Ok(ack) => (StatusCode::OK, Json(ack)).into_response(),
        Err(e) => error_response(&e),
    }
}

fn error_response(err: &DecisionServiceError) -> Response {
    let status =
        StatusCode::from_u16(error_to_status(err)).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let body = Json(WireError::new(err));
    (status, body).into_response()
}

#[cfg(test)]
mod tests {
    //! Pure-handler tests against the noop service. The mTLS / wire
    //! tests live in `tests/http_companion_test.rs`.

    use super::*;
    use crate::http_companion::{
        build_router,
        service::{DecisionStub, NoopDecisionService},
    };
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    fn router(svc: Arc<NoopDecisionService>) -> axum::Router {
        build_router(svc, 4 * 1024 * 1024)
    }

    #[tokio::test]
    async fn tokenize_basic_returns_200_and_count() {
        let svc = Arc::new(NoopDecisionService::default());
        let app = router(svc.clone());
        let body = serde_json::json!({
            "provider": "openai",
            "model": "gpt-4o-mini",
            "prompt": "hello world",
        })
        .to_string();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/tokenize")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let parsed: TokenizeResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed.input_tokens, 8);
        assert_eq!(svc.tokenize_count(), 1);
    }

    #[tokio::test]
    async fn tokenize_rejects_missing_provider() {
        let svc = Arc::new(NoopDecisionService::default());
        let app = router(svc.clone());
        let body = serde_json::json!({ "provider": "", "model": "m", "prompt": "p" }).to_string();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/tokenize")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn decision_allow_returns_reservation_id() {
        let svc = Arc::new(NoopDecisionService::default());
        let app = router(svc.clone());
        let body = serde_json::json!({
            "tenant_id": "t1",
            "claim_estimate_atomic": "100",
            "prompt_class": "general",
            "model_class": "openai/gpt-4o-mini",
            "idempotency_key": "k1",
        })
        .to_string();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/decision")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let parsed: DecisionResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed.verdict, DecisionVerdict::Allow);
        assert!(!parsed.reservation_id.is_empty());
    }

    #[tokio::test]
    async fn decision_deny_carries_no_reservation() {
        let svc = Arc::new(NoopDecisionService::default());
        svc.set_next_decision(DecisionStub {
            verdict: DecisionVerdict::Deny,
            reservation_id: "".into(),
            decision_id: "d1".into(),
        });
        let app = router(svc.clone());
        let body = serde_json::json!({
            "tenant_id": "t1",
            "claim_estimate_atomic": "999999",
            "prompt_class": "general",
            "model_class": "openai/gpt-4o-mini",
            "idempotency_key": "k1",
        })
        .to_string();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/decision")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let parsed: DecisionResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed.verdict, DecisionVerdict::Deny);
        assert!(parsed.reservation_id.is_empty());
    }

    #[tokio::test]
    async fn decision_missing_idempotency_key_rejected() {
        let svc = Arc::new(NoopDecisionService::default());
        let app = router(svc.clone());
        let body = serde_json::json!({
            "tenant_id": "t1",
            "claim_estimate_atomic": "100",
            "prompt_class": "general",
            "model_class": "openai/gpt-4o-mini",
            "idempotency_key": "",
        })
        .to_string();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/decision")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn trace_replay_returns_same_ack() {
        let svc = Arc::new(NoopDecisionService::default());
        let body = serde_json::json!({
            "reservation_id": "r1",
            "outcome": "ACCEPTED",
            "provider_event_id": "chatcmpl-abc",
            "input_tokens": 8,
            "output_tokens": 16,
            "actual_amount_atomic": "1000",
        })
        .to_string();

        let app1 = router(svc.clone());
        let resp1 = app1
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/trace")
                    .header("content-type", "application/json")
                    .body(Body::from(body.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp1.status(), StatusCode::OK);
        let bytes1 = axum::body::to_bytes(resp1.into_body(), 4096).await.unwrap();
        let ack1: TraceAck = serde_json::from_slice(&bytes1).unwrap();

        let app2 = router(svc.clone());
        let resp2 = app2
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/trace")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes2 = axum::body::to_bytes(resp2.into_body(), 4096).await.unwrap();
        let ack2: TraceAck = serde_json::from_slice(&bytes2).unwrap();
        assert_eq!(ack1, ack2);
    }

    #[tokio::test]
    async fn trace_run_aborted_routed_through() {
        // Stub returns Accepted regardless; this test is the wire
        // shape gate — production impl maps RUN_ABORTED to release
        // lane in SLICE 3.
        let svc = Arc::new(NoopDecisionService::default());
        let app = router(svc.clone());
        let body = serde_json::json!({
            "reservation_id": "r2",
            "outcome": "REJECTED",
            "provider_event_id": null,
        })
        .to_string();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/trace")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(svc.trace_count(), 1);
    }

    #[tokio::test]
    async fn malformed_json_returns_4xx() {
        let svc = Arc::new(NoopDecisionService::default());
        let app = router(svc.clone());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/decision")
                    .header("content-type", "application/json")
                    .body(Body::from("not json"))
                    .unwrap(),
            )
            .await
            .unwrap();
        // axum returns 400 for JSON parse failures.
        assert!(
            resp.status() == StatusCode::BAD_REQUEST
                || resp.status() == StatusCode::UNPROCESSABLE_ENTITY,
            "expected 4xx for malformed JSON, got {}",
            resp.status()
        );
    }
}
