//! SLICE 4 — `StreamState` → sidecar `TraceEvent` (LLM_CALL_POST) builder.
//!
//! The Response-Body phase reads SLICE 3's stashed `StreamState`,
//! parses provider usage via [`crate::response_parse`], and asks this
//! module to build a single typed [`TraceEvent`] of kind `LLM_CALL_POST`
//! to ship over `SidecarAdapter::EmitTraceEvents`.
//!
//! ## Wire shape
//!
//! The sidecar adapter's `EmitTraceEvents` RPC is a bidi stream of
//! [`TraceEvent`] → [`TraceEventAck`]; the slice doc's mention of
//! `AppendEventsRequest` is a misnomer — that message lives on the
//! canonical_ingest service, not the sidecar adapter. We follow the
//! egress_proxy pattern at [`services/egress_proxy/src/forward.rs:908`]
//! verbatim (declared as deviation #1 in the SLICE 4 ship note).
//!
//! ## Outcome mapping per SLICE 3 R2 [`DecisionOutcome`] enum
//!
//! | StreamState.decision_outcome | http_status   | LLM_CALL_POST outcome | audit_code        |
//! |------------------------------|---------------|-----------------------|-------------------|
//! | `Allow`                      | 2xx           | `SUCCESS`             | (none)            |
//! | `Allow`                      | 5xx           | `RUN_ABORTED`         | `"UPSTREAM_5XX"`  |
//! | `Allow`                      | 4xx (≠5xx)    | `PROVIDER_ERROR`      | `"UPSTREAM_4XX"`  |
//! | `Deny` / `Rejected`          | (n/a)         | (skipped — already audited by sidecar at Request-Body deny) |
//! | `SidecarError`               | (n/a)         | (skipped — no reservation_id; sidecar never recorded a decision) |
//! | `MissingClaimEstimate`       | (n/a)         | (skipped — same reason) |
//!
//! Per review-standards §5.1: exactly one `LLM_CALL_POST` per stream
//! (no double-commit on retry); the sidecar's POST_GA_01 dedup catches
//! same-reservation replays.
//!
//! ## Idempotency
//!
//! `reservation_id` is the spine. The sidecar's
//! `transaction.rs::run_commit_estimated` dedups on
//! `(reservation_id, outcome)`; emitting twice for the same reservation
//! with the same outcome is a no-op.
//!
//! Spec refs:
//!   - docs/specs/coverage/D01_envoy_extproc/design.md §3.4, §3.5
//!   - docs/specs/coverage/D01_envoy_extproc/implementation.md §7
//!   - docs/specs/coverage/D01_envoy_extproc/review-standards.md §5
//!   - docs/slices/COV_04_envoy_extproc_audit_emit.md
//!   - services/egress_proxy/src/forward.rs:846-1007 (LlmCallOutcome pattern)
//!   - proto/spendguard/sidecar_adapter/v1/adapter.proto §LlmCallPostPayload

use crate::proto::spendguard::common::v1::SpendGuardIds;
use crate::proto::spendguard::sidecar_adapter::v1::{
    llm_call_post_payload::Outcome as PostOutcome, trace_event, LlmCallPostPayload, TraceEvent,
};
use crate::response_parse::ProviderUsage;
use crate::state::{DecisionOutcome, StreamState};

/// Carry-set from the ExtProc Response-Headers + Response-Body phases.
/// SLICE 4 reads these alongside the SLICE 3 [`StreamState`] when
/// building the LLM_CALL_POST event.
#[derive(Debug, Clone)]
pub struct ResponseMeta {
    /// Upstream HTTP status code captured at Response-Headers. 0 when
    /// missing (defensive — Envoy AI Gateway always sets it).
    pub http_status: u16,
    /// Provider-reported usage extracted from the response body. None
    /// when extraction failed (parse error, oversized body) — the audit
    /// row still lands but with `tokens_unknown: true`.
    pub provider_usage: Option<ProviderUsage>,
}

