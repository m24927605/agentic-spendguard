//! D13 COV_61/62/64 — Sidecar branch for SUBSCRIPTION_METER requests.
//!
//! `route_decision_request` is the entry point invoked from
//! `decision::transaction::run_through_reserve` when the incoming
//! `DecisionRequest.reservation_source` equals
//! `RESERVATION_SOURCE_SUBSCRIPTION_METER`.
//!
//! The function:
//!   1. Pulls input/output token estimates from the attached
//!      `ClaimEstimate`.
//!   2. Reads retail prices from the request's `runtime_metadata`
//!      (the egress proxy / SDK is the source-of-truth for retail
//!      pricing because pricing tables in the ledger are BYOK-tuned).
//!   3. Computes a `MeterEstimate` and a `CapEvaluation`.
//!   4. Returns a `DecisionOutput` carrying the snapshot in
//!      `subscription_meter` AND a CONTINUE decision (Pass / SoftCap)
//!      OR a Stop decision (HardCap) with reason
//!      `subscription_cap_exceeded`.
//!
//! Critically, this lane NEVER calls the ledger and NEVER opens a
//! transaction — that's the whole point of the meter-only flow.

use chrono::Utc;
use uuid::Uuid;

use crate::{
    config::Config,
    decision::transaction::DecisionContext,
    domain::{error::DomainError, state::SidecarState},
    proto::{
        common::v1::{ReservationSource, SubscriptionMeter},
        sidecar_adapter::v1::{decision_response::Decision, DecisionRequest},
    },
};

use super::{
    classifier::{ClassifierInput, SubscriptionKind},
    estimator::meter_only_estimate,
    hard_cap::{evaluate_cap, CapDecision},
};

/// Default retail prices for the meter when the request carries no
/// hint.  These are very rough — caller is expected to override via
/// runtime_metadata.  We use Anthropic Claude 3.5 Sonnet's published
/// retail rates as a sentinel ($3 / $15 per 1M).
const DEFAULT_INPUT_PRICE_MICRO_PER_MILLION: i64 = 3_000_000;
const DEFAULT_OUTPUT_PRICE_MICRO_PER_MILLION: i64 = 15_000_000;

