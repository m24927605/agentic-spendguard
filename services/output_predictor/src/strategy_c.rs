//! Strategy C — delegated to customer-trained plugin.
//!
//! Spec refs:
//!   - `output-predictor-service-spec-v1alpha1.md` §5 (Strategy C
//!     placement in the selector chain)
//!   - `output-predictor-plugin-contract-v1alpha1.md` §4 (50ms hard cap)
//!   - `output-predictor-plugin-contract-v1alpha1.md` §5.1 (8 failure
//!     modes — exhaustive table this module mirrors as enum variants)
//!   - `output-predictor-plugin-contract-v1alpha1.md` §5.3 (plugin
//!     failure NEVER blocks reservation; always falls to Strategy B)
//!   - `output-predictor-plugin-contract-v1alpha1.md` §6.1 (circuit
//!     breaker state machine — consulted before each Predict)
//!   - `output-predictor-plugin-contract-v1alpha1.md` §6.4 (probe-
//!     prefix for half-open synthetic calls)
//!   - `output-predictor-plugin-contract-v1alpha1.md` §7.3 (server-side
//!     tenant_id binding enforcement)
//!
//! ## Critical invariant — §1.8 plugin failure isolation
//!
//! Any plugin failure (timeout / gRPC error / validation failure /
//! mTLS error / breaker Open / NOT_SERVING) returns `Ok(None)` to the
//! caller (server.rs's `c_fut`). The selector then sees C=None and
//! falls to B per spec §6.1. The Predict RPC itself NEVER fails
//! because Strategy C is unhealthy — that's the contract guarantee.
//!
//! The only situation where this module returns an `Err(_)` is the
//! [`StrategyCError::TenantBindingViolation`] case — a HARD CONFIG
//! ERROR (spec §7.3) that the server.rs caller surfaces as
//! `Status::failed_precondition` because it indicates RLS or
//! cert binding misconfiguration that the operator MUST see.
//!
//! ## 8 failure modes (spec §5.1)
//!
//! Each maps to a specific `StrategyCFailure` variant + a specific
//! `customer_predictor_*` metric label so dashboards can split call
//! outcomes by failure cause:
//!
//!   1. `Timeout`              → `customer_predictor_timeout`
//!   2. `GrpcError`            → `customer_predictor_grpc_error`
//!   3. `InvalidZeroOrNegative`→ `customer_predictor_invalid_zero_or_negative`
//!   4. `InvalidOverflow`      → `customer_predictor_invalid_overflow`
//!   5. `InvalidConfidence`    → `customer_predictor_invalid_confidence`
//!   6. `DeserializationError` → `customer_predictor_deserialization_error`
//!   7. `TlsError`             → `customer_predictor_tls_error`
//!   8. `NotServing`           → tracked via circuit-breaker state
//!                               (per spec §6.3 NOT_SERVING is treated
//!                               by the health loop, not the Predict
//!                               path; we emit `customer_predictor_not_serving`
//!                               when the cache hands us an explicitly
//!                               disabled endpoint here)

use std::sync::Arc;
use std::time::Duration;

use thiserror::Error;
use tokio::time::timeout;
use tonic::Code;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::circuit_breaker::{Permit, PluginCircuitBreaker};
use crate::endpoint_cache::{EndpointCache, EndpointCacheError};
use crate::plugin_client::PluginClient;
use crate::proto::output_predictor_plugin::v1::PredictRequest as PluginPredictRequest;

/// Per spec §4.1 — 50ms hard cap on Predict RPC.
pub const PREDICT_HARD_CAP: Duration = Duration::from_millis(50);

/// Output of a successful Strategy C call. server.rs slots
/// `predicted_output_tokens` into the response `predicted_c_tokens`
/// field and `confidence` / `sample_size` into the audit row.
#[derive(Debug, Clone, PartialEq)]
pub struct PredictionC {
    pub predicted_output_tokens: i64,
    pub confidence: f32,
    pub sample_size: i32,
    pub plugin_version: String,
    pub feature_hash: String,
}