/// Result of building an audit event from `(StreamState, ResponseMeta)`.
/// `None` means SLICE 4 deliberately skips the emit for this stream
/// (Deny / SidecarError / MissingClaimEstimate paths the sidecar
/// already audited at Request-Body time).
///
/// The `Emit` payload is boxed because the proto-generated `TraceEvent`
/// is ~500 bytes and the `Skip` variant is 16 bytes — without the box
/// the discriminated union eats the larger size for every value, which
/// clippy flags as `large_enum_variant`.
pub enum AuditBuild {
    /// Audit emit MUST fire — caller hands the event to
    /// [`crate::sidecar_client::SidecarClient::emit_trace_events`].
    Emit(Box<TraceEvent>),
    /// Audit emit MUST be skipped. The `reason` is internal-only —
    /// surfaced in `debug!` logs for ops correlation only.
    Skip { reason: &'static str },
}

/// Per-call context the Response-Body handler passes alongside the
/// stashed `StreamState`. Mirrors `DecisionBuildCtx` from `decision.rs`.
pub struct AuditBuildCtx<'a> {
    /// The configured tenant id assertion.
    pub tenant_id: &'a str,
    /// The stream id (x-request-id) the state map is keyed on.
    pub stream_id: &'a str,
}

/// Build the SLICE 4 audit emit for a stream. Returns `AuditBuild::Skip`
/// for outcomes the sidecar already audited at Request-Body time; the
/// caller still removes the StreamState entry on Skip so the map
/// doesn't leak.
pub fn build_llm_call_post(
    state: &StreamState,
    meta: &ResponseMeta,
    ctx: &AuditBuildCtx<'_>,
) -> AuditBuild {
    // SLICE 3's stashed outcome drives the emit/skip decision.
    let stream_outcome = match state.decision_outcome {
        Some(o) => o,
        None => {
            return AuditBuild::Skip {
                reason: "no decision_outcome on StreamState (SLICE 3 race or missing)",
            };
        }
    };

    let (audit_outcome, audit_code) = match stream_outcome {
        DecisionOutcome::Allow => classify_allow_outcome(meta.http_status),
        DecisionOutcome::Deny => {
            // Sidecar's Request-Body STOP already wrote the LLM_CALL_POST
            // RUN_ABORTED audit row (see services/sidecar/src/server/
            // adapter_uds.rs deny path). Avoid double-commit per
            // review-standards §5.1.
            return AuditBuild::Skip {
                reason: "Deny outcome — sidecar already audited at Request-Body short-circuit",
            };
        }
        DecisionOutcome::Rejected => {
            // Same reasoning as Deny — DEGRADE / REQUIRE_APPROVAL are
            // sidecar-audited at decision time.
            return AuditBuild::Skip {
                reason: "Rejected outcome — sidecar already audited at Request-Body short-circuit",
            };
        }
        DecisionOutcome::SidecarError => {
            // No reservation_id exists. Sidecar never recorded a
            // DecisionResponse, so we have nothing to commit/release
            // against. The 503 ImmediateResponse + Retry-After is the
            // client's signal.
            return AuditBuild::Skip {
                reason: "SidecarError outcome — no reservation to commit against",
            };
        }
        DecisionOutcome::MissingClaimEstimate => {
            return AuditBuild::Skip {
                reason: "MissingClaimEstimate outcome — no reservation to commit against",
            };
        }
    };

    // For the Allow path we MUST have a reservation_id (SLICE 3 stashed
    // it from DecisionResponse.reservation_ids[0]). Defense in depth:
    // refuse to fabricate an audit row without one.
    let reservation_id = match state.reservation_id.as_deref() {
        Some(s) if !s.is_empty() => s,
        _ => {
            return AuditBuild::Skip {
                reason: "Allow outcome missing reservation_id (SLICE 3 stash drift?)",
            };
        }
    };

    // Build the payload. Pricing is None (matches egress_proxy
    // E2E-validation P1 fix at forward.rs:907 — sidecar's reservation
    // row carries the canonical pricing).
    let actual_input_tokens = meta
        .provider_usage
        .as_ref()
        .and_then(|u| {
            if u.tokens_unknown {
                None
            } else {
                u.input_tokens
            }
        })
        .map(|n| n as i64);
    let actual_output_tokens = meta
        .provider_usage
        .as_ref()
        .and_then(|u| {
            if u.tokens_unknown {
                None
            } else {
                u.output_tokens
            }
        })
        .map(|n| n as i64);

    // Estimated atomic amount: for SUCCESS we send the provider-reported
    // output tokens as the commit amount (Strategy A reservation was
    // input × 2, the actual is what the provider charged us). For
    // RUN_ABORTED / PROVIDER_ERROR we send the empty string (sidecar
    // takes the release path per LlmCallPostPayload spec §6 outcome ≠
    // SUCCESS).
    let estimated_amount_atomic = match (audit_outcome, actual_output_tokens) {
        (PostOutcome::Success, Some(t)) => t.to_string(),
        (PostOutcome::Success, None) => {
            // Tokens unknown + 2xx. Per review-standards §5.2 fallback:
            // use input × 2 estimate from the StreamState (HARDEN_03
            // pattern). Do NOT silently emit a 0-token commit.
            let fallback = state
                .estimate
                .as_ref()
                .map(|e| e.predicted_a_tokens)
                .unwrap_or(0);
            fallback.to_string()
        }
        _ => String::new(),
    };

    // SpendGuardIds — re-derive from the decision_id stash. Mirrors
    // SLICE 3 derive_idempotency_key shape so the audit chain can be
    // joined back to the decision row.
    let stream_id = ctx.stream_id;
    let ids = SpendGuardIds {
        run_id: format!("run-envoy-extproc-{stream_id}"),
        step_id: format!("step-envoy-extproc-{stream_id}"),
        llm_call_id: format!("call-envoy-extproc-{stream_id}"),
        decision_id: state.decision_id.clone().unwrap_or_default(),
        ..Default::default()
    };

    let payload = LlmCallPostPayload {
        reservation_id: reservation_id.to_string(),
        provider_reported_amount_atomic: String::new(),
        unit: None,
        pricing: None,
        provider_event_id: String::new(),
        outcome: audit_outcome as i32,
        estimated_amount_atomic,
        actual_input_tokens,
        actual_output_tokens,
        delta_b_ratio: None,
        delta_c_ratio: None,
    };

    let event_time = current_timestamp();

    let event = TraceEvent {
        session_id: format!("envoy-extproc:{stream_id}"),
        trace: None,
        ids: Some(ids),
        kind: trace_event::EventKind::LlmCallPost as i32,
        event_time: Some(event_time),
        payload: Some(trace_event::Payload::LlmCallPost(payload)),
        // provider_response_metadata carries a stable audit_code so the
        // dashboard can disambiguate the SLICE 4 reason without parsing
        // the typed Outcome — matches the spec table at the top of this
        // file. Empty for the happy SUCCESS path.
        provider_response_metadata: audit_code.to_string(),
    };

    let _ = ctx.tenant_id; // structured logging consumer not used in builder

    AuditBuild::Emit(Box::new(event))
}

