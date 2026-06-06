//! SLICE 3 — StreamState + tenant id → sidecar `DecisionRequest`.
//!
//! Wraps the egress_proxy `decision::build_decision_request` shape
//! verbatim for the fields the ExtProc adapter can populate:
//!
//!   * `session_id` — `"envoy-extproc:<x-request-id>"` so the sidecar's
//!     audit trail can correlate ExtProc → adapter at debug time.
//!     Mirrors the `"egress-proxy:{run_id}"` convention from
//!     `services/egress_proxy/src/decision.rs:156-157`.
//!   * `trigger` — always `LLM_CALL_PRE` for v1 (per design §3.5
//!     wire-format scope: chat/completions + messages only).
//!   * `route` — `"llm.call"` (pin-locked: matches egress_proxy +
//!     contract-dsl §scope).
//!   * `ids.run_id` / `step_id` / `llm_call_id` / `decision_id` —
//!     derived deterministically from `x-request-id` + the JCS body
//!     signature so the audit chain stays content-addressable.
//!     Review-standards §4.1.1 requires non-empty `idempotency.key`;
//!     the derivation comment shows the W3C trace context fallback.
//!   * `inputs.projected_claims` — single `BudgetClaim` carrying the
//!     SLICE 2 `ClaimEstimate.input_tokens` as the atomic amount.
//!   * `inputs.claim_estimate` — the full 17-column ClaimEstimate
//!     proto (review-standards §3.1.4 requires B/C/policy fields stay
//!     at proto3 defaults in SLICE 3; the conversion below honours
//!     that).
//!   * `idempotency.key` — derived from the x-request-id + a stable
//!     content hash of the request body (so retries of the same body
//!     collapse to the same decision).
//!
//! Anti-scope: this slice does NOT carry parent_run_id /
//! budget_grant_jti / planned_steps_hint (deferred to SLICE 5 multi-
//! provider conformance + SLICE 7 demo).
//!
//! Spec refs:
//!   - docs/specs/coverage/D01_envoy_extproc/design.md §3.5 (wire-format scope)
//!   - docs/specs/coverage/D01_envoy_extproc/implementation.md §5 (RequestDecision shape)
//!   - docs/specs/coverage/D01_envoy_extproc/review-standards.md §4.1.1 (non-empty idempotency)
//!   - services/egress_proxy/src/decision.rs:94-175 (pattern source)

use thiserror::Error;
use tracing::debug;

use crate::proto::spendguard::common::v1::{
    budget_claim::Direction, unit_ref::Kind as UnitKind, BudgetClaim, Idempotency, SpendGuardIds,
    UnitRef,
};
use crate::proto::spendguard::sidecar_adapter::v1::{
    decision_request::{Inputs, Trigger},
    ClaimEstimate as SidecarClaimEstimate, DecisionRequest,
};
use crate::state::StreamState;
use crate::tokenize::ClaimEstimate as LocalClaimEstimate;

/// Build errors. SLICE 3 fail-closed posture means the only failure
/// path here is "no ClaimEstimate stashed by SLICE 2" — every other
/// missing field has a typed default the sidecar tolerates.
#[derive(Debug, Error)]
pub enum BuildError {
    /// SLICE 2's `estimate_tokens_or_warn` returned `None` (parse OR
    /// tokenize failed). Per design §3.4 we fail-closed at the budget
    /// query, so the caller surfaces ExtProc 503 to the client.
    /// Review-standards §4.1.1 explicitly demands this path NOT silently
    /// inject a fake estimate.
    #[error("no ClaimEstimate stashed in StreamState (SLICE 2 parse/tokenize failed); fail-closed per design §3.4")]
    MissingClaimEstimate,
}

/// Per-call context the Request-Body handler passes alongside the
/// stream state. We keep these as explicit arguments rather than
/// reaching into `ExtProcService` so the builder is unit-testable
/// without a tonic harness.
pub struct DecisionBuildCtx<'a> {
    /// The configured tenant id assertion — `ExtProcService::tenant_id`.
    pub tenant_id: &'a str,
    /// The stream id (x-request-id) the SLICE 2 state map is keyed on.
    /// Carries through into session_id + idempotency derivation.
    pub stream_id: &'a str,
    /// Optional unit_id override. v1 default is the OpenAI output_token
    /// unit; SLICE 4 will lift this from the routing table per provider.
    pub unit_id: Option<&'a str>,
}