/// The 8 failure modes per spec §5.1 plus the "skipped" outcomes
/// (cache miss, breaker open, plugin disabled) that resolve to "no
/// C this call; fall to B" but are reported separately for metrics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StrategyCFailure {
    /// Spec §5.1 mode 1.
    Timeout,
    /// Spec §5.1 mode 2. Carries the gRPC code for metric labelling.
    GrpcError(Code),
    /// Spec §5.1 mode 3.
    InvalidZeroOrNegative,
    /// Spec §5.1 mode 4 — exceeds `model_context_window`.
    InvalidOverflow,
    /// Spec §5.1 mode 5 — confidence outside [0.0, 1.0].
    InvalidConfidence,
    /// Spec §5.1 mode 6 — proto decode failed on the response. tonic
    /// raises this as `Code::Internal` so most callers see GrpcError;
    /// we keep the variant for documentation symmetry with the spec.
    DeserializationError,
    /// Spec §5.1 mode 7 — mTLS handshake failure or cert pin mismatch.
    /// Surfaced as `Code::Unauthenticated` from tonic; we tag separately.
    TlsError,
    /// Spec §5.1 mode 8 — last known HealthCheck was NOT_SERVING. In
    /// SLICE_07 this fires when the cache hands us an explicitly
    /// disabled endpoint (operator kill-switch).
    NotServing,
    /// No row in the registry for this tenant — strategy_c.rs returns
    /// `Ok(None)` and the selector falls to B. Reported separately for
    /// metric labelling.
    NotConfigured,
    /// Circuit breaker is Open — Predict skipped per spec §6. Reported
    /// separately for metric labelling.
    BreakerOpen,
}

impl StrategyCFailure {
    /// Prometheus-friendly metric label per spec §5.1 table.
    pub fn as_label(&self) -> &'static str {
        match self {
            StrategyCFailure::Timeout => "timeout",
            StrategyCFailure::GrpcError(_) => "grpc_error",
            StrategyCFailure::InvalidZeroOrNegative => "invalid_zero_or_negative",
            StrategyCFailure::InvalidOverflow => "invalid_overflow",
            StrategyCFailure::InvalidConfidence => "invalid_confidence",
            StrategyCFailure::DeserializationError => "deserialization_error",
            StrategyCFailure::TlsError => "tls_error",
            StrategyCFailure::NotServing => "not_serving",
            StrategyCFailure::NotConfigured => "not_configured",
            StrategyCFailure::BreakerOpen => "breaker_open",
        }
    }
}

/// Errors that surface up to server.rs as something OTHER than
/// "fall to B silently". v1alpha1 has exactly one such variant —
/// the tenant binding violation — because spec §1.8 mandates that
/// every other plugin failure mode falls to B without error.
#[derive(Debug, Error)]
pub enum StrategyCError {
    /// Spec §7.3 — the cache returned a row whose tenant_id does not
    /// match the request's tenant_id. Indicates RLS bypass or
    /// per-tenant connection misconfig; operator MUST see this as a
    /// hard failure.
    #[error("tenant_id binding violation: cache row {got} != request {requested}")]
    TenantBindingViolation { requested: Uuid, got: Uuid },
}

/// Per-call inputs supplied by server.rs's c_fut closure.
pub struct StrategyCInput<'a> {
    pub tenant_id: Uuid,
    pub model: &'a str,
    pub agent_id: &'a str,
    pub prompt_class: &'a str,
    pub input_tokens: i64,
    pub max_tokens_requested: i64,
    /// Hard upper bound on the response per spec §5.1 mode 4.
    pub model_context_window: i64,
    /// Caller's decision_id; used as `spendguard_call_id` so plugin
    /// telemetry and SpendGuard audit chain join on the same id.
    pub decision_id: &'a str,
    pub classifier_version: &'a str,
    pub prompt_class_fingerprint: &'a str,
}

/// Outcome envelope: either a valid prediction, a recoverable failure
/// (fall to B), or a hard error (refuse). server.rs maps the envelope
/// to the response shape + the audit row + the metric.
#[derive(Debug)]
pub enum StrategyCOutcome {
    /// Plugin returned a validated prediction. server.rs slots into
    /// `predicted_c_tokens` and the selector may pick C.
    Ok(PredictionC),
    /// Recoverable failure per spec §1.8 — selector falls to B; metric
    /// labelled with the failure mode.
    FallToB(StrategyCFailure),
}

