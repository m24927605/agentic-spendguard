//! Async shadow worker — drift detection + `tokenizer_drift_alert`
//! CloudEvent emission.
//!
//! Spec refs:
//!   - `tokenizer-service-spec-v1alpha1.md` §4 (Tier 1 shadow architecture)
//!   - `tokenizer-service-spec-v1alpha1.md` §4.1 (sampling + worker
//!     pseudocode)
//!   - `tokenizer-service-spec-v1alpha1.md` §4.2 (per-kind drift threshold;
//!     consumed via `EncoderKind::drift_threshold()` so SLICE_04 owns
//!     the values)
//!   - `tokenizer-service-spec-v1alpha1.md` §4.3 (1h cool-down window)
//!   - `tokenizer-service-spec-v1alpha1.md` §4.4 (`tokenizer_t1_samples`
//!     persistence)
//!   - `stats-aggregator-spec-v1alpha1.md` §7.2 (CloudEvent schema
//!     conventions — reused for the `tokenizer_drift_alert` family)
//!
//! ## Hot path invariant
//!
//! The gRPC `Tokenize` handler calls [`ShadowWorkerHandle::try_send`]
//! AFTER returning the Tier 2 response to the caller. The send is
//! non-blocking and returns immediately on a full channel — Tier 2
//! latency is structurally protected from any back-pressure here.
//!
//! This module is referenced ONLY from `services/tokenizer/src/main.rs`
//! (worker spawn) and `services/tokenizer/src/server.rs` (try_send). It
//! is NEVER referenced from `services/sidecar/` or
//! `services/egress_proxy/` — spec §1.3 invariant.
//!
//! ## Drift alert event shape
//!
//! Per spec §4 + stats-aggregator-spec §7.2 the emitted CloudEvent is
//! (R2 B1: `spendguard.audit.*` prefix routes to ImmutableAuditLog):
//!
//! ```yaml
//! type:   spendguard.audit.tokenizer_drift_alert.v1alpha1
//! source: spendguard://tokenizer-service/<instance>
//! data:
//!   sample_id:              <uuid>      # R2 M6
//!   tenant_id:              <uuid>      # R2 B5
//!   model:                  <string>
//!   tokenizer_version_id:   <uuid>
//!   tier2_count:            <int>
//!   tier1_count:            <int>
//!   drift_pct:              <float>
//!   threshold:              <float>
//!   encoder_kind:           "ANTHROPIC_BPE" | "GEMINI_BPE" | ...
//! ```
//!
//! The CloudEvent is signed in place via the canonical Ed25519 path
//! (mirrors `services/sidecar/src/audit.rs::sign_cloudevent_in_place`).

use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use chrono::Utc;
use prost::Message as _;
use spendguard_signing::Signer;
use spendguard_tokenizer::encoders::EncoderKind;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use super::circuit_breaker::{CircuitBreakerState, Permit};
use super::provider_clients::{
    anthropic::AnthropicClient, cohere::CohereClient, gemini::GeminiClient, llama::LlamaClient,
    ProviderError,
};
use super::sample_rate_state::{SampleRateState, ShadowKey};
use super::security::{DynCountTokensQuota, DynShadowSecurityStore};
use crate::proto::common::v1::CloudEvent;

/// CloudEvent type string for tokenizer drift alerts (spec §4 +
/// stats-aggregator §7.2 family).
///
/// R2 B1 (event prefix): `spendguard.audit.*` routes the event to
/// ImmutableAuditLog via canonical_ingest's `event_routing::classify`
/// (`spendguard.audit.` prefix arm at
/// `services/canonical_ingest/src/domain/event_routing.rs:33-36`).
/// The previous `spendguard.tokenizer.drift_alert.*` prefix fell
/// through to ProfilePayloadBlob — which is RTBF-deletable — and
/// violated the audit-chain immutability claim in spec §6 + slice
/// doc §6.
pub const DRIFT_ALERT_EVENT_TYPE: &str = "spendguard.audit.tokenizer_drift_alert.v1alpha1";

/// Default bounded-channel capacity from the gRPC handler to the
/// shadow worker. Bounded so Tier 2 hot path never queues forever when
/// the worker is overloaded; spec §10.2 sample queue lag p99 < 30s SLO
/// covers tail latency, not hard cap.
pub const DEFAULT_CHANNEL_CAPACITY: usize = 1024;

/// One sampled tokenize event headed to the shadow worker.
///
/// Carries enough context to:
///   * decide whether to actually sample (Phase B rate gating),
///   * call the provider Tier 1 endpoint (Phase C),
///   * compute drift vs Tier 2 result (this phase),
///   * persist to `tokenizer_t1_samples` (this phase),
///   * emit `tokenizer_drift_alert` if drift > threshold (this phase).
///
/// R2 B5: `tenant_id` is `uuid::Uuid` matching ledger schema. The gRPC
/// boundary parses the inbound string header and returns
/// `Status::invalid_argument` on a malformed UUID so anonymous /
/// misconfigured callers fail closed.
#[derive(Debug, Clone)]
pub struct ShadowEvent {
    pub tenant_id: uuid::Uuid,
    pub model: String,
    pub encoder_kind: EncoderKind,
    pub t2_input_tokens: i64,
    pub t2_tokenizer_version_id: String,
    /// Raw text the caller tokenized. Bounded upstream by the shared
    /// 4 MiB tokenizer request cap so channel memory pressure is bounded.
    pub raw_text: String,
}

impl ShadowEvent {
    /// R2 B5: convert the UUID tenant_id to the String form used as
    /// the in-memory ShadowKey for per-(tenant, model) state lookup.
    fn shadow_key(&self) -> ShadowKey {
        ShadowKey {
            tenant_id: self.tenant_id.to_string(),
            model: self.model.clone(),
        }
    }
}

/// Outcome of processing one shadow event. Surfaced via the metrics
/// hook so the /metrics endpoint can render per-state counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowOutcome {
    /// Sample was dropped (rate gate or breaker open).
    Skipped,
    /// Provider call returned; no drift detected.
    Sampled,
    /// Provider call returned; drift exceeded threshold; alert emitted.
    Alerted,
    /// Provider call failed; circuit breaker counter incremented.
    ProviderFailed,
    /// Provider returned schema drift OR auth failure; sample skipped,
    /// breaker NOT incremented.
    ProviderSchemaOrAuth,
}

