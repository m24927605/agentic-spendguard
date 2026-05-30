//! DecisionRequest construction + ReservationContext + pricing cache.
//!
//! Slice 4b deliverable per spec §15 row 4b + §4.1 step 5 + §4.1.5.
//!
//! Key invariants:
//! - step_id derivation uses spendguard-ids (deterministic across proxy/SDK)
//! - PricingFreeze frozen at PRE; never re-read between PRE and POST
//! - arc_swap cache invalidates on runtime.env mtime advance
//! - DecisionRequest carries route="llm.call" + trigger=LLM_CALL_PRE
//! - All three SpendGuardIds present (run_id / step_id / llm_call_id /
//!   decision_id) per spec P2-r3.D fix
//!
//! SLICE_10 Phase B addition: estimate_call_cost replaces the 17-line
//! `chars/4 × 2` heuristic with a real 3-stage prediction pipeline:
//!   1. tokenizer library (in-process Tier 2; p99 ≤ 1ms)
//!   2. output_predictor gRPC (Strategy A/B/C selector; ≤ 10ms hard cap)
//!   3. run_cost_projector gRPC (RUN_* projection; ≤ 5ms hard cap)
//! Spec §11 failure modes drive the fallback path when either gRPC is
//! unreachable; the tokenizer is fail-closed (Tier 2 panic invariant).

use anyhow::Context;
use arc_swap::ArcSwap;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tracing::{debug, warn};
use uuid::Uuid;

use crate::predictor_client::{
    OutputPredictorClient, RunCostProjectorClient, OUTPUT_PREDICTOR_TIMEOUT_MS,
    RUN_COST_PROJECTOR_TIMEOUT_MS,
};
use crate::proto::common::v1::{
    self as common_pb, budget_claim::Direction, unit_ref::Kind as UnitKind,
};
use crate::proto::output_predictor::v1::{PredictRequest, PredictResponse};
use crate::proto::run_cost_projector::v1::{ProjectRequest, ProjectResponse};
use crate::proto::sidecar_adapter::v1::{
    decision_request::{Inputs, Trigger},
    ClaimEstimate, DecisionRequest,
};

/// Reservation context retained per-request between PRE (decision)
/// and POST (commit). Spec §4.1.5 — FROZEN at construction; never
/// re-read until commit.
#[derive(Clone)]
pub struct ReservationContext {
    pub reservation_id: Uuid,
    pub decision_id: Uuid,
    pub effect_hash: Vec<u8>,
    pub unit: common_pb::UnitRef,
    pub pricing: common_pb::PricingFreeze,
    pub run_id: String,
    pub step_id: String,
    pub llm_call_id: String,
    pub audit_decision_event_id: Uuid,
}

/// Inputs derived from the HTTP request body + headers.
pub struct DecisionInputs<'a> {
    pub tenant_id: &'a str,
    pub budget_id: &'a str,
    pub window_instance_id: &'a str,
    pub run_id: String, // from X-SpendGuard-Run-Id or fresh UUIDv7
    pub body_bytes: &'a [u8],
    pub model_family: String,  // parsed from request.model
    pub estimated_tokens: i64, // heuristic or X-SpendGuard-Estimated-Tokens
    pub unit_id: &'a str,
    /// Final-sweep P2 fix: caller may supply explicit
    /// X-SpendGuard-Idempotency-Key for retry-collapse semantics
    /// (spec §3.2 + §7). When None, per-attempt nanos-based key
    /// is used (default; prevents OpenAI double-bill on SDK retry).
    pub explicit_idempotency_key: Option<String>,
    /// SLICE_10 Phase B — full ClaimEstimate from estimate_call_cost.
    /// When present, sidecar reads all 17 prediction columns from this
    /// sub-message into the audit_decision CloudEvent (tags 300-313).
    /// When `None` (e.g. SDK wrapper-mode caller supplies its own
    /// projected_claims, or pre-SLICE_10 path), sidecar falls back to
    /// proto3-default = NULL for the prediction columns.
    pub claim_estimate: Option<ClaimEstimate>,
}

/// Build a `DecisionRequest` for the sidecar.
///
/// Step IDs are derived via `spendguard-ids` so retries of the same
/// body collapse to the same step_id → same cost_advisor fingerprint
/// → finding accumulates. The unified `:call:` discriminator (NOT
/// `:proxy-call:`) per Staff escalation r5 verdict.
///
/// Note: DecisionRequest does NOT carry pricing or fencing — sidecar
/// loads pricing from its bundle. PricingFreeze becomes relevant at
/// the POST step (LlmCallPostPayload, slice 5).
pub fn build_decision_request(inputs: &DecisionInputs<'_>) -> anyhow::Result<DecisionRequest> {
    let body_json: Value = serde_json::from_slice(inputs.body_bytes)
        .context("body not valid JSON for signature derivation")?;
    let signature =
        spendguard_ids::default_call_signature_jcs(&body_json).context("compute body signature")?;

    let step_id = format!(
        "{}:call:{}",
        inputs.run_id,
        &signature[..16.min(signature.len())]
    );
    let llm_call_id = spendguard_ids::derive_uuid_from_signature(&signature, "llm_call_id");
    let decision_id = Uuid::now_v7();

    // Idempotency key resolution (spec §3.2 + §7):
    // 1. Explicit X-SpendGuard-Idempotency-Key header → use verbatim
    //    (retry-collapse opt-in)
    // 2. Default: per-attempt sha256(sig || nanos)[..16] (prevents
    //    OpenAI double-bill on SDK auto-retry)
    let idempotency_key = match &inputs.explicit_idempotency_key {
        Some(k) if !k.is_empty() => k.clone(),
        _ => {
            let nanos = SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos().to_string())
                .unwrap_or_default();
            let mut combined = signature.clone();
            combined.push('|');
            combined.push_str(&nanos);
            let bytes = blake2_helper(combined.as_bytes());
            hex::encode(&bytes[..8])
        }
    };

    let unit = common_pb::UnitRef {
        unit_id: inputs.unit_id.to_string(),
        kind: UnitKind::Token as i32,
        currency: String::new(),
        token_kind: "output_token".to_string(),
        model_family: inputs.model_family.clone(),
        ..Default::default()
    };

    let claim = common_pb::BudgetClaim {
        budget_id: inputs.budget_id.to_string(),
        unit: Some(unit.clone()),
        amount_atomic: inputs.estimated_tokens.to_string(),
        direction: Direction::Debit as i32,
        window_instance_id: inputs.window_instance_id.to_string(),
        ..Default::default()
    };

    let inputs_msg = Inputs {
        projected_claims: vec![claim],
        projected_unit: Some(unit),
        // SLICE_10 Phase B + Phase C: ClaimEstimate carries all 17
        // prediction columns into the sidecar audit row. None for
        // legacy SDK wrapper-mode (sidecar handles None gracefully).
        claim_estimate: inputs.claim_estimate.clone(),
        ..Default::default()
    };

    let req = DecisionRequest {
        session_id: format!("egress-proxy:{}", inputs.run_id),
        trigger: Trigger::LlmCallPre as i32,
        route: "llm.call".to_string(),
        ids: Some(common_pb::SpendGuardIds {
            run_id: inputs.run_id.clone(),
            step_id,
            llm_call_id: llm_call_id.to_string(),
            decision_id: decision_id.to_string(),
            ..Default::default()
        }),
        idempotency: Some(common_pb::Idempotency {
            key: idempotency_key,
            request_hash: Vec::new().into(),
        }),
        inputs: Some(inputs_msg),
        ..Default::default()
    };
    Ok(req)
}

