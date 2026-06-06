//! SLICE 3 — sidecar `DecisionResponse` → ExtProc `ProcessingResponse`.
//!
//! The mapping table (per docs/specs/coverage/D01_envoy_extproc/implementation.md §6
//! + review-standards §4.1):
//!
//! | sidecar Decision           | ExtProc response                    | HTTP status |
//! |----------------------------|-------------------------------------|-------------|
//! | CONTINUE                   | CommonResponse(status=CONTINUE)     | (forwarded) |
//! | STOP, STOP_RUN_PROJECTION  | ImmediateResponse                   | 429         |
//! | REQUIRE_APPROVAL           | ImmediateResponse                   | 403         |
//! | DEGRADE                    | ImmediateResponse (fail-closed v1)  | 403         |
//! | SKIP                       | CommonResponse(status=CONTINUE)     | (forwarded) |
//! | UNSPECIFIED (proto3 dflt)  | ImmediateResponse                   | 503         |
//!
//! Header convention — every ImmediateResponse carries:
//!   - `x-spendguard-decision: <allow|deny|degrade|sidecar-unavailable>`
//!   - `x-spendguard-reason-codes: <comma-joined>` (only if non-empty)
//!   - `retry-after: <seconds>` on 503 (per review-standards §4.1.2)
//!
//! ## SLICE 3 vs SLICE 4 fail-closed split
//!
//! Per design §3.4 + design §3.5: v1 (SLICE 1-5) ships fail-closed. The
//! SLICE 4 audit-emit will preserve this posture; the BodyMutation arm
//! (DEGRADE → mutate request body) is wired in SLICE 5 conformance once
//! the mutation patch JSON wire shape is locked. SLICE 3 treats DEGRADE
//! as a deny (403) to honour the "no silent pass" rule from
//! review-standards §1.8.
//!
//! ## Failure-mode rendering — no info disclosure
//!
//! Per review-standards §4.1.3 the ExtProc response body MUST NOT leak
//! sidecar internal error details. The `details` field on
//! ImmediateResponse is set to a FIXED-shape string (e.g.
//! `"spendguard-deny"`); the body is empty. The structured logs (warn!)
//! carry the full reason for ops; the wire response is sanitized.
//!
//! Spec refs:
//!   - docs/specs/coverage/D01_envoy_extproc/implementation.md §6
//!   - docs/specs/coverage/D01_envoy_extproc/review-standards.md §4.1
//!   - docs/specs/coverage/D01_envoy_extproc/design.md §3.4 (fail-closed)

use tracing::{info, warn};

use crate::proto::envoy::config::core::v3::{HeaderValue, HeaderValueOption};
use crate::proto::envoy::r#type::v3::{HttpStatus, StatusCode};
use crate::proto::envoy::service::ext_proc::v3::{
    common_response::ResponseStatus, processing_response::Response as PResp, BodyResponse,
    CommonResponse, HeaderMutation, ImmediateResponse, ProcessingResponse,
};
use crate::proto::spendguard::sidecar_adapter::v1::{
    decision_response::Decision, DecisionResponse,
};
use crate::sidecar_client::SidecarError;
use crate::state::DecisionOutcome;

/// Result of mapping a sidecar [`DecisionResponse`] to an ExtProc
/// [`ProcessingResponse`]. Carries the [`DecisionOutcome`] enum so the
/// server loop can stash it on the per-stream state for SLICE 4.
pub struct MappedResponse {
    pub processing: ProcessingResponse,
    pub outcome: DecisionOutcome,
    /// reservation_id stash for SLICE 4 audit emit. None when the
    /// decision was not ALLOW (i.e. no reservation was created).
    pub reservation_id: Option<String>,
    /// decision_id stash for SLICE 4 audit emit. None when the sidecar
    /// errored (no decision was recorded).
    pub decision_id: Option<String>,
}

/// Map an ALLOW decision to a CONTINUE response on the RequestBody phase.
/// Always returns a `processing_response.body_response` arm so the
/// inbound `request_body` frame is properly closed.
pub fn allow_continue_body(
    reservation_id: Option<String>,
    decision_id: Option<String>,
) -> MappedResponse {
    let common = CommonResponse {
        status: ResponseStatus::Continue as i32,
        ..Default::default()
    };
    let processing = ProcessingResponse {
        response: Some(PResp::RequestBody(BodyResponse {
            response: Some(common),
        })),
        ..Default::default()
    };
    MappedResponse {
        processing,
        outcome: DecisionOutcome::Allow,
        reservation_id,
        decision_id,
    }
}