/// Optional source of durable per-(tenant, model) sampling overrides.
///
/// The control-plane API persists overrides under RLS. The shadow worker
/// refreshes the specific tenant/model key it is about to evaluate, so
/// it never needs an all-tenant read path or a BYPASSRLS role.
#[async_trait]
pub trait SampleRateOverrideStore: Send + Sync {
    async fn load_override(&self, key: &ShadowKey) -> anyhow::Result<Option<f64>>;
}

/// Handle returned to main.rs for graceful shutdown + best-effort
/// event submission from the gRPC handler.
#[derive(Debug, Clone)]
pub struct ShadowWorkerHandle {
    sender: mpsc::Sender<ShadowEvent>,
    sample_rate: Option<Arc<SampleRateState>>,
}

impl ShadowWorkerHandle {
    /// Non-blocking try-send. The gRPC server handler ignores the
    /// result — Tier 2 hot path is not allowed to be perturbed by the
    /// shadow path per spec §1.3 invariant. We return the typed error
    /// for tests + Phase F metric emission.
    pub fn try_send(
        &self,
        event: ShadowEvent,
    ) -> Result<(), mpsc::error::TrySendError<ShadowEvent>> {
        self.sender.try_send(event)
    }

    /// Sampling state owned by the real shadow worker. Drop-only handles
    /// return None; production paths use this for the durable override
    /// sync that feeds the same state used by the rate gate.
    pub fn sample_rate_state(&self) -> Option<Arc<SampleRateState>> {
        self.sample_rate.clone()
    }
}

/// One persisted sample — the shadow worker hands this to the
/// [`SamplePersister`] trait. Phase F may add a buffered batch
/// persister; the SQL-direct path is the default.
///
/// R2 B5: `tenant_id` is `uuid::Uuid` matching the migration 0051
/// schema (`tenant_id UUID NOT NULL`). R2 M9: `drift_alert_decided`
/// reflects the worker's decision; CloudEvent emission outcome is
/// tracked separately via [`SamplePersister::mark_drift_alert_emitted`].
#[derive(Debug, Clone)]
pub struct SampleRow {
    pub sample_id: uuid::Uuid,
    pub tenant_id: uuid::Uuid,
    pub model: String,
    pub t1_input_tokens: i64,
    pub t2_input_tokens: i64,
    pub t2_tokenizer_version_id: String,
    pub drift_ratio: f32,
    /// R2 M9: TRUE iff drift_ratio > per-kind threshold (the *decision*).
    /// The CloudEvent emission acknowledgement is recorded separately by
    /// [`SamplePersister::mark_drift_alert_emitted`].
    pub drift_alert_decided: bool,
    pub provider_request_id: Option<String>,
    /// Wallclock at sample observation — passed explicitly by the worker
    /// (event time) rather than relying on the DB default so a slow
    /// worker queue does not skew retention math (R2 M8 + DB m8).
    pub sampled_at: chrono::DateTime<chrono::Utc>,
}

/// Persistence abstraction so tests can plug in an in-memory recorder
/// without spinning up a Postgres instance.
///
/// R2 M9: two-step semantics. `persist` writes the SampleRow with
/// `drift_alert_decided = TRUE` but `drift_alert_emitted_at = NULL`.
/// `mark_drift_alert_emitted` is called after the CloudEvent successfully
/// lands in canonical_ingest. Failure of the second call surfaces in
/// metrics — the SampleRow remains in the table with `emitted_at NULL`
/// so operators can diagnose the canonical_ingest outage without losing
/// the underlying drift signal.
#[async_trait::async_trait]
pub trait SamplePersister: Send + Sync {
    async fn persist(&self, sample: SampleRow) -> Result<(), anyhow::Error>;
    async fn mark_drift_alert_emitted(
        &self,
        sample_id: uuid::Uuid,
        sampled_at: chrono::DateTime<chrono::Utc>,
        emitted_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), anyhow::Error>;
}

/// CloudEvent emission abstraction. The real path forwards to
/// canonical_ingest's AppendEvents RPC; tests collect the events in a
/// Vec.
#[async_trait::async_trait]
pub trait DriftAlertSink: Send + Sync {
    async fn emit(&self, event: CloudEvent) -> Result<(), anyhow::Error>;
}

/// Dispatching client surface — the worker may have one, the other, or
/// both. Drift detection only fires when the call returns a valid
/// Tier 1 count.
#[derive(Clone, Default)]
pub struct ProviderRoster {
    pub anthropic: Option<AnthropicClient>,
    pub cohere: Option<CohereClient>,
    pub gemini: Option<GeminiClient>,
    pub llama: Option<LlamaClient>,
}

impl std::fmt::Debug for ProviderRoster {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderRoster")
            .field("anthropic", &self.anthropic.is_some())
            .field("cohere", &self.cohere.is_some())
            .field("gemini", &self.gemini.is_some())
            .field("llama", &self.llama.is_some())
            .finish()
    }
}

/// Shadow worker dependencies bundled into one struct so the spawn
/// signature stays sane.
pub struct ShadowWorkerDeps {
    pub sample_rate: Arc<SampleRateState>,
    pub circuit_breaker: Arc<CircuitBreakerState>,
    pub providers: ProviderRoster,
    pub persister: Arc<dyn SamplePersister>,
    pub alert_sink: Arc<dyn DriftAlertSink>,
    pub sample_rate_overrides: Option<Arc<dyn SampleRateOverrideStore>>,
    pub security: DynShadowSecurityStore,
    pub count_tokens_quota: DynCountTokensQuota,
    pub signer: Arc<dyn Signer>,
    /// Producer source URI for the signed CloudEvent, e.g.
    /// `spendguard://tokenizer-service/region-us-west2`.
    pub event_source: String,
    /// Channel capacity from gRPC handler → worker.
    pub channel_capacity: usize,
}

impl ShadowWorkerDeps {
    pub fn channel_capacity_or_default(&self) -> usize {
        if self.channel_capacity == 0 {
            DEFAULT_CHANNEL_CAPACITY
        } else {
            self.channel_capacity
        }
    }
}

/// Spawn the shadow worker. Returns the handle the gRPC server uses to
/// fire-and-forget events.
pub fn spawn_shadow_worker(deps: ShadowWorkerDeps) -> ShadowWorkerHandle {
    let cap = deps.channel_capacity_or_default();
    let (tx, rx) = mpsc::channel::<ShadowEvent>(cap);
    let sample_rate = Some(Arc::clone(&deps.sample_rate));
    tokio::spawn(run_loop(rx, deps));
    ShadowWorkerHandle {
        sender: tx,
        sample_rate,
    }
}