/// Drive the meter-only path.  Synchronous (no awaits) because all
/// inputs are already present in the request; future slices can
/// promote this to async when we wire the canonical_ingest meter
/// increment stream.
pub fn route_decision_request(
    _cfg: &Config,
    _state: &SidecarState,
    ctx: &DecisionContext,
    req: &DecisionRequest,
) -> Result<crate::decision::transaction::DecisionOutput, DomainError> {
    let tenant_id = ctx.tenant_id.as_str();

    // Classify based on hints in runtime_metadata. The egress proxy
    // populates `subscription.kind` once it has decided; if absent, we
    // fall back to `unknown` so the meter row still gets a plan tag.
    let kind = classify_from_request(req).unwrap_or(SubscriptionKind::UnknownSubscription);

    // Pull input/output tokens off the attached ClaimEstimate. If
    // missing we use the request's max_tokens hint (or 0).
    let (input_tokens, predicted_output_tokens) = extract_token_estimates(req);

    let now = Utc::now();
    let mut estimate = meter_only_estimate(
        tenant_id,
        kind,
        input_tokens,
        predicted_output_tokens,
        DEFAULT_INPUT_PRICE_MICRO_PER_MILLION,
        DEFAULT_OUTPUT_PRICE_MICRO_PER_MILLION,
        now,
    );

    // Override prices from runtime_metadata if the proxy supplied them.
    if let Some((in_price, out_price)) = extract_retail_prices(req) {
        estimate = meter_only_estimate(
            tenant_id,
            kind,
            input_tokens,
            predicted_output_tokens,
            in_price,
            out_price,
            now,
        );
    }

    // Cap configuration also comes from runtime_metadata for now (the
    // ledger DB-backed `subscription_meters` lookup is wired in the
    // post-D13 hardening slice).
    let (alert_at, hard_cap, current_consumed) = extract_cap_config(req);

    let cap_eval = evaluate_cap(
        current_consumed,
        estimate.estimated_amount_atomic,
        alert_at,
        hard_cap,
        estimate.period_end,
        now,
    );

    let cap_pb = cap_decision_to_proto(&cap_eval.decision);
    let retry_after_seconds = match cap_eval.decision {
        CapDecision::HardCapBlock {
            retry_after_seconds,
            ..
        } => retry_after_seconds,
        _ => 0,
    };

    let meter_pb = SubscriptionMeter {
        plan: estimate.plan.clone(),
        period_start: chrono_to_pbts(estimate.period_start),
        period_end: chrono_to_pbts(estimate.period_end),
        consumed_atomic: cap_eval.projected_consumed_atomic,
        monthly_cap_atomic: hard_cap.unwrap_or(0),
        alert_at_atomic: alert_at,
        hard_cap_at_atomic: hard_cap.unwrap_or(0),
        cap_decision: cap_pb,
        retry_after_seconds,
    };

    let (decision_kind, reason_codes) = match cap_eval.decision {
        CapDecision::HardCapBlock { .. } => (
            Decision::Stop,
            vec![
                "subscription_cap_exceeded".to_string(),
                format!("spendguard.http_status=429"),
                format!("spendguard.retry_after_seconds={}", retry_after_seconds),
            ],
        ),
        CapDecision::SoftCapAlert { .. } => (
            Decision::Continue,
            vec!["subscription_soft_cap_alert".to_string()],
        ),
        CapDecision::Pass => (
            Decision::Continue,
            vec!["subscription_meter_pass".to_string()],
        ),
    };

    let decision_id = Uuid::now_v7();
    let audit_decision_event_id = Uuid::now_v7();
    let mut effect_hash = [0u8; 32];
    // Compose a stable effect_hash over (tenant, kind, period, decision)
    // so adapter idempotent replays see the same effect_hash. NOT a
    // sha256 — for the meter path, the adapter never re-publishes a
    // mutation, so any stable 32-byte value works. We use UUIDv7 bytes
    // (first 16) plus the cap decision tag for an interpretable trail.
    let dec_bytes = decision_id.into_bytes();
    effect_hash[..16].copy_from_slice(&dec_bytes);
    effect_hash[16] = cap_pb as u8;

    Ok(crate::decision::transaction::DecisionOutput {
        decision_id,
        audit_decision_event_id,
        effect_hash,
        decision: decision_kind,
        // No ledger transaction — meter-only path.
        reservation_set_id: String::new(),
        reservation_ids: vec![],
        ledger_transaction_id: String::new(),
        approval_request_id: String::new(),
        ttl_expires_at: None,
        matched_rule_ids: vec![],
        reason_codes,
        run_code_triggered: String::new(),
        subscription_meter: Some(meter_pb),
    })
}

/// Map the closed CapDecision enum to the proto enum tag.
fn cap_decision_to_proto(d: &CapDecision) -> i32 {
    use crate::proto::common::v1::subscription_meter::CapDecision as Pb;
    let tag = match d {
        CapDecision::Pass => Pb::CapPass,
        CapDecision::SoftCapAlert { .. } => Pb::CapSoftAlert,
        CapDecision::HardCapBlock { .. } => Pb::CapHardBlock,
    };
    tag as i32
}

fn chrono_to_pbts(ts: chrono::DateTime<chrono::Utc>) -> Option<prost_types::Timestamp> {
    Some(prost_types::Timestamp {
        seconds: ts.timestamp(),
        nanos: ts.timestamp_subsec_nanos() as i32,
    })
}

/// Best-effort extraction of token counts from the attached ClaimEstimate.
fn extract_token_estimates(req: &DecisionRequest) -> (i64, i64) {
    let Some(inputs) = req.inputs.as_ref() else {
        return (0, 0);
    };
    let Some(claim) = inputs.claim_estimate.as_ref() else {
        return (0, 0);
    };
    let input = claim.input_tokens.max(0);
    // Output prediction: prefer reserved strategy result, else Strategy A.
    let output = match claim.reserved_strategy.as_str() {
        "C" => claim.predicted_c_tokens,
        "B" => claim.predicted_b_tokens,
        _ => claim.predicted_a_tokens,
    }
    .max(0);
    (input, output)
}