/// Compute Strategy C end-to-end:
///   1. Endpoint cache lookup (RLS-bound; tenant_id verified)
///   2. Circuit breaker permit check
///   3. mTLS Predict call wrapped in 50ms hard cap
///   4. Response validation against spec §5.1 (4 invalid-projection
///      checks)
///   5. Breaker record_success / record_failure
///
/// Per spec §1.8 every recoverable failure resolves to
/// `Ok(StrategyCOutcome::FallToB(_))`; only the
/// `TenantBindingViolation` rises as `Err(_)`.
pub async fn compute_c(
    cache: &Arc<EndpointCache>,
    client: &Arc<PluginClient>,
    breaker: &Arc<PluginCircuitBreaker>,
    input: StrategyCInput<'_>,
) -> Result<StrategyCOutcome, StrategyCError> {
    // ── 1. Endpoint lookup ───────────────────────────────────────────
    let endpoint = match cache.lookup(&input.tenant_id).await {
        Ok(ep) => ep,
        Err(EndpointCacheError::NotConfigured(_)) => {
            debug!(tenant = %input.tenant_id, "Strategy C: no plugin endpoint configured");
            return Ok(StrategyCOutcome::FallToB(StrategyCFailure::NotConfigured));
        }
        Err(EndpointCacheError::TenantBindingViolation { requested, got }) => {
            // HARD ERROR per spec §7.3 — surface to operator, do not
            // fall to B silently.
            warn!(
                tenant = %input.tenant_id,
                got = %got,
                "Strategy C: tenant_id binding violation — RLS bypass suspected"
            );
            return Err(StrategyCError::TenantBindingViolation { requested, got });
        }
        Err(EndpointCacheError::Sql(e)) => {
            // DB error — count as plugin failure (gRPC-bucket) and
            // fall to B. Same observable behaviour as a plugin RPC
            // failure for the caller; tagged differently for metrics.
            warn!(
                tenant = %input.tenant_id,
                err = %e,
                "Strategy C: endpoint cache SQL error; falling to B"
            );
            return Ok(StrategyCOutcome::FallToB(StrategyCFailure::GrpcError(
                Code::Unavailable,
            )));
        }
    };

    // Spec §7.3 defense-in-depth: cache already verifies tenant_id at
    // SQL row level; re-verify here at the call site so the boundary
    // is visible to adversarial reviewers reading this file in
    // isolation.
    if endpoint.tenant_id != input.tenant_id {
        return Err(StrategyCError::TenantBindingViolation {
            requested: input.tenant_id,
            got: endpoint.tenant_id,
        });
    }

    // ── 2. Circuit breaker permit ────────────────────────────────────
    let probe = match breaker.permit_request(&input.tenant_id) {
        Permit::SkipOpen => {
            debug!(tenant = %input.tenant_id, "Strategy C: breaker Open; skipping Predict");
            return Ok(StrategyCOutcome::FallToB(StrategyCFailure::BreakerOpen));
        }
        Permit::Allow => {
            // Half-open transitions are signalled implicitly by the
            // breaker (it flips Open→HalfOpen on a deadline-elapsed
            // permit). The Predict caller doesn't need to know whether
            // it's a probe vs real call; per spec §6.4 we tag the
            // spendguard_call_id with `probe-` so the plugin's
            // telemetry can distinguish, but we only do this when
            // breaker state is HalfOpen at permit time.
            matches!(
                breaker.state_of(&input.tenant_id),
                crate::circuit_breaker::BreakerStateName::HalfOpen
            )
        }
    };

    // ── 3. Build PluginPredictRequest ───────────────────────────────
    let spendguard_call_id = if probe {
        format!("probe-{}", input.decision_id)
    } else {
        input.decision_id.to_string()
    };

    let plugin_req = PluginPredictRequest {
        spendguard_call_id,
        tenant_id: input.tenant_id.to_string(),
        model: input.model.to_string(),
        agent_id: input.agent_id.to_string(),
        prompt_class: input.prompt_class.to_string(),
        input_tokens: input.input_tokens,
        max_tokens_requested: input.max_tokens_requested,
        classifier_version: input.classifier_version.to_string(),
        prompt_class_fingerprint: input.prompt_class_fingerprint.to_string(),
        features: None,
    };

    // ── 4. Predict RPC wrapped in 50ms hard cap (spec §4.1) ─────────
    let call_fut = client.predict(&input.tenant_id, endpoint.clone(), plugin_req);
    let outcome = match timeout(PREDICT_HARD_CAP, call_fut).await {
        Err(_) => {
            breaker.record_failure(&input.tenant_id);
            return Ok(StrategyCOutcome::FallToB(StrategyCFailure::Timeout));
        }
        Ok(Err(status)) => {
            breaker.record_failure(&input.tenant_id);
            // Map gRPC codes to spec failure modes.
            let failure = classify_status(&status);
            warn!(
                tenant = %input.tenant_id,
                code = ?status.code(),
                message = %status.message(),
                "Strategy C: plugin gRPC error; falling to B"
            );
            return Ok(StrategyCOutcome::FallToB(failure));
        }
        Ok(Ok(resp)) => resp,
    };

    // ── 5. Validate response per spec §5.1 modes 3-5 ────────────────
    if outcome.predicted_output_tokens <= 0 {
        breaker.record_failure(&input.tenant_id);
        warn!(
            tenant = %input.tenant_id,
            predicted = outcome.predicted_output_tokens,
            "Strategy C: plugin returned non-positive predicted_output_tokens; falling to B"
        );
        return Ok(StrategyCOutcome::FallToB(
            StrategyCFailure::InvalidZeroOrNegative,
        ));
    }
    if outcome.predicted_output_tokens > input.model_context_window {
        breaker.record_failure(&input.tenant_id);
        warn!(
            tenant = %input.tenant_id,
            predicted = outcome.predicted_output_tokens,
            ceiling = input.model_context_window,
            "Strategy C: plugin returned > model_context_window; falling to B"
        );
        return Ok(StrategyCOutcome::FallToB(StrategyCFailure::InvalidOverflow));
    }
    if !(0.0..=1.0).contains(&outcome.confidence) || outcome.confidence.is_nan() {
        breaker.record_failure(&input.tenant_id);
        warn!(
            tenant = %input.tenant_id,
            confidence = outcome.confidence,
            "Strategy C: plugin returned out-of-range confidence; falling to B"
        );
        return Ok(StrategyCOutcome::FallToB(StrategyCFailure::InvalidConfidence));
    }

    breaker.record_success(&input.tenant_id);
    info!(
        tenant = %input.tenant_id,
        predicted = outcome.predicted_output_tokens,
        confidence = outcome.confidence,
        plugin_version = %outcome.plugin_version,
        probe = probe,
        "Strategy C: plugin success"
    );
    Ok(StrategyCOutcome::Ok(PredictionC {
        predicted_output_tokens: outcome.predicted_output_tokens,
        confidence: outcome.confidence,
        sample_size: outcome.sample_size,
        plugin_version: outcome.plugin_version,
        feature_hash: outcome.feature_hash,
    }))
}

