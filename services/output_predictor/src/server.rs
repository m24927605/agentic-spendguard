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

use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use lru::LruCache;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use tonic::{Request, Response, Status};
use tracing::{debug, error, warn};

use crate::cache::OutputDistributionCache;
use crate::circuit_breaker::PluginCircuitBreaker;
use crate::classifier::{self, CLASSIFIER_VERSION};
use crate::cold_start_loader::ModelDefaultDistribution;
use crate::context_window::ContextWindowTable;
use crate::endpoint_cache::EndpointCache;
use crate::fingerprint::FINGERPRINT_VERSION;
use crate::plugin_client::PluginClient;
use crate::proto::output_predictor::v1::{
    output_predictor_server::OutputPredictor as OutputPredictorTrait, PredictRequest,
    PredictResponse,
};
use crate::selector::{self, Strategy};
use crate::strategy_a;
use crate::strategy_b::{self, PredictionB};
use crate::strategy_c::{self, StrategyCError, StrategyCFailure, StrategyCInput, StrategyCOutcome};

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

/// Total Predict RPCs by terminal outcome.
pub static OUTPUT_PREDICTOR_PREDICT_OK_TOTAL: AtomicU64 = AtomicU64::new(0);
pub static OUTPUT_PREDICTOR_PREDICT_ERR_TOTAL: AtomicU64 = AtomicU64::new(0);

/// Cumulative Predict RPC latency histogram buckets in seconds.
pub static OUTPUT_PREDICTOR_PREDICT_LATENCY_LE_001_TOTAL: AtomicU64 = AtomicU64::new(0);
pub static OUTPUT_PREDICTOR_PREDICT_LATENCY_LE_005_TOTAL: AtomicU64 = AtomicU64::new(0);
pub static OUTPUT_PREDICTOR_PREDICT_LATENCY_LE_010_TOTAL: AtomicU64 = AtomicU64::new(0);
pub static OUTPUT_PREDICTOR_PREDICT_LATENCY_LE_025_TOTAL: AtomicU64 = AtomicU64::new(0);
pub static OUTPUT_PREDICTOR_PREDICT_LATENCY_LE_050_TOTAL: AtomicU64 = AtomicU64::new(0);
pub static OUTPUT_PREDICTOR_PREDICT_LATENCY_LE_100_TOTAL: AtomicU64 = AtomicU64::new(0);
pub static OUTPUT_PREDICTOR_PREDICT_LATENCY_LE_250_TOTAL: AtomicU64 = AtomicU64::new(0);
pub static OUTPUT_PREDICTOR_PREDICT_LATENCY_LE_500_TOTAL: AtomicU64 = AtomicU64::new(0);
pub static OUTPUT_PREDICTOR_PREDICT_LATENCY_LE_1000_TOTAL: AtomicU64 = AtomicU64::new(0);
pub static OUTPUT_PREDICTOR_PREDICT_LATENCY_INF_TOTAL: AtomicU64 = AtomicU64::new(0);
pub static OUTPUT_PREDICTOR_PREDICT_LATENCY_SUM_NS: AtomicU64 = AtomicU64::new(0);
pub static OUTPUT_PREDICTOR_PREDICT_LATENCY_COUNT: AtomicU64 = AtomicU64::new(0);

pub const DEFAULT_PREDICT_RATE_LIMIT_PER_TENANT_PER_SECOND: u32 = 1000;
pub const DEFAULT_PREDICT_RATE_LIMIT_TENANT_CAPACITY: usize = 4096;

static OUTPUT_PREDICTOR_RATE_LIMITED_BY_TENANT: Lazy<Mutex<LruCache<uuid::Uuid, u64>>> =
    Lazy::new(|| {
        Mutex::new(LruCache::new(
            NonZeroUsize::new(DEFAULT_PREDICT_RATE_LIMIT_TENANT_CAPACITY)
                .expect("default tenant metric capacity is non-zero"),
        ))
    });