/// Map STOP / STOP_RUN_PROJECTION to a 429 ImmediateResponse.
/// `reason_codes` becomes the `x-spendguard-reason-codes` header value
/// (joined by `,`). The body is empty (review-standards §4.1.3).
pub fn deny_immediate(
    reason_codes: &[String],
    run_code_triggered: &str,
    decision_id: Option<String>,
) -> MappedResponse {
    let mut set_headers = vec![
        header("x-spendguard-decision", "deny"),
        header("content-length", "0"),
    ];
    if !reason_codes.is_empty() {
        set_headers.push(header("x-spendguard-reason-codes", &reason_codes.join(",")));
    }
    if !run_code_triggered.is_empty() {
        set_headers.push(header("x-spendguard-run-code", run_code_triggered));
    }

    let immediate = ImmediateResponse {
        status: Some(HttpStatus {
            code: StatusCode::TooManyRequests as i32,
        }),
        headers: Some(HeaderMutation {
            set_headers,
            ..Default::default()
        }),
        body: bytes::Bytes::new(),
        // grpc_status omitted — Envoy only translates this for gRPC
        // downstream, which is NOT our deployment shape.
        grpc_status: None,
        // `details` is internal; the upstream Envoy log will surface
        // this. Fixed-shape string per review-standards §4.1.3.
        details: "spendguard-deny".to_string(),
    };
    let processing = ProcessingResponse {
        response: Some(PResp::ImmediateResponse(immediate)),
        ..Default::default()
    };
    MappedResponse {
        processing,
        outcome: DecisionOutcome::Deny,
        reservation_id: None,
        decision_id,
    }
}

/// Map DEGRADE / REQUIRE_APPROVAL to a 403 ImmediateResponse. Per the
/// SLICE 3 fail-closed carve-out, DEGRADE's mutation_patch_json is NOT
/// applied yet (SLICE 5 conformance lands BodyMutation). REQUIRE_APPROVAL
/// surfaces the `approval_request_id` so the client can poll out-of-band
/// (review-standards §4.1.5).
pub fn degrade_or_approval_immediate(
    label: &'static str,
    reason_codes: &[String],
    approval_request_id: Option<&str>,
    decision_id: Option<String>,
) -> MappedResponse {
    let mut set_headers = vec![
        header("x-spendguard-decision", label),
        header("content-length", "0"),
    ];
    if !reason_codes.is_empty() {
        set_headers.push(header("x-spendguard-reason-codes", &reason_codes.join(",")));
    }
    if let Some(id) = approval_request_id {
        if !id.is_empty() {
            set_headers.push(header("x-spendguard-approval-request-id", id));
        }
    }

    let immediate = ImmediateResponse {
        status: Some(HttpStatus {
            code: StatusCode::Forbidden as i32,
        }),
        headers: Some(HeaderMutation {
            set_headers,
            ..Default::default()
        }),
        body: bytes::Bytes::new(),
        grpc_status: None,
        details: format!("spendguard-{label}"),
    };
    let processing = ProcessingResponse {
        response: Some(PResp::ImmediateResponse(immediate)),
        ..Default::default()
    };
    MappedResponse {
        processing,
        outcome: DecisionOutcome::Rejected,
        reservation_id: None,
        decision_id,
    }
}

/// Map a sidecar-unreachable / unspecified-decision error to a 503
/// ImmediateResponse with Retry-After. Per review-standards §4.1.2.
pub fn sidecar_error_immediate(retry_after_seconds: u32) -> MappedResponse {
    let set_headers = vec![
        header("x-spendguard-decision", "sidecar-unavailable"),
        header("retry-after", &retry_after_seconds.to_string()),
        header("content-length", "0"),
    ];
    let immediate = ImmediateResponse {
        status: Some(HttpStatus {
            code: StatusCode::ServiceUnavailable as i32,
        }),
        headers: Some(HeaderMutation {
            set_headers,
            ..Default::default()
        }),
        body: bytes::Bytes::new(),
        grpc_status: None,
        details: "spendguard-sidecar-unavailable".to_string(),
    };
    let processing = ProcessingResponse {
        response: Some(PResp::ImmediateResponse(immediate)),
        ..Default::default()
    };
    MappedResponse {
        processing,
        outcome: DecisionOutcome::SidecarError,
        reservation_id: None,
        decision_id: None,
    }
}

