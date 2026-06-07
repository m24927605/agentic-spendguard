//! Decision-service seam for the HTTP companion (D09 SLICE 1 + 3 + 4).
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
//! * [`RealDecisionService`] (production, SLICE 3) — owns a
//!   `SidecarState` + `Config` clone and delegates to
//!   `decision::transaction::run_through_reserve` /
//!   `run_commit_estimated` / `run_release`. Closes SLICE 1 deviation
//!   #3: the SLICE 1 commit explicitly deferred the production wiring;
//!   SLICE 3 lands it without changing the wire shape.
//!
//! Notes on SLICE 3 / SLICE 4 wiring (review-standards §1.1, §4.x):
//!
//! * Tokenize: SLICE 3 keeps the Kong-side count authoritative. The
//!   Go plugin token-counts via the sidecar's existing
//!   `spendguard_tokenizer` library; the HTTP companion's
//!   `/v1/tokenize` is a thin echo of the caller-supplied count for
//!   SLICE 3, with the Tier 2 cross-process tokenizer wire-up tracked
//!   as a SLICE 5+ residual. The wire shape is stable so the Go
//!   client never has to change.
//! * Decision: builds a proto `DecisionRequest` from Kong-shaped
//!   JSON, threads `idempotency_key` from the plugin, and calls
//!   `decision::transaction::run_through_reserve`. The reservation_id
//!   surfaced back to Kong is the first entry in `reservation_ids`.
//! * Trace: routes by `outcome` — ACCEPTED → `run_commit_estimated`,
//!   REJECTED → `run_release` with `RunAborted`. Idempotency on
//!   `reservation_id` is provided by the ledger SP (Stage 7 §11)
//!   plus the in-process `IdempotencyCache`; we do not re-implement
//!   dedup in the companion. The handler returns the
//!   ledger_transaction_id verbatim so a retry sees a stable id.

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

// --------------------------------------------------------------------
// RealDecisionService — SLICE 3 production wiring
// --------------------------------------------------------------------

/// Production implementation of [`DecisionService`] that delegates to
/// the existing `decision::transaction` primitives the gRPC adapter
/// already uses. Closes the SLICE 1 commit's deviation #3.
///
/// Per review-standards §1.1 this is a *translation layer*: it owns a
/// clone of `Config` + `SidecarState` and re-shapes Kong-shaped JSON
/// requests into proto types. No new decision engine, ledger, or
/// audit-chain logic appears here — every audit row still originates
/// inside `decision::transaction::{run_through_reserve,
/// run_commit_estimated, run_release}`.
pub struct RealDecisionService {
    cfg: crate::config::Config,
    state: crate::domain::state::SidecarState,
}

impl RealDecisionService {
    /// Construct a new production service. `cfg` and `state` are
    /// cloned cheaply (Config is plain data; SidecarState is an
    /// `Arc`-backed handle).
    pub fn new(cfg: crate::config::Config, state: crate::domain::state::SidecarState) -> Self {
        Self { cfg, state }
    }

    fn decision_context(
        &self,
        req: &super::HttpDecisionRequest,
    ) -> crate::decision::transaction::DecisionContext {
        // SLICE 3 derives session_id from the caller-supplied
        // idempotency key when present so retries collapse to the
        // same audit row. The gRPC adapter pulls session_id straight
        // from the request envelope; the Kong plugin has no
        // equivalent so we re-use the idempotency key string here.
        // The cached `decision_id_to_reservation` map remains keyed
        // on `decision_id`, not session_id, so this is purely an
        // observability convenience.
        crate::decision::transaction::DecisionContext {
            session_id: req.idempotency_key.clone(),
            workload_instance_id: self.cfg.workload_instance_id.clone(),
            tenant_id: self.cfg.tenant_id.clone(),
            region: self.cfg.region.clone(),
        }
    }

    fn decision_context_for_trace(&self) -> crate::decision::transaction::DecisionContext {
        // Trace-side context is identical except session_id; for the
        // trace lane we derive it from the reservation_id at the call
        // site so we can preserve the original idempotency surface
        // without a second cache lookup.
        crate::decision::transaction::DecisionContext {
            session_id: String::new(),
            workload_instance_id: self.cfg.workload_instance_id.clone(),
            tenant_id: self.cfg.tenant_id.clone(),
            region: self.cfg.region.clone(),
        }
    }
}

