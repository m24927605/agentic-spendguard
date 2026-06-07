//! Decision-service seam for the HTTP companion (D09 SLICE 1).
//!
//! Per `docs/specs/coverage/D09_kong_ai_gateway/review-standards.md`
//! §1.1 the HTTP companion is a *translation layer*; this trait names
//! the only three operations it needs from the rest of the sidecar so
//! the handlers stay thin and so unit tests can verify routing
//! without standing up a ledger / canonical ingest / contract bundle.
//!
//! Implementations:
//!
//! * [`NoopDecisionService`] — in-process stub used by every test that
//!   doesn't need to assert against the durable audit chain. Lets the
//!   test inject ALLOW/DENY/DEGRADE per call and counts idempotent
//!   replays.
//! * `RealDecisionService` (production, lands with SLICE 3) — owns a
//!   `SidecarState` + `Config` clone and delegates to
//!   `decision::transaction::run_through_reserve` /
//!   `run_commit_estimated` / `run_release`. **Intentionally not
//!   defined in this file.** D09 SLICE 1 ships only the wire surface
//!   + stub; SLICE 3 wires the production implementation.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use parking_lot::Mutex;

/// Stable error surface returned to HTTP handlers. Translated into
/// concrete HTTP status codes inside [`super::handlers`]; the upstream
/// service impl does not need to know about HTTP at all.
#[derive(Debug, thiserror::Error)]
pub enum DecisionServiceError {
    /// Caller supplied a structurally invalid request (missing field,
    /// out-of-range integer, malformed UUID). Maps to 400.
    #[error("invalid request: {0}")]
    InvalidRequest(String),

    /// Caller authenticated, the request parsed, but a dependency
    /// (ledger / canonical ingest / projector) is unhealthy. Maps to
    /// 503 — the Kong plugin then translates into fail-closed
    /// `kong.response.exit(503)` unless the operator has opted into
    /// `fail_open: true`.
    #[error("dependency unavailable: {0}")]
    DependencyUnavailable(String),

    /// Idempotency-Key reused with a different request fingerprint.
    /// Maps to 409 — mirrors `DomainError::IdempotencyConflict`.
    #[error("idempotency conflict: {0}")]
    IdempotencyConflict(String),

    /// Internal error inside the decision lane (panic, signing failure,
    /// contract evaluation bug). Maps to 500.
    #[error("internal error: {0}")]
    Internal(String),
}

/// The three operations the HTTP companion exposes. All methods are
/// `async` because the production impls hit the network (ledger gRPC,
/// canonical ingest gRPC, projector gRPC).
#[async_trait]
pub trait DecisionService: Send + Sync {
    /// Token-count a prompt for a (provider, model) pair. SLICE 3
    /// wires this through to `spendguard-tokenizer`; SLICE 1 ships a
    /// stub that lets Kong tests verify the wire shape.
    async fn tokenize(
        &self,
        req: super::TokenizeRequest,
    ) -> Result<super::TokenizeResponse, DecisionServiceError>;

    /// Run a reserve transaction. The production impl translates the
    /// Kong-shaped JSON into a proto `DecisionRequest`, then delegates
    /// to `decision::transaction::run_through_reserve`. The stub
    /// returns whatever verdict the test pre-loaded.
    async fn decision(
        &self,
        req: super::HttpDecisionRequest,
    ) -> Result<super::HttpDecisionResponse, DecisionServiceError>;

    /// Emit a single trace event (commit or release).
    /// Idempotent on `reservation_id`; duplicate calls SHOULD return
    /// the same ack without producing a second audit row.
    async fn trace(
        &self,
        req: super::TraceRequest,
    ) -> Result<super::TraceAck, DecisionServiceError>;
}

// --------------------------------------------------------------------
// NoopDecisionService — test stub
// --------------------------------------------------------------------

/// Test stub that lets each integration test pre-load a fixed
/// verdict and observe how many times each handler was invoked.
/// Records the last-seen `reservation_id` for /v1/trace so the
/// idempotent-replay test can assert dedup behavior.
#[derive(Default)]
pub struct NoopDecisionService {
    inner: Mutex<NoopState>,
}