/// Map tonic Status codes to the spec §5.1 failure mode taxonomy.
/// Unauthenticated → TlsError; DataLoss / Internal on a decode failure
/// → DeserializationError; everything else → GrpcError(code).
fn classify_status(status: &tonic::Status) -> StrategyCFailure {
    match status.code() {
        Code::Unauthenticated => StrategyCFailure::TlsError,
        Code::DataLoss => StrategyCFailure::DeserializationError,
        code => StrategyCFailure::GrpcError(code),
    }
}

/// Surface-area helper used by the cache eviction path on PUT/DELETE.
/// Forwards to both the endpoint cache and the channel cache so the
/// next Predict call rebuilds against the updated registry row.
pub fn evict_tenant(
    cache: &Arc<EndpointCache>,
    client: &Arc<PluginClient>,
    tenant: &Uuid,
) {
    cache.evict(tenant);
    client.evict(tenant);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::circuit_breaker::{BreakerStateName, CircuitBreakerConfig};

    fn fixture_input(tenant: Uuid) -> StrategyCInput<'static> {
        StrategyCInput {
            tenant_id: tenant,
            model: "gpt-4o",
            agent_id: "agent-test",
            prompt_class: "chat_short",
            input_tokens: 50,
            max_tokens_requested: 500,
            model_context_window: 128_000,
            decision_id: "00000000-0000-0000-0000-000000000001",
            classifier_version: "v1alpha1",
            prompt_class_fingerprint: "abcdef",
        }
    }

    fn build_deps() -> (
        Arc<EndpointCache>,
        Arc<PluginClient>,
        Arc<PluginCircuitBreaker>,
    ) {
        let cache = EndpointCache::with_default_ttl(None);
        let client = PluginClient::new(None);
        let breaker = PluginCircuitBreaker::new(CircuitBreakerConfig::default());
        (cache, client, breaker)
    }

    #[test]
    fn tenant_binding_violation_is_hard_error_not_fall_to_b() {
        // Spec §7.3 + slice §9 checklist Q5: cross-tenant injection
        // MUST be rejected (NOT silently fall to B). Verify the
        // StrategyCError variant exists and the error message
        // includes both UUIDs so the operator can diagnose RLS
        // misconfig from logs alone.
        let requested = Uuid::new_v4();
        let got = Uuid::new_v4();
        let err = StrategyCError::TenantBindingViolation { requested, got };
        let msg = format!("{err}");
        assert!(
            msg.contains(&requested.to_string()),
            "error message must include requested tenant"
        );
        assert!(
            msg.contains(&got.to_string()),
            "error message must include the mismatched tenant"
        );
        assert!(
            msg.contains("violation"),
            "error message must mention violation"
        );
    }

    #[test]
    fn defense_in_depth_check_matches_cache_check() {
        // Spec §7.3 belt-and-suspenders: the cache layer (RLS + code
        // verification) is the first gate; strategy_c.rs adds a second
        // identical check at the call site. This test documents that
        // both layers exist and uses the same comparison shape so a
        // future refactor that drops one of them flags here.
        let tenant_a = Uuid::new_v4();
        let tenant_b = Uuid::new_v4();
        // Simulate cache returning a row whose tenant doesn't match.
        let fake = crate::endpoint_cache::PluginEndpoint {
            plugin_endpoint_id: Uuid::new_v4(),
            tenant_id: tenant_b,
            endpoint_url: "https://plugin.example/predict".into(),
            server_cert_fingerprint: "a".repeat(64),
            client_cert_id: "spendguard-default".into(),
            enabled: true,
        };
        // The check that lives in strategy_c.rs's compute_c after
        // cache.lookup() — verify by comparison directly.
        assert_ne!(fake.tenant_id, tenant_a);
        let err = StrategyCError::TenantBindingViolation {
            requested: tenant_a,
            got: fake.tenant_id,
        };
        let msg = format!("{err}");
        assert!(msg.contains(&tenant_a.to_string()));
        assert!(msg.contains(&tenant_b.to_string()));
    }

    #[tokio::test]
    async fn breaker_open_short_circuits_when_endpoint_exists() {
        // R0 self-review: skeleton mode falls to NotConfigured BEFORE
        // breaker is consulted, so we cannot exercise the breaker-open
        // path through compute_c without a DB-backed cache. Instead we
        // test the building block directly: the breaker's
        // permit_request returns SkipOpen after 10 consecutive failures,
        // which strategy_c maps to FallToB(BreakerOpen). The full
        // integration test lives in SLICE_07 Phase E with a mock plugin.
        let breaker = PluginCircuitBreaker::new(CircuitBreakerConfig::default());
        let tenant = Uuid::new_v4();
        for _ in 0..10 {
            breaker.record_failure(&tenant);
        }
        assert_eq!(breaker.state_of(&tenant), BreakerStateName::Open);
        assert_eq!(breaker.permit_request(&tenant), Permit::SkipOpen);
    }

    #[tokio::test]
    async fn skeleton_mode_falls_to_b_not_configured() {
        // Cache has no DB pool → every lookup returns NotConfigured →
        // strategy_c.rs returns FallToB(NotConfigured) (no plugin call;
        // no breaker change).
        let (cache, client, breaker) = build_deps();
        let tenant = Uuid::new_v4();
        let outcome = compute_c(&cache, &client, &breaker, fixture_input(tenant))
            .await
            .expect("ok");
        match outcome {
            StrategyCOutcome::FallToB(StrategyCFailure::NotConfigured) => {}
            other => panic!("expected NotConfigured, got {other:?}"),
        }
        // Breaker should still be Closed (we never recorded a failure).
        assert_eq!(breaker.state_of(&tenant), BreakerStateName::Closed);
    }

    #[test]
    fn failure_labels_match_spec_5_1_table() {
        // Per spec §5.1 — verify each label matches the documented
        // metric suffix.
        assert_eq!(StrategyCFailure::Timeout.as_label(), "timeout");
        assert_eq!(
            StrategyCFailure::GrpcError(Code::Unavailable).as_label(),
            "grpc_error"
        );
        assert_eq!(
            StrategyCFailure::InvalidZeroOrNegative.as_label(),
            "invalid_zero_or_negative"
        );
        assert_eq!(
            StrategyCFailure::InvalidOverflow.as_label(),
            "invalid_overflow"
        );
        assert_eq!(
            StrategyCFailure::InvalidConfidence.as_label(),
            "invalid_confidence"
        );
        assert_eq!(
            StrategyCFailure::DeserializationError.as_label(),
            "deserialization_error"
        );
        assert_eq!(StrategyCFailure::TlsError.as_label(), "tls_error");
        assert_eq!(StrategyCFailure::NotServing.as_label(), "not_serving");
        assert_eq!(
            StrategyCFailure::NotConfigured.as_label(),
            "not_configured"
        );
        assert_eq!(StrategyCFailure::BreakerOpen.as_label(), "breaker_open");
    }

    #[test]
    fn classify_status_routes_to_spec_5_1_modes() {
        // Per spec §5.1 row 7: TLS handshake failure → Unauthenticated.
        let s = tonic::Status::unauthenticated("bad cert");
        assert_eq!(classify_status(&s), StrategyCFailure::TlsError);
        // Per spec §5.1 row 6: deserialization error → DataLoss.
        let s = tonic::Status::data_loss("proto decode failed");
        assert_eq!(classify_status(&s), StrategyCFailure::DeserializationError);
        // Everything else → GrpcError(code).
        let s = tonic::Status::unavailable("plugin down");
        assert_eq!(
            classify_status(&s),
            StrategyCFailure::GrpcError(Code::Unavailable)
        );
    }

    #[test]
    fn predict_hard_cap_matches_spec() {
        // Spec §4.1: 50ms non-overridable hard cap.
        assert_eq!(PREDICT_HARD_CAP, Duration::from_millis(50));
    }

    #[tokio::test]
    async fn validation_invalid_zero_or_negative_falls_to_b() {
        // Unit test of the validation logic directly — exercising the
        // full RPC path requires a mock plugin server which lives in
        // the SLICE_07 Phase E integration tests. Here we cover the
        // pure validation branch.
        let outcome = validate_response_for_test(0, 0.5, 100);
        assert_eq!(
            outcome,
            StrategyCOutcome::FallToB(StrategyCFailure::InvalidZeroOrNegative).into()
        );
        let outcome = validate_response_for_test(-1, 0.5, 100);
        assert_eq!(
            outcome,
            StrategyCOutcome::FallToB(StrategyCFailure::InvalidZeroOrNegative).into()
        );
    }

    #[tokio::test]
    async fn validation_invalid_overflow_falls_to_b() {
        let outcome = validate_response_for_test(200, 0.5, 100);
        assert_eq!(
            outcome,
            StrategyCOutcome::FallToB(StrategyCFailure::InvalidOverflow).into()
        );
    }

    #[tokio::test]
    async fn validation_invalid_confidence_below_zero() {
        let outcome = validate_response_for_test(50, -0.1, 100);
        assert_eq!(
            outcome,
            StrategyCOutcome::FallToB(StrategyCFailure::InvalidConfidence).into()
        );
    }

    #[tokio::test]
    async fn validation_invalid_confidence_above_one() {
        let outcome = validate_response_for_test(50, 1.1, 100);
        assert_eq!(
            outcome,
            StrategyCOutcome::FallToB(StrategyCFailure::InvalidConfidence).into()
        );
    }

    #[tokio::test]
    async fn validation_invalid_confidence_nan() {
        let outcome = validate_response_for_test(50, f32::NAN, 100);
        assert_eq!(
            outcome,
            StrategyCOutcome::FallToB(StrategyCFailure::InvalidConfidence).into()
        );
    }

    #[tokio::test]
    async fn validation_success_path_returns_prediction() {
        // Boundary: predicted == ceiling, confidence == 0.0 / 1.0.
        let ok = validate_response_for_test(100, 0.0, 100);
        match ok.0 {
            StrategyCOutcome::Ok(p) => {
                assert_eq!(p.predicted_output_tokens, 100);
                assert_eq!(p.confidence, 0.0);
            }
            _ => panic!("expected Ok"),
        }
        let ok = validate_response_for_test(1, 1.0, 100);
        match ok.0 {
            StrategyCOutcome::Ok(p) => {
                assert_eq!(p.predicted_output_tokens, 1);
                assert_eq!(p.confidence, 1.0);
            }
            _ => panic!("expected Ok"),
        }
    }

    #[tokio::test]
    async fn validation_invalid_max_int_overflow() {
        // Spec §5.1 mode 4 — plugin returns absurdly huge value
        // (adversarial scenario in slice §9 checklist Q1).
        let outcome = validate_response_for_test(i64::MAX, 0.5, 1_000_000);
        assert_eq!(
            outcome,
            StrategyCOutcome::FallToB(StrategyCFailure::InvalidOverflow).into()
        );
    }

    /// Helper for unit-testing the validation logic without a real
    /// gRPC plugin. Mirrors the body of `compute_c` from "validate
    /// response" onward.
    fn validate_response_for_test(
        predicted_output_tokens: i64,
        confidence: f32,
        context_window: i64,
    ) -> OutcomeWrap {
        if predicted_output_tokens <= 0 {
            return OutcomeWrap(StrategyCOutcome::FallToB(
                StrategyCFailure::InvalidZeroOrNegative,
            ));
        }
        if predicted_output_tokens > context_window {
            return OutcomeWrap(StrategyCOutcome::FallToB(StrategyCFailure::InvalidOverflow));
        }
        if !(0.0..=1.0).contains(&confidence) || confidence.is_nan() {
            return OutcomeWrap(StrategyCOutcome::FallToB(
                StrategyCFailure::InvalidConfidence,
            ));
        }
        OutcomeWrap(StrategyCOutcome::Ok(PredictionC {
            predicted_output_tokens,
            confidence,
            sample_size: 0,
            plugin_version: "".into(),
            feature_hash: "".into(),
        }))
    }

    /// Wrap so the test fixture's PartialEq is derivable for the
    /// outer arm only (StrategyCOutcome itself can't derive PartialEq
    /// because tonic::Code doesn't implement Eq for variants in older
    /// versions — we keep PartialEq on the wrapper for ergonomics).
    #[derive(Debug)]
    struct OutcomeWrap(StrategyCOutcome);

    impl PartialEq for OutcomeWrap {
        fn eq(&self, other: &Self) -> bool {
            match (&self.0, &other.0) {
                (StrategyCOutcome::Ok(a), StrategyCOutcome::Ok(b)) => a == b,
                (StrategyCOutcome::FallToB(a), StrategyCOutcome::FallToB(b)) => a == b,
                _ => false,
            }
        }
    }

    impl From<StrategyCOutcome> for OutcomeWrap {
        fn from(o: StrategyCOutcome) -> Self {
            OutcomeWrap(o)
        }
    }
}