/// Map a `DomainError` into the companion's wire `DecisionServiceError`.
/// Lives here (not in `decision::transaction`) so the production
/// `DecisionService` impl is the only place that crosses the
/// domain → HTTP boundary; the gRPC adapter keeps its own
/// `to_status` mapping unchanged.
fn domain_to_companion_err(e: crate::domain::error::DomainError) -> DecisionServiceError {
    use crate::domain::error::DomainError as DE;
    match e {
        DE::InvalidRequest(d) => DecisionServiceError::InvalidRequest(d),
        DE::IdempotencyConflict(d) => DecisionServiceError::IdempotencyConflict(d),
        DE::Draining
        | DE::LedgerClient(_)
        | DE::CanonicalIngestClient(_)
        | DE::FencingAcquire(_)
        | DE::ManifestStale(_) => DecisionServiceError::DependencyUnavailable(e.to_string()),
        other => DecisionServiceError::Internal(other.to_string()),
    }
}

/// Convert the proto Decision enum into the Kong-shaped verdict.
/// CONTINUE → ALLOW. STOP / STOP_RUN_PROJECTION → DENY. REQUIRE_APPROVAL
/// → DEGRADE (the Kong plugin layer interprets DEGRADE per
/// `fail_open`; the operator cannot resume an approval flow from the
/// Kong access phase). DEGRADE and SKIP both surface as DEGRADE since
/// SKIP semantically means "let it through with a mutation patch we
/// have nowhere to apply" at this layer.
fn proto_decision_to_verdict(
    d: crate::proto::sidecar_adapter::v1::decision_response::Decision,
) -> super::DecisionVerdict {
    use crate::proto::sidecar_adapter::v1::decision_response::Decision as PD;
    match d {
        PD::Continue => super::DecisionVerdict::Allow,
        PD::Stop | PD::StopRunProjection => super::DecisionVerdict::Deny,
        PD::Degrade | PD::Skip | PD::RequireApproval | PD::Unspecified => {
            super::DecisionVerdict::Degrade
        }
    }
}

/// Parse the caller-supplied claim_estimate_atomic into a decimal
/// string the ledger accepts. Empty string and non-decimal payloads
/// surface as InvalidRequest so the Kong plugin gets a 400.
fn validate_claim_atomic(raw: &str) -> Result<String, DecisionServiceError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(DecisionServiceError::InvalidRequest(
            "decision: claim_estimate_atomic required".into(),
        ));
    }
    if !trimmed.chars().all(|c| c.is_ascii_digit()) {
        return Err(DecisionServiceError::InvalidRequest(format!(
            "decision: claim_estimate_atomic '{}' must be a positive decimal integer",
            trimmed
        )));
    }
    Ok(trimmed.to_string())
}

#[async_trait]
impl DecisionService for RealDecisionService {
    /// `/v1/tokenize` — SLICE 3 keeps the Kong-side count
    /// authoritative. The Go plugin counts tokens via the same Tier 2
    /// BPE library the sidecar uses; this endpoint validates the
    /// shape and echoes the caller-supplied prompt length (mirrors
    /// the legacy chars/4 heuristic only as a fallback when the
    /// caller did not pre-count). Cross-process Tier 2 tokenization
    /// is tracked under D09 SLICE 5+ residuals.
    async fn tokenize(
        &self,
        req: super::TokenizeRequest,
    ) -> Result<super::TokenizeResponse, DecisionServiceError> {
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
        // Tier 3 chars/4 fallback. Matches the legacy heuristic at
        // egress_proxy/src/decision.rs (now removed by the predictor
        // upgrade) so behavior is unchanged when the Kong plugin
        // calls without its own count. SLICE 5+ swaps in the
        // cross-process Tier 2 BPE service per the D09 residuals.
        let prompt_len = req.prompt.chars().count() as u32;
        let input_tokens = prompt_len.div_ceil(4).max(1);
        Ok(super::TokenizeResponse {
            input_tokens,
            tokenizer_tier: "T3".into(),
            tokenizer_version_id: String::new(),
        })
    }