/// Inert handle for `services/tokenizer/src/main.rs` to use during the
/// boot phase when no provider clients are configured (demo mode).
/// Drops every event silently. Wraps the same channel shape so the
/// gRPC handler does not need to know about the difference.
pub fn spawn_drop_handle(buffer: usize) -> ShadowWorkerHandle {
    let cap = if buffer == 0 {
        DEFAULT_CHANNEL_CAPACITY
    } else {
        buffer
    };
    let (tx, mut rx) = mpsc::channel::<ShadowEvent>(cap);
    tokio::spawn(async move {
        while rx.recv().await.is_some() {
            // Drain. Demo mode.
        }
    });
    ShadowWorkerHandle {
        sender: tx,
        sample_rate: None,
    }
}

async fn run_loop(mut rx: mpsc::Receiver<ShadowEvent>, deps: ShadowWorkerDeps) {
    info!("shadow worker started");
    while let Some(event) = rx.recv().await {
        let outcome = process_one(&event, &deps).await;
        debug!(?outcome, model = %event.model, "shadow event processed");
    }
    warn!("shadow worker channel closed; exiting");
}

/// Process one shadow event end-to-end. Visible for direct testing in
/// addition to the channel-driven loop.
pub async fn process_one(event: &ShadowEvent, deps: &ShadowWorkerDeps) -> ShadowOutcome {
    let key = event.shadow_key();

    refresh_sampling_override(&key, deps).await;

    // Rate gate — should_sample handles the cool-down 100% case.
    if !deps.sample_rate.should_sample(&key) {
        return ShadowOutcome::Skipped;
    }

    // Circuit breaker — skip if Open.
    match deps.circuit_breaker.permit_request(&key) {
        Permit::SkipOpen => return ShadowOutcome::Skipped,
        Permit::Allow => {}
    }

    let provider = match event.encoder_kind {
        EncoderKind::Anthropic => "anthropic",
        EncoderKind::Cohere => "cohere",
        EncoderKind::Gemini => "gemini",
        EncoderKind::Llama => "llama",
        // OpenAI's tiktoken is byte-exact (per spec §4.2 threshold 0.0)
        // — drift detection lives elsewhere (CI golden fixture diff).
        _ => return ShadowOutcome::Skipped,
    };
    match provider {
        "anthropic" if deps.providers.anthropic.is_none() => {
            debug!(model = %event.model, "anthropic shadow client not configured; skipping sample");
            return ShadowOutcome::Skipped;
        }
        "cohere" if deps.providers.cohere.is_none() => {
            debug!(model = %event.model, "cohere shadow client not configured; skipping sample");
            return ShadowOutcome::Skipped;
        }
        "gemini" if deps.providers.gemini.is_none() => {
            debug!(model = %event.model, "gemini shadow client not configured; skipping sample");
            return ShadowOutcome::Skipped;
        }
        "llama" if deps.providers.llama.is_none() => {
            debug!(model = %event.model, "llama shadow client not configured; skipping sample");
            return ShadowOutcome::Skipped;
        }
        _ => {}
    }

    let settings = match deps.security.load_settings(event.tenant_id).await {
        Ok(settings) => settings,
        Err(e) => {
            warn!(
                error = ?e,
                tenant = %event.tenant_id,
                "failed to load tokenizer shadow security settings; failing closed"
            );
            return ShadowOutcome::Skipped;
        }
    };
    if !settings.pii_shadow_enabled {
        debug!(
            tenant = %event.tenant_id,
            provider,
            "tenant has not opted into raw-text tokenizer shadow; skipping provider call"
        );
        return ShadowOutcome::Skipped;
    }
    match deps
        .count_tokens_quota
        .try_acquire(
            event.tenant_id,
            provider,
            settings.count_tokens_quota_per_minute,
        )
        .await
    {
        Ok(true) => {}
        Ok(false) => {
            TOKENIZER_RATE_LIMITED_TOTAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            debug!(
                tenant = %event.tenant_id,
                provider,
                quota_per_minute = settings.count_tokens_quota_per_minute,
                "tokenizer count_tokens quota exhausted; skipping provider call"
            );
            return ShadowOutcome::Skipped;
        }
        Err(e) => {
            warn!(
                error = ?e,
                tenant = %event.tenant_id,
                provider,
                "failed to claim tokenizer count_tokens quota; failing closed"
            );
            return ShadowOutcome::Skipped;
        }
    }

    // Dispatch to the right provider only after tenant opt-in and quota pass.
    let provider_call = match event.encoder_kind {
        EncoderKind::Anthropic => match deps.providers.anthropic.as_ref() {
            Some(c) => c.count_tokens(&event.model, &event.raw_text).await,
            None => return ShadowOutcome::Skipped,
        },
        EncoderKind::Cohere => match deps.providers.cohere.as_ref() {
            Some(c) => c.count_tokens(&event.model, &event.raw_text).await,
            None => return ShadowOutcome::Skipped,
        },
        EncoderKind::Gemini => match deps.providers.gemini.as_ref() {
            Some(c) => c.count_tokens(&event.model, &event.raw_text).await,
            None => return ShadowOutcome::Skipped,
        },
        EncoderKind::Llama => match deps.providers.llama.as_ref() {
            Some(c) => c.count_tokens(&event.model, &event.raw_text).await,
            None => return ShadowOutcome::Skipped,
        },
        _ => return ShadowOutcome::Skipped,
    };

    let provider_count = match provider_call {
        Ok(c) => c,
        Err(err) => {
            return on_provider_error(&key, &err, deps).await;
        }
    };

    // Probe success → close the breaker (no-op if already Closed).
    deps.circuit_breaker.record_success(&key);

    let drift_ratio = compute_drift_ratio(
        provider_count.input_tokens,
        event.t2_input_tokens.max(0) as u64,
    );
    let threshold = event.encoder_kind.drift_threshold();
    let alert = drift_ratio > threshold;

    // R2 M6: mint sample_id BEFORE alert branch so the CloudEvent can
    // carry it for forensic traceability from audit chain back to the
    // t1_samples row.
    let sample_id = uuid::Uuid::now_v7();
    let sampled_at = Utc::now();

    let sample_row = SampleRow {
        sample_id,
        tenant_id: event.tenant_id,
        model: event.model.clone(),
        t1_input_tokens: provider_count.input_tokens as i64,
        t2_input_tokens: event.t2_input_tokens,
        t2_tokenizer_version_id: event.t2_tokenizer_version_id.clone(),
        drift_ratio,
        drift_alert_decided: alert,
        provider_request_id: provider_count.request_id.clone(),
        sampled_at,
    };

    // Persist FIRST so the t1_samples row exists before the CloudEvent
    // emission. If emission fails we still have the underlying drift
    // signal in the DB for operator triage.
    if let Err(e) = deps.persister.persist(sample_row).await {
        error!(error = ?e, "failed to persist tokenizer_t1_samples row");
    }

    if alert {
        deps.sample_rate.enter_cool_down(&key);

        // R2 M10: track 1h rolling alert count for on-call escalation.
        let escalate = deps.sample_rate.record_alert(&key);
        if escalate {
            ALERT_ONCALL_ESCALATION_TOTAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            warn!(
                tenant = %key.tenant_id,
                model = %key.model,
                "≥3 drift alerts in 1h window — on-call escalation event"
            );
        }

        match emit_drift_alert(
            event,
            sample_id,
            &provider_count,
            drift_ratio,
            threshold,
            deps,
        )
        .await
        {
            Ok(()) => {
                // R2 M9: ack via mark_drift_alert_emitted so the row's
                // drift_alert_emitted_at column moves from NULL → now().
                let emitted_at = Utc::now();
                if let Err(e) = deps
                    .persister
                    .mark_drift_alert_emitted(sample_id, sampled_at, emitted_at)
                    .await
                {
                    error!(
                        error = ?e,
                        sample_id = %sample_id,
                        "failed to mark drift_alert_emitted_at — row remains with NULL ack"
                    );
                }
            }
            Err(e) => {
                error!(
                    error = ?e,
                    sample_id = %sample_id,
                    "failed to emit tokenizer_drift_alert CloudEvent — row remains with NULL ack",
                );
            }
        }
    }

    if alert {
        ShadowOutcome::Alerted
    } else {
        ShadowOutcome::Sampled
    }
}