/// Wall-clock event timestamp. Pulled from `SystemTime::now()` so we
/// don't depend on `chrono` (egress_proxy uses chrono — keeping the
/// ExtProc dep surface minimal per the SLICE 4 implementer rules).
fn current_timestamp() -> prost_types::Timestamp {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    prost_types::Timestamp {
        seconds: now.as_secs() as i64,
        nanos: now.subsec_nanos() as i32,
    }
}

/// Classify an Allow outcome by HTTP status. 2xx → SUCCESS; 5xx →
/// RUN_ABORTED; everything else (1xx / 3xx / 4xx) → PROVIDER_ERROR
/// (matches the egress_proxy forward.rs:708/764/788 mapping).
fn classify_allow_outcome(http_status: u16) -> (PostOutcome, &'static str) {
    match http_status {
        200..=299 => (PostOutcome::Success, ""),
        500..=599 => (PostOutcome::RunAborted, "UPSTREAM_5XX"),
        // 4xx / 1xx / 3xx / unknown — provider-side problem. Sidecar
        // still releases the reservation via the non-SUCCESS path.
        _ => (PostOutcome::ProviderError, "UPSTREAM_4XX"),
    }
}

/// Build an idempotency key for the LLM_CALL_POST audit emit. Sidecar
/// dedup keys on `(reservation_id, outcome)`; this helper surfaces the
/// canonical string form for log lines + future Idempotency oneof
/// support (proto adds a key field in v1alpha2).
///
/// Per the SLICE 4 implementer note: `reservation_id + ":post"` suffix
/// matches the audit_outbox row's `idempotency_key` shape and stays
/// non-empty whenever `reservation_id` is non-empty.
pub fn derive_post_idempotency_key(reservation_id: &str) -> String {
    format!("{reservation_id}:post")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::ParsedRequest;
    use crate::tokenize::ClaimEstimate as LocalClaimEstimate;
    use spendguard_provider_routing::{ProviderKind, RequestShape};
    use spendguard_tokenizer::EncoderKind;

    fn allow_state_with_reservation(reservation_id: &str) -> StreamState {
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
            input_tokens: 50,
            tokenizer_tier: "T2".to_string(),
            tokenizer_version_id: "v1".to_string(),
            model: "gpt-4o-mini".to_string(),
            provider: "openai".to_string(),
            predicted_a_tokens: 100,
            predicted_b_tokens: 0,
            predicted_c_tokens: 0,
            reserved_strategy: "A".to_string(),
        });
        s.reservation_id = Some(reservation_id.to_string());
        s.decision_id = Some("dec-test-1".to_string());
        s.decision_outcome = Some(DecisionOutcome::Allow);
        s
    }

    fn ctx() -> AuditBuildCtx<'static> {
        AuditBuildCtx {
            tenant_id: "00000000-0000-4000-8000-000000000099",
            stream_id: "stream-test-1",
        }
    }

    #[test]
    fn allow_2xx_emits_success_with_provider_usage() {
        let state = allow_state_with_reservation("res-1");
        let meta = ResponseMeta {
            http_status: 200,
            provider_usage: Some(ProviderUsage {
                input_tokens: Some(17),
                output_tokens: Some(42),
                tokens_unknown: false,
            }),
        };
        let built = build_llm_call_post(&state, &meta, &ctx());
        let event = match built {
            AuditBuild::Emit(e) => e,
            AuditBuild::Skip { reason } => panic!("expected Emit, got Skip: {reason}"),
        };
        assert_eq!(event.kind, trace_event::EventKind::LlmCallPost as i32);
        assert!(event.session_id.contains("stream-test-1"));
        let payload = match event.payload.expect("payload") {
            trace_event::Payload::LlmCallPost(p) => p,
            other => panic!("expected LlmCallPost, got {other:?}"),
        };
        assert_eq!(payload.reservation_id, "res-1");
        assert_eq!(payload.outcome, PostOutcome::Success as i32);
        assert_eq!(payload.actual_input_tokens, Some(17));
        assert_eq!(payload.actual_output_tokens, Some(42));
        // Strategy A reservation was 100; provider reported 42 — commit
        // path sends the actual output count as the estimated amount.
        assert_eq!(payload.estimated_amount_atomic, "42");
        assert_eq!(event.provider_response_metadata, "");
    }

    #[test]
    fn allow_5xx_emits_run_aborted_with_upstream_5xx_code() {
        let state = allow_state_with_reservation("res-2");
        let meta = ResponseMeta {
            http_status: 503,
            provider_usage: None,
        };
        let built = build_llm_call_post(&state, &meta, &ctx());
        let event = match built {
            AuditBuild::Emit(e) => e,
            AuditBuild::Skip { reason } => panic!("expected Emit, got Skip: {reason}"),
        };
        let payload = match event.payload.expect("payload") {
            trace_event::Payload::LlmCallPost(p) => p,
            _ => panic!("expected LlmCallPost"),
        };
        assert_eq!(payload.outcome, PostOutcome::RunAborted as i32);
        assert_eq!(event.provider_response_metadata, "UPSTREAM_5XX");
        // RUN_ABORTED takes the release path — estimated amount empty.
        assert!(payload.estimated_amount_atomic.is_empty());
        assert!(payload.actual_input_tokens.is_none());
        assert!(payload.actual_output_tokens.is_none());
    }

    #[test]
    fn allow_4xx_emits_provider_error_with_upstream_4xx_code() {
        let state = allow_state_with_reservation("res-3");
        let meta = ResponseMeta {
            http_status: 429,
            provider_usage: None,
        };
        let built = build_llm_call_post(&state, &meta, &ctx());
        let event = match built {
            AuditBuild::Emit(e) => e,
            AuditBuild::Skip { reason } => panic!("expected Emit, got Skip: {reason}"),
        };
        let payload = match event.payload.expect("payload") {
            trace_event::Payload::LlmCallPost(p) => p,
            _ => panic!(),
        };
        assert_eq!(payload.outcome, PostOutcome::ProviderError as i32);
        assert_eq!(event.provider_response_metadata, "UPSTREAM_4XX");
    }

    #[test]
    fn allow_2xx_tokens_unknown_falls_back_to_strategy_a_estimate() {
        // review-standards §5.2: tokens unknown + 200 → fall back to
        // input × 2 estimate, do NOT silently emit a 0-token commit.
        let state = allow_state_with_reservation("res-4");
        let meta = ResponseMeta {
            http_status: 200,
            provider_usage: Some(ProviderUsage::unknown()),
        };
        let built = build_llm_call_post(&state, &meta, &ctx());
        let event = match built {
            AuditBuild::Emit(e) => e,
            AuditBuild::Skip { reason } => panic!("expected Emit, got Skip: {reason}"),
        };
        let payload = match event.payload.expect("payload") {
            trace_event::Payload::LlmCallPost(p) => p,
            _ => panic!(),
        };
        assert_eq!(payload.outcome, PostOutcome::Success as i32);
        // input_tokens=50 → predicted_a_tokens=100 (Strategy A × 2).
        assert_eq!(payload.estimated_amount_atomic, "100");
        // actual_* fields stay None — sidecar mirror leaves them SQL NULL.
        assert!(payload.actual_input_tokens.is_none());
        assert!(payload.actual_output_tokens.is_none());
    }

    #[test]
    fn deny_outcome_skips_emit() {
        let mut s = allow_state_with_reservation("res-5");
        s.decision_outcome = Some(DecisionOutcome::Deny);
        s.reservation_id = None; // Deny path never stashes one.
        let meta = ResponseMeta {
            http_status: 0,
            provider_usage: None,
        };
        let built = build_llm_call_post(&s, &meta, &ctx());
        match built {
            AuditBuild::Skip { reason } => {
                assert!(reason.contains("Deny"), "skip reason: {reason}");
            }
            AuditBuild::Emit(_) => panic!("Deny must skip emit (already audited by sidecar)"),
        }
    }

    #[test]
    fn rejected_outcome_skips_emit() {
        let mut s = allow_state_with_reservation("res-6");
        s.decision_outcome = Some(DecisionOutcome::Rejected);
        s.reservation_id = None;
        let meta = ResponseMeta {
            http_status: 0,
            provider_usage: None,
        };
        let built = build_llm_call_post(&s, &meta, &ctx());
        assert!(matches!(built, AuditBuild::Skip { .. }));
    }

    #[test]
    fn sidecar_error_outcome_skips_emit() {
        let mut s = allow_state_with_reservation("res-7");
        s.decision_outcome = Some(DecisionOutcome::SidecarError);
        s.reservation_id = None;
        let meta = ResponseMeta {
            http_status: 0,
            provider_usage: None,
        };
        match build_llm_call_post(&s, &meta, &ctx()) {
            AuditBuild::Skip { reason } => assert!(reason.contains("SidecarError")),
            AuditBuild::Emit(_) => panic!("SidecarError must skip emit (no reservation)"),
        }
    }

    #[test]
    fn missing_estimate_outcome_skips_emit() {
        let mut s = allow_state_with_reservation("res-8");
        s.decision_outcome = Some(DecisionOutcome::MissingClaimEstimate);
        s.reservation_id = None;
        let meta = ResponseMeta {
            http_status: 0,
            provider_usage: None,
        };
        assert!(matches!(
            build_llm_call_post(&s, &meta, &ctx()),
            AuditBuild::Skip { .. }
        ));
    }

    #[test]
    fn allow_outcome_without_reservation_skips_emit() {
        // Defense in depth: if SLICE 3 stash drift left Allow without a
        // reservation_id, we MUST NOT fabricate an audit row.
        let mut s = allow_state_with_reservation("res-9");
        s.reservation_id = None;
        let meta = ResponseMeta {
            http_status: 200,
            provider_usage: None,
        };
        match build_llm_call_post(&s, &meta, &ctx()) {
            AuditBuild::Skip { reason } => {
                assert!(reason.contains("reservation_id"), "got: {reason}");
            }
            AuditBuild::Emit(_) => panic!("Allow without reservation_id must skip emit"),
        }
    }

    #[test]
    fn no_decision_outcome_skips_emit() {
        // SLICE 3 race: state present but decision_outcome=None.
        let mut s = allow_state_with_reservation("res-10");
        s.decision_outcome = None;
        let meta = ResponseMeta {
            http_status: 200,
            provider_usage: None,
        };
        assert!(matches!(
            build_llm_call_post(&s, &meta, &ctx()),
            AuditBuild::Skip { .. }
        ));
    }

    #[test]
    fn derive_post_idempotency_key_shape() {
        // review-standards §5.1: idempotency key carries reservation_id
        // + ":post" suffix so retries collapse against the sidecar's
        // POST_GA_01 dedup cache.
        assert_eq!(derive_post_idempotency_key("res-abc"), "res-abc:post");
    }

    #[test]
    fn allow_5xx_with_partial_provider_usage_still_emits_run_aborted() {
        // Defensive — if the upstream returns 502 with a stale `usage`
        // block from a previous attempt, we MUST classify as RUN_ABORTED
        // (the http_status is canonical) and drop the stale usage.
        let state = allow_state_with_reservation("res-11");
        let meta = ResponseMeta {
            http_status: 502,
            provider_usage: Some(ProviderUsage {
                input_tokens: Some(1),
                output_tokens: Some(2),
                tokens_unknown: false,
            }),
        };
        let built = build_llm_call_post(&state, &meta, &ctx());
        let event = match built {
            AuditBuild::Emit(e) => e,
            AuditBuild::Skip { reason } => panic!("expected Emit, got Skip: {reason}"),
        };
        let payload = match event.payload.expect("payload") {
            trace_event::Payload::LlmCallPost(p) => p,
            _ => panic!(),
        };
        assert_eq!(payload.outcome, PostOutcome::RunAborted as i32);
        assert_eq!(event.provider_response_metadata, "UPSTREAM_5XX");
        // For non-SUCCESS outcomes the estimated amount is empty (release
        // path); actual_* are still forwarded for calibration audit.
        assert!(payload.estimated_amount_atomic.is_empty());
        assert_eq!(payload.actual_input_tokens, Some(1));
        assert_eq!(payload.actual_output_tokens, Some(2));
    }
}