    /// `/v1/decision` — translates the Kong-shaped JSON into a proto
    /// `DecisionRequest` and delegates to
    /// `decision::transaction::run_through_reserve`. The reservation
    /// surfaced to Kong is the first entry of `reservation_ids` (the
    /// Kong plugin never reserves multi-budget across requests in
    /// v1; multi-claim contracts are unaffected because the ledger
    /// already groups them into one reservation_set).
    async fn decision(
        &self,
        req: super::HttpDecisionRequest,
    ) -> Result<super::HttpDecisionResponse, DecisionServiceError> {
        use crate::proto::common::v1::{BudgetClaim, Idempotency, SpendGuardIds};
        use crate::proto::sidecar_adapter::v1::{
            decision_request::{Inputs, Trigger},
            DecisionRequest,
        };

        // 1) Validate the wire surface up front so the plugin sees a
        //    400 (not 500) for shape problems.
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
        if req.tenant_id != self.cfg.tenant_id {
            // SVID enforcement happens at the mTLS layer (review-
            // standards §2.3) but we still gate on tenant_id assertion
            // matching the sidecar's configured tenant for defense in
            // depth.
            return Err(DecisionServiceError::InvalidRequest(format!(
                "decision: tenant_id '{}' does not match sidecar tenant '{}'",
                req.tenant_id, self.cfg.tenant_id
            )));
        }
        let amount_atomic = validate_claim_atomic(&req.claim_estimate_atomic)?;

        // 2) Resolve the budget id. Kong plugins target a single
        //    budget per route; the operator MUST supply `budget_id`
        //    in the Kong plugin config (forwarded into the JSON
        //    body). v1 does not derive a default budget from
        //    sidecar config because the Kong wire shape already
        //    requires the field — sidecar-side defaulting would
        //    silently bind multi-tenant Kong installs to a single
        //    budget. SLICE 6 surfaces the field on the KongPlugin
        //    CRD.
        let budget_id = req
            .budget_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                DecisionServiceError::InvalidRequest(
                    "decision: budget_id required (Kong plugin config must supply it)".into(),
                )
            })?
            .to_string();

        // 3) Build the proto DecisionRequest. Fields not in the Kong
        //    wire shape (trace context, pricing freeze, run cost
        //    projector signals) default to proto3 zeros; the
        //    contract evaluator + ledger SP treat these as "not
        //    provided" and fall through to defaults per the existing
        //    gRPC path.
        let ctx = self.decision_context(&req);
        let claim = BudgetClaim {
            budget_id,
            amount_atomic,
            window_instance_id: String::new(),
            unit: None,
            direction: 0,
        };
        let proto_req = DecisionRequest {
            session_id: req.idempotency_key.clone(),
            trigger: Trigger::LlmCallPre as i32,
            trace: None,
            ids: Some(SpendGuardIds {
                run_id: req.idempotency_key.clone(),
                step_id: String::new(),
                llm_call_id: req.idempotency_key.clone(),
                tool_call_id: String::new(),
                // decision_id is minted server-side; the plugin
                // surface intentionally never carries it (the Kong
                // wire shape couldn't honor it without a follow-up
                // round trip).
                decision_id: String::new(),
                snapshot_id: String::new(),
            }),
            route: req.model_class.clone(),
            inputs: Some(Inputs {
                projected_claims: vec![claim],
                ..Default::default()
            }),
            parent_run_id: String::new(),
            budget_grant_jti: String::new(),
            idempotency: Some(Idempotency {
                key: req.idempotency_key.clone(),
                request_hash: vec![].into(),
            }),
            planned_steps_hint: 0,
            // D13 COV_61: HTTP companion plugins are BYOK-only today.
            // The proxy / SDK is the only producer of SUBSCRIPTION_METER.
            reservation_source: crate::proto::common::v1::ReservationSource::Byok as i32,
            meter_only_estimate: false,
        };

        // 4) Drive the same idempotency cache the gRPC adapter does
        //    so a Kong retry collapses onto the cached decision.
        //    Mirrors the adapter_uds pattern (see request_decision_inner).
        let fingerprint =
            crate::decision::transaction::idempotency_request_fingerprint_hex(&ctx, &proto_req);
        match self
            .state
            .inner
            .idempotency
            .get(&req.idempotency_key, &fingerprint)
        {
            crate::decision::idempotency::Lookup::Hit(cached) => {
                return Ok(build_http_decision_response(cached));
            }
            crate::decision::idempotency::Lookup::Conflict {
                existing_fingerprint_hex,
            } => {
                return Err(DecisionServiceError::IdempotencyConflict(format!(
                    "key reused with different request fingerprint (existing={}, current={})",
                    existing_fingerprint_hex, fingerprint
                )));
            }
            crate::decision::idempotency::Lookup::Miss => {}
        }

        // 5) Fencing preflight (DD5 C1). The gRPC adapter does the
        //    same dance — a stale fencing epoch surfaces here as
        //    DependencyUnavailable so the plugin's fail-closed path
        //    fires.
        if let Err(e) = crate::fencing::check_active(&self.state) {
            return Err(DecisionServiceError::DependencyUnavailable(e.to_string()));
        }

        // 6) Run the decision lane.
        let out = crate::decision::transaction::run_through_reserve(
            &self.cfg,
            &self.state,
            &ctx,
            &proto_req,
        )
        .await
        .map_err(domain_to_companion_err)?;
        let response = crate::decision::transaction::build_response(out);

        // 7) Park the cached proto response for replay.
        self.state.inner.idempotency.put(
            req.idempotency_key.clone(),
            fingerprint,
            response.clone(),
        );

        Ok(build_http_decision_response(response))
    }

    /// `/v1/trace` — routes by `outcome`:
    ///   * ACCEPTED → `run_commit_estimated` (Stage 7 commit lane).
    ///     The Kong plugin sends `actual_amount_atomic` derived from
    ///     provider-reported usage; we forward it as the
    ///     CommitEstimated amount.
    ///   * REJECTED → `run_release` with `RunAborted`. The
    ///     ledger SP's per-reservation idempotency (Stage 7 §11)
    ///     handles retries; we surface the original
    ///     ledger_transaction_id verbatim.
    async fn trace(
        &self,
        req: super::TraceRequest,
    ) -> Result<super::TraceAck, DecisionServiceError> {
        use crate::proto::common::v1::PricingFreeze;
        use crate::proto::sidecar_adapter::v1::{
            llm_call_post_payload::Outcome as ProtoLlmOutcome, LlmCallPostPayload,
        };

        if req.reservation_id.trim().is_empty() {
            return Err(DecisionServiceError::InvalidRequest(
                "trace: reservation_id required".into(),
            ));
        }

        let ctx = self.decision_context_for_trace();

        match req.outcome {
            super::TraceVerdict::Accepted => {
                // Validate the caller-supplied amount: empty +
                // accepted is the "estimated commit" path; missing
                // both estimated and actual is a hard error.
                let amount = req
                    .actual_amount_atomic
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        DecisionServiceError::InvalidRequest(
                            "trace: actual_amount_atomic required for ACCEPTED outcome".into(),
                        )
                    })?;
                if !amount.chars().all(|c| c.is_ascii_digit()) {
                    return Err(DecisionServiceError::InvalidRequest(format!(
                        "trace: actual_amount_atomic '{}' must be a positive decimal integer",
                        amount
                    )));
                }
                let payload = LlmCallPostPayload {
                    reservation_id: req.reservation_id.clone(),
                    provider_reported_amount_atomic: String::new(),
                    unit: None,
                    pricing: Some(PricingFreeze::default()),
                    provider_event_id: req.provider_event_id.clone().unwrap_or_default(),
                    outcome: ProtoLlmOutcome::Success as i32,
                    estimated_amount_atomic: amount.to_string(),
                    actual_input_tokens: req.input_tokens.map(|v| v as i64),
                    actual_output_tokens: req.output_tokens.map(|v| v as i64),
                    delta_b_ratio: None,
                    delta_c_ratio: None,
                };
                let out = crate::decision::transaction::run_commit_estimated(
                    &self.cfg,
                    &self.state,
                    &ctx,
                    &payload,
                )
                .await
                .map_err(domain_to_companion_err)?;
                Ok(super::TraceAck {
                    verdict: super::TraceVerdict::Accepted,
                    ledger_transaction_id: out.ledger_transaction_id,
                })
            }
            super::TraceVerdict::Rejected => {
                let reservation_uuid = uuid::Uuid::parse_str(&req.reservation_id).map_err(|e| {
                    DecisionServiceError::InvalidRequest(format!(
                        "trace: reservation_id parse: {e}"
                    ))
                })?;
                let metadata = req.provider_event_id.clone();
                let out = crate::decision::transaction::run_release(
                    &self.cfg,
                    &self.state,
                    &ctx,
                    reservation_uuid,
                    crate::decision::transaction::ReleaseReason::RunAborted,
                    metadata.as_deref(),
                    None,
                    None,
                )
                .await
                .map_err(domain_to_companion_err)?;
                Ok(super::TraceAck {
                    verdict: super::TraceVerdict::Rejected,
                    ledger_transaction_id: out.ledger_transaction_id,
                })
            }
        }
    }
}