#[derive(Default)]
struct NoopState {
    /// Pre-loaded verdict for the next call to `decision()`. Default
    /// `Allow` so tests that don't set a verdict get the boring path.
    next_decision_verdict: Option<DecisionStub>,

    /// Pre-loaded trace verdict.
    next_trace_verdict: Option<TraceStub>,

    /// Pre-loaded tokenize result. Default 8 input tokens so tests
    /// that don't care still get a non-zero count.
    next_tokenize: Option<TokenizeStub>,

    /// Call counters surfaced by `*_count()` getters.
    tokenize_calls: u64,
    decision_calls: u64,
    trace_calls: u64,

    /// Per-reservation_id trace dedup. The production impl uses
    /// `decision::transaction` + ledger idempotency; the stub fakes
    /// it so we can exercise the replay path.
    seen_reservations: std::collections::HashMap<String, super::TraceAck>,
}

#[derive(Clone)]
pub struct DecisionStub {
    pub verdict: super::DecisionVerdict,
    pub reservation_id: String,
    pub decision_id: String,
}

#[derive(Clone)]
pub struct TraceStub {
    pub verdict: super::TraceVerdict,
    pub ledger_transaction_id: String,
}

#[derive(Clone)]
pub struct TokenizeStub {
    pub input_tokens: u32,
    pub tokenizer_tier: String,
    pub tokenizer_version_id: String,
}

impl NoopDecisionService {
    /// Pre-load the verdict the next `decision()` call should return.
    /// Subsequent calls use the same value (we do not pop the slot)
    /// so the idempotent-replay path can hit the same verdict twice.
    pub fn set_next_decision(&self, stub: DecisionStub) {
        self.inner.lock().next_decision_verdict = Some(stub);
    }

    pub fn set_next_trace(&self, stub: TraceStub) {
        self.inner.lock().next_trace_verdict = Some(stub);
    }

    pub fn set_next_tokenize(&self, stub: TokenizeStub) {
        self.inner.lock().next_tokenize = Some(stub);
    }

    pub fn tokenize_count(&self) -> u64 {
        self.inner.lock().tokenize_calls
    }
    pub fn decision_count(&self) -> u64 {
        self.inner.lock().decision_calls
    }
    pub fn trace_count(&self) -> u64 {
        self.inner.lock().trace_calls
    }
}

#[async_trait]
impl DecisionService for NoopDecisionService {
    async fn tokenize(
        &self,
        req: super::TokenizeRequest,
    ) -> Result<super::TokenizeResponse, DecisionServiceError> {
        let mut g = self.inner.lock();
        g.tokenize_calls += 1;
        if req.provider.trim().is_empty() {
            return Err(DecisionServiceError::InvalidRequest(
                "tokenize: provider required".into(),
            ));
        }
        if req.model.trim().is_empty() {
            return Err(DecisionServiceError::InvalidRequest(
                "tokenize: model required".into(),
            ));
        }
        let stub = g.next_tokenize.clone().unwrap_or(TokenizeStub {
            input_tokens: 8,
            tokenizer_tier: "T2".into(),
            tokenizer_version_id: "stub-v1".into(),
        });
        Ok(super::TokenizeResponse {
            input_tokens: stub.input_tokens,
            tokenizer_tier: stub.tokenizer_tier,
            tokenizer_version_id: stub.tokenizer_version_id,
        })
    }