async fn refresh_sampling_override(key: &ShadowKey, deps: &ShadowWorkerDeps) {
    let Some(store) = deps.sample_rate_overrides.as_ref() else {
        return;
    };
    match store.load_override(key).await {
        Ok(rate) => deps.sample_rate.set_override_rate(key, rate),
        Err(e) => warn!(
            error = ?e,
            tenant = %key.tenant_id,
            model = %key.model,
            "failed to refresh tokenizer sampling-rate override; using last known/default rate"
        ),
    }
}

/// R2 M10: rolling 1h escalation counter. Atomic so the metrics
/// endpoint can read without acquiring a lock.
pub static ALERT_ONCALL_ESCALATION_TOTAL: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// R2 M5 — shadow events dropped because the worker channel was full.
/// Surfaces silent shadow loss in `/metrics`.
pub static SHADOW_DROPPED_FULL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// R2 M5 — shadow events dropped because the worker channel was closed
/// (worker task crashed). Higher severity than DROPPED_FULL: it implies
/// drift detection is offline.
pub static SHADOW_WORKER_DEAD: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// R2 M2 — chat-shape (messages array) requests skipped from shadow
/// sampling because the SLICE_05 flatten was a false-positive source.
pub static SHADOW_SKIPPED_CHAT_SHAPE: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// R2 M11 — Tier 1 provider returned a response that failed to parse
/// as the documented count_tokens schema (vendor API drift). Distinct
/// from network errors so operators can alert on it independently.
pub static PROVIDER_COUNT_TOKENS_SCHEMA_DRIFT: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// POST_GA_03 / #110 — Tier 1 provider count_tokens calls skipped by the
/// shared per-tenant quota. Kept aggregate to avoid high-cardinality tenant
/// labels on the built-in endpoint; logs carry tenant/model for forensic
/// correlation.
pub static TOKENIZER_RATE_LIMITED_TOTAL: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