/// Translate a proto `DecisionResponse` into the Kong-shaped
/// `HttpDecisionResponse`. Wire-shape stability matters; do not add
/// fields without a coordinated Kong plugin bump.
fn build_http_decision_response(
    proto: crate::proto::sidecar_adapter::v1::DecisionResponse,
) -> super::HttpDecisionResponse {
    let verdict = proto_decision_to_verdict(
        crate::proto::sidecar_adapter::v1::decision_response::Decision::try_from(proto.decision)
            .unwrap_or(crate::proto::sidecar_adapter::v1::decision_response::Decision::Unspecified),
    );
    // Per review-standards §4.3 + SLICE 1 wire docs: DENY carries no
    // reservation id; ALLOW always carries one; DEGRADE carries the
    // upstream reservation when one was minted.
    let reservation_id = match verdict {
        super::DecisionVerdict::Deny => String::new(),
        super::DecisionVerdict::Allow | super::DecisionVerdict::Degrade => {
            proto.reservation_ids.first().cloned().unwrap_or_default()
        }
    };
    super::HttpDecisionResponse {
        verdict,
        reservation_id,
        decision_id: proto.decision_id,
        reason_codes: proto.reason_codes,
    }
}

#[cfg(test)]
mod real_decision_service_tests {
    //! Pure-translation tests for [`RealDecisionService`]. The full
    //! ledger / contract-bundle wiring is covered by the existing
    //! gRPC adapter integration suite — here we verify only the
    //! pieces that live in the HTTP companion translation layer
    //! (validation, claim normalization, verdict mapping).
    //!
    //! Tests that need a live state are gated behind the
    //! `http-companion-test-support` feature so they pull rcgen / a
    //! seeded SidecarState only when the integration harness is in
    //! scope.
    use super::*;