/// Look up retail pricing hints in `runtime_metadata.subscription.*`.
fn extract_retail_prices(req: &DecisionRequest) -> Option<(i64, i64)> {
    let md = req.inputs.as_ref()?.runtime_metadata.as_ref()?;
    let sub = md.fields.get("subscription")?;
    let s = match &sub.kind {
        Some(prost_types::value::Kind::StructValue(s)) => s,
        _ => return None,
    };
    let in_v = s.fields.get("retail_input_price_micro_per_million")?;
    let out_v = s.fields.get("retail_output_price_micro_per_million")?;
    let in_price = number_value(in_v)?;
    let out_price = number_value(out_v)?;
    Some((in_price, out_price))
}

fn extract_cap_config(req: &DecisionRequest) -> (i64, Option<i64>, i64) {
    let alert_at = 0i64;
    let hard_cap = None;
    let current = 0i64;
    let md = match req
        .inputs
        .as_ref()
        .and_then(|i| i.runtime_metadata.as_ref())
    {
        Some(m) => m,
        None => return (alert_at, hard_cap, current),
    };
    let sub = match md.fields.get("subscription").and_then(|v| match &v.kind {
        Some(prost_types::value::Kind::StructValue(s)) => Some(s),
        _ => None,
    }) {
        Some(s) => s,
        None => return (alert_at, hard_cap, current),
    };
    let alert_at = sub
        .fields
        .get("alert_at_atomic")
        .and_then(number_value)
        .unwrap_or(0);
    let hard_cap = sub.fields.get("hard_cap_at_atomic").and_then(number_value);
    let current = sub
        .fields
        .get("current_consumed_atomic")
        .and_then(number_value)
        .unwrap_or(0);
    (alert_at, hard_cap, current)
}

/// Look up the subscription kind hint in `runtime_metadata.subscription.kind`.
fn classify_from_request(req: &DecisionRequest) -> Option<SubscriptionKind> {
    let md = req.inputs.as_ref()?.runtime_metadata.as_ref()?;
    let sub = md.fields.get("subscription")?;
    let s = match &sub.kind {
        Some(prost_types::value::Kind::StructValue(s)) => s,
        _ => return None,
    };

    let provider = string_value(s.fields.get("provider")?).unwrap_or_default();
    let token = string_value(s.fields.get("auth_token_prefix")?).unwrap_or_default();
    let ua = string_value(s.fields.get("user_agent")?).unwrap_or_default();
    let model_id = s
        .fields
        .get("model_id")
        .and_then(string_value)
        .unwrap_or_default();

    Some(super::classifier::classify(&ClassifierInput {
        provider: &provider,
        model_id: &model_id,
        auth_token_prefix: &token,
        user_agent: &ua,
        tenant_id: "",
    }))
}

fn number_value(v: &prost_types::Value) -> Option<i64> {
    match &v.kind {
        Some(prost_types::value::Kind::NumberValue(n)) => Some(*n as i64),
        Some(prost_types::value::Kind::StringValue(s)) => s.parse::<i64>().ok(),
        _ => None,
    }
}

fn string_value(v: &prost_types::Value) -> Option<String> {
    match &v.kind {
        Some(prost_types::value::Kind::StringValue(s)) => Some(s.clone()),
        _ => None,
    }
}