    async fn decision(
        &self,
        req: super::HttpDecisionRequest,
    ) -> Result<super::HttpDecisionResponse, DecisionServiceError> {
        let mut g = self.inner.lock();
        g.decision_calls += 1;
        if req.tenant_id.trim().is_empty() {
            return Err(DecisionServiceError::InvalidRequest(
                "decision: tenant_id required".into(),
            ));
        }
        if req.idempotency_key.trim().is_empty() {
            return Err(DecisionServiceError::InvalidRequest(
                "decision: idempotency_key required".into(),
            ));
        }
        let stub = g.next_decision_verdict.clone().unwrap_or(DecisionStub {
            verdict: super::DecisionVerdict::Allow,
            reservation_id: "00000000-0000-0000-0000-000000000001".into(),
            decision_id: "00000000-0000-0000-0000-000000000002".into(),
        });
        Ok(super::HttpDecisionResponse {
            verdict: stub.verdict,
            reservation_id: stub.reservation_id,
            decision_id: stub.decision_id,
            reason_codes: vec![],
        })
    }

    async fn trace(
        &self,
        req: super::TraceRequest,
    ) -> Result<super::TraceAck, DecisionServiceError> {
        let mut g = self.inner.lock();
        g.trace_calls += 1;
        if req.reservation_id.trim().is_empty() {
            return Err(DecisionServiceError::InvalidRequest(
                "trace: reservation_id required".into(),
            ));
        }
        // Idempotency on reservation_id (mirrors the production
        // ledger dedup). Second + later calls with the same id return
        // the cached ack without re-incrementing the verdict counter.
        if let Some(existing) = g.seen_reservations.get(&req.reservation_id) {
            return Ok(existing.clone());
        }
        let stub = g.next_trace_verdict.clone().unwrap_or(TraceStub {
            verdict: super::TraceVerdict::Accepted,
            ledger_transaction_id: "00000000-0000-0000-0000-000000000099".into(),
        });
        let ack = super::TraceAck {
            verdict: stub.verdict,
            ledger_transaction_id: stub.ledger_transaction_id,
        };
        g.seen_reservations
            .insert(req.reservation_id.clone(), ack.clone());
        Ok(ack)
    }
}

// --------------------------------------------------------------------
// Wire-level helpers — kept here so production code reusing the trait
// can map `DecisionServiceError` into HTTP status codes once and only
// once. The mapping is exercised by the handler unit tests.
// --------------------------------------------------------------------

/// Wire-status mapping. The handlers do the actual `StatusCode` build.
/// Lives next to the error so a future audit grep can find every
/// status decision in one place.
pub fn error_to_status(err: &DecisionServiceError) -> u16 {
    match err {
        DecisionServiceError::InvalidRequest(_) => 400,
        DecisionServiceError::DependencyUnavailable(_) => 503,
        DecisionServiceError::IdempotencyConflict(_) => 409,
        DecisionServiceError::Internal(_) => 500,
    }
}

// --------------------------------------------------------------------
// JSON-shape helpers — pulled out so handlers stay thin and the spec's
// "Kong-shaped JSON" surface lives in one place.
// --------------------------------------------------------------------

/// Convenience for plugin-side debugging: the literal JSON wire shape
/// the companion emits on every failure. Exists for documentation +
/// `serde` round-trip sanity tests, not for hot-path use.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct WireError {
    pub error: String,
    pub code: String,
}

impl WireError {
    pub fn new(err: &DecisionServiceError) -> Self {
        Self {
            error: err.to_string(),
            code: code_for(err).into(),
        }
    }
}

fn code_for(err: &DecisionServiceError) -> &'static str {
    match err {
        DecisionServiceError::InvalidRequest(_) => "SPENDGUARD_BAD_REQUEST",
        DecisionServiceError::DependencyUnavailable(_) => "SPENDGUARD_DEPENDENCY_UNAVAILABLE",
        DecisionServiceError::IdempotencyConflict(_) => "SPENDGUARD_IDEMPOTENCY_CONFLICT",
        DecisionServiceError::Internal(_) => "SPENDGUARD_INTERNAL",
    }
}