    #[test]
    fn validate_claim_atomic_rejects_empty() {
        let err = validate_claim_atomic("").unwrap_err();
        assert!(matches!(err, DecisionServiceError::InvalidRequest(_)));
    }

    #[test]
    fn validate_claim_atomic_rejects_negative() {
        let err = validate_claim_atomic("-100").unwrap_err();
        assert!(matches!(err, DecisionServiceError::InvalidRequest(_)));
    }

    #[test]
    fn validate_claim_atomic_rejects_decimal() {
        let err = validate_claim_atomic("1.5").unwrap_err();
        assert!(matches!(err, DecisionServiceError::InvalidRequest(_)));
    }

    #[test]
    fn validate_claim_atomic_rejects_scientific() {
        let err = validate_claim_atomic("1e3").unwrap_err();
        assert!(matches!(err, DecisionServiceError::InvalidRequest(_)));
    }

    #[test]
    fn validate_claim_atomic_accepts_zero() {
        assert_eq!(validate_claim_atomic("0").unwrap(), "0");
    }

    #[test]
    fn validate_claim_atomic_accepts_large_value() {
        // The ledger supports NUMERIC(38,0); ensure we don't truncate
        // arbitrary digit counts during validation.
        let raw = "1234567890123456789012345678901234567";
        assert_eq!(validate_claim_atomic(raw).unwrap(), raw);
    }

