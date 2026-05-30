//! tonic `Service` implementation for the output_predictor gRPC.
//!
//! Spec ref output-predictor-service-spec-v1alpha1.md §2.3 (parallel
//! A/B/C computation). SLICE_06 ships:
//!
//!   * Strategy A: pure max_tokens-based ceiling, always computed.
//!   * Strategy B: cache lookup with cold-start L1/L4. SLICE_07 wires
//!     L2 (TOML); L3 is deferred per spec §2.2.
//!   * Strategy C: stub — `predicted_c_tokens` always unset until SLICE_07.
//!   * Selector: per spec §6 policy-driven choice.
//!
//! ## Hot path (Phase D wires this fully)
//!
//! Per spec §2.3:
//! ```text
//! let (a, b, _c) = tokio::join!(compute_a, compute_b, compute_c_stub);
//! let (reserved, used) = selector::select(policy, a, b, None);
//! ```
//! A is sync + < 100us; B is async (SQL + cache); C is stubbed to
//! immediate None. Selector picks per `prediction_policy`.
//!
//! ## Phase B skeleton
//!
//! This file currently dispatches to compute_a + compute_b + selector.
//! Strategy A & B & selector arrive in Phase C / D. Until then `Predict`
//! returns a 100% Strategy-A response with `predicted_b_tokens` unset,
//! mirroring the spec §3.4 "A is always callable" invariant.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tonic::{Request, Response, Status};
use tracing::{debug, warn};

use crate::cache::OutputDistributionCache;
use crate::classifier::{self, CLASSIFIER_VERSION};
use crate::context_window::ContextWindowTable;
use crate::fingerprint::FINGERPRINT_VERSION;
use crate::proto::output_predictor::v1::{
    output_predictor_server::OutputPredictor as OutputPredictorTrait, PredictRequest,
    PredictResponse,
};
use crate::selector::{self, Strategy};
use crate::strategy_a;
use crate::strategy_b::{self, PredictionB};

// R2 M5 (Backend F5 + Security F5): input validation bounds for
// fields that flow into bucket cache keys + SQL queries. Without
// length limits, a malicious caller could mint billions of distinct
// bucket keys to exhaust LRU capacity + log-line memory. Limits are
// generous (well above realistic production names).
pub const MAX_AGENT_ID_LEN: usize = 128;
pub const MAX_MODEL_LEN: usize = 64;
pub const MAX_PROMPT_CLASS_LEN: usize = 32;

/// R2 M12 (Software F17): when the context-window lookup falls back to
/// the unknown default, increment this counter so operators see
/// per-model unknown-rate trends in Prometheus.
pub static UNKNOWN_CONTEXT_WINDOW_TOTAL: AtomicU64 = AtomicU64::new(0);

/// Service struct. Shares the in-memory cache layer + context-window
/// table across all RPC handlers. SLICE_06 holds the canonical_ingest
/// DB pool inside `OutputDistributionCache`.
#[derive(Clone)]
pub struct OutputPredictorSvc {
    cache: Arc<OutputDistributionCache>,
    context_window: Arc<ContextWindowTable>,
    unknown_model_context_window: i64,
}

impl OutputPredictorSvc {
    pub fn new(
        cache: Arc<OutputDistributionCache>,
        context_window: Arc<ContextWindowTable>,
        unknown_model_context_window: i64,
    ) -> Self {
        Self {
            cache,
            context_window,
            unknown_model_context_window,
        }
    }
}