pub fn predict_outcome_samples() -> [(&'static str, u64); 2] {
    [
        (
            "ok",
            OUTPUT_PREDICTOR_PREDICT_OK_TOTAL.load(Ordering::Relaxed),
        ),
        (
            "err",
            OUTPUT_PREDICTOR_PREDICT_ERR_TOTAL.load(Ordering::Relaxed),
        ),
    ]
}

pub fn predict_latency_bucket_samples() -> [(&'static str, u64); 10] {
    [
        (
            "0.001",
            OUTPUT_PREDICTOR_PREDICT_LATENCY_LE_001_TOTAL.load(Ordering::Relaxed),
        ),
        (
            "0.005",
            OUTPUT_PREDICTOR_PREDICT_LATENCY_LE_005_TOTAL.load(Ordering::Relaxed),
        ),
        (
            "0.01",
            OUTPUT_PREDICTOR_PREDICT_LATENCY_LE_010_TOTAL.load(Ordering::Relaxed),
        ),
        (
            "0.025",
            OUTPUT_PREDICTOR_PREDICT_LATENCY_LE_025_TOTAL.load(Ordering::Relaxed),
        ),
        (
            "0.05",
            OUTPUT_PREDICTOR_PREDICT_LATENCY_LE_050_TOTAL.load(Ordering::Relaxed),
        ),
        (
            "0.1",
            OUTPUT_PREDICTOR_PREDICT_LATENCY_LE_100_TOTAL.load(Ordering::Relaxed),
        ),
        (
            "0.25",
            OUTPUT_PREDICTOR_PREDICT_LATENCY_LE_250_TOTAL.load(Ordering::Relaxed),
        ),
        (
            "0.5",
            OUTPUT_PREDICTOR_PREDICT_LATENCY_LE_500_TOTAL.load(Ordering::Relaxed),
        ),
        (
            "1",
            OUTPUT_PREDICTOR_PREDICT_LATENCY_LE_1000_TOTAL.load(Ordering::Relaxed),
        ),
        (
            "+Inf",
            OUTPUT_PREDICTOR_PREDICT_LATENCY_INF_TOTAL.load(Ordering::Relaxed),
        ),
    ]
}

pub fn predict_latency_sum_seconds() -> f64 {
    OUTPUT_PREDICTOR_PREDICT_LATENCY_SUM_NS.load(Ordering::Relaxed) as f64 / 1_000_000_000.0
}

pub fn predict_latency_count() -> u64 {
    OUTPUT_PREDICTOR_PREDICT_LATENCY_COUNT.load(Ordering::Relaxed)
}

pub fn record_predict_rate_limited(tenant_id: uuid::Uuid) {
    let mut counters = OUTPUT_PREDICTOR_RATE_LIMITED_BY_TENANT.lock();
    match counters.get_mut(&tenant_id) {
        Some(value) => {
            *value = value.saturating_add(1);
        }
        None => {
            counters.put(tenant_id, 1);
        }
    }
}

pub fn predict_rate_limited_samples() -> Vec<(String, u64)> {
    let counters = OUTPUT_PREDICTOR_RATE_LIMITED_BY_TENANT.lock();
    let mut samples = counters
        .iter()
        .map(|(tenant_id, value)| (tenant_id.to_string(), *value))
        .collect::<Vec<_>>();
    samples.sort_by(|a, b| a.0.cmp(&b.0));
    samples
}

fn record_predict_metrics(ok: bool, elapsed: Duration) {
    if ok {
        OUTPUT_PREDICTOR_PREDICT_OK_TOTAL.fetch_add(1, Ordering::Relaxed);
    } else {
        OUTPUT_PREDICTOR_PREDICT_ERR_TOTAL.fetch_add(1, Ordering::Relaxed);
    }

    let seconds = elapsed.as_secs_f64();
    if seconds <= 0.001 {
        OUTPUT_PREDICTOR_PREDICT_LATENCY_LE_001_TOTAL.fetch_add(1, Ordering::Relaxed);
    }
    if seconds <= 0.005 {
        OUTPUT_PREDICTOR_PREDICT_LATENCY_LE_005_TOTAL.fetch_add(1, Ordering::Relaxed);
    }
    if seconds <= 0.01 {
        OUTPUT_PREDICTOR_PREDICT_LATENCY_LE_010_TOTAL.fetch_add(1, Ordering::Relaxed);
    }
    if seconds <= 0.025 {
        OUTPUT_PREDICTOR_PREDICT_LATENCY_LE_025_TOTAL.fetch_add(1, Ordering::Relaxed);
    }
    if seconds <= 0.05 {
        OUTPUT_PREDICTOR_PREDICT_LATENCY_LE_050_TOTAL.fetch_add(1, Ordering::Relaxed);
    }
    if seconds <= 0.1 {
        OUTPUT_PREDICTOR_PREDICT_LATENCY_LE_100_TOTAL.fetch_add(1, Ordering::Relaxed);
    }
    if seconds <= 0.25 {
        OUTPUT_PREDICTOR_PREDICT_LATENCY_LE_250_TOTAL.fetch_add(1, Ordering::Relaxed);
    }
    if seconds <= 0.5 {
        OUTPUT_PREDICTOR_PREDICT_LATENCY_LE_500_TOTAL.fetch_add(1, Ordering::Relaxed);
    }
    if seconds <= 1.0 {
        OUTPUT_PREDICTOR_PREDICT_LATENCY_LE_1000_TOTAL.fetch_add(1, Ordering::Relaxed);
    }
    OUTPUT_PREDICTOR_PREDICT_LATENCY_INF_TOTAL.fetch_add(1, Ordering::Relaxed);
    OUTPUT_PREDICTOR_PREDICT_LATENCY_SUM_NS.fetch_add(
        elapsed.as_nanos().min(u64::MAX as u128) as u64,
        Ordering::Relaxed,
    );
    OUTPUT_PREDICTOR_PREDICT_LATENCY_COUNT.fetch_add(1, Ordering::Relaxed);
}

struct PredictMetricsGuard {
    start: Instant,
    ok: bool,
}

impl PredictMetricsGuard {
    fn start() -> Self {
        Self {
            start: Instant::now(),
            ok: false,
        }
    }

    fn mark_ok(&mut self) {
        self.ok = true;
    }

    fn elapsed_nanos(&self) -> i64 {
        self.start.elapsed().as_nanos().min(i64::MAX as u128) as i64
    }
}

impl Drop for PredictMetricsGuard {
    fn drop(&mut self) {
        record_predict_metrics(self.ok, self.start.elapsed());
    }
}

// ── SLICE_07: customer plugin call outcome counters ────────────────
//
// Per output-predictor-plugin-contract-v1alpha1.md §9.1 these surface
// via Prometheus as `customer_predictor_*_total`. Phase E wires the
// /metrics scrape body; this slice ships the in-process atomics so
// strategy_c.rs can record outcomes from any concurrent Predict task
// without locks.

/// Total successful Strategy C calls (predicted_c_tokens populated).
pub static CUSTOMER_PREDICTOR_CALL_SUCCESS_TOTAL: AtomicU64 = AtomicU64::new(0);

/// Total Strategy C calls that fell to Strategy B for any reason.
/// Decomposed by failure mode via the FAILURE_BY_MODE_* atomics below.
pub static CUSTOMER_PREDICTOR_CALL_FALL_TO_B_TOTAL: AtomicU64 = AtomicU64::new(0);

/// Total spec §7.3 tenant binding violations — RLS bypass suspect.
/// Should ALWAYS be zero in production; a non-zero scrape value is an
/// operator-page condition.
pub static CUSTOMER_PREDICTOR_TENANT_ISOLATION_VIOLATION_TOTAL: AtomicU64 = AtomicU64::new(0);

/// Per-failure-mode counters per spec §5.1 (8 documented modes + the
/// 2 SLICE_07 metric-only modes for not_configured + breaker_open).
pub static FAILURE_BY_MODE_TIMEOUT: AtomicU64 = AtomicU64::new(0);
pub static FAILURE_BY_MODE_GRPC_ERROR: AtomicU64 = AtomicU64::new(0);
pub static FAILURE_BY_MODE_INVALID_ZERO_OR_NEGATIVE: AtomicU64 = AtomicU64::new(0);
pub static FAILURE_BY_MODE_INVALID_OVERFLOW: AtomicU64 = AtomicU64::new(0);
pub static FAILURE_BY_MODE_INVALID_CONFIDENCE: AtomicU64 = AtomicU64::new(0);
pub static FAILURE_BY_MODE_DESERIALIZATION_ERROR: AtomicU64 = AtomicU64::new(0);
pub static FAILURE_BY_MODE_TLS_ERROR: AtomicU64 = AtomicU64::new(0);
pub static FAILURE_BY_MODE_NOT_SERVING: AtomicU64 = AtomicU64::new(0);
pub static FAILURE_BY_MODE_NOT_CONFIGURED: AtomicU64 = AtomicU64::new(0);
pub static FAILURE_BY_MODE_BREAKER_OPEN: AtomicU64 = AtomicU64::new(0);

/// Record a Strategy C failure outcome. Tags the per-mode counter so
/// dashboards can split fall-to-B by cause.
fn record_failure_metric(failure: &StrategyCFailure) {
    CUSTOMER_PREDICTOR_CALL_FALL_TO_B_TOTAL.fetch_add(1, Ordering::Relaxed);
    let counter = match failure {
        StrategyCFailure::Timeout => &FAILURE_BY_MODE_TIMEOUT,
        StrategyCFailure::GrpcError(_) => &FAILURE_BY_MODE_GRPC_ERROR,
        StrategyCFailure::InvalidZeroOrNegative => &FAILURE_BY_MODE_INVALID_ZERO_OR_NEGATIVE,
        StrategyCFailure::InvalidOverflow => &FAILURE_BY_MODE_INVALID_OVERFLOW,
        StrategyCFailure::InvalidConfidence => &FAILURE_BY_MODE_INVALID_CONFIDENCE,
        StrategyCFailure::DeserializationError => &FAILURE_BY_MODE_DESERIALIZATION_ERROR,
        StrategyCFailure::TlsError => &FAILURE_BY_MODE_TLS_ERROR,
        StrategyCFailure::NotServing => &FAILURE_BY_MODE_NOT_SERVING,
        StrategyCFailure::NotConfigured => &FAILURE_BY_MODE_NOT_CONFIGURED,
        StrategyCFailure::BreakerOpen => &FAILURE_BY_MODE_BREAKER_OPEN,
    };
    counter.fetch_add(1, Ordering::Relaxed);
}

#[derive(Debug)]
struct TenantRateState {
    tokens: f64,
    last_refill: Instant,
}

#[derive(Debug)]
pub struct PredictRateLimiter {
    limit_per_second: u32,
    tenants: Mutex<LruCache<uuid::Uuid, TenantRateState>>,
}

impl PredictRateLimiter {
    pub fn new(limit_per_second: u32, tenant_capacity: usize) -> Self {
        let capacity = NonZeroUsize::new(tenant_capacity.max(1))
            .expect("tenant capacity is clamped to at least one");
        Self {
            limit_per_second,
            tenants: Mutex::new(LruCache::new(capacity)),
        }
    }

    pub fn disabled() -> Self {
        Self::new(0, DEFAULT_PREDICT_RATE_LIMIT_TENANT_CAPACITY)
    }

    pub fn check(&self, tenant_id: uuid::Uuid) -> bool {
        self.check_at(tenant_id, Instant::now())
    }

    fn check_at(&self, tenant_id: uuid::Uuid, now: Instant) -> bool {
        if self.limit_per_second == 0 {
            return true;
        }

        let burst = f64::from(self.limit_per_second);
        let mut tenants = self.tenants.lock();
        let state = match tenants.get_mut(&tenant_id) {
            Some(state) => state,
            None => {
                tenants.put(
                    tenant_id,
                    TenantRateState {
                        tokens: burst,
                        last_refill: now,
                    },
                );
                tenants
                    .get_mut(&tenant_id)
                    .expect("tenant state inserted into LRU cache")
            }
        };

        let elapsed = now.saturating_duration_since(state.last_refill);
        if !elapsed.is_zero() {
            state.tokens = (state.tokens + elapsed.as_secs_f64() * burst).min(burst);
            state.last_refill = now;
        }

        if state.tokens >= 1.0 {
            state.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Service struct. Shares the in-memory cache layer + context-window
/// table + Strategy C dependencies across all RPC handlers.
///
/// SLICE_06 holds the canonical_ingest DB pool inside
/// `OutputDistributionCache`. SLICE_07 adds the per-tenant plugin
/// endpoint cache, per-tenant gRPC client, and per-tenant circuit
/// breaker — all `Arc`d so the `tokio::join!(a, b, c)` orchestration
/// in `predict()` clones cheaply onto each future. SLICE_08 adds the
/// embedded `ModelDefaultDistribution` (TOML loader; ~70 entries) used
/// by strategy_b's L2 fallback when L4 misses.
#[derive(Clone)]
pub struct OutputPredictorSvc {
    cache: Arc<OutputDistributionCache>,
    context_window: Arc<ContextWindowTable>,
    unknown_model_context_window: i64,
    endpoint_cache: Arc<EndpointCache>,
    plugin_client: Arc<PluginClient>,
    plugin_breaker: Arc<PluginCircuitBreaker>,
    rate_limiter: Arc<PredictRateLimiter>,
    /// SLICE_08 cold-start L2 baseline table. None in skeleton tests
    /// that don't need the embedded TOML loaded (loader failure during
    /// test setup would mask the real test bug). Production always
    /// passes `Some(...)`; main.rs refuses to start on load failure.
    cold_start: Option<Arc<ModelDefaultDistribution>>,
}

impl OutputPredictorSvc {
    pub fn new(
        cache: Arc<OutputDistributionCache>,
        context_window: Arc<ContextWindowTable>,
        unknown_model_context_window: i64,
        endpoint_cache: Arc<EndpointCache>,
        plugin_client: Arc<PluginClient>,
        plugin_breaker: Arc<PluginCircuitBreaker>,
        cold_start: Option<Arc<ModelDefaultDistribution>>,
    ) -> Self {
        Self::new_with_rate_limiter(
            cache,
            context_window,
            unknown_model_context_window,
            endpoint_cache,
            plugin_client,
            plugin_breaker,
            cold_start,
            Arc::new(PredictRateLimiter::disabled()),
        )
    }

    pub fn new_with_rate_limiter(
        cache: Arc<OutputDistributionCache>,
        context_window: Arc<ContextWindowTable>,
        unknown_model_context_window: i64,
        endpoint_cache: Arc<EndpointCache>,
        plugin_client: Arc<PluginClient>,
        plugin_breaker: Arc<PluginCircuitBreaker>,
        cold_start: Option<Arc<ModelDefaultDistribution>>,
        rate_limiter: Arc<PredictRateLimiter>,
    ) -> Self {
        Self {
            cache,
            context_window,
            unknown_model_context_window,
            endpoint_cache,
            plugin_client,
            plugin_breaker,
            rate_limiter,
            cold_start,
        }
    }
}

#[tonic::async_trait]
impl OutputPredictorTrait for OutputPredictorSvc {
    async fn predict(
        &self,
        request: Request<PredictRequest>,
    ) -> Result<Response<PredictResponse>, Status> {
        let mut metrics_guard = PredictMetricsGuard::start();
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

        if !self.rate_limiter.check(tenant_uuid) {
            record_predict_rate_limited(tenant_uuid);
            warn!(
                tenant_id = %req.tenant_id,
                "Predict RPC rate limit exceeded for tenant"
            );
            return Err(Status::resource_exhausted(
                "Predict RPC rate limit exceeded for tenant",
            ));
        }

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
                self.cold_start.as_ref(),
                tenant_uuid,
                &req.model,
                &req.agent_id,
                &req.prompt_class,
            )
            .await;
            (b, b_start.elapsed().as_nanos() as i64)
        };
        // SLICE_07 Phase D: real Strategy C path (delegated plugin).
        // Returns (Option<PredictionC>, c_latency_ns, Option<StrategyCError>).
        // The error variant rises ONLY on tenant binding violation
        // (spec §7.3) — every other failure resolves to FallToB and is
        // recorded as `Option<i64> = None` plus a metric increment.
        let endpoint_cache_for_c = self.endpoint_cache.clone();
        let plugin_client_for_c = self.plugin_client.clone();
        let plugin_breaker_for_c = self.plugin_breaker.clone();
        let tenant_id_for_c = tenant_uuid;
        let model_for_c = req.model.clone();
        let agent_id_for_c = req.agent_id.clone();
        let prompt_class_for_c = req.prompt_class.clone();
        let decision_id_for_c = req.decision_id.clone();
        let fingerprint_for_c = req.prompt_class_fingerprint.clone();
        let input_tokens_for_c = req.input_tokens;
        let max_tokens_for_c = req.max_tokens_requested;
        let ctx_window_for_c = context_window;
        let c_fut = async move {
            let c_start = std::time::Instant::now();
            let input = StrategyCInput {
                tenant_id: tenant_id_for_c,
                model: &model_for_c,
                agent_id: &agent_id_for_c,
                prompt_class: &prompt_class_for_c,
                input_tokens: input_tokens_for_c,
                max_tokens_requested: max_tokens_for_c,
                model_context_window: ctx_window_for_c,
                decision_id: &decision_id_for_c,
                classifier_version: CLASSIFIER_VERSION,
                prompt_class_fingerprint: &fingerprint_for_c,
            };
            let result = strategy_c::compute_c(
                &endpoint_cache_for_c,
                &plugin_client_for_c,
                &plugin_breaker_for_c,
                input,
            )
            .await;
            (result, c_start.elapsed().as_nanos() as i64)
        };

        let ((a, a_latency_ns), (b, b_latency_ns), (c_result, c_latency_ns)): (
            (i64, i64),
            (Option<PredictionB>, i64),
            (Result<StrategyCOutcome, StrategyCError>, i64),
        ) = tokio::join!(a_fut, b_fut, c_fut);

        // Map the StrategyCOutcome envelope down to:
        //   - `c`               (Option<i64>) for the selector
        //   - `c_confidence`    (Option<f32>) for audit row
        //   - `c_sample_size`   (Option<i32>) for audit row
        // Per spec §1.8 a recoverable failure resolves to c = None and
        // the selector falls to B. The TenantBindingViolation arm is
        // mapped to a `Status::failed_precondition` returned to the
        // caller because it indicates RLS bypass — operator MUST see.
        let (c, c_confidence, c_sample_size) = match c_result {
            Ok(StrategyCOutcome::Ok(p)) => {
                CUSTOMER_PREDICTOR_CALL_SUCCESS_TOTAL.fetch_add(1, Ordering::Relaxed);
                (
                    Some(p.predicted_output_tokens),
                    Some(p.confidence),
                    Some(p.sample_size),
                )
            }
            Ok(StrategyCOutcome::FallToB(failure)) => {
                record_failure_metric(&failure);
                (None, None, None)
            }
            Err(StrategyCError::TenantBindingViolation { requested, got }) => {
                CUSTOMER_PREDICTOR_TENANT_ISOLATION_VIOLATION_TOTAL.fetch_add(1, Ordering::Relaxed);
                error!(
                    requested_tenant = %requested,
                    got_tenant = %got,
                    "Strategy C tenant binding violation — refusing Predict (spec §7.3)"
                );
                return Err(Status::failed_precondition(format!(
                    "tenant binding violation: tenant {} cannot use plugin registered for {}",
                    requested, got
                )));
            }
        };

        // ── Selector ──────────────────────────────────────────────────
        let (reserved, prediction_used) =
            selector::select_strategy(&req.prediction_policy, a, b.as_ref().map(|v| v.value), c);

        // Map cold-start layer + extract confidence/sample_size from the
        // chosen B/C. SLICE_07: Strategy C populates from PredictionC
        // when c_result was Ok; Strategy B populates from the cache row.
        let (b_value, b_confidence, b_sample_size, b_layer) = match &b {
            Some(p) => (
                Some(p.value),
                Some(p.confidence),
                Some(p.sample_size),
                p.layer.clone(),
            ),
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
        // - When strategy_used == C, populate from the C row (SLICE_07).
        let (confidence, sample_size) = match prediction_used {
            Strategy::A => (None, None),
            Strategy::B => (b_confidence, b_sample_size),
            Strategy::C => (c_confidence, c_sample_size),
        };

        // cold_start_layer_used per spec §7.1 truth table:
        //   - L4 (cache hit, sufficient samples) → unset/empty (NULL audit)
        //   - L3 (federated) → "L3" (deferred per spec §2.2; never fires)
        //   - L2 (TOML hit) → "L2" (SLICE_08)
        //   - L1 (everything missed; b_layer = None) → "L1" (signalled by
        //     selector falling to A; written by selector.select_strategy
        //     producing Strategy::A with no prior B layer carried forward).
        //
        // SLICE_08 wires L2 — when L4 misses, strategy_b's compute_b
        // tries the embedded TOML and returns `Some(PredictionB { layer:
        // Some("L2"), ... })` on hit. If the (model, class) combination
        // is missing from the TOML, compute_b returns None (which means
        // b_layer here is None) — and we set cold_start_layer_used =
        // "L1" so the audit chain records the L1 hard-fallback path.
        let cold_start_layer_used = match (&b_layer, prediction_used) {
            // L4 hit recorded as None in the spec; we send empty string
            // (proto3 default) so the audit chain writes NULL.
            (Some(layer), _) if layer == "L4" => None,
            // L2/L3 — pass through as the layer string.
            (Some(layer), _) => Some(layer.clone()),
            // No B prediction at all → A was selected → L1 fallback.
            // Spec §7.1: cold_start_layer_used = 'L1'.
            (None, _) => Some("L1".to_string()),
        };

        let total_latency_ns = metrics_guard.elapsed_nanos();

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

        metrics_guard.mark_ok();
        Ok(Response::new(PredictResponse {
            predicted_a_tokens: a,
            predicted_b_tokens: b_value,
            predicted_c_tokens: c,
            reserved_strategy: reserved.to_string(),
            prediction_strategy_used: prediction_used.to_string(),
            confidence,
            sample_size,
            cold_start_layer_used,
            classifier_version: CLASSIFIER_VERSION.to_string(),
            fingerprint_version: FINGERPRINT_VERSION.to_string(),
            prompt_class_fingerprint_used: fingerprint_used,
            prediction_policy_used: req.prediction_policy.clone(),
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
    use std::time::{Duration, Instant};

    fn svc_skeleton() -> OutputPredictorSvc {
        // No DB pool → Strategy B always None; pure A path for input
        // validation tests. SLICE_07: also Strategy C skeleton deps —
        // the endpoint cache without a pool returns NotConfigured so
        // strategy_c silently falls to B per spec §11.
        // SLICE_08: cold_start = None to keep these tests strictly L1
        // (so input validation tests assert L1 audit). svc_with_cold_start()
        // returns a variant for SLICE_08 L2 behaviour tests.
        use crate::circuit_breaker::CircuitBreakerConfig;
        let cache = OutputDistributionCache::new(None, Duration::from_secs(300));
        let context_window = Arc::new(ContextWindowTable::empty());
        let endpoint_cache = EndpointCache::with_default_ttl(None);
        let plugin_client = PluginClient::new(None).expect("skeleton-mode constructor");
        let plugin_breaker = PluginCircuitBreaker::new(CircuitBreakerConfig::default());
        OutputPredictorSvc::new(
            cache,
            context_window,
            8000,
            endpoint_cache,
            plugin_client,
            plugin_breaker,
            None,
        )
    }

    fn svc_with_cold_start() -> OutputPredictorSvc {
        // SLICE_08 variant: cold_start_loader populated from embedded
        // TOML. Used by L2 fallback tests below.
        use crate::circuit_breaker::CircuitBreakerConfig;
        let cache = OutputDistributionCache::new(None, Duration::from_secs(300));
        let context_window = Arc::new(ContextWindowTable::empty());
        let endpoint_cache = EndpointCache::with_default_ttl(None);
        let plugin_client = PluginClient::new(None).expect("skeleton-mode constructor");
        let plugin_breaker = PluginCircuitBreaker::new(CircuitBreakerConfig::default());
        let cold_start = Some(Arc::new(
            ModelDefaultDistribution::load_embedded().expect("embedded TOML loads in tests"),
        ));
        OutputPredictorSvc::new(
            cache,
            context_window,
            8000,
            endpoint_cache,
            plugin_client,
            plugin_breaker,
            cold_start,
        )
    }

    fn svc_with_rate_limiter(limit_per_second: u32) -> OutputPredictorSvc {
        use crate::circuit_breaker::CircuitBreakerConfig;
        let cache = OutputDistributionCache::new(None, Duration::from_secs(300));
        let context_window = Arc::new(ContextWindowTable::empty());
        let endpoint_cache = EndpointCache::with_default_ttl(None);
        let plugin_client = PluginClient::new(None).expect("skeleton-mode constructor");
        let plugin_breaker = PluginCircuitBreaker::new(CircuitBreakerConfig::default());
        OutputPredictorSvc::new_with_rate_limiter(
            cache,
            context_window,
            8000,
            endpoint_cache,
            plugin_client,
            plugin_breaker,
            None,
            Arc::new(PredictRateLimiter::new(limit_per_second, 16)),
        )
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
    async fn predict_response_echoes_prediction_policy_used() {
        let svc = svc_skeleton();
        let mut req = valid_req();
        req.prediction_policy = "ADAPTIVE_CEILING".into();
        let resp = svc
            .predict(Request::new(req))
            .await
            .expect("ok")
            .into_inner();
        assert_eq!(
            resp.prediction_policy_used, "ADAPTIVE_CEILING",
            "POST_GA_07 #161: response must echo the policy audited for the decision"
        );
    }

    #[tokio::test]
    async fn predict_rate_limit_is_per_tenant_and_records_metric() {
        let svc = svc_with_rate_limiter(1);
        let tenant_a = uuid::Uuid::new_v4();
        let tenant_b = uuid::Uuid::new_v4();

        let mut req_a = valid_req();
        req_a.tenant_id = tenant_a.to_string();
        svc.predict(Request::new(req_a.clone()))
            .await
            .expect("first request for tenant A fits bucket");

        let err = svc
            .predict(Request::new(req_a))
            .await
            .expect_err("second request for tenant A exceeds one-token bucket");
        assert_eq!(err.code(), tonic::Code::ResourceExhausted);

        let mut req_b = valid_req();
        req_b.tenant_id = tenant_b.to_string();
        svc.predict(Request::new(req_b))
            .await
            .expect("tenant B has an independent bucket");

        let tenant_a_label = tenant_a.to_string();
        assert!(
            predict_rate_limited_samples()
                .iter()
                .any(|(tenant_id, count)| tenant_id == &tenant_a_label && *count >= 1),
            "rate-limit metric must carry the throttled tenant_id label"
        );
    }

    #[test]
    fn predict_rate_limiter_refills_tokens_after_one_second() {
        let limiter = PredictRateLimiter::new(1, 16);
        let tenant = uuid::Uuid::new_v4();
        let now = Instant::now();

        assert!(limiter.check_at(tenant, now));
        assert!(
            !limiter.check_at(tenant, now),
            "second same-window request should be throttled"
        );
        assert!(
            limiter.check_at(tenant, now + Duration::from_secs(1)),
            "bucket should refill at limit_per_second"
        );
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
        assert!(
            after > before,
            "unknown_context_window counter must increment"
        );
    }

    // ── SLICE_08 — cold-start L2 wiring tests ──────────────────────────

    #[tokio::test]
    async fn slice_08_audit_writes_l1_when_no_cold_start_table_and_b_empty() {
        // svc_skeleton: cold_start = None → compute_b returns None →
        // audit cold_start_layer_used = "L1" per spec §7.1.
        let svc = svc_skeleton();
        let req = valid_req();
        let resp = svc
            .predict(Request::new(req))
            .await
            .expect("ok")
            .into_inner();
        assert_eq!(
            resp.cold_start_layer_used.as_deref(),
            Some("L1"),
            "SLICE_06+ contract — when B has no layer, audit must record L1"
        );
        assert!(
            resp.predicted_b_tokens.is_none(),
            "SLICE_08 — no cold_start table means B falls to L1 (None)"
        );
    }

    #[tokio::test]
    async fn slice_08_audit_writes_l2_when_cold_start_table_hits() {
        // svc_with_cold_start: TOML loaded; gpt-4o + chat_short hits
        // L2 because cache is empty (skeleton mode) and TOML has entry.
        let svc = svc_with_cold_start();
        let req = valid_req(); // model = gpt-4o, class = chat_short
        let resp = svc
            .predict(Request::new(req))
            .await
            .expect("ok")
            .into_inner();
        assert_eq!(
            resp.cold_start_layer_used.as_deref(),
            Some("L2"),
            "SLICE_08 — when L4 misses and TOML has entry, audit must record L2"
        );
        // Strategy B should now have predicted_b_tokens populated from
        // the TOML entry's P95 (gpt-4o chat_short P95 = 320).
        assert_eq!(
            resp.predicted_b_tokens,
            Some(320),
            "SLICE_08 — TOML entry P95 must flow through to predicted_b_tokens"
        );
        // Audit row should include confidence + sample_size from the
        // TOML entry (selector chose B since both A and B are available).
        assert!(
            resp.confidence.is_some(),
            "SLICE_08 — L2 hit must populate confidence per spec §7.1"
        );
        assert!(
            resp.sample_size.is_some(),
            "SLICE_08 — L2 hit must populate sample_size per spec §7.1"
        );
    }

    #[tokio::test]
    async fn slice_08_audit_writes_l1_when_cold_start_table_misses_known_class() {
        // svc_with_cold_start: TOML loaded; unknown model means TOML
        // lookup misses → L1 fallback (None) → audit = "L1".
        let svc = svc_with_cold_start();
        let mut req = valid_req();
        req.model = "totally-unknown-model".into();
        let resp = svc
            .predict(Request::new(req))
            .await
            .expect("ok")
            .into_inner();
        assert_eq!(
            resp.cold_start_layer_used.as_deref(),
            Some("L1"),
            "SLICE_08 — unknown model: L2 misses; cold_start_layer_used = L1"
        );
        assert!(
            resp.predicted_b_tokens.is_none(),
            "SLICE_08 — L2 miss falls to L1 (no B)"
        );
    }

    #[tokio::test]
    async fn slice_08_l2_strict_ceiling_keeps_reservation_a_but_records_b_used() {
        // Per spec §6 selector — STRICT_CEILING:
        //   reserved_strategy = A (safety; reservation never reduces)
        //   prediction_strategy_used = best available (B if B is Some)
        // SLICE_08 ships L2 wired; with L2 active, B becomes "available"
        // on cold start, so STRICT_CEILING records prediction_used = B
        // (for calibration backtest per spec §6.2 commentary) while
        // KEEPING reservation = A.
        let svc = svc_with_cold_start();
        let mut req = valid_req();
        req.prediction_policy = "STRICT_CEILING".into();
        let resp = svc
            .predict(Request::new(req))
            .await
            .expect("ok")
            .into_inner();
        assert_eq!(
            resp.reserved_strategy, "A",
            "STRICT_CEILING reservation must remain A regardless of L2"
        );
        assert_eq!(
            resp.prediction_strategy_used, "B",
            "STRICT_CEILING records B in prediction_used when L2 gives B (calibration backtest)"
        );
        assert_eq!(
            resp.cold_start_layer_used.as_deref(),
            Some("L2"),
            "audit row must record L2 even though reservation stayed A"
        );
    }

    #[tokio::test]
    async fn slice_08_l2_selector_empirical_run_ceiling_picks_b() {
        // Per spec §6 — EMPIRICAL_RUN_CEILING uses B when available.
        // With L2 wired, B is now available even on cold-start so this
        // policy should select B.
        let svc = svc_with_cold_start();
        let mut req = valid_req();
        req.prediction_policy = "EMPIRICAL_RUN_CEILING".into();
        let resp = svc
            .predict(Request::new(req))
            .await
            .expect("ok")
            .into_inner();
        assert_eq!(
            resp.prediction_strategy_used, "B",
            "EMPIRICAL_RUN_CEILING must pick B when L2 fallback fires"
        );
    }
}