async fn on_provider_error(
    key: &ShadowKey,
    err: &ProviderError,
    deps: &ShadowWorkerDeps,
) -> ShadowOutcome {
    if err.counts_as_breaker_failure() {
        deps.circuit_breaker.record_failure(key);
        warn!(error = %err, tenant = %key.tenant_id, model = %key.model,
              "shadow provider call failed; breaker counter incremented");
        ShadowOutcome::ProviderFailed
    } else {
        // Schema drift / auth — operator attention required; don't trip
        // the breaker on a stuck vendor response.
        // R2 M11: per-variant metric so operators can alert on schema
        // drift (vendor API change) independently of auth (key rotation).
        if matches!(err, ProviderError::Schema(_)) {
            PROVIDER_COUNT_TOKENS_SCHEMA_DRIFT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        warn!(error = %err, tenant = %key.tenant_id, model = %key.model,
              "shadow provider returned schema-drift or auth error; sample skipped");
        ShadowOutcome::ProviderSchemaOrAuth
    }
}

/// `|T1 - T2| / max(T1, 1)` — defensively guards against T1=0 which
/// would cause a division by zero. T1=0 is a vendor bug; treat as a
/// 100% drift so the alert fires.
pub fn compute_drift_ratio(t1: u64, t2: u64) -> f32 {
    if t1 == 0 {
        return if t2 == 0 { 0.0 } else { 1.0 };
    }
    let t1_f = t1 as f64;
    let t2_f = t2 as f64;
    ((t1_f - t2_f).abs() / t1_f) as f32
}

/// Build + sign + emit one `tokenizer_drift_alert` CloudEvent.
///
/// R2 M6: `sample_id` is carried in the CloudEvent `data` so operators
/// can pivot from an audit-chain replay back to the originating row in
/// tokenizer_t1_samples.
async fn emit_drift_alert(
    event: &ShadowEvent,
    sample_id: uuid::Uuid,
    provider_count: &super::provider_clients::ProviderCount,
    drift_ratio: f32,
    threshold: f32,
    deps: &ShadowWorkerDeps,
) -> Result<(), anyhow::Error> {
    let data = serde_json::json!({
        "sample_id": sample_id.to_string(),
        "tenant_id": event.tenant_id.to_string(),
        "model": event.model,
        "canonical_model": event.model,
        "tokenizer_version_id": event.t2_tokenizer_version_id,
        "tier2_count": event.t2_input_tokens,
        "tier1_count": provider_count.input_tokens,
        "drift_pct": drift_ratio,
        "threshold": threshold,
        "encoder_kind": event.encoder_kind.as_str(),
        "provider_request_id": provider_count.request_id,
        "provider_latency_ms": provider_count.latency.as_millis() as u64,
    });
    let data_bytes = serde_json::to_vec(&data)?;

    let now = Utc::now();
    // CloudEvent has many tag-300+ prediction-extension fields; we only
    // populate the envelope + spendguard extensions we own + clear the
    // sig fields. Default::default() supplies the rest (proto3 default
    // == "field absent on this event" — see common.proto §3.2 wire
    // semantics).
    let mut ce = CloudEvent {
        specversion: "1.0".to_string(),
        r#type: DRIFT_ALERT_EVENT_TYPE.to_string(),
        source: deps.event_source.clone(),
        id: uuid::Uuid::now_v7().to_string(),
        time: Some(prost_types::Timestamp {
            seconds: now.timestamp(),
            nanos: now.timestamp_subsec_nanos() as i32,
        }),
        datacontenttype: "application/json".to_string(),
        data: data_bytes.into(),
        tenant_id: event.tenant_id.to_string(),
        producer_id: deps.signer.producer_identity().to_string(),
        producer_signature: Bytes::new(),
        ..Default::default()
    };
    sign_in_place(deps.signer.as_ref(), &mut ce).await?;
    deps.alert_sink.emit(ce).await?;
    Ok(())
}

/// Canonical-bytes sign-in-place mirror of
/// `services/sidecar/src/audit.rs::sign_cloudevent_in_place`. We
/// duplicate the helper rather than introduce a cross-crate dep on
/// sidecar's domain error type — both call sites converge on the same
/// "set key_id, clear signature, encode, sign, write signature" recipe.
async fn sign_in_place(signer: &dyn Signer, event: &mut CloudEvent) -> Result<(), anyhow::Error> {
    event.signing_key_id = signer.key_id().to_string();
    event.producer_signature = Bytes::new();
    let canonical = event.encode_to_vec();
    let sig = signer
        .sign(&canonical)
        .await
        .map_err(|e| anyhow::anyhow!("sign drift_alert CloudEvent: {e}"))?;
    event.producer_signature = sig.bytes.into();
    Ok(())
}

// ──────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────

/// Drop-in persister for tests + demo mode. Records persisted rows in
/// an in-memory vec.
///
/// R2 M9: `mark_drift_alert_emitted` updates the matching row's
/// `drift_alert_emitted_at` so tests can assert the two-step semantics
/// (decided → emitted ack).
#[derive(Default, Debug)]
pub struct InMemorySamplePersister {
    pub rows: parking_lot::Mutex<Vec<SampleRow>>,
    pub emitted_marks: parking_lot::Mutex<
        Vec<(
            uuid::Uuid,
            chrono::DateTime<chrono::Utc>,
            chrono::DateTime<chrono::Utc>,
        )>,
    >,
}

#[async_trait::async_trait]
impl SamplePersister for InMemorySamplePersister {
    async fn persist(&self, sample: SampleRow) -> Result<(), anyhow::Error> {
        self.rows.lock().push(sample);
        Ok(())
    }

    async fn mark_drift_alert_emitted(
        &self,
        sample_id: uuid::Uuid,
        sampled_at: chrono::DateTime<chrono::Utc>,
        emitted_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), anyhow::Error> {
        self.emitted_marks
            .lock()
            .push((sample_id, sampled_at, emitted_at));
        Ok(())
    }
}

/// Drop-in alert sink for tests. Records emitted CloudEvents.
#[derive(Default, Debug)]
pub struct InMemoryDriftAlertSink {
    pub events: parking_lot::Mutex<Vec<CloudEvent>>,
}

#[async_trait::async_trait]
impl DriftAlertSink for InMemoryDriftAlertSink {
    async fn emit(&self, event: CloudEvent) -> Result<(), anyhow::Error> {
        self.events.lock().push(event);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::circuit_breaker::CircuitBreakerConfig;
    use super::super::sample_rate_state::SampleRateConfig;
    use super::super::security::{LocalCountTokensQuota, StaticShadowSecurityStore};
    use super::*;
    use spendguard_signing::DisabledSigner;
    use std::time::Duration;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    struct StaticOverrideStore(Option<f64>);

    #[async_trait::async_trait]
    impl SampleRateOverrideStore for StaticOverrideStore {
        async fn load_override(&self, _key: &ShadowKey) -> anyhow::Result<Option<f64>> {
            Ok(self.0)
        }
    }

    fn deps_for_test(providers: ProviderRoster) -> ShadowWorkerDeps {
        let sample_rate = SampleRateState::new(SampleRateConfig {
            // Always-sample so tests don't deal with probabilistic gating.
            default_rate: 1.0,
            cool_down: Duration::from_secs(3600),
            cool_down_rate: 1.0,
        });
        let cb = CircuitBreakerState::new(CircuitBreakerConfig {
            failure_threshold: 2,
            open_duration: Duration::from_millis(200),
        });
        ShadowWorkerDeps {
            sample_rate,
            circuit_breaker: cb,
            providers,
            persister: Arc::new(InMemorySamplePersister::default()),
            alert_sink: Arc::new(InMemoryDriftAlertSink::default()),
            sample_rate_overrides: None,
            security: Arc::new(StaticShadowSecurityStore::allow_all_for_tests(60)),
            count_tokens_quota: Arc::new(LocalCountTokensQuota::default()),
            signer: Arc::new(DisabledSigner::for_test("tokenizer-service:test".into())),
            event_source: "spendguard://tokenizer-service/test".into(),
            channel_capacity: 16,
        }
    }

    fn deps_for_test_sample_rate_zero(providers: ProviderRoster) -> ShadowWorkerDeps {
        let mut d = deps_for_test(providers);
        d.sample_rate = SampleRateState::new(SampleRateConfig {
            default_rate: 0.0,
            cool_down: Duration::from_secs(3600),
            cool_down_rate: 1.0,
        });
        d
    }

    #[tokio::test]
    async fn persisted_sampling_override_feeds_rate_gate_state() {
        let mut deps = deps_for_test(ProviderRoster {
            anthropic: None,
            gemini: None,
            ..ProviderRoster::default()
        });
        deps.sample_rate_overrides = Some(Arc::new(StaticOverrideStore(Some(0.0))));
        let key = ShadowKey {
            tenant_id: test_tenant_id().to_string(),
            model: "claude-3-5-sonnet-20241022".into(),
        };

        refresh_sampling_override(&key, &deps).await;

        assert_eq!(deps.sample_rate.effective_rate(&key), 0.0);
    }

    fn test_tenant_id() -> uuid::Uuid {
        uuid::Uuid::parse_str("01918000-0000-7c10-8c10-0000000000aa").unwrap()
    }

    fn ev_anthropic(t2_count: i64, raw: &str) -> ShadowEvent {
        ShadowEvent {
            tenant_id: test_tenant_id(),
            model: "claude-3-5-sonnet-20241022".into(),
            encoder_kind: EncoderKind::Anthropic,
            t2_input_tokens: t2_count,
            t2_tokenizer_version_id: "01918000-0000-7c10-8c10-000000000010".into(),
            raw_text: raw.into(),
        }
    }

    fn ev_gemini(t2_count: i64, raw: &str) -> ShadowEvent {
        ShadowEvent {
            tenant_id: test_tenant_id(),
            model: "gemini-1.5-flash".into(),
            encoder_kind: EncoderKind::Gemini,
            t2_input_tokens: t2_count,
            t2_tokenizer_version_id: "01918000-0000-7c10-8c10-000000000020".into(),
            raw_text: raw.into(),
        }
    }

    fn ev_cohere(t2_count: i64, raw: &str) -> ShadowEvent {
        ShadowEvent {
            tenant_id: test_tenant_id(),
            model: "command-r-plus-08-2024".into(),
            encoder_kind: EncoderKind::Cohere,
            t2_input_tokens: t2_count,
            t2_tokenizer_version_id: "01918000-0000-7c10-8c10-000000000030".into(),
            raw_text: raw.into(),
        }
    }

    fn ev_llama(t2_count: i64, raw: &str) -> ShadowEvent {
        ShadowEvent {
            tenant_id: test_tenant_id(),
            model: "meta.llama3-1-8b-instruct-v1:0".into(),
            encoder_kind: EncoderKind::Llama,
            t2_input_tokens: t2_count,
            t2_tokenizer_version_id: "01918000-0000-7c10-8c10-000000000040".into(),
            raw_text: raw.into(),
        }
    }

    async fn anthropic_mock_returning(tokens: u64) -> (MockServer, AnthropicClient) {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages/count_tokens"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("request-id", "req_test")
                    .set_body_json(serde_json::json!({ "input_tokens": tokens })),
            )
            .mount(&server)
            .await;
        let c = AnthropicClient::with_base_url("test-key", server.uri()).unwrap();
        (server, c)
    }

    async fn cohere_mock_returning(tokens: u64) -> (MockServer, CohereClient) {
        let server = MockServer::start().await;
        let token_ids: Vec<u64> = (0..tokens).collect();
        Mock::given(method("POST"))
            .and(path("/v1/tokenize"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "tokens": token_ids,
                "token_strings": []
            })))
            .mount(&server)
            .await;
        let c = CohereClient::with_base_url("test-key", server.uri()).unwrap();
        (server, c)
    }

    async fn llama_mock_returning(tokens: u64) -> (MockServer, LlamaClient) {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/model/meta.llama3-1-8b-instruct-v1:0/count-tokens"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "inputTokens": tokens
            })))
            .mount(&server)
            .await;
        let c = LlamaClient::with_base_url("test-key", server.uri()).unwrap();
        (server, c)
    }

    #[test]
    fn compute_drift_ratio_basic_cases() {
        // No drift.
        assert_eq!(compute_drift_ratio(100, 100), 0.0);
        // 5% drift.
        let d = compute_drift_ratio(100, 95);
        assert!((d - 0.05).abs() < 1e-5);
        // T1 < T2 (Tier 2 over-counted).
        let d = compute_drift_ratio(100, 110);
        assert!((d - 0.10).abs() < 1e-5);
        // T1 = 0, T2 > 0 → 100% drift.
        assert_eq!(compute_drift_ratio(0, 5), 1.0);
        // T1 = 0, T2 = 0 → 0% drift.
        assert_eq!(compute_drift_ratio(0, 0), 0.0);
    }

    #[tokio::test]
    async fn sample_skipped_when_rate_zero() {
        let (_s, c) = anthropic_mock_returning(100).await;
        let providers = ProviderRoster {
            anthropic: Some(c),
            gemini: None,
            ..ProviderRoster::default()
        };
        let deps = deps_for_test_sample_rate_zero(providers);
        let out = process_one(&ev_anthropic(100, "hi"), &deps).await;
        assert_eq!(out, ShadowOutcome::Skipped);
    }

    #[tokio::test]
    async fn raw_text_not_sent_without_tenant_pii_opt_in() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages/count_tokens"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "input_tokens": 100
            })))
            .mount(&server)
            .await;
        let client = AnthropicClient::with_base_url("test-key", server.uri()).unwrap();
        let providers = ProviderRoster {
            anthropic: Some(client),
            gemini: None,
            ..ProviderRoster::default()
        };
        let mut deps = deps_for_test(providers);
        deps.security = Arc::new(StaticShadowSecurityStore::deny_all());

        let out = process_one(&ev_anthropic(100, "sensitive prompt body"), &deps).await;
        assert_eq!(out, ShadowOutcome::Skipped);

        let requests = server.received_requests().await.unwrap_or_default();
        assert!(
            requests.is_empty(),
            "provider received raw prompt despite tenant opt-in=false"
        );
    }

    #[tokio::test]
    async fn raw_text_not_sent_to_new_providers_without_tenant_pii_opt_in() {
        let (cohere_server, cohere) = cohere_mock_returning(100).await;
        let (llama_server, llama) = llama_mock_returning(100).await;
        let providers = ProviderRoster {
            cohere: Some(cohere),
            llama: Some(llama),
            ..ProviderRoster::default()
        };
        let mut deps = deps_for_test(providers);
        deps.security = Arc::new(StaticShadowSecurityStore::deny_all());

        let cohere_out = process_one(&ev_cohere(100, "sensitive cohere body"), &deps).await;
        let llama_out = process_one(&ev_llama(100, "sensitive llama body"), &deps).await;

        assert_eq!(cohere_out, ShadowOutcome::Skipped);
        assert_eq!(llama_out, ShadowOutcome::Skipped);
        assert!(cohere_server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty());
        assert!(llama_server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty());
    }

    #[tokio::test]
    async fn count_tokens_quota_blocks_excess_per_tenant_provider_calls() {
        let (_server, c) = anthropic_mock_returning(100).await;
        let providers = ProviderRoster {
            anthropic: Some(c),
            gemini: None,
            ..ProviderRoster::default()
        };
        let persister = Arc::new(InMemorySamplePersister::default());
        let mut deps = deps_for_test(providers);
        deps.security = Arc::new(StaticShadowSecurityStore::allow_all_for_tests(1));
        deps.persister = persister.clone();

        let first = process_one(&ev_anthropic(100, "first"), &deps).await;
        assert_eq!(first, ShadowOutcome::Sampled);
        let second = process_one(&ev_anthropic(100, "second"), &deps).await;
        assert_eq!(second, ShadowOutcome::Skipped);
        assert_eq!(persister.rows.lock().len(), 1);
    }

    #[tokio::test]
    async fn count_tokens_quota_is_per_tenant_and_provider_for_new_clients() {
        let (_cohere_server, cohere) = cohere_mock_returning(100).await;
        let (_llama_server, llama) = llama_mock_returning(100).await;
        let providers = ProviderRoster {
            cohere: Some(cohere),
            llama: Some(llama),
            ..ProviderRoster::default()
        };
        let persister = Arc::new(InMemorySamplePersister::default());
        let mut deps = deps_for_test(providers);
        deps.security = Arc::new(StaticShadowSecurityStore::allow_all_for_tests(1));
        deps.persister = persister.clone();

        let first_cohere = process_one(&ev_cohere(100, "first cohere"), &deps).await;
        let second_cohere = process_one(&ev_cohere(100, "second cohere"), &deps).await;
        let first_llama = process_one(&ev_llama(100, "first llama"), &deps).await;

        assert_eq!(first_cohere, ShadowOutcome::Sampled);
        assert_eq!(second_cohere, ShadowOutcome::Skipped);
        assert_eq!(first_llama, ShadowOutcome::Sampled);
        assert_eq!(persister.rows.lock().len(), 2);
    }

    #[tokio::test]
    async fn happy_path_no_drift_persists_sample() {
        // T1 = T2 = 100 → drift_ratio = 0 ≤ threshold 0.01 → no alert.
        let (_server, c) = anthropic_mock_returning(100).await;
        let providers = ProviderRoster {
            anthropic: Some(c),
            gemini: None,
            ..ProviderRoster::default()
        };
        let persister = Arc::new(InMemorySamplePersister::default());
        let alert_sink = Arc::new(InMemoryDriftAlertSink::default());
        let mut deps = deps_for_test(providers);
        deps.persister = persister.clone();
        deps.alert_sink = alert_sink.clone();

        let out = process_one(&ev_anthropic(100, "hi"), &deps).await;
        assert_eq!(out, ShadowOutcome::Sampled);

        let rows = persister.rows.lock();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].t1_input_tokens, 100);
        assert_eq!(rows[0].t2_input_tokens, 100);
        assert_eq!(rows[0].drift_ratio, 0.0);
        assert!(!rows[0].drift_alert_decided);
        // R2 M9: emitted_marks empty too — no alert decision means no
        // CloudEvent emission path.
        assert!(persister.emitted_marks.lock().is_empty());
        let events = alert_sink.events.lock();
        assert!(events.is_empty(), "no alert event for no-drift sample");
    }

    #[derive(Debug, Default)]
    struct SlowSamplePersister {
        rows: parking_lot::Mutex<Vec<SampleRow>>,
        persist_finished_at: parking_lot::Mutex<Option<chrono::DateTime<chrono::Utc>>>,
    }

    #[async_trait::async_trait]
    impl SamplePersister for SlowSamplePersister {
        async fn persist(&self, sample: SampleRow) -> Result<(), anyhow::Error> {
            tokio::time::sleep(Duration::from_millis(75)).await;
            let finished_at = Utc::now();
            self.rows.lock().push(sample);
            *self.persist_finished_at.lock() = Some(finished_at);
            Ok(())
        }

        async fn mark_drift_alert_emitted(
            &self,
            _sample_id: uuid::Uuid,
            _sampled_at: chrono::DateTime<chrono::Utc>,
            _emitted_at: chrono::DateTime<chrono::Utc>,
        ) -> Result<(), anyhow::Error> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn sampled_at_is_captured_before_persistence_latency() {
        let (_server, c) = anthropic_mock_returning(100).await;
        let providers = ProviderRoster {
            anthropic: Some(c),
            gemini: None,
            ..ProviderRoster::default()
        };
        let persister = Arc::new(SlowSamplePersister::default());
        let mut deps = deps_for_test(providers);
        deps.persister = persister.clone();

        let before = Utc::now();
        let out = process_one(&ev_anthropic(100, "hi"), &deps).await;
        let after = Utc::now();

        assert_eq!(out, ShadowOutcome::Sampled);
        let rows = persister.rows.lock();
        assert_eq!(rows.len(), 1);
        let sampled_at = rows[0].sampled_at;
        drop(rows);
        let finished_at = persister
            .persist_finished_at
            .lock()
            .expect("persist finished");
        assert!(sampled_at >= before);
        assert!(sampled_at <= after);
        assert!(
            sampled_at < finished_at,
            "sampled_at regressed to persistence completion time"
        );
    }

    #[tokio::test]
    async fn drift_above_threshold_emits_signed_cloudevent() {
        // Anthropic threshold is 0.01. T1=100, T2=90 → drift=10% ≫ 0.01.
        let (_server, c) = anthropic_mock_returning(100).await;
        let providers = ProviderRoster {
            anthropic: Some(c),
            gemini: None,
            ..ProviderRoster::default()
        };
        let persister = Arc::new(InMemorySamplePersister::default());
        let alert_sink = Arc::new(InMemoryDriftAlertSink::default());
        let mut deps = deps_for_test(providers);
        deps.persister = persister.clone();
        deps.alert_sink = alert_sink.clone();

        let out = process_one(&ev_anthropic(90, "hi"), &deps).await;
        assert_eq!(out, ShadowOutcome::Alerted);

        let rows = persister.rows.lock();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].drift_alert_decided);
        assert!((rows[0].drift_ratio - 0.10).abs() < 1e-4);
        let row_sample_id = rows[0].sample_id;
        let row_sampled_at = rows[0].sampled_at;
        drop(rows);

        let events = alert_sink.events.lock();
        assert_eq!(events.len(), 1);
        let ce = &events[0];
        assert_eq!(ce.specversion, "1.0");
        assert_eq!(ce.r#type, DRIFT_ALERT_EVENT_TYPE);
        assert_eq!(ce.datacontenttype, "application/json");
        // signing_key_id populated by sign_in_place (DisabledSigner →
        // "disabled" surface; the prod LocalEd25519Signer returns the
        // ed25519:<hex> form).
        assert!(!ce.signing_key_id.is_empty());

        // Decode the JSON data and verify required fields.
        let data: serde_json::Value = serde_json::from_slice(&ce.data).unwrap();
        assert_eq!(data["model"], "claude-3-5-sonnet-20241022");
        assert_eq!(data["canonical_model"], "claude-3-5-sonnet-20241022");
        assert_eq!(data["tier1_count"], 100);
        assert_eq!(data["tier2_count"], 90);
        assert_eq!(data["encoder_kind"], "ANTHROPIC_BPE");
        assert!(data["drift_pct"].as_f64().unwrap() > 0.09);
        assert!((data["threshold"].as_f64().unwrap() - 0.01).abs() < 1e-6);
        // R2 M6: sample_id is in the CloudEvent data for forensic pivot
        // back to the t1_samples row.
        let ce_sample_id = data["sample_id"].as_str().expect("sample_id present");
        assert_eq!(ce_sample_id, row_sample_id.to_string());

        // R2 M9: emitted_marks captures the persister-side ack with
        // sample_id + sampled_at + emitted_at.
        let marks = persister.emitted_marks.lock();
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].0, row_sample_id);
        assert_eq!(marks[0].1, row_sampled_at);
        assert!(marks[0].2 >= row_sampled_at);
    }

    #[tokio::test]
    async fn drift_alert_enters_cool_down() {
        // After an alert, sample rate snapshot should report 100% rate.
        let (_server, c) = anthropic_mock_returning(100).await;
        let providers = ProviderRoster {
            anthropic: Some(c),
            gemini: None,
            ..ProviderRoster::default()
        };
        let deps = deps_for_test(providers);
        let sample_rate = deps.sample_rate.clone();

        let ev = ev_anthropic(90, "hi");
        let _ = process_one(&ev, &deps).await;
        let snap = sample_rate.snapshot(&ev.shadow_key());
        assert!(
            snap.in_cool_down,
            "drift alert must enter cool-down per spec §4.3"
        );
        assert!((snap.effective_rate - 1.0).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn provider_5xx_increments_breaker() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages/count_tokens"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let c = AnthropicClient::with_base_url("test-key", server.uri()).unwrap();
        let providers = ProviderRoster {
            anthropic: Some(c),
            gemini: None,
            ..ProviderRoster::default()
        };
        let deps = deps_for_test(providers);
        let key = ShadowKey {
            tenant_id: test_tenant_id().to_string(),
            model: "claude-3-5-sonnet-20241022".into(),
        };

        let out = process_one(&ev_anthropic(100, "hi"), &deps).await;
        assert_eq!(out, ShadowOutcome::ProviderFailed);
        assert_eq!(deps.circuit_breaker.consecutive_failures(&key), 1);

        let _ = process_one(&ev_anthropic(100, "hi"), &deps).await;
        // After 2 failures (test threshold) breaker is Open and the
        // next attempt is Skipped.
        let out = process_one(&ev_anthropic(100, "hi"), &deps).await;
        assert_eq!(out, ShadowOutcome::Skipped);
    }

    #[tokio::test]
    async fn schema_drift_skips_sample_without_tripping_breaker() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages/count_tokens"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "nope": "wrong-shape" })),
            )
            .mount(&server)
            .await;
        let c = AnthropicClient::with_base_url("test-key", server.uri()).unwrap();
        let providers = ProviderRoster {
            anthropic: Some(c),
            gemini: None,
            ..ProviderRoster::default()
        };
        let deps = deps_for_test(providers);
        let key = ShadowKey {
            tenant_id: test_tenant_id().to_string(),
            model: "claude-3-5-sonnet-20241022".into(),
        };
        for _ in 0..5 {
            let out = process_one(&ev_anthropic(100, "hi"), &deps).await;
            assert_eq!(out, ShadowOutcome::ProviderSchemaOrAuth);
        }
        // 5 schema-drift responses MUST NOT trip the breaker.
        assert_eq!(deps.circuit_breaker.consecutive_failures(&key), 0);
    }

    #[tokio::test]
    async fn unsupported_encoder_kind_skips() {
        // OpenAI is byte-exact Tier 2 and has no Tier 1 shadow provider.
        let providers = ProviderRoster::default();
        let deps = deps_for_test(providers);
        let mut ev = ev_anthropic(100, "hi");
        ev.encoder_kind = EncoderKind::OpenAi;
        let out = process_one(&ev, &deps).await;
        assert_eq!(out, ShadowOutcome::Skipped);
    }

    #[tokio::test]
    async fn gemini_dispatch_path() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/models/gemini-1.5-flash:countTokens"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "totalTokens": 100,
                "totalBillableCharacters": 50
            })))
            .mount(&server)
            .await;
        let c = GeminiClient::with_base_url("test-key", server.uri()).unwrap();
        let providers = ProviderRoster {
            anthropic: None,
            gemini: Some(c),
            ..ProviderRoster::default()
        };
        let persister = Arc::new(InMemorySamplePersister::default());
        let mut deps = deps_for_test(providers);
        deps.persister = persister.clone();

        let out = process_one(&ev_gemini(100, "hi"), &deps).await;
        assert_eq!(out, ShadowOutcome::Sampled);
        assert_eq!(persister.rows.lock().len(), 1);
    }

    #[tokio::test]
    async fn cohere_dispatch_path() {
        let (_server, c) = cohere_mock_returning(100).await;
        let providers = ProviderRoster {
            cohere: Some(c),
            ..ProviderRoster::default()
        };
        let persister = Arc::new(InMemorySamplePersister::default());
        let mut deps = deps_for_test(providers);
        deps.persister = persister.clone();

        let out = process_one(&ev_cohere(100, "hi"), &deps).await;
        assert_eq!(out, ShadowOutcome::Sampled);
        let rows = persister.rows.lock();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].model, "command-r-plus-08-2024");
        assert_eq!(rows[0].t1_input_tokens, 100);
    }

    #[tokio::test]
    async fn llama_dispatch_path() {
        let (_server, c) = llama_mock_returning(100).await;
        let providers = ProviderRoster {
            llama: Some(c),
            ..ProviderRoster::default()
        };
        let persister = Arc::new(InMemorySamplePersister::default());
        let mut deps = deps_for_test(providers);
        deps.persister = persister.clone();

        let out = process_one(&ev_llama(100, "hi"), &deps).await;
        assert_eq!(out, ShadowOutcome::Sampled);
        let rows = persister.rows.lock();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].model, "meta.llama3-1-8b-instruct-v1:0");
        assert_eq!(rows[0].t1_input_tokens, 100);
    }

    #[tokio::test]
    async fn handle_try_send_smoke() {
        let providers = ProviderRoster::default();
        let deps = deps_for_test(providers);
        let h = spawn_shadow_worker(deps);
        let ev = ev_anthropic(100, "hi");
        h.try_send(ev)
            .expect("first send succeeds on fresh channel");
    }

    #[tokio::test]
    async fn drop_handle_compiles_and_drains() {
        let h = spawn_drop_handle(8);
        let ev = ev_anthropic(100, "hi");
        h.try_send(ev).expect("demo drop handle accepts events");
    }
}