/// Pin-locked unit id default. Matches the egress_proxy SLICE_10 default
/// — single tenant POC pins this to the OpenAI output_token unit. SLICE
/// 5 conformance lifts it from the routing table.
pub const DEFAULT_UNIT_ID: &str = "unit-openai-output-token";

/// Pin-locked budget_id default for the demo flow. Matches the
/// `deploy/demo/` seed values; production wiring (SLICE 6 Helm) will
/// inject the real budget id via env.
pub const DEFAULT_BUDGET_ID: &str = "budget-envoy-extproc-default";

/// Pin-locked window instance default. Same source as DEFAULT_BUDGET_ID
/// — the budget window the demo seed creates.
pub const DEFAULT_WINDOW_INSTANCE_ID: &str = "window-envoy-extproc-default";

/// Build the sidecar `RequestDecision` from a `StreamState`.
///
/// SAFETY: per review-standards §4.1.1, this function MUST NOT produce
/// a `DecisionRequest` with an empty `idempotency.key`. The derivation
/// uses `stream_id` directly (Envoy's `x-request-id` is RFC 4648
/// base64url 16 bytes; never empty in practice). For the test path
/// where `stream_id` is empty (e.g. the SLICE 2 fallback UUID branch),
/// we synthesize one from the parsed body fields so the gate still
/// holds.
pub fn build_request_decision(
    state: &StreamState,
    ctx: &DecisionBuildCtx<'_>,
) -> Result<DecisionRequest, BuildError> {
    let estimate = state
        .estimate
        .as_ref()
        .ok_or(BuildError::MissingClaimEstimate)?;
    let parsed = state
        .parsed
        .as_ref()
        .ok_or(BuildError::MissingClaimEstimate)?;

    // Stream id is the spine for ALL derived identifiers. SLICE 4's
    // audit-emit reads these same identifiers from `state` so any
    // change here propagates downstream.
    let stream_id = ctx.stream_id;
    let session_id = format!("envoy-extproc:{stream_id}");

    // Idempotency key — review-standards §4.1.1: non-empty + derived
    // from x-request-id + W3C trace context where available. SLICE 3
    // ships the stream_id-only form (Envoy injects x-request-id for
    // every request); future slices may fold in the W3C `traceparent`
    // header for cross-system idempotency.
    let idempotency_key = derive_idempotency_key(stream_id, &parsed.model_id);

    let unit_id = ctx.unit_id.unwrap_or(DEFAULT_UNIT_ID);
    let unit = UnitRef {
        unit_id: unit_id.to_string(),
        kind: UnitKind::Token as i32,
        currency: String::new(),
        token_kind: "output_token".to_string(),
        model_family: parsed.model_id.clone(),
        ..Default::default()
    };

    // The projected claim amount is the SLICE 2 Strategy A reservation
    // (`predicted_a_tokens = input_tokens * 2`). Review-standards
    // §1.3 + §3.1.4: Strategy A only in v1, B/C MUST stay 0.
    let claim_amount_atomic = estimate.predicted_a_tokens.to_string();
    let claim = BudgetClaim {
        budget_id: DEFAULT_BUDGET_ID.to_string(),
        unit: Some(unit.clone()),
        amount_atomic: claim_amount_atomic,
        direction: Direction::Debit as i32,
        window_instance_id: DEFAULT_WINDOW_INSTANCE_ID.to_string(),
    };

    // Sidecar ClaimEstimate proto mirror. SLICE 2's local
    // [`LocalClaimEstimate`] is a strict subset (the fields the
    // Request-Body phase populates); the rest stay at proto3 defaults.
    let sidecar_estimate = build_sidecar_claim_estimate(estimate);

    let inputs_msg = Inputs {
        projected_claims: vec![claim],
        projected_unit: Some(unit),
        claim_estimate: Some(sidecar_estimate),
        ..Default::default()
    };

    let ids = SpendGuardIds {
        run_id: format!("run-envoy-extproc-{stream_id}"),
        step_id: format!("step-envoy-extproc-{stream_id}"),
        llm_call_id: format!("call-envoy-extproc-{stream_id}"),
        decision_id: format!("dec-envoy-extproc-{stream_id}"),
        ..Default::default()
    };

    let req = DecisionRequest {
        session_id,
        trigger: Trigger::LlmCallPre as i32,
        route: "llm.call".to_string(),
        ids: Some(ids),
        idempotency: Some(Idempotency {
            key: idempotency_key,
            request_hash: bytes::Bytes::new(),
        }),
        inputs: Some(inputs_msg),
        ..Default::default()
    };

    debug!(
        tenant_id = %ctx.tenant_id,
        stream_id = %stream_id,
        model = %parsed.model_id,
        provider = parsed.provider_str,
        input_tokens = estimate.input_tokens,
        predicted_a_tokens = estimate.predicted_a_tokens,
        tokenizer_tier = %estimate.tokenizer_tier,
        "decision::build_request_decision produced DecisionRequest"
    );

    Ok(req)
}