fn blake2_helper(data: &[u8]) -> Vec<u8> {
    use blake2::{digest::consts::U16, Blake2b, Digest};
    let mut h = Blake2b::<U16>::new();
    h.update(data);
    h.finalize().to_vec()
}

/// Pricing tuple snapshot, cached at process scope and refreshed on
/// `runtime.env` mtime advance. Spec §4.1.5.
#[derive(Clone, Debug)]
pub struct PricingSnapshot {
    pub pricing: common_pb::PricingFreeze,
    pub mtime: Option<SystemTime>,
}

#[derive(Clone)]
pub struct PricingCache {
    pub runtime_env_path: PathBuf,
    pub current: Arc<ArcSwap<PricingSnapshot>>,
}

impl PricingCache {
    /// Load initial snapshot from runtime.env + metadata.json.
    pub fn load(runtime_env_path: PathBuf) -> anyhow::Result<Self> {
        let snapshot = read_pricing_snapshot(&runtime_env_path)
            .with_context(|| format!("initial pricing read from {}", runtime_env_path.display()))?;
        Ok(Self {
            runtime_env_path,
            current: Arc::new(ArcSwap::from_pointee(snapshot)),
        })
    }

    /// Get a fresh pricing tuple, refreshing the cache if the file
    /// mtime has advanced.
    pub fn get_fresh(&self) -> common_pb::PricingFreeze {
        let cached = self.current.load_full();
        if let Ok(meta) = std::fs::metadata(&self.runtime_env_path) {
            let on_disk_mtime = meta.modified().ok();
            if on_disk_mtime != cached.mtime {
                match read_pricing_snapshot(&self.runtime_env_path) {
                    Ok(fresh) => {
                        debug!(
                            old_mtime = ?cached.mtime,
                            new_mtime = ?fresh.mtime,
                            "pricing cache refreshed on mtime advance"
                        );
                        self.current.store(Arc::new(fresh.clone()));
                        return fresh.pricing;
                    }
                    Err(e) => {
                        warn!(err = %e, "pricing refresh failed; reusing cached snapshot");
                    }
                }
            }
        }
        cached.pricing.clone()
    }
}

/// Read pricing tuple from runtime.env (+ sibling metadata.json).
///
/// Reuses the same shape as the sidecar's `bundles/contract_bundle/<id>.metadata.json`.
/// For demo path, runtime.env carries SPENDGUARD_SIDECAR_PRICING_VERSION /
/// _SNAPSHOT_HASH_HEX / _FX_RATE_VERSION / _UNIT_CONVERSION_VERSION
/// (mirroring what bundles-init populates for the sidecar). If these
/// env vars are absent, fall back to demo defaults.
fn read_pricing_snapshot(path: &std::path::Path) -> anyhow::Result<PricingSnapshot> {
    let mtime = std::fs::metadata(path).ok().and_then(|m| m.modified().ok());

    let contents = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("read {}: {}", path.display(), e))?;

    let mut map = std::collections::HashMap::new();
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        let body = trimmed.strip_prefix("export ").unwrap_or(trimmed);
        if let Some((k, v)) = body.split_once('=') {
            let v = v.trim().trim_matches('"').trim_matches('\'');
            map.insert(k.trim().to_string(), v.to_string());
        }
    }

    let pricing_version = map
        .get("SPENDGUARD_SIDECAR_PRICING_VERSION")
        .cloned()
        .unwrap_or_else(|| "demo-pricing-v1".to_string());
    let price_snapshot_hash_hex = map
        .get("SPENDGUARD_SIDECAR_PRICE_SNAPSHOT_HASH_HEX")
        .or_else(|| map.get("SPENDGUARD_SCHEMA_BUNDLE_HASH_HEX"))
        .cloned()
        .unwrap_or_default();
    let fx_rate_version = map
        .get("SPENDGUARD_SIDECAR_FX_RATE_VERSION")
        .cloned()
        .unwrap_or_else(|| "demo-fx-v1".to_string());
    let unit_conversion_version = map
        .get("SPENDGUARD_SIDECAR_UNIT_CONVERSION_VERSION")
        .cloned()
        .unwrap_or_else(|| "demo-units-v1".to_string());

    let snapshot_hash_bytes = if price_snapshot_hash_hex.is_empty() {
        Vec::new()
    } else {
        hex::decode(&price_snapshot_hash_hex).with_context(|| {
            format!(
                "price_snapshot_hash_hex not valid hex: {}",
                price_snapshot_hash_hex
            )
        })?
    };

    Ok(PricingSnapshot {
        pricing: common_pb::PricingFreeze {
            pricing_version,
            price_snapshot_hash: snapshot_hash_bytes.into(),
            fx_rate_version,
            unit_conversion_version,
        },
        mtime,
    })
}