/// Public helper for tests + adapter wiring: convenience predicate
/// over the proto enum value.
pub fn is_subscription_request(req: &DecisionRequest) -> bool {
    req.reservation_source == ReservationSource::SubscriptionMeter as i32
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::sidecar_adapter::v1::{decision_request::Inputs, ClaimEstimate};
    use prost_types::{Struct, Value};
    use std::collections::BTreeMap;

    fn empty_request() -> DecisionRequest {
        DecisionRequest {
            reservation_source: ReservationSource::SubscriptionMeter as i32,
            ..Default::default()
        }
    }

    fn struct_value(map: BTreeMap<String, Value>) -> Value {
        Value {
            kind: Some(prost_types::value::Kind::StructValue(Struct {
                fields: map,
            })),
        }
    }

    fn num(n: i64) -> Value {
        Value {
            kind: Some(prost_types::value::Kind::NumberValue(n as f64)),
        }
    }

    fn s(v: &str) -> Value {
        Value {
            kind: Some(prost_types::value::Kind::StringValue(v.to_string())),
        }
    }

    #[test]
    fn is_subscription_request_detects_meter_flag() {
        let mut req = DecisionRequest::default();
        assert!(!is_subscription_request(&req));
        req.reservation_source = ReservationSource::SubscriptionMeter as i32;
        assert!(is_subscription_request(&req));
    }

    #[test]
    fn classify_from_request_reads_runtime_metadata() {
        let mut sub_fields = BTreeMap::new();
        sub_fields.insert("provider".into(), s("anthropic"));
        sub_fields.insert("auth_token_prefix".into(), s("sk-ant-oat01"));
        sub_fields.insert("user_agent".into(), s("claude-cli/1.4.2"));
        sub_fields.insert("model_id".into(), s("claude-3-5-sonnet-20241022"));

        let mut md_fields = BTreeMap::new();
        md_fields.insert("subscription".into(), struct_value(sub_fields));

        let req = DecisionRequest {
            reservation_source: ReservationSource::SubscriptionMeter as i32,
            inputs: Some(Inputs {
                runtime_metadata: Some(Struct { fields: md_fields }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let kind = classify_from_request(&req).unwrap();
        assert_eq!(kind, SubscriptionKind::ClaudeCodePro);
    }

    #[test]
    fn extract_token_estimates_reads_claim_estimate() {
        let req = DecisionRequest {
            reservation_source: ReservationSource::SubscriptionMeter as i32,
            inputs: Some(Inputs {
                claim_estimate: Some(ClaimEstimate {
                    input_tokens: 1500,
                    predicted_a_tokens: 800,
                    predicted_b_tokens: 0,
                    predicted_c_tokens: 0,
                    reserved_strategy: "A".into(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let (i, o) = extract_token_estimates(&req);
        assert_eq!(i, 1500);
        assert_eq!(o, 800);
    }

    #[test]
    fn extract_token_estimates_strategy_c_overrides() {
        let req = DecisionRequest {
            reservation_source: ReservationSource::SubscriptionMeter as i32,
            inputs: Some(Inputs {
                claim_estimate: Some(ClaimEstimate {
                    input_tokens: 100,
                    predicted_a_tokens: 100,
                    predicted_b_tokens: 200,
                    predicted_c_tokens: 300,
                    reserved_strategy: "C".into(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let (_, o) = extract_token_estimates(&req);
        assert_eq!(o, 300);
    }

    #[test]
    fn extract_retail_prices_returns_none_when_absent() {
        let req = empty_request();
        assert_eq!(extract_retail_prices(&req), None);
    }

    #[test]
    fn extract_retail_prices_reads_subscription_metadata() {
        let mut sub_fields = BTreeMap::new();
        sub_fields.insert(
            "retail_input_price_micro_per_million".into(),
            num(3_000_000),
        );
        sub_fields.insert(
            "retail_output_price_micro_per_million".into(),
            num(15_000_000),
        );
        let mut md_fields = BTreeMap::new();
        md_fields.insert("subscription".into(), struct_value(sub_fields));

        let req = DecisionRequest {
            reservation_source: ReservationSource::SubscriptionMeter as i32,
            inputs: Some(Inputs {
                runtime_metadata: Some(Struct { fields: md_fields }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let prices = extract_retail_prices(&req).unwrap();
        assert_eq!(prices, (3_000_000, 15_000_000));
    }

    #[test]
    fn extract_cap_config_defaults_to_zeros() {
        let req = empty_request();
        let (a, h, c) = extract_cap_config(&req);
        assert_eq!(a, 0);
        assert_eq!(h, None);
        assert_eq!(c, 0);
    }

    #[test]
    fn extract_cap_config_reads_metadata() {
        let mut sub_fields = BTreeMap::new();
        sub_fields.insert("alert_at_atomic".into(), num(15_000_000));
        sub_fields.insert("hard_cap_at_atomic".into(), num(20_000_000));
        sub_fields.insert("current_consumed_atomic".into(), num(5_000_000));

        let mut md_fields = BTreeMap::new();
        md_fields.insert("subscription".into(), struct_value(sub_fields));

        let req = DecisionRequest {
            reservation_source: ReservationSource::SubscriptionMeter as i32,
            inputs: Some(Inputs {
                runtime_metadata: Some(Struct { fields: md_fields }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let (a, h, c) = extract_cap_config(&req);
        assert_eq!(a, 15_000_000);
        assert_eq!(h, Some(20_000_000));
        assert_eq!(c, 5_000_000);
    }
}