/// Map the SLICE 2 carry-over case (no ClaimEstimate present) to 503.
/// Distinct from `sidecar_error_immediate` so SLICE 4 can audit-emit
/// the right outcome code.
pub fn missing_estimate_immediate(retry_after_seconds: u32) -> MappedResponse {
    let set_headers = vec![
        header("x-spendguard-decision", "missing-estimate"),
        header("retry-after", &retry_after_seconds.to_string()),
        header("content-length", "0"),
    ];
    let immediate = ImmediateResponse {
        status: Some(HttpStatus {
            code: StatusCode::ServiceUnavailable as i32,
        }),
        headers: Some(HeaderMutation {
            set_headers,
            ..Default::default()
        }),
        body: bytes::Bytes::new(),
        grpc_status: None,
        details: "spendguard-missing-estimate".to_string(),
    };
    let processing = ProcessingResponse {
        response: Some(PResp::ImmediateResponse(immediate)),
        ..Default::default()
    };
    MappedResponse {
        processing,
        outcome: DecisionOutcome::MissingClaimEstimate,
        reservation_id: None,
        decision_id: None,
    }
}

/// Top-level mapper. Reads the typed [`Decision`] off the response and
/// dispatches to the right helper.
///
/// Defense-in-depth: an `Unspecified` proto3-default decision is treated
/// as a sidecar error (review-standards §4.1.2.2). We deliberately do
/// NOT continue on Unspecified.
pub fn build_extproc_response(decision: DecisionResponse) -> MappedResponse {
    let decision_id_opt = if decision.decision_id.is_empty() {
        None
    } else {
        Some(decision.decision_id.clone())
    };

    match Decision::try_from(decision.decision).unwrap_or(Decision::Unspecified) {
        Decision::Continue | Decision::Skip => {
            // Take the first reservation_id for SLICE 4's audit emit
            // (single reservation per call in v1; future multi-claim
            // shapes will surface a Vec). Cloning a String is cheap.
            let reservation_id = decision.reservation_ids.first().cloned();
            info!(
                decision = "allow",
                decision_id = %decision.decision_id,
                reservation_id = ?reservation_id,
                "ExtProc maps sidecar CONTINUE/SKIP → CommonResponse(CONTINUE)"
            );
            allow_continue_body(reservation_id, decision_id_opt)
        }
        Decision::Stop | Decision::StopRunProjection => {
            warn!(
                decision = "deny",
                decision_id = %decision.decision_id,
                run_code = %decision.run_code_triggered,
                reason_codes = ?decision.reason_codes,
                "ExtProc maps sidecar STOP → ImmediateResponse 429"
            );
            deny_immediate(
                &decision.reason_codes,
                &decision.run_code_triggered,
                decision_id_opt,
            )
        }
        Decision::Degrade => {
            warn!(
                decision = "degrade",
                decision_id = %decision.decision_id,
                "ExtProc maps sidecar DEGRADE → ImmediateResponse 403 (SLICE 3 fail-closed; SLICE 5 wires BodyMutation)"
            );
            degrade_or_approval_immediate("degrade", &decision.reason_codes, None, decision_id_opt)
        }
        Decision::RequireApproval => {
            warn!(
                decision = "require-approval",
                decision_id = %decision.decision_id,
                approval_request_id = %decision.approval_request_id,
                "ExtProc maps sidecar REQUIRE_APPROVAL → ImmediateResponse 403"
            );
            degrade_or_approval_immediate(
                "require-approval",
                &decision.reason_codes,
                Some(&decision.approval_request_id),
                decision_id_opt,
            )
        }
        Decision::Unspecified => {
            warn!(
                decision = "unspecified",
                decision_id = %decision.decision_id,
                "ExtProc maps sidecar UNSPECIFIED (proto3 default) → 503 fail-closed (review-standards §4.1.2.2)"
            );
            sidecar_error_immediate(1)
        }
    }
}