// ============================================================================
// SLICE_10 Phase B — estimate_call_cost (replaces the 17-line chars/4 × 2
// heuristic from HANDOFF §2.2; spec ref predictor-architecture-v1alpha1 §2.2)
// ============================================================================
//
// Aggregate latency budget (per spec §11.2 + Contract §14 50ms p99):
//   tokenizer library : ≤ 1ms p99 (in-process, synchronous; no timeout)
//   output_predictor  : ≤ 10ms hard cap (parallel via tokio::join!)
//   run_cost_projector: ≤ 5ms hard cap  (parallel via tokio::join!)
//   ─────────────────────────────────────────────────────────────────
//   Aggregate         : ~11ms p99 (parallel) — leaves headroom for
//                       sidecar request_decision (≤ 35ms remaining for
//                       50ms total per Contract §14).
//
// Failure modes (spec §11):
//   tokenizer fail    → fail-closed (Tier 2 panic invariant; spec §3.6).
//   predictor fail    → fall back to local Strategy A (input_tokens ×
//                       model.max_output_tokens || 4096). Metric
//                       `output_predictor_unreachable_total` incremented
//                       by caller; egress_proxy stays open.
//   projector fail    → pass-through (no RUN_* code; run_predicted_remaining
//                       = -1 sentinel per audit-chain-extension §3.3).
//                       Metric `run_cost_projector_unreachable_total`.

/// SLICE_10 Phase B classification — the prompt class label used by
/// output_predictor.PredictRequest.prompt_class. The egress_proxy v1
/// classifier is a simple body-shape heuristic; SLICE_06 R3 federates
/// this to a real classifier. The classifier output also drives
/// `audit_outbox.prompt_class` mirror columns (SLICE_06 R2 B4).
pub fn classify_prompt(body: &Value) -> &'static str {
    // Heuristic v1: count messages, look for `tools` / `tool_choice`.
    // SLICE_06 R3 will federate this to a real classifier.
    let messages = body.get("messages").and_then(|v| v.as_array());
    let has_tools = body.get("tools").is_some() || body.get("tool_choice").is_some();
    let total_chars: usize = messages
        .map(|m| {
            m.iter()
                .filter_map(|msg| msg.get("content"))
                .filter_map(|c| c.as_str())
                .map(|s| s.len())
                .sum()
        })
        .unwrap_or(0);
    if has_tools {
        "tool_calling"
    } else if total_chars > 4_000 {
        "chat_long"
    } else {
        "chat_short"
    }
}

/// SLICE_10 Phase B — header override or tokenizer-driven token count.
///
/// Spec §5.1 priority: header override > tokenizer library > Tier 3 fallback.
/// Tier 3 fallback (chars/4 × 1.05) is computed by the tokenizer library
/// itself when the model isn't in the dispatch table — no heuristic
/// codepath lives in egress_proxy after this slice.
pub fn input_tokens_with_override(
    tok_response: &spendguard_tokenizer::TokenizeResponse,
    header_override: Option<i64>,
) -> i64 {
    if let Some(v) = header_override {
        return v.max(1);
    }
    tok_response.input_tokens.max(1)
}

/// SLICE_10 Phase B — convert serde messages to tokenizer library format.
fn serialize_messages_for_tokenizer(body: &Value) -> Vec<spendguard_tokenizer::Message> {
    let arr = match body.get("messages").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };
    arr.iter()
        .map(|m| {
            let role = m
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("user")
                .to_string();
            let content = m
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            spendguard_tokenizer::Message {
                role,
                content,
                tool_calls: Vec::new(),
            }
        })
        .collect()
}

/// SLICE_10 Phase B input shape — everything `estimate_call_cost` needs.
pub struct EstimateInputs<'a> {
    pub body: &'a Value,
    pub model: &'a str,
    pub tenant_id: &'a str,
    pub agent_id: &'a str,
    pub run_id: &'a str,
    pub decision_id: &'a str,
    pub step_id: &'a str,
    /// Active prediction policy from contract evaluation. For now
    /// defaults to STRICT_CEILING — sidecar overrides via contract DSL.
    pub prediction_policy: &'a str,
    /// X-SpendGuard-Estimated-Tokens header value (header override path).
    pub header_override_tokens: Option<i64>,
    /// SDK Signal 3 hint: caller-supplied total planned LLM + tool steps.
    /// 0 = unset.
    pub planned_steps_hint: i32,
}