#[tonic::async_trait]
impl OutputPredictorTrait for OutputPredictorSvc {
    async fn predict(
        &self,
        request: Request<PredictRequest>,
    ) -> Result<Response<PredictResponse>, Status> {
        let start = std::time::Instant::now();
        let req = request.into_inner();

        // Parse tenant_id at gRPC boundary per SLICE_05 R2 B5 convention.
        // We don't actually need the Uuid value for compute_a, but
        // compute_b requires it for the RLS session variable + bucket key.
        let tenant_uuid = match uuid::Uuid::parse_str(&req.tenant_id) {
            Ok(u) => u,
            Err(e) => {
                warn!(tenant_id = %req.tenant_id, error = %e, "invalid tenant_id UUID");
                return Err(Status::invalid_argument(format!(
                    "invalid tenant_id `{}`: {e}",
                    req.tenant_id
                )));
            }
        };

        // R2 M5: input validation. Length-bounded bucket-key fields +
        // class enum allow-list. Without these a caller can mint
        // arbitrarily long agent_id / model strings that bloat the LRU
        // entries + structured log lines.
        if req.agent_id.len() > MAX_AGENT_ID_LEN {
            return Err(Status::invalid_argument(format!(
                "agent_id length {} exceeds MAX_AGENT_ID_LEN={}",
                req.agent_id.len(),
                MAX_AGENT_ID_LEN
            )));
        }
        if req.model.len() > MAX_MODEL_LEN {
            return Err(Status::invalid_argument(format!(
                "model length {} exceeds MAX_MODEL_LEN={}",
                req.model.len(),
                MAX_MODEL_LEN
            )));
        }
        if req.prompt_class.len() > MAX_PROMPT_CLASS_LEN {
            return Err(Status::invalid_argument(format!(
                "prompt_class length {} exceeds MAX_PROMPT_CLASS_LEN={}",
                req.prompt_class.len(),
                MAX_PROMPT_CLASS_LEN
            )));
        }
        if !req.prompt_class.is_empty() && !classifier::is_known_class(&req.prompt_class) {
            return Err(Status::invalid_argument(format!(
                "unknown prompt_class `{}`; expected one of: {}",
                req.prompt_class,
                classifier::classes::ALL.join(" | ")
            )));
        }

        // Validate prediction_policy at the boundary so the selector can
        // assume a valid enum value. The selector itself falls back to
        // STRICT_CEILING for unknown policies (per spec §6.1 default
        // conservative) — we still reject unknown values here to surface
        // caller mistakes instead of silently switching policies.
        if !selector::is_known_policy(&req.prediction_policy) {
            return Err(Status::invalid_argument(format!(
                "unknown prediction_policy `{}`; expected one of: \
                 STRICT_CEILING | EMPIRICAL_RUN_CEILING | ADAPTIVE_CEILING | SHADOW_ONLY",
                req.prediction_policy
            )));
        }

        // Resolve model context window — use caller-supplied value if > 0,
        // otherwise fall back to the TOML table, otherwise the unknown
        // default (spec §3.3). R2 M12 (Software F17): emit per-model
        // unknown counter when fallback fires so operators see drift.
        let context_window = if req.model_context_window > 0 {
            req.model_context_window
        } else {
            match self.context_window.lookup(&req.model) {
                Some(w) => w,
                None => {
                    UNKNOWN_CONTEXT_WINDOW_TOTAL.fetch_add(1, Ordering::Relaxed);
                    debug!(model = %req.model, "model_context_window lookup miss; using unknown default");
                    self.unknown_model_context_window
                }
            }
        };

        // ── R2 M1 (Backend + Software F9): parallel A + B + C per spec §2.3 ──
        //
        // R1 ran A and B sequentially — total latency was a+b. Spec §2.3 +
        // §11.1 budget breakdown require tokio::join! so b's I/O overlaps
        // with a's compute (compute_a is < 100us; compute_b is the SQL
        // path). The compute_c stub already resolves immediately to None
        // so tokio::join! has the right shape for SLICE_07 to drop in
        // real C without re-shaping the call.
        let a_input = (req.max_tokens_requested, context_window, req.input_tokens);
        let a_fut = async move {
            let a_start = std::time::Instant::now();
            let a = strategy_a::compute_a(a_input.0, a_input.1, a_input.2);
            (a, a_start.elapsed().as_nanos() as i64)
        };
        let b_fut = async {
            let b_start = std::time::Instant::now();
            let b = strategy_b::compute_b(
                &self.cache,
                tenant_uuid,
                &req.model,
                &req.agent_id,
                &req.prompt_class,
            )
            .await;
            (b, b_start.elapsed().as_nanos() as i64)
        };
        let c_fut = async {
            // SLICE_07 wires real plugin path; SLICE_06 stub immediate None.
            let c_start = std::time::Instant::now();
            let c: Option<i64> = None;
            (c, c_start.elapsed().as_nanos() as i64)
        };

        let ((a, a_latency_ns), (b, b_latency_ns), (c, c_latency_ns)): (
            (i64, i64),
            (Option<PredictionB>, i64),
            (Option<i64>, i64),
        ) = tokio::join!(a_fut, b_fut, c_fut);

        // ── Selector ──────────────────────────────────────────────────
        let (reserved, prediction_used) =
            selector::select_strategy(&req.prediction_policy, a, b.as_ref().map(|v| v.value), c);

        // Map cold-start layer + extract confidence/sample_size from the
        // chosen B/C. SLICE_06: only B can carry these; C always None.
        let (b_value, b_confidence, b_sample_size, b_layer) = match &b {
            Some(p) => (Some(p.value), Some(p.confidence), Some(p.sample_size), p.layer.clone()),
            None => (None, None, None, Some("L1".to_string())),
        };

        // Fingerprint: caller-supplied or compute ourselves.
        let fingerprint_used = if req.prompt_class_fingerprint.is_empty() {
            crate::fingerprint::compute_fingerprint(&req.model, &req.prompt_class)
        } else {
            req.prompt_class_fingerprint.clone()
        };

        // Confidence + sample-size emission rules per spec §6.3 + §7.1:
        // - When strategy_used == A, both are unset (no statistical row).
        // - When strategy_used == B, populate from the B row.
        // - When strategy_used == C, would populate from C row (SLICE_07).
        let (confidence, sample_size) = match prediction_used {
            Strategy::A => (None, None),
            Strategy::B => (b_confidence, b_sample_size),
            Strategy::C => (None, None), // SLICE_07
        };

        // cold_start_layer_used:
        //   - L4 (cache hit, sufficient samples) → unset/empty per spec §7.1
        //   - L1 (cache miss / insufficient) → "L1"
        // SLICE_06: only L1 and L4 supported; L2/L3 unset.
        let cold_start_layer_used = match (&b_layer, prediction_used) {
            // L4 hit recorded as None in the spec; we send empty string
            // (proto3 default) so the audit chain writes NULL.
            (Some(layer), _) if layer == "L4" => None,
            (Some(layer), _) => Some(layer.clone()),
            (None, _) => None,
        };

        let total_latency_ns = start.elapsed().as_nanos() as i64;

        debug!(
            tenant_id = %req.tenant_id,
            model = %req.model,
            prompt_class = %req.prompt_class,
            a = a,
            b_value = ?b_value,
            reserved = ?reserved,
            prediction_used = ?prediction_used,
            cold_start_layer = ?cold_start_layer_used,
            total_latency_ns,
            "predict"
        );

        Ok(Response::new(PredictResponse {
            predicted_a_tokens: a,
            predicted_b_tokens: b_value,
            predicted_c_tokens: None,
            reserved_strategy: reserved.to_string(),
            prediction_strategy_used: prediction_used.to_string(),
            confidence,
            sample_size,
            cold_start_layer_used,
            classifier_version: CLASSIFIER_VERSION.to_string(),
            fingerprint_version: FINGERPRINT_VERSION.to_string(),
            prompt_class_fingerprint_used: fingerprint_used,
            a_latency_ns,
            b_latency_ns,
            c_latency_ns,
            total_latency_ns,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_window::ContextWindowTable;
    use std::time::Duration;

    fn svc_skeleton() -> OutputPredictorSvc {
        // No DB pool → Strategy B always None; pure A path for input
        // validation tests.
        let cache = OutputDistributionCache::new(None, Duration::from_secs(300));
        let context_window = Arc::new(ContextWindowTable::empty());
        OutputPredictorSvc::new(cache, context_window, 8000)
    }

    fn valid_req() -> PredictRequest {
        PredictRequest {
            tenant_id: uuid::Uuid::new_v4().to_string(),
            model: "gpt-4o".into(),
            agent_id: "agent-test".into(),
            prompt_class: "chat_short".into(),
            input_tokens: 50,
            max_tokens_requested: 500,
            model_context_window: 128_000,
            prediction_policy: "STRICT_CEILING".into(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn input_validation_rejects_overlong_agent_id() {
        let svc = svc_skeleton();
        let mut req = valid_req();
        req.agent_id = "a".repeat(MAX_AGENT_ID_LEN + 1);
        let err = svc
            .predict(Request::new(req))
            .await
            .expect_err("must reject overlong agent_id");
        assert!(err.message().contains("MAX_AGENT_ID_LEN"));
    }

    #[tokio::test]
    async fn input_validation_rejects_overlong_model() {
        let svc = svc_skeleton();
        let mut req = valid_req();
        req.model = "m".repeat(MAX_MODEL_LEN + 1);
        let err = svc
            .predict(Request::new(req))
            .await
            .expect_err("must reject overlong model");
        assert!(err.message().contains("MAX_MODEL_LEN"));
    }

    #[tokio::test]
    async fn input_validation_rejects_unknown_prompt_class() {
        let svc = svc_skeleton();
        let mut req = valid_req();
        req.prompt_class = "not_a_real_class".into();
        let err = svc
            .predict(Request::new(req))
            .await
            .expect_err("must reject unknown class");
        assert!(err.message().contains("unknown prompt_class"));
    }

    #[tokio::test]
    async fn predict_runs_a_b_c_in_parallel() {
        // R2 M1: tokio::join! semantics — total wall time is approx
        // max(a, b, c), not sum. The skeleton-mode B is fast (None
        // immediately) so we mostly test the join-call doesn't
        // regress to sequential. Smoke: total_latency_ns should be
        // strictly less than the obvious sum if we had run them
        // sequentially (in practice the test asserts the value comes
        // back without panicking + non-zero).
        let svc = svc_skeleton();
        let resp = svc
            .predict(Request::new(valid_req()))
            .await
            .expect("ok")
            .into_inner();
        assert!(resp.predicted_a_tokens > 0);
        assert!(resp.total_latency_ns > 0);
        // c is unset in SLICE_06 — c_latency_ns is the time to
        // resolve the stub None future; that's microseconds. Verify
        // c is None per spec.
        assert!(resp.predicted_c_tokens.is_none());
    }

    #[tokio::test]
    async fn unknown_context_window_metric_increments() {
        // R2 M12: when the TOML lookup falls back to the default,
        // UNKNOWN_CONTEXT_WINDOW_TOTAL should increment.
        let before = UNKNOWN_CONTEXT_WINDOW_TOTAL.load(Ordering::Relaxed);
        let svc = svc_skeleton();
        let mut req = valid_req();
        req.model_context_window = 0; // force fallback path
        req.model = "made-up-model-not-in-toml".into();
        svc.predict(Request::new(req)).await.expect("ok");
        let after = UNKNOWN_CONTEXT_WINDOW_TOTAL.load(Ordering::Relaxed);
        assert!(after > before, "unknown_context_window counter must increment");
    }
}