/// Map a [`SidecarError`] to a 503 ImmediateResponse. Distinct from
/// `build_extproc_response` so the server loop can call the right
/// helper depending on whether it got a `DecisionResponse` back or a
/// transport error.
pub fn build_sidecar_error_response(err: &SidecarError) -> MappedResponse {
    // Retry-After per error class:
    //   * Timeout       → 1s (transient; expect immediate recovery)
    //   * Transport     → 5s (socket flap; back off harder)
    //   * Rpc           → 1s (sidecar logic returned non-OK; usually retryable)
    let retry_after = match err {
        SidecarError::Transport { .. } => 5,
        SidecarError::Timeout { .. } | SidecarError::Rpc { .. } => 1,
    };
    sidecar_error_immediate(retry_after)
}

fn header(key: &str, value: &str) -> HeaderValueOption {
    HeaderValueOption {
        header: Some(HeaderValue {
            key: key.to_string(),
            value: value.to_string(),
            raw_value: bytes::Bytes::new(),
        }),
        append: None,
        append_action: 0,
        keep_empty_value: false,
    }
}

/// Re-export so callers can match on the same body_response arm shape
/// without re-importing the deep proto path. Marker trait — used by
/// the integration test to confirm the response arm carries a
/// CONTINUE-status BodyResponse.
#[allow(dead_code)]
pub(crate) fn extract_body_response_status(p: &ProcessingResponse) -> Option<i32> {
    match p.response.as_ref()? {
        PResp::RequestBody(b) => b.response.as_ref().map(|c| c.status),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::spendguard::sidecar_adapter::v1::DecisionResponse;

    fn make_decision(decision: Decision) -> DecisionResponse {
        DecisionResponse {
            decision_id: "dec-test-1".to_string(),
            decision: decision as i32,
            ..Default::default()
        }
    }

    #[test]
    fn allow_maps_to_common_response_continue() {
        let mut d = make_decision(Decision::Continue);
        d.reservation_ids = vec!["res-1".to_string()];
        let mapped = build_extproc_response(d);
        assert_eq!(mapped.outcome, DecisionOutcome::Allow);
        assert_eq!(mapped.reservation_id.as_deref(), Some("res-1"));
        assert_eq!(mapped.decision_id.as_deref(), Some("dec-test-1"));

        match mapped.processing.response.expect("response set") {
            PResp::RequestBody(br) => {
                let common = br.response.expect("common set");
                assert_eq!(common.status, ResponseStatus::Continue as i32);
            }
            other => panic!("expected RequestBody CONTINUE, got {other:?}"),
        }
    }

    #[test]
    fn skip_also_maps_to_continue() {
        // Skip is semantically equivalent to Continue at the ExtProc
        // wire layer — both let the request flow through unmodified.
        let mut d = make_decision(Decision::Skip);
        d.reservation_ids = vec!["res-2".to_string()];
        let mapped = build_extproc_response(d);
        assert_eq!(mapped.outcome, DecisionOutcome::Allow);
        assert_eq!(mapped.reservation_id.as_deref(), Some("res-2"));
    }

    #[test]
    fn deny_maps_to_immediate_response_429_with_reason_codes() {
        let mut d = make_decision(Decision::Stop);
        d.reason_codes = vec!["BUDGET_EXHAUSTED".to_string(), "RATE_LIMIT".to_string()];
        d.run_code_triggered = "RUN_BUDGET_PROJECTION_EXCEEDED".to_string();
        let mapped = build_extproc_response(d);
        assert_eq!(mapped.outcome, DecisionOutcome::Deny);
        assert!(mapped.reservation_id.is_none());

        match mapped.processing.response.expect("response set") {
            PResp::ImmediateResponse(ir) => {
                assert_eq!(
                    ir.status.expect("status set").code,
                    StatusCode::TooManyRequests as i32
                );
                let mutation = ir.headers.expect("headers set");
                let decision_header = mutation
                    .set_headers
                    .iter()
                    .find(|h| {
                        h.header.as_ref().map(|hv| hv.key.as_str()) == Some("x-spendguard-decision")
                    })
                    .expect("x-spendguard-decision header set");
                assert_eq!(decision_header.header.as_ref().unwrap().value, "deny");
                let reason_header = mutation
                    .set_headers
                    .iter()
                    .find(|h| {
                        h.header.as_ref().map(|hv| hv.key.as_str())
                            == Some("x-spendguard-reason-codes")
                    })
                    .expect("x-spendguard-reason-codes header set");
                assert_eq!(
                    reason_header.header.as_ref().unwrap().value,
                    "BUDGET_EXHAUSTED,RATE_LIMIT"
                );
                let run_code_header = mutation
                    .set_headers
                    .iter()
                    .find(|h| {
                        h.header.as_ref().map(|hv| hv.key.as_str()) == Some("x-spendguard-run-code")
                    })
                    .expect("x-spendguard-run-code header set");
                assert_eq!(
                    run_code_header.header.as_ref().unwrap().value,
                    "RUN_BUDGET_PROJECTION_EXCEEDED"
                );
                // Review-standards §4.1.3: no info disclosure in body.
                assert!(
                    ir.body.is_empty(),
                    "body must be empty (no info disclosure)"
                );
                // Fixed-shape internal details string.
                assert_eq!(ir.details, "spendguard-deny");
            }
            other => panic!("expected ImmediateResponse, got {other:?}"),
        }
    }

    #[test]
    fn stop_run_projection_also_maps_to_429() {
        let mut d = make_decision(Decision::StopRunProjection);
        d.reason_codes = vec!["RUN_STEPS_EXCEEDED".to_string()];
        d.run_code_triggered = "RUN_STEPS_EXCEEDED".to_string();
        let mapped = build_extproc_response(d);
        assert_eq!(mapped.outcome, DecisionOutcome::Deny);
        match mapped.processing.response.unwrap() {
            PResp::ImmediateResponse(ir) => {
                assert_eq!(ir.status.unwrap().code, StatusCode::TooManyRequests as i32);
            }
            other => panic!("expected ImmediateResponse, got {other:?}"),
        }
    }

    #[test]
    fn degrade_maps_to_immediate_response_403_fail_closed() {
        let mut d = make_decision(Decision::Degrade);
        d.reason_codes = vec!["MUTATION_REQUIRED".to_string()];
        d.mutation_patch_json =
            r#"[{"op":"replace","path":"/max_tokens","value":100}]"#.to_string();
        let mapped = build_extproc_response(d);
        assert_eq!(mapped.outcome, DecisionOutcome::Rejected);

        match mapped.processing.response.expect("response set") {
            PResp::ImmediateResponse(ir) => {
                assert_eq!(
                    ir.status.expect("status set").code,
                    StatusCode::Forbidden as i32
                );
                let mutation = ir.headers.expect("headers set");
                let dec = mutation
                    .set_headers
                    .iter()
                    .find(|h| {
                        h.header.as_ref().map(|hv| hv.key.as_str()) == Some("x-spendguard-decision")
                    })
                    .expect("x-spendguard-decision set");
                assert_eq!(dec.header.as_ref().unwrap().value, "degrade");
                // SLICE 3 fail-closed: mutation_patch_json is NOT applied.
                // Body MUST stay empty. SLICE 5 wires BodyMutation.
                assert!(ir.body.is_empty());
                assert_eq!(ir.details, "spendguard-degrade");
            }
            other => panic!("expected ImmediateResponse, got {other:?}"),
        }
    }

    #[test]
    fn require_approval_maps_to_403_with_approval_id() {
        let mut d = make_decision(Decision::RequireApproval);
        d.approval_request_id = "appr-xyz-99".to_string();
        d.reason_codes = vec!["HIGH_RISK".to_string()];
        let mapped = build_extproc_response(d);
        assert_eq!(mapped.outcome, DecisionOutcome::Rejected);

        match mapped.processing.response.expect("response set") {
            PResp::ImmediateResponse(ir) => {
                assert_eq!(
                    ir.status.expect("status set").code,
                    StatusCode::Forbidden as i32
                );
                let mutation = ir.headers.expect("headers set");
                let appr = mutation
                    .set_headers
                    .iter()
                    .find(|h| {
                        h.header.as_ref().map(|hv| hv.key.as_str())
                            == Some("x-spendguard-approval-request-id")
                    })
                    .expect("x-spendguard-approval-request-id set");
                assert_eq!(appr.header.as_ref().unwrap().value, "appr-xyz-99");
                assert_eq!(ir.details, "spendguard-require-approval");
            }
            other => panic!("expected ImmediateResponse, got {other:?}"),
        }
    }

    #[test]
    fn unspecified_decision_maps_to_503_fail_closed() {
        // Defense-in-depth per review-standards §4.1.2.2.
        let d = make_decision(Decision::Unspecified);
        let mapped = build_extproc_response(d);
        assert_eq!(mapped.outcome, DecisionOutcome::SidecarError);
        match mapped.processing.response.expect("response set") {
            PResp::ImmediateResponse(ir) => {
                assert_eq!(
                    ir.status.expect("status set").code,
                    StatusCode::ServiceUnavailable as i32
                );
                let mutation = ir.headers.expect("headers set");
                let dec = mutation
                    .set_headers
                    .iter()
                    .find(|h| {
                        h.header.as_ref().map(|hv| hv.key.as_str()) == Some("x-spendguard-decision")
                    })
                    .expect("decision header set");
                assert_eq!(dec.header.as_ref().unwrap().value, "sidecar-unavailable");
                let retry = mutation
                    .set_headers
                    .iter()
                    .find(|h| h.header.as_ref().map(|hv| hv.key.as_str()) == Some("retry-after"))
                    .expect("retry-after set");
                assert_eq!(retry.header.as_ref().unwrap().value, "1");
            }
            other => panic!("expected ImmediateResponse, got {other:?}"),
        }
    }

    #[test]
    fn sidecar_transport_error_maps_to_503_with_longer_backoff() {
        let err = SidecarError::Transport {
            message: "uds connect failed".to_string(),
        };
        let mapped = build_sidecar_error_response(&err);
        assert_eq!(mapped.outcome, DecisionOutcome::SidecarError);
        match mapped.processing.response.expect("response set") {
            PResp::ImmediateResponse(ir) => {
                assert_eq!(
                    ir.status.expect("status set").code,
                    StatusCode::ServiceUnavailable as i32
                );
                let mutation = ir.headers.expect("headers set");
                let retry = mutation
                    .set_headers
                    .iter()
                    .find(|h| h.header.as_ref().map(|hv| hv.key.as_str()) == Some("retry-after"))
                    .expect("retry-after set");
                assert_eq!(retry.header.as_ref().unwrap().value, "5");
                // No info disclosure: body empty + details fixed string.
                assert!(ir.body.is_empty());
                assert_eq!(ir.details, "spendguard-sidecar-unavailable");
            }
            other => panic!("expected ImmediateResponse, got {other:?}"),
        }
    }

    #[test]
    fn sidecar_timeout_maps_to_503_with_short_backoff() {
        let err = SidecarError::Timeout { timeout_ms: 75 };
        let mapped = build_sidecar_error_response(&err);
        match mapped.processing.response.expect("response set") {
            PResp::ImmediateResponse(ir) => {
                let mutation = ir.headers.expect("headers set");
                let retry = mutation
                    .set_headers
                    .iter()
                    .find(|h| h.header.as_ref().map(|hv| hv.key.as_str()) == Some("retry-after"))
                    .expect("retry-after set");
                assert_eq!(retry.header.as_ref().unwrap().value, "1");
            }
            other => panic!("expected ImmediateResponse, got {other:?}"),
        }
    }

    #[test]
    fn missing_estimate_maps_to_503_with_distinct_label() {
        // SLICE 4 audit-emit reads `MissingClaimEstimate` outcome
        // separately from `SidecarError` so the dashboard can
        // distinguish "tokenizer broken" from "sidecar broken".
        let mapped = missing_estimate_immediate(1);
        assert_eq!(mapped.outcome, DecisionOutcome::MissingClaimEstimate);
        match mapped.processing.response.expect("response set") {
            PResp::ImmediateResponse(ir) => {
                let mutation = ir.headers.expect("headers set");
                let dec = mutation
                    .set_headers
                    .iter()
                    .find(|h| {
                        h.header.as_ref().map(|hv| hv.key.as_str()) == Some("x-spendguard-decision")
                    })
                    .unwrap();
                assert_eq!(dec.header.as_ref().unwrap().value, "missing-estimate");
                assert_eq!(ir.details, "spendguard-missing-estimate");
            }
            other => panic!("expected ImmediateResponse, got {other:?}"),
        }
    }
}