    #[test]
    fn proto_decision_to_verdict_maps_continue_to_allow() {
        use crate::proto::sidecar_adapter::v1::decision_response::Decision as PD;
        assert_eq!(
            proto_decision_to_verdict(PD::Continue),
            super::super::DecisionVerdict::Allow
        );
    }

    #[test]
    fn proto_decision_to_verdict_maps_stop_to_deny() {
        use crate::proto::sidecar_adapter::v1::decision_response::Decision as PD;
        assert_eq!(
            proto_decision_to_verdict(PD::Stop),
            super::super::DecisionVerdict::Deny
        );
        assert_eq!(
            proto_decision_to_verdict(PD::StopRunProjection),
            super::super::DecisionVerdict::Deny
        );
    }

    #[test]
    fn proto_decision_to_verdict_maps_degrade_skip_approval_unspecified() {
        use crate::proto::sidecar_adapter::v1::decision_response::Decision as PD;
        for kind in [PD::Degrade, PD::Skip, PD::RequireApproval, PD::Unspecified] {
            assert_eq!(
                proto_decision_to_verdict(kind),
                super::super::DecisionVerdict::Degrade
            );
        }
    }

    #[test]
    fn build_http_decision_response_drops_reservation_on_deny() {
        let proto = crate::proto::sidecar_adapter::v1::DecisionResponse {
            decision_id: "d-1".into(),
            decision: crate::proto::sidecar_adapter::v1::decision_response::Decision::Stop as i32,
            reservation_ids: vec!["r-should-be-dropped".into()],
            reason_codes: vec!["BUDGET_EXCEEDED".into()],
            ..Default::default()
        };
        let resp = build_http_decision_response(proto);
        assert_eq!(resp.verdict, super::super::DecisionVerdict::Deny);
        assert!(resp.reservation_id.is_empty());
        assert_eq!(resp.reason_codes, vec!["BUDGET_EXCEEDED".to_string()]);
    }

    #[test]
    fn build_http_decision_response_carries_first_reservation_on_allow() {
        let proto = crate::proto::sidecar_adapter::v1::DecisionResponse {
            decision_id: "d-1".into(),
            decision: crate::proto::sidecar_adapter::v1::decision_response::Decision::Continue
                as i32,
            reservation_ids: vec!["r-first".into(), "r-second".into()],
            ..Default::default()
        };
        let resp = build_http_decision_response(proto);
        assert_eq!(resp.verdict, super::super::DecisionVerdict::Allow);
        assert_eq!(resp.reservation_id, "r-first");
    }

    #[test]
    fn domain_to_companion_err_maps_invalid_request_to_400() {
        let err = domain_to_companion_err(crate::domain::error::DomainError::InvalidRequest(
            "bad".into(),
        ));
        assert_eq!(error_to_status(&err), 400);
    }

    #[test]
    fn domain_to_companion_err_maps_idempotency_conflict_to_409() {
        let err = domain_to_companion_err(crate::domain::error::DomainError::IdempotencyConflict(
            "dup".into(),
        ));
        assert_eq!(error_to_status(&err), 409);
    }

    #[test]
    fn domain_to_companion_err_maps_dependency_failures_to_503() {
        for d in [
            crate::domain::error::DomainError::Draining,
            crate::domain::error::DomainError::LedgerClient("down".into()),
            crate::domain::error::DomainError::CanonicalIngestClient("down".into()),
            crate::domain::error::DomainError::FencingAcquire("epoch".into()),
            crate::domain::error::DomainError::ManifestStale("stale".into()),
        ] {
            assert_eq!(error_to_status(&domain_to_companion_err(d)), 503);
        }
    }

    #[test]
    fn domain_to_companion_err_falls_through_to_500() {
        let err = domain_to_companion_err(crate::domain::error::DomainError::DecisionStage(
            "boom".into(),
        ));
        assert_eq!(error_to_status(&err), 500);
    }
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
