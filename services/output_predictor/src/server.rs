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

use std::sync::Arc;

use tonic::{Request, Response, Status};
use tracing::{debug, warn};

use crate::cache::OutputDistributionCache;
use crate::classifier::CLASSIFIER_VERSION;
use crate::context_window::ContextWindowTable;
use crate::fingerprint::FINGERPRINT_VERSION;
use crate::proto::output_predictor::v1::{
    output_predictor_server::OutputPredictor as OutputPredictorTrait, PredictRequest,
    PredictResponse,
};
use crate::selector::{self, Strategy};
use crate::strategy_a;
use crate::strategy_b::{self, PredictionB};

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
        // default (spec §3.3).
        let context_window = if req.model_context_window > 0 {
            req.model_context_window
        } else {
            self.context_window
                .lookup(&req.model)
                .unwrap_or(self.unknown_model_context_window)
        };

        // ── Strategy A (sync, always computed) ────────────────────────
        let a_start = std::time::Instant::now();
        let a = strategy_a::compute_a(
            req.max_tokens_requested,
            context_window,
            req.input_tokens,
        );
        let a_latency_ns = a_start.elapsed().as_nanos() as i64;

        // ── Strategy B (async; cache lookup + cold-start L4 → L1) ─────
        let b_start = std::time::Instant::now();
        let b: Option<PredictionB> = strategy_b::compute_b(
            &self.cache,
            tenant_uuid,
            &req.model,
            &req.agent_id,
            &req.prompt_class,
        )
        .await;
        let b_latency_ns = b_start.elapsed().as_nanos() as i64;

        // ── Strategy C (stub until SLICE_07) ──────────────────────────
        let c: Option<i64> = None;
        let c_latency_ns: i64 = 0;

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