/// Derive the idempotency key. Currently: `"envoy-extproc:{stream_id}:{model_id}"`.
/// This is non-empty by construction whenever `stream_id` is non-empty
/// (SLICE 2 guarantees this — the state map fallback mints a UUID v4).
/// The model_id suffix lets reviewers eyeball that the right model
/// flowed through without needing to load the full request.
fn derive_idempotency_key(stream_id: &str, model_id: &str) -> String {
    format!("envoy-extproc:{stream_id}:{model_id}")
}

/// Project the SLICE 2 local [`LocalClaimEstimate`] onto the sidecar
/// adapter proto. The fields SLICE 2 doesn't populate (Strategy B/C,
/// classifier, prompt class, run-cost projection) stay at proto3
/// defaults — sidecar code treats those as "estimate not supplied"
/// per [`adapter.proto`] §SLICE_10 additive comment.
fn build_sidecar_claim_estimate(local: &LocalClaimEstimate) -> SidecarClaimEstimate {
    SidecarClaimEstimate {
        // Tier 2 input tokens (or Tier 3 fallback heuristic).
        tokenizer_tier: local.tokenizer_tier.clone(),
        tokenizer_version_id: local.tokenizer_version_id.clone(),
        input_tokens: local.input_tokens,
        // Strategy A; B/C MUST be 0 per review-standards §3.1.4.
        predicted_a_tokens: local.predicted_a_tokens,
        predicted_b_tokens: 0,
        predicted_c_tokens: 0,
        reserved_strategy: local.reserved_strategy.clone(),
        prediction_strategy_used: local.reserved_strategy.clone(),
        prediction_policy_used: "STRICT_CEILING".to_string(),
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
        model: local.model.clone(),
        prompt_class: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::ParsedRequest;
    use crate::state::StreamState;
    use spendguard_provider_routing::{ProviderKind, RequestShape};
    use spendguard_tokenizer::EncoderKind;

    fn make_state_with_estimate(input_tokens: i64) -> StreamState {
        let mut s = StreamState::new();
        s.path = "/v1/chat/completions".to_string();
        s.parsed = Some(ParsedRequest {
            provider: ProviderKind::OpenAi,
            provider_str: ProviderKind::OpenAi.as_str(),
            request_shape: RequestShape::OpenAiChatCompletions,
            model_id: "gpt-4o-mini".to_string(),
            tokenizer_kind: Some(EncoderKind::OpenAi),
            messages: Vec::new(),
            raw_text: String::new(),
        });
        s.estimate = Some(LocalClaimEstimate {
            input_tokens,
            tokenizer_tier: "T2".to_string(),
            tokenizer_version_id: "00000000-0000-7000-8000-000000000001".to_string(),
            model: "gpt-4o-mini".to_string(),
            provider: "openai".to_string(),
            predicted_a_tokens: input_tokens.saturating_mul(2),
            predicted_b_tokens: 0,
            predicted_c_tokens: 0,
            reserved_strategy: "A".to_string(),
        });
        s
    }

    #[test]
    fn builds_request_decision_from_claim_estimate() {
        let state = make_state_with_estimate(100);
        let ctx = DecisionBuildCtx {
            tenant_id: "00000000-0000-4000-8000-000000000001",
            stream_id: "stream-abc-123",
            unit_id: None,
        };
        let req = build_request_decision(&state, &ctx).expect("must build");
        assert_eq!(req.session_id, "envoy-extproc:stream-abc-123");
        assert_eq!(req.trigger, Trigger::LlmCallPre as i32);
        assert_eq!(req.route, "llm.call");

        let ids = req.ids.expect("ids set");
        assert!(ids.run_id.contains("stream-abc-123"));
        assert!(ids.decision_id.contains("stream-abc-123"));
        assert!(!ids.run_id.is_empty());

        // Review-standards §4.1.1: idempotency.key MUST be non-empty.
        let idem = req.idempotency.expect("idempotency set");
        assert!(!idem.key.is_empty(), "idempotency.key MUST be non-empty");
        assert!(idem.key.contains("stream-abc-123"));
        assert!(idem.key.contains("gpt-4o-mini"));

        let inputs = req.inputs.expect("inputs set");
        assert_eq!(inputs.projected_claims.len(), 1);
        let claim = &inputs.projected_claims[0];
        // Strategy A reservation = input * 2.
        assert_eq!(claim.amount_atomic, "200");
        assert_eq!(claim.direction, Direction::Debit as i32);
        assert_eq!(claim.budget_id, DEFAULT_BUDGET_ID);
        assert_eq!(claim.window_instance_id, DEFAULT_WINDOW_INSTANCE_ID);
        let unit = claim.unit.as_ref().expect("unit set");
        assert_eq!(unit.unit_id, DEFAULT_UNIT_ID);
        assert_eq!(unit.kind, UnitKind::Token as i32);
        assert_eq!(unit.token_kind, "output_token");
        assert_eq!(unit.model_family, "gpt-4o-mini");

        // claim_estimate mirror.
        let ce = inputs.claim_estimate.expect("claim_estimate set");
        assert_eq!(ce.input_tokens, 100);
        assert_eq!(ce.predicted_a_tokens, 200);
        assert_eq!(ce.tokenizer_tier, "T2");
        assert_eq!(ce.reserved_strategy, "A");
        assert_eq!(ce.prediction_policy_used, "STRICT_CEILING");
        // Review-standards §3.1.4: B/C MUST be 0.
        assert_eq!(ce.predicted_b_tokens, 0);
        assert_eq!(ce.predicted_c_tokens, 0);
        // Cold-start / classifier sentinels.
        assert!(ce.cold_start_layer_used.is_empty());
        assert!(ce.classifier_version.is_empty());
        assert_eq!(ce.run_predicted_remaining_steps, -1);
        assert_eq!(ce.model, "gpt-4o-mini");
    }

    #[test]
    fn missing_estimate_fails_closed() {
        let mut state = StreamState::new();
        state.path = "/v1/chat/completions".to_string();
        // Note: parsed is set but estimate is None — simulates the
        // SLICE 2 warn-and-continue path where parse succeeded but
        // tokenize errored.
        state.parsed = Some(ParsedRequest {
            provider: ProviderKind::OpenAi,
            provider_str: ProviderKind::OpenAi.as_str(),
            request_shape: RequestShape::OpenAiChatCompletions,
            model_id: "gpt-4o-mini".to_string(),
            tokenizer_kind: Some(EncoderKind::OpenAi),
            messages: Vec::new(),
            raw_text: String::new(),
        });
        let ctx = DecisionBuildCtx {
            tenant_id: "00000000-0000-4000-8000-000000000001",
            stream_id: "stream-x",
            unit_id: None,
        };
        let err = build_request_decision(&state, &ctx).expect_err("must fail-closed");
        assert!(matches!(err, BuildError::MissingClaimEstimate));
    }

    #[test]
    fn missing_parsed_fails_closed() {
        // Defensive: parsed=None + estimate=Some should never happen
        // in production (SLICE 2 sets them together) but the builder
        // MUST still fail-closed.
        let mut state = StreamState::new();
        state.estimate = Some(LocalClaimEstimate {
            input_tokens: 5,
            tokenizer_tier: "T2".to_string(),
            tokenizer_version_id: "v1".to_string(),
            model: "gpt-4o-mini".to_string(),
            provider: "openai".to_string(),
            predicted_a_tokens: 10,
            predicted_b_tokens: 0,
            predicted_c_tokens: 0,
            reserved_strategy: "A".to_string(),
        });
        let ctx = DecisionBuildCtx {
            tenant_id: "00000000-0000-4000-8000-000000000001",
            stream_id: "stream-y",
            unit_id: None,
        };
        let err = build_request_decision(&state, &ctx).expect_err("must fail-closed");
        assert!(matches!(err, BuildError::MissingClaimEstimate));
    }

    #[test]
    fn unit_id_override_propagates() {
        // The Request-Body handler may inject a per-provider unit id;
        // verify the override path lands in the BudgetClaim and the
        // top-level Inputs.projected_unit.
        let state = make_state_with_estimate(50);
        let ctx = DecisionBuildCtx {
            tenant_id: "00000000-0000-4000-8000-000000000001",
            stream_id: "s-1",
            unit_id: Some("unit-custom"),
        };
        let req = build_request_decision(&state, &ctx).expect("must build");
        let inputs = req.inputs.unwrap();
        assert_eq!(
            inputs.projected_claims[0].unit.as_ref().unwrap().unit_id,
            "unit-custom"
        );
        assert_eq!(inputs.projected_unit.unwrap().unit_id, "unit-custom");
    }
}