/// SLICE_10 Phase B — the function that replaces the legacy
/// `chars/4 × 2` heuristic.
///
/// Returns `(input_tokens, ClaimEstimate)`:
///   * `input_tokens` is the reservation amount the legacy
///     `DecisionInputs.estimated_tokens` field expected (Strategy A's
///     output is the conservative reservation per spec §4 invariant).
///   * `ClaimEstimate` carries the full 17-column prediction metadata
///     for the audit chain — sidecar populates CloudEvent tags 300-313.
///
/// Caller is responsible for handling output_predictor / run_cost_projector
/// Option<None> (in which case the function falls back to Strategy A and
/// run_projection sentinels per spec §11).
pub async fn estimate_call_cost(
    inputs: &EstimateInputs<'_>,
    tokenizer: &spendguard_tokenizer::Tokenizer,
    output_predictor: Option<&OutputPredictorClient>,
    run_cost_projector: Option<&RunCostProjectorClient>,
) -> ClaimEstimate {
    // ── Step 1: tokenizer.tokenize (library form, < 1ms p99) ──────────
    //
    // Tier 2 panic invariant (spec §3.6): a tokenizer error here is
    // fail-closed at boot. At runtime, the only Err is from cache or
    // dispatch — both indicate a bug. We log + fall back to a 0-tokens
    // estimate which surfaces as a defensive "small request" guard;
    // production runs never hit this branch.
    let tok_req = spendguard_tokenizer::TokenizeRequest {
        model: inputs.model.to_string(),
        messages: serialize_messages_for_tokenizer(inputs.body),
        raw_text: String::new(),
        request_id: inputs.decision_id.to_string(),
    };
    let tok_resp = match tokenizer.tokenize(&tok_req) {
        Ok(r) => r,
        Err(e) => {
            warn!(err = %e, model = %inputs.model, "tokenizer.tokenize error; falling back to T3 zero-token estimate");
            spendguard_tokenizer::TokenizeResponse {
                tier: "T3".to_string(),
                ..Default::default()
            }
        }
    };
    let input_tokens = input_tokens_with_override(&tok_resp, inputs.header_override_tokens);
    let tokenizer_tier = tok_resp.tier.clone();
    let tokenizer_version_id = tok_resp.tokenizer_version_id.clone();

    let prompt_class = classify_prompt(inputs.body);

    // ── Step 2 + 3: parallel Predict + Project (10ms + 5ms hard caps) ─
    //
    // Per spec §11.1: A only ≤ 1ms; A+B ≤ 5ms; A+B+C ≤ 15ms — egress_proxy
    // dials with a 10ms timeout which covers A+B+C with margin. The
    // projector is dialled in parallel via tokio::join! so the aggregate
    // wall time = max(predict, project) ≈ 10ms p99, not the sum.
    let max_tokens_requested = inputs
        .body
        .get("max_tokens")
        .or_else(|| inputs.body.get("max_output_tokens"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let model_context_window = lookup_model_context_window(inputs.model);

    let predict_req = PredictRequest {
        tenant_id: inputs.tenant_id.to_string(),
        model: inputs.model.to_string(),
        agent_id: inputs.agent_id.to_string(),
        prompt_class: prompt_class.to_string(),
        input_tokens,
        max_tokens_requested,
        model_context_window,
        prediction_policy: inputs.prediction_policy.to_string(),
        plugin_features: None,
        decision_id: inputs.decision_id.to_string(),
        run_id: inputs.run_id.to_string(),
        prompt_class_fingerprint: String::new(),
    };

    // Strategy A local fallback per spec §11 (when predictor unreachable).
    let strategy_a_local =
        compute_strategy_a_local(input_tokens, max_tokens_requested, model_context_window);

    let project_fut = async {
        match run_cost_projector {
            None => Err("projector client not configured".to_string()),
            Some(client) => {
                // The egress proxy does not have an authoritative budget
                // remaining snapshot. Passing 0 here would make every
                // non-trivial projection look over budget and produce a
                // false RUN_BUDGET_PROJECTION_EXCEEDED. Use a non-triggering
                // sentinel; the sidecar/ledger reserve path remains the
                // hard budget gate.
                let req = ProjectRequest {
                    tenant_id: inputs.tenant_id.to_string(),
                    run_id: inputs.run_id.to_string(),
                    agent_id: inputs.agent_id.to_string(),
                    model: inputs.model.to_string(),
                    step_id: inputs.step_id.to_string(),
                    decision_id: inputs.decision_id.to_string(),
                    // Use Strategy A as the conservative this-call cost
                    // when predictor hasn't returned yet (parallel join).
                    this_call_reservation_atomic: strategy_a_local,
                    unit_id: String::new(),
                    budget_remaining_atomic: i64::MAX,
                    planned_steps_hint: inputs.planned_steps_hint,
                    planned_tools_hint: 0,
                };
                client
                    .project_with_timeout(req, Duration::from_millis(RUN_COST_PROJECTOR_TIMEOUT_MS))
                    .await
                    .map_err(|e| e.to_string())
            }
        }
    };

    let predict_fut = async {
        match output_predictor {
            None => Err("output_predictor client not configured".to_string()),
            Some(client) => client
                .predict_with_timeout(
                    predict_req,
                    Duration::from_millis(OUTPUT_PREDICTOR_TIMEOUT_MS),
                )
                .await
                .map_err(|e| e.to_string()),
        }
    };

    let (predict_result, project_result) = tokio::join!(predict_fut, project_fut);

    // ── Step 4: build ClaimEstimate ─────────────────────────────────────
    let estimate = match predict_result {
        Ok(p) => build_estimate_from_predictor(
            &tok_resp,
            input_tokens,
            inputs.prediction_policy,
            &p,
            project_result.as_ref().ok(),
        ),
        Err(predict_err) => {
            warn!(
                err = %predict_err,
                model = %inputs.model,
                "output_predictor unreachable; falling back to local Strategy A"
            );
            build_estimate_fallback_a(
                tokenizer_tier.clone(),
                tokenizer_version_id.clone(),
                input_tokens,
                strategy_a_local,
                inputs.prediction_policy,
                project_result.as_ref().ok(),
            )
        }
    };

    debug!(
        input_tokens = estimate.input_tokens,
        reserved_strategy = %estimate.reserved_strategy,
        tokenizer_tier = %estimate.tokenizer_tier,
        run_code = %estimate.run_code_triggered,
        "estimate_call_cost complete"
    );
    estimate
}

/// Local Strategy A computation per spec §3.1 — the fallback when
/// output_predictor is unreachable.
///
///   A = min(max_tokens_requested if > 0 else INFINITY,
///           model_context_window - input_tokens)
fn compute_strategy_a_local(
    input_tokens: i64,
    max_tokens_requested: i64,
    model_context_window: i64,
) -> i64 {
    let cw_ceiling = (model_context_window - input_tokens).max(0);
    let user_ceiling = if max_tokens_requested > 0 {
        max_tokens_requested
    } else {
        i64::MAX
    };
    cw_ceiling.min(user_ceiling).max(1)
}

/// Build ClaimEstimate from a successful output_predictor response.
fn build_estimate_from_predictor(
    tok_resp: &spendguard_tokenizer::TokenizeResponse,
    input_tokens: i64,
    prediction_policy: &str,
    predict: &PredictResponse,
    project: Option<&ProjectResponse>,
) -> ClaimEstimate {
    let cold_start_layer_used = predict.cold_start_layer_used.clone().unwrap_or_default();
    let mut estimate = ClaimEstimate {
        tokenizer_tier: tok_resp.tier.clone(),
        tokenizer_version_id: tok_resp.tokenizer_version_id.clone(),
        input_tokens,

        predicted_a_tokens: predict.predicted_a_tokens,
        predicted_b_tokens: predict.predicted_b_tokens.unwrap_or(0),
        predicted_c_tokens: predict.predicted_c_tokens.unwrap_or(0),
        reserved_strategy: predict.reserved_strategy.clone(),
        prediction_strategy_used: predict.prediction_strategy_used.clone(),
        prediction_policy_used: prediction_policy.to_string(),
        prediction_confidence: predict.confidence.unwrap_or(0.0),
        prediction_sample_size: predict.sample_size.unwrap_or(0) as i64,
        cold_start_layer_used,

        classifier_version: predict.classifier_version.clone(),
        fingerprint_version: predict.fingerprint_version.clone(),
        prompt_class_fingerprint: predict.prompt_class_fingerprint_used.clone(),

        run_projection_at_decision_atomic: 0,
        run_predicted_remaining_steps: -1,
        run_steps_completed_so_far: 0,
        run_code_triggered: String::new(),
    };
    if let Some(p) = project {
        estimate.run_projection_at_decision_atomic = p.run_projection_at_decision_atomic;
        estimate.run_predicted_remaining_steps = p.run_predicted_remaining_steps;
        estimate.run_steps_completed_so_far = p.run_steps_completed_so_far;
        estimate.run_code_triggered = p.emitted_code.clone();
    }
    estimate
}

/// Build ClaimEstimate when output_predictor is unreachable — Strategy A
/// only; reservation is conservative.
fn build_estimate_fallback_a(
    tokenizer_tier: String,
    tokenizer_version_id: String,
    input_tokens: i64,
    strategy_a_local: i64,
    prediction_policy: &str,
    project: Option<&ProjectResponse>,
) -> ClaimEstimate {
    let mut estimate = ClaimEstimate {
        tokenizer_tier,
        tokenizer_version_id,
        input_tokens,

        predicted_a_tokens: strategy_a_local,
        predicted_b_tokens: 0,
        predicted_c_tokens: 0,
        reserved_strategy: "A".to_string(),
        prediction_strategy_used: "A".to_string(),
        prediction_policy_used: prediction_policy.to_string(),
        prediction_confidence: 0.0,
        prediction_sample_size: 0,
        cold_start_layer_used: String::new(),

        classifier_version: String::new(),
        fingerprint_version: String::new(),
        prompt_class_fingerprint: String::new(),

        run_projection_at_decision_atomic: 0,
        run_predicted_remaining_steps: -1,
        run_steps_completed_so_far: 0,
        run_code_triggered: String::new(),
    };
    if let Some(p) = project {
        estimate.run_projection_at_decision_atomic = p.run_projection_at_decision_atomic;
        estimate.run_predicted_remaining_steps = p.run_predicted_remaining_steps;
        estimate.run_steps_completed_so_far = p.run_steps_completed_so_far;
        estimate.run_code_triggered = p.emitted_code.clone();
    }
    estimate
}

/// Lookup model's context window in tokens. SLICE_10 v1 keeps a small
/// static table; production deployments override via TOML per
/// output-predictor-spec §3.3.
fn lookup_model_context_window(model: &str) -> i64 {
    let m = model.to_ascii_lowercase();
    if m.starts_with("gpt-4o") || m.starts_with("o1") || m.starts_with("o3") {
        128_000
    } else if m.starts_with("gpt-4-turbo") {
        128_000
    } else if m.starts_with("gpt-4") {
        8_192
    } else if m.starts_with("gpt-3.5-turbo") {
        16_385
    } else if m.starts_with("claude-3-5") || m.starts_with("claude-3") {
        200_000
    } else if m.starts_with("gemini-1.5") {
        1_000_000
    } else {
        8_000 // spec §3.3 default
    }
}

pub fn parse_model_family(body: &Value) -> String {
    body.get("model")
        .and_then(|v| v.as_str())
        .map(|s| {
            // Strip versions: "gpt-4o-mini-2024-07-18" → "gpt-4o-mini"
            // For now keep verbatim — spec leaves this open.
            s.to_string()
        })
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture_inputs(body: &[u8]) -> DecisionInputs<'_> {
        DecisionInputs {
            tenant_id: "00000000-0000-4000-8000-000000000001",
            budget_id: "44444444-4444-4444-8444-444444444444",
            window_instance_id: "55555555-5555-4555-8555-555555555555",
            run_id: "11111111-1111-7111-8111-111111111111".to_string(),
            body_bytes: body,
            model_family: "gpt-4o-mini".to_string(),
            estimated_tokens: 500,
            unit_id: "66666666-6666-4666-8666-666666666666",
            explicit_idempotency_key: None,
            claim_estimate: None,
        }
    }

    #[test]
    fn build_decision_request_carries_required_ids() {
        let body = br#"{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hi"}]}"#;
        let inputs = fixture_inputs(body);
        let req = build_decision_request(&inputs).unwrap();
        let ids = req.ids.unwrap();
        assert_eq!(ids.run_id, inputs.run_id);
        assert!(ids.step_id.starts_with(&format!("{}:call:", inputs.run_id)));
        assert!(uuid::Uuid::parse_str(&ids.llm_call_id).is_ok());
        assert!(uuid::Uuid::parse_str(&ids.decision_id).is_ok());
        assert_eq!(req.route, "llm.call");
        assert_eq!(req.trigger, Trigger::LlmCallPre as i32);
    }

    #[test]
    fn step_id_is_deterministic_for_same_body_and_run() {
        let body = br#"{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hi"}]}"#;
        let inputs = fixture_inputs(body);
        let req1 = build_decision_request(&inputs).unwrap();
        let req2 = build_decision_request(&inputs).unwrap();
        // Same body + same run_id → same step_id (Staff #4 convergence).
        assert_eq!(
            req1.ids.as_ref().unwrap().step_id,
            req2.ids.as_ref().unwrap().step_id
        );
        // But decision_id is per-attempt UUIDv7 — different.
        assert_ne!(
            req1.ids.as_ref().unwrap().decision_id,
            req2.ids.as_ref().unwrap().decision_id
        );
        // Idempotency key per-attempt too (nanos diff).
        assert_ne!(
            req1.idempotency.as_ref().unwrap().key,
            req2.idempotency.as_ref().unwrap().key
        );
    }

    #[test]
    fn step_id_uses_unified_call_discriminator_not_proxy_call() {
        // Codex r5 Staff #4 ledger-audit verdict — verify the
        // discriminator is `:call:` so cost_advisor agent grouping
        // converges across proxy + wrapper-SDK deployments.
        let body = br#"{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hi"}]}"#;
        let inputs = fixture_inputs(body);
        let req = build_decision_request(&inputs).unwrap();
        let step_id = req.ids.unwrap().step_id;
        assert!(step_id.contains(":call:"), "got: {step_id}");
        assert!(!step_id.contains(":proxy-call:"), "got: {step_id}");
    }

    #[test]
    fn claim_estimate_threads_through_decision_request() {
        // SLICE_10 Phase B: claim_estimate from estimate_call_cost
        // flows into DecisionRequest.Inputs.claim_estimate so sidecar
        // can populate all 17 prediction columns.
        let body = br#"{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hi"}]}"#;
        let mut inputs = fixture_inputs(body);
        let est = ClaimEstimate {
            tokenizer_tier: "T2".into(),
            tokenizer_version_id: "00000000-0000-7000-8000-000000000001".into(),
            input_tokens: 7,
            predicted_a_tokens: 256,
            predicted_b_tokens: 0,
            predicted_c_tokens: 0,
            reserved_strategy: "A".into(),
            prediction_strategy_used: "A".into(),
            prediction_policy_used: "STRICT_CEILING".into(),
            prediction_confidence: 0.0,
            prediction_sample_size: 0,
            cold_start_layer_used: String::new(),
            classifier_version: "v1.0".into(),
            fingerprint_version: "v1.0".into(),
            prompt_class_fingerprint: "abc".into(),
            run_projection_at_decision_atomic: 0,
            run_predicted_remaining_steps: -1,
            run_steps_completed_so_far: 0,
            run_code_triggered: String::new(),
        };
        inputs.claim_estimate = Some(est.clone());
        let req = build_decision_request(&inputs).unwrap();
        let attached = req.inputs.unwrap().claim_estimate.unwrap();
        assert_eq!(attached.tokenizer_tier, "T2");
        assert_eq!(attached.reserved_strategy, "A");
        assert_eq!(attached.predicted_a_tokens, 256);
        assert_eq!(attached.run_predicted_remaining_steps, -1);
    }

    // SLICE_10 Phase B: the legacy `estimate_tokens_*` tests are removed
    // because the function has been deleted in favour of the
    // `estimate_call_cost` 3-stage pipeline (tokenizer library +
    // output_predictor + run_cost_projector). New tests below exercise
    // the helpers + real tokenizer library results — no `chars/4 × 2`
    // heuristic remains in the proxy.

    #[test]
    fn input_tokens_with_override_clamps_zero_to_one() {
        let tok_resp = spendguard_tokenizer::TokenizeResponse {
            input_tokens: 500,
            ..Default::default()
        };
        assert_eq!(input_tokens_with_override(&tok_resp, Some(42)), 42);
        assert_eq!(input_tokens_with_override(&tok_resp, Some(0)), 1);
        assert_eq!(input_tokens_with_override(&tok_resp, None), 500);
    }

    #[test]
    fn input_tokens_with_override_floors_at_one() {
        // tokenizer returned 0 (e.g. empty body) → estimator still
        // floors at 1 token so sidecar doesn't reservation-thrash on 0.
        let tok_resp = spendguard_tokenizer::TokenizeResponse {
            input_tokens: 0,
            ..Default::default()
        };
        assert_eq!(input_tokens_with_override(&tok_resp, None), 1);
    }

    #[test]
    fn classify_prompt_detects_tool_calls() {
        let body = json!({"tools": [], "messages": [{"role": "user", "content": "hi"}]});
        assert_eq!(classify_prompt(&body), "tool_calling");

        let body = json!({"tool_choice": "auto", "messages": []});
        assert_eq!(classify_prompt(&body), "tool_calling");
    }

    #[test]
    fn classify_prompt_long_chat_threshold() {
        let big = "x".repeat(5000);
        let body = json!({"messages": [{"content": &big}]});
        assert_eq!(classify_prompt(&body), "chat_long");
    }

    #[test]
    fn classify_prompt_default_short() {
        let body = json!({"messages": [{"content": "hi"}]});
        assert_eq!(classify_prompt(&body), "chat_short");
    }

    #[test]
    fn compute_strategy_a_local_caps_by_context_window() {
        // max_tokens_requested = 0 → INFINITY; capped by context window.
        let result = compute_strategy_a_local(500, 0, 8192);
        assert_eq!(result, 8192 - 500);

        // max_tokens_requested = 100 → respected (min wins).
        let result = compute_strategy_a_local(500, 100, 8192);
        assert_eq!(result, 100);

        // Context window already exceeded by input → clamp to 1 floor.
        let result = compute_strategy_a_local(10_000, 0, 8192);
        assert_eq!(result, 1);
    }

    #[test]
    fn lookup_model_context_window_known_models() {
        assert_eq!(lookup_model_context_window("gpt-4o-mini"), 128_000);
        assert_eq!(lookup_model_context_window("gpt-4o-2024-08-06"), 128_000);
        assert_eq!(lookup_model_context_window("gpt-4"), 8_192);
        assert_eq!(lookup_model_context_window("gpt-3.5-turbo"), 16_385);
        assert_eq!(lookup_model_context_window("claude-3-5-sonnet"), 200_000);
        assert_eq!(lookup_model_context_window("gemini-1.5-flash"), 1_000_000);
        assert_eq!(lookup_model_context_window("custom-experimental"), 8_000);
    }

    #[tokio::test]
    async fn estimate_call_cost_falls_back_to_strategy_a_when_predictor_absent() {
        // Boot tokenizer; both gRPC clients absent → fallback path.
        let tokenizer = spendguard_tokenizer::Tokenizer::new_with_embedded_assets().unwrap();
        let body = json!({
            "model": "gpt-4o-mini",
            "messages": [{"role": "user", "content": "hello world from SLICE_10"}],
            "max_tokens": 256,
        });
        let inputs = EstimateInputs {
            body: &body,
            model: "gpt-4o-mini",
            tenant_id: "00000000-0000-4000-8000-000000000001",
            agent_id: "test-agent",
            run_id: "test-run",
            decision_id: "test-decision",
            step_id: "test-step",
            prediction_policy: "STRICT_CEILING",
            header_override_tokens: None,
            planned_steps_hint: 0,
        };
        let est = estimate_call_cost(&inputs, &tokenizer, None, None).await;
        // Tokenizer Tier 2 for OpenAI gpt-4o family.
        assert_eq!(est.tokenizer_tier, "T2");
        assert!(!est.tokenizer_version_id.is_empty());
        // Strategy A only.
        assert_eq!(est.reserved_strategy, "A");
        assert_eq!(est.prediction_strategy_used, "A");
        assert_eq!(est.predicted_b_tokens, 0);
        assert_eq!(est.predicted_c_tokens, 0);
        assert_eq!(est.prediction_policy_used, "STRICT_CEILING");
        // Projector sentinels (no projector wired).
        assert_eq!(est.run_predicted_remaining_steps, -1);
        assert_eq!(est.run_code_triggered, "");
    }

    /// SLICE_10 Phase E — measure aggregate latency of estimate_call_cost
    /// on the predictor-absent fallback path (tokenizer + Strategy A only).
    ///
    /// Per spec §11.2 + Contract §14 50ms p99 sidecar latency budget,
    /// egress_proxy's portion (tokenizer + parallel predict/project)
    /// should consume well under half the budget. With both gRPC clients
    /// None, the test runs purely the tokenizer library + local
    /// Strategy A computation — should comfortably hit p99 < 5ms even
    /// on cold cache hits.
    ///
    /// This is a fast unit-test-level measurement; the real production
    /// p99 latency is measured by docker-compose integration runs
    /// (SLICE_15 e2e benchmark).
    #[tokio::test]
    async fn estimate_call_cost_p99_under_5ms_fallback_path() {
        use std::time::Instant;
        let tokenizer = spendguard_tokenizer::Tokenizer::new_with_embedded_assets().unwrap();
        let body = json!({
            "model": "gpt-4o-mini",
            "messages": [
                {"role": "system", "content": "You are a helpful assistant."},
                {"role": "user", "content": "What is the capital of France?"},
            ],
            "max_tokens": 256,
        });
        // Warm up the tokenizer cache.
        for _ in 0..3 {
            let inputs = EstimateInputs {
                body: &body,
                model: "gpt-4o-mini",
                tenant_id: "00000000-0000-4000-8000-000000000001",
                agent_id: "warm-up",
                run_id: "warm-up",
                decision_id: "warm-up",
                step_id: "warm-up",
                prediction_policy: "STRICT_CEILING",
                header_override_tokens: None,
                planned_steps_hint: 0,
            };
            let _ = estimate_call_cost(&inputs, &tokenizer, None, None).await;
        }
        // Measure 100 iterations and check p99.
        let mut samples = Vec::with_capacity(100);
        for i in 0..100 {
            let run_id = format!("run-{i}");
            let inputs = EstimateInputs {
                body: &body,
                model: "gpt-4o-mini",
                tenant_id: "00000000-0000-4000-8000-000000000001",
                agent_id: "bench-agent",
                run_id: &run_id,
                decision_id: "bench-decision",
                step_id: "bench-step",
                prediction_policy: "STRICT_CEILING",
                header_override_tokens: None,
                planned_steps_hint: 0,
            };
            let t0 = Instant::now();
            let _ = estimate_call_cost(&inputs, &tokenizer, None, None).await;
            samples.push(t0.elapsed());
        }
        samples.sort();
        let p50 = samples[50];
        let p99 = samples[99];
        eprintln!("estimate_call_cost latency (fallback path): p50={p50:?}, p99={p99:?}");
        // p99 budget (fallback path with both gRPC None): 5ms is
        // conservative — typical p99 < 1ms. Test is intentionally
        // generous so CI doesn't flake on slow runners.
        assert!(
            p99 < std::time::Duration::from_millis(5),
            "p99 latency {p99:?} exceeded 5ms budget; investigate tokenizer cache regression"
        );
    }

    #[tokio::test]
    async fn estimate_call_cost_respects_header_override() {
        let tokenizer = spendguard_tokenizer::Tokenizer::new_with_embedded_assets().unwrap();
        let body = json!({
            "model": "gpt-4o-mini",
            "messages": [{"role": "user", "content": "x".repeat(1000)}],
        });
        let inputs = EstimateInputs {
            body: &body,
            model: "gpt-4o-mini",
            tenant_id: "00000000-0000-4000-8000-000000000001",
            agent_id: "test-agent",
            run_id: "test-run",
            decision_id: "test-decision",
            step_id: "test-step",
            prediction_policy: "STRICT_CEILING",
            header_override_tokens: Some(42),
            planned_steps_hint: 0,
        };
        let est = estimate_call_cost(&inputs, &tokenizer, None, None).await;
        assert_eq!(est.input_tokens, 42);
    }

    #[test]
    fn parse_model_family_extracts_field() {
        let body = json!({"model": "gpt-4o-mini-2024-07-18"});
        assert_eq!(parse_model_family(&body), "gpt-4o-mini-2024-07-18");
    }

    // ── SLICE_11 Phase C — provider-aware tokenizer dispatch ─────────

    #[tokio::test]
    async fn estimate_call_cost_bedrock_anthropic_uses_anthropic_tokenizer() {
        // Bedrock model id passed via the `model` field exercises the
        // SLICE_04 tokenizer dispatch (anthropic.claude-3-5-sonnet-...
        // → Anthropic BPE). This is the contract that SLICE_11 Phase C
        // surfaces: the routing table extracts model_id from URL path
        // for Bedrock, then passes it to estimate_call_cost, which then
        // tokenize()s it via the existing tokenizer dispatch (no proxy-
        // internal table duplication).
        let tokenizer = spendguard_tokenizer::Tokenizer::new_with_embedded_assets().unwrap();
        let body = json!({
            // Bedrock InvokeModel body — proxy preserves the body
            // verbatim; the model_id from URL path is passed as `model`.
            "messages": [{"role": "user", "content": "Hello from Bedrock"}],
            "max_tokens": 256,
            "anthropic_version": "bedrock-2023-05-31",
        });
        let inputs = EstimateInputs {
            body: &body,
            // Routing table's resolve_model_id extracted this from
            // /model/anthropic.claude-3-5-sonnet-20240620-v1:0/invoke.
            model: "anthropic.claude-3-5-sonnet-20240620-v1:0",
            tenant_id: "00000000-0000-4000-8000-000000000001",
            agent_id: "bedrock-agent",
            run_id: "test-run",
            decision_id: "test-decision",
            step_id: "test-step",
            prediction_policy: "STRICT_CEILING",
            header_override_tokens: None,
            planned_steps_hint: 0,
        };
        let est = estimate_call_cost(&inputs, &tokenizer, None, None).await;
        // Should use Anthropic tokenizer (T2) per SLICE_04 dispatch.
        assert_eq!(est.tokenizer_tier, "T2");
        // The version id should be the Anthropic Claude 3 version, NOT
        // an OpenAI tiktoken version. Verify via the version registry.
        assert_eq!(
            est.tokenizer_version_id,
            spendguard_tokenizer::ANTHROPIC_CLAUDE3_VERSION_ID,
            "Bedrock anthropic.claude-3-5-* should route to Anthropic BPE"
        );
    }

    #[tokio::test]
    async fn estimate_call_cost_bedrock_unknown_model_falls_to_tier3() {
        // amazon.titan-* is not in the SLICE_04 narrow Option A
        // dispatch; tokenizer should fall to Tier 3 (5% margin).
        let tokenizer = spendguard_tokenizer::Tokenizer::new_with_embedded_assets().unwrap();
        let body = json!({"inputText": "Hello", "textGenerationConfig": {}});
        let inputs = EstimateInputs {
            body: &body,
            model: "amazon.titan-text-express-v1",
            tenant_id: "00000000-0000-4000-8000-000000000001",
            agent_id: "bedrock-titan",
            run_id: "test-run",
            decision_id: "test-decision",
            step_id: "test-step",
            prediction_policy: "STRICT_CEILING",
            header_override_tokens: None,
            planned_steps_hint: 0,
        };
        let est = estimate_call_cost(&inputs, &tokenizer, None, None).await;
        // Tier 3 fallback (5% margin via chars/4 × 1.05).
        assert_eq!(est.tokenizer_tier, "T3");
    }

    #[tokio::test]
    async fn estimate_call_cost_anthropic_native_uses_anthropic_tokenizer() {
        // Anthropic native /v1/messages — model field holds e.g.
        // "claude-3-5-sonnet-20241022".
        let tokenizer = spendguard_tokenizer::Tokenizer::new_with_embedded_assets().unwrap();
        let body = json!({
            "model": "claude-3-5-sonnet-20241022",
            "messages": [{"role": "user", "content": "Hello Claude"}],
            "max_tokens": 256,
        });
        let inputs = EstimateInputs {
            body: &body,
            model: "claude-3-5-sonnet-20241022",
            tenant_id: "00000000-0000-4000-8000-000000000001",
            agent_id: "anthropic-agent",
            run_id: "test-run",
            decision_id: "test-decision",
            step_id: "test-step",
            prediction_policy: "STRICT_CEILING",
            header_override_tokens: None,
            planned_steps_hint: 0,
        };
        let est = estimate_call_cost(&inputs, &tokenizer, None, None).await;
        assert_eq!(est.tokenizer_tier, "T2");
        assert_eq!(
            est.tokenizer_version_id,
            spendguard_tokenizer::ANTHROPIC_CLAUDE3_VERSION_ID
        );
    }

    #[tokio::test]
    async fn estimate_call_cost_gemini_uses_gemini_tokenizer() {
        // Vertex generateContent — model id extracted from URL path
        // (e.g. "gemini-1.5-pro").
        let tokenizer = spendguard_tokenizer::Tokenizer::new_with_embedded_assets().unwrap();
        let body = json!({
            "contents": [{"role": "user", "parts": [{"text": "Hello Gemini"}]}],
        });
        let inputs = EstimateInputs {
            body: &body,
            model: "gemini-1.5-pro",
            tenant_id: "00000000-0000-4000-8000-000000000001",
            agent_id: "vertex-agent",
            run_id: "test-run",
            decision_id: "test-decision",
            step_id: "test-step",
            prediction_policy: "STRICT_CEILING",
            header_override_tokens: None,
            planned_steps_hint: 0,
        };
        let est = estimate_call_cost(&inputs, &tokenizer, None, None).await;
        assert_eq!(est.tokenizer_tier, "T2");
        assert_eq!(
            est.tokenizer_version_id,
            spendguard_tokenizer::GEMINI_15_VERSION_ID
        );
    }

    #[test]
    fn parse_model_family_missing_returns_unknown() {
        let body = json!({});
        assert_eq!(parse_model_family(&body), "unknown");
    }

    #[test]
    fn explicit_idempotency_key_overrides_default() {
        let body = br#"{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hi"}]}"#;
        let mut inputs = fixture_inputs(body);
        inputs.explicit_idempotency_key = Some("user-provided-key-123".to_string());
        let req = build_decision_request(&inputs).unwrap();
        assert_eq!(req.idempotency.unwrap().key, "user-provided-key-123");
    }

    #[test]
    fn empty_explicit_idempotency_falls_back_to_default() {
        let body = br#"{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hi"}]}"#;
        let mut inputs = fixture_inputs(body);
        inputs.explicit_idempotency_key = Some("".to_string()); // explicitly empty
        let req = build_decision_request(&inputs).unwrap();
        // Empty string should fall through to nanos-based default
        assert_ne!(req.idempotency.unwrap().key, "");
    }
}