#[allow(dead_code)]
pub(crate) fn _arc<T: DecisionService + 'static>(t: T) -> Arc<T> {
    Arc::new(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http_companion::{
        DecisionVerdict, HttpDecisionRequest, TokenizeRequest, TraceRequest, TraceVerdict,
    };

    #[tokio::test]
    async fn noop_tokenize_default() {
        let svc = NoopDecisionService::default();
        let out = svc
            .tokenize(TokenizeRequest {
                provider: "openai".into(),
                model: "gpt-4o-mini".into(),
                prompt: "hello".into(),
            })
            .await
            .unwrap();
        assert_eq!(out.input_tokens, 8);
        assert_eq!(svc.tokenize_count(), 1);
    }

    #[tokio::test]
    async fn noop_tokenize_validates_provider() {
        let svc = NoopDecisionService::default();
        let err = svc
            .tokenize(TokenizeRequest {
                provider: "".into(),
                model: "x".into(),
                prompt: "y".into(),
            })
            .await
            .unwrap_err();
        assert!(matches!(err, DecisionServiceError::InvalidRequest(_)));
        assert_eq!(error_to_status(&err), 400);
    }

    #[tokio::test]
    async fn noop_decision_defaults_to_allow() {
        let svc = NoopDecisionService::default();
        let out = svc
            .decision(HttpDecisionRequest {
                tenant_id: "t1".into(),
                claim_estimate_atomic: "100".into(),
                prompt_class: "general".into(),
                model_class: "openai/gpt-4o-mini".into(),
                idempotency_key: "k1".into(),
                budget_id: None,
            })
            .await
            .unwrap();
        assert_eq!(out.verdict, DecisionVerdict::Allow);
    }

    #[tokio::test]
    async fn noop_decision_returns_loaded_deny() {
        let svc = NoopDecisionService::default();
        svc.set_next_decision(DecisionStub {
            verdict: DecisionVerdict::Deny,
            reservation_id: "".into(),
            decision_id: "d1".into(),
        });
        let out = svc
            .decision(HttpDecisionRequest {
                tenant_id: "t1".into(),
                claim_estimate_atomic: "9000000".into(),
                prompt_class: "general".into(),
                model_class: "openai/gpt-4o-mini".into(),
                idempotency_key: "k1".into(),
                budget_id: None,
            })
            .await
            .unwrap();
        assert_eq!(out.verdict, DecisionVerdict::Deny);
    }

    #[tokio::test]
    async fn noop_trace_idempotent_on_reservation_id() {
        let svc = NoopDecisionService::default();
        let ack1 = svc
            .trace(TraceRequest {
                reservation_id: "r1".into(),
                outcome: TraceVerdict::Accepted,
                provider_event_id: Some("evt1".into()),
                input_tokens: Some(8),
                output_tokens: Some(12),
                actual_amount_atomic: Some("1000".into()),
            })
            .await
            .unwrap();
        let ack2 = svc
            .trace(TraceRequest {
                reservation_id: "r1".into(),
                outcome: TraceVerdict::Accepted,
                provider_event_id: Some("evt1".into()),
                input_tokens: Some(8),
                output_tokens: Some(12),
                actual_amount_atomic: Some("1000".into()),
            })
            .await
            .unwrap();
        assert_eq!(ack1, ack2);
        // Both calls increment the counter (the dedup is on output;
        // we still log the request to surface replay attempts).
        assert_eq!(svc.trace_count(), 2);
    }

    #[tokio::test]
    async fn wire_error_codes() {
        let cases = [
            (
                DecisionServiceError::InvalidRequest("x".into()),
                "SPENDGUARD_BAD_REQUEST",
                400,
            ),
            (
                DecisionServiceError::DependencyUnavailable("x".into()),
                "SPENDGUARD_DEPENDENCY_UNAVAILABLE",
                503,
            ),
            (
                DecisionServiceError::IdempotencyConflict("x".into()),
                "SPENDGUARD_IDEMPOTENCY_CONFLICT",
                409,
            ),
            (
                DecisionServiceError::Internal("x".into()),
                "SPENDGUARD_INTERNAL",
                500,
            ),
        ];
        for (err, code, status) in cases {
            assert_eq!(error_to_status(&err), status);
            let wire = WireError::new(&err);
            assert_eq!(wire.code, code);
        }
    }
}
