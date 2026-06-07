//! Decision transaction state machine (Contract §6).
//!
//! 8 stages:
//!   1. snapshot           — capture event_time, evaluator_time, ledger_state, risk_band
//!   2. evaluate           — CEL predicate evaluation against snapshot
//!   3. prepare_effect     — compute mutation patch / decision (pure)
//!   4. reserve            — Ledger.ReserveSet (atomic with audit_decision)
//!   5. audit_decision     — folded into reserve via Ledger.audit_outbox (Stage 2 §4)
//!   6. publish_effect     — Adapter applies mutation (idempotent via effect_hash)
//!   7. commit_or_release  — Ledger.CommitEstimated / Release
//!   8. audit_outcome      — folded into commit_or_release via audit_outbox
//!
//! POC scope: stages 1-4 + 6 (publish_effect handled inline by handler).
//! Stages 5 + 8 are folded into ledger writes per Stage 2 §4.
//! Stage 7 commit path (`run_commit_estimated`) is implemented for the
//! CommitEstimated lane (Phase 2B Step 7); Release + ProviderReport are
//! deferred to a future slice.

use std::collections::HashMap;

use chrono::Utc;
use prost::Message as _;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    config::Config,
    contract,
    domain::{
        error::DomainError,
        state::{CachedReleaseSignature, ReservationCtx, SidecarState},
    },
    proto::{
        common::v1::{
            BudgetClaim, CloudEvent, ContractBundleRef, Fencing, Idempotency, LockOrderToken,
            PricingFreeze, UnitRef,
        },
        ledger::v1::{
            commit_estimated_response::Outcome as CommitOutcome,
            query_decision_outcome_response::Stage as QueryDecisionStage,
            query_reservation_context_response::Outcome as QrcOutcome,
            record_denied_decision_response::Outcome as DeniedOutcome,
            release_request::Reason as ReleaseReasonProto,
            release_response::Outcome as ReleaseOutcome, reserve_set_response::Outcome,
            CommitEstimatedRequest, CommitEstimatedResponse, QueryDecisionOutcomeRequest,
            QueryReservationContextRequest, RecordDeniedDecisionRequest,
            RecordDeniedDecisionResponse, ReleaseRequest, ReleaseResponse, ReserveSetRequest,
            ReserveSetResponse,
        },
        sidecar_adapter::v1::{
            decision_response::Decision, ClaimEstimate, DecisionRequest, DecisionResponse,
            LlmCallPostPayload,
        },
    },
};

pub struct DecisionContext {
    pub session_id: String,
    pub workload_instance_id: String,
    pub tenant_id: String,
    pub region: String,
}

/// Cost Advisor P0.5 enrichment fields threaded from `DecisionRequest`
/// into the emitted audit.decision CloudEvent. All four fields default
/// to empty string when absent — this is a degraded-but-valid state
/// (Cost Advisor rules treat empty strings as "field not enriched" and
/// don't fire on those rows; see
/// `docs/specs/cost-advisor-p0-audit-report.md` §8.5).
///
/// Only the audit.decision emission carries enrichment in P0.5. The
/// audit.outcome (commit_estimated / release) emissions stay sparse:
/// Cost Advisor rules JOIN by decision_id to pull enrichment from the
/// matching decision row, so duplicating fields on outcome would waste
/// payload bytes without changing rule behavior.
#[derive(Debug, Default, Clone)]
pub(crate) struct AuditEnrichment {
    pub run_id: String,
    pub agent_id: String,
    pub model_family: String,
    pub prompt_hash: String,
    /// GH #77 — additive JSONB sub-object emitted into
    /// `payload_json.data.spendguard.*` for the audit.decision
    /// CloudEvent. Populated from `req.inputs.runtime_metadata`
    /// fields under the `spendguard.*` namespace via a strict
    /// 12-key allowlist (no arbitrary PII can be smuggled into the
    /// signed audit chain). Empty `Value::Null` = SDK didn't send
    /// enrichment (legacy / non-LiteLLM integration).
    pub spendguard_context: serde_json::Value,
}

/// GH #77 — allowlisted enrichment keys that the SDK may pass via
/// `runtime_metadata.fields`. Values must be string-typed; non-string
/// or unknown keys are silently dropped (fail-closed against SDK
/// drift or PII smuggling). See `docs/specs/litellm-integration/
/// DESIGN.md` §8.2a for the LiteLLM-specific 12-field contract.
const SPENDGUARD_ENRICHMENT_ALLOWLIST: &[&str] = &[
    "integration",
    "litellm_call_id",
    "model",
    "pricing_version",
    "price_snapshot_hash_hex",
    "fx_rate_version",
    "unit_conversion_version",
    "prompt_hash",
    "call_type",
    "stream",
    "mode",
    "team_id",
];

const RELEASE_SIGNATURE_CACHE_MIN_TTL_SECONDS: i64 = 600;
const RELEASE_SIGNATURE_CACHE_MAX_TTL_SECONDS: i64 = 3600;

/// Extract the four enrichment fields from a `DecisionRequest`. Any
/// missing field becomes empty string (degraded path).
///
/// - `run_id` ← `req.ids.run_id` (SpendGuardIds proto, common.v1).
/// - `agent_id` ← `req.ids.step_id` — Cost Advisor uses step_id as
///   the agent identifier; "step_id" is the canonical name in
///   SpendGuard's trace schema, but cost_advisor rules group by
///   "agent_id" (the customer-facing term per spec §4.0). The
///   mapping is intentional.
/// - `model_family` ← `req.inputs.projected_unit.model_family`
///   (TOKEN units carry it per `common.v1.UnitRef` proto). MONETARY
///   units leave it empty — cost_advisor only meaningfully reasons
///   about model_family for token-scoped rules.
/// - `prompt_hash` ← `req.inputs.runtime_metadata.fields["prompt_hash"]`
///   if present and `string_value`. Adapters (Pydantic-AI etc.)
///   compute via `services/sidecar/src/prompt_hash.rs::compute` and
///   pass through `runtime_metadata` per the proto's
///   "free-form runtime metadata" comment.
pub(crate) fn extract_enrichment(req: &DecisionRequest) -> AuditEnrichment {
    let ids = req.ids.as_ref();

    // Codex r1 P2-2 fix: run_id flows into canonical_events.run_id
    // (UUID column). If the adapter sends garbage, canonical_ingest's
    // strict UUID parser would QUARANTINE the row instead of persisting.
    // Validate at the sidecar boundary: if parse fails, treat as empty.
    let run_id = ids
        .map(|i| i.run_id.clone())
        .filter(|s| Uuid::parse_str(s).is_ok())
        .unwrap_or_default();

    // Codex r1 P3-1 fix: step_id whitespace-only would let rules group
    // findings under " " instead of skipping. Treat whitespace-only as
    // empty. Exact non-blank step_ids pass through unchanged.
    let agent_id = ids
        .map(|i| i.step_id.clone())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_default();

    let inputs = req.inputs.as_ref();
    let model_family = inputs
        .and_then(|i| i.projected_unit.as_ref())
        .map(|u| u.model_family.clone())
        .unwrap_or_default();

    let prompt_hash = inputs
        .and_then(|i| i.runtime_metadata.as_ref())
        .and_then(|m| m.fields.get("prompt_hash"))
        .and_then(|v| match v.kind.as_ref() {
            Some(prost_types::value::Kind::StringValue(s)) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_default();

    // GH #77 — extract allowlisted spendguard.* enrichment keys.
    // Slice C1 R1 P0 fix (Backend Architect): SDK sends bool / None
    // values through proto Struct as BoolValue / NullValue (verified:
    // `"stream": bool(...)` in client decision_context_json). The
    // initial StringValue-only filter silently dropped these, locking
    // in broken-shape signed CloudEvent payloads.
    //
    // Allowed coercions per `100-percent-design.md` §Epic C lines 254-257:
    //   StringValue   → Value::String
    //   BoolValue     → Value::Bool
    //   NumberValue   → Value::from(f64) (architect mandate; no
    //                   current 12-field uses numbers but forward-compat)
    //   NullValue     → Value::Null (preserve explicit null semantics)
    //   StructValue | ListValue → DROP with WARN (PII smuggling guard)
    //
    // Unknown keys (outside the 12-key allowlist) → DROP with WARN.
    // Empty map → Null so the CloudEvent payload omits the sub-object
    // (backward compat for legacy pre-GH#77 adapters).
    let spendguard_context = inputs
        .and_then(|i| i.runtime_metadata.as_ref())
        .map(|m| {
            let mut obj = serde_json::Map::new();
            for &key in SPENDGUARD_ENRICHMENT_ALLOWLIST {
                if let Some(v) = m.fields.get(key) {
                    match v.kind.as_ref() {
                        Some(prost_types::value::Kind::StringValue(s)) => {
                            obj.insert(key.into(), serde_json::Value::String(s.clone()));
                        }
                        Some(prost_types::value::Kind::BoolValue(b)) => {
                            obj.insert(key.into(), serde_json::Value::Bool(*b));
                        }
                        Some(prost_types::value::Kind::NumberValue(n)) => {
                            // serde_json::Number from f64 — None only
                            // for NaN/Inf which the SDK never sends.
                            if let Some(num) = serde_json::Number::from_f64(*n) {
                                obj.insert(key.into(), serde_json::Value::Number(num));
                            }
                        }
                        Some(prost_types::value::Kind::NullValue(_)) => {
                            obj.insert(key.into(), serde_json::Value::Null);
                        }
                        Some(other) => {
                            // StructValue / ListValue — fail-closed
                            // against PII smuggling per architect NG2.
                            tracing::warn!(
                                key = key,
                                kind = ?std::mem::discriminant(other),
                                "spendguard enrichment: dropping \
                                 allowlisted key with non-scalar kind \
                                 (StructValue/ListValue)",
                            );
                        }
                        None => {
                            // Empty Value (no kind set) — silently skip.
                        }
                    }
                }
            }
            // Architect P0: WARN on unknown keys (PII smuggling guard).
            // We don't iterate the full m.fields map for performance;
            // adapters that drift add a new field that's known to be
            // intentional via the spec process. Per architect, debug-
            // level rate-limited log on unknown keys is sufficient.
            for (k, _v) in m.fields.iter() {
                if !SPENDGUARD_ENRICHMENT_ALLOWLIST.contains(&k.as_str())
                    // Cost Advisor P0.5 keys that DO belong elsewhere
                    // (already extracted as top-level fields), not a
                    // smuggling signal:
                    && k != "prompt_hash"
                {
                    tracing::debug!(
                        unknown_key = %k,
                        "spendguard enrichment: dropped key not in \
                         allowlist (DESIGN.md §8.2a)",
                    );
                }
            }
            if obj.is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::Value::Object(obj)
            }
        })
        .unwrap_or_default();

    AuditEnrichment {
        run_id,
        agent_id,
        model_family,
        prompt_hash,
        spendguard_context,
    }
}

#[derive(Debug)]
pub struct DecisionOutput {
    pub decision_id: Uuid,
    pub audit_decision_event_id: Uuid,
    pub effect_hash: [u8; 32],
    pub decision: Decision,
    pub reservation_set_id: String,
    pub reservation_ids: Vec<String>,
    pub ledger_transaction_id: String,
    /// Round-2 #9 part 2: when decision = REQUIRE_APPROVAL, the
    /// approval_id minted (or replayed) by the ledger's
    /// `post_approval_required_decision` SP. Empty for other decision
    /// kinds.
    pub approval_request_id: String,
    /// Reservation TTL — adapter MUST commit/release before this deadline.
    /// None when decision != CONTINUE (no reservation).
    pub ttl_expires_at: Option<prost_types::Timestamp>,
    /// Phase 3 wedge: contract evaluator outputs. Carried through to
    /// DecisionResponse so adapter sees which rules fired and why.
    pub matched_rule_ids: Vec<String>,
    pub reason_codes: Vec<String>,
    /// SLICE_09 Phase E: which RUN_* code (if any) drove this decision.
    /// One of "" | "RUN_BUDGET_PROJECTION_EXCEEDED" | "RUN_DRIFT_DETECTED"
    /// | "RUN_STEPS_EXCEEDED" per contract-dsl-v1alpha2 §3.x. Carried into
    /// DecisionResponse.run_code_triggered tag 16 (SLICE_02 wired the
    /// field; SLICE_09 populates the value).
    pub run_code_triggered: String,
    /// D13 COV_61 — Optional subscription meter snapshot.  Populated
    /// only when the request was routed through the meter-only path
    /// (DecisionRequest.reservation_source == SUBSCRIPTION_METER).
    /// build_response() forwards this verbatim into DecisionResponse
    /// tag 17.
    pub subscription_meter: Option<crate::proto::common::v1::SubscriptionMeter>,
}

/// SLICE_09 Phase E — projector call with built-in fall-through.
///
/// Returns `Ok(ProjectResponse)` on a successful Project RPC, OR
/// `Err(DomainError)` for any of: client = None (Helm not configured),
/// client.project returns Err (timeout, network, validation, etc.).
///
/// The caller (run_through_reserve) treats Err as "projector unreachable"
/// per spec §10 — no RUN_* code emitted, audit row uses sentinels.
///
/// Inputs derive from `req` + `claims` + `enrichment`:
///   * `this_call_reservation_atomic` = sum of `claim.amount_atomic`
///     across all projected_claims (parsed to i64; clamps on overflow).
///     This is what the per-call reservation would be if we accepted
///     unchanged; projector uses it as Signal 2's baseline.
///   * `budget_remaining_atomic` is read from explicit runtime metadata only
///     when the demo/test `allow_untrusted_budget_metadata` gate is enabled.
///     Production keeps that gate false until the sidecar derives budget
///     remaining from a signed/fenced ledger snapshot. Missing, malformed, or
///     disabled snapshots fall back to the unknown-budget sentinel so the
///     projector records trajectory without trusting caller-controlled budget
///     values. The ledger reserve path remains the hard budget oracle.
async fn call_projector_safe(
    state: &SidecarState,
    ctx: &DecisionContext,
    req: &DecisionRequest,
    claims: &[BudgetClaim],
    enrichment: &AuditEnrichment,
    adapter_idempotency_key: &str,
) -> Result<crate::proto::run_cost_projector::v1::ProjectResponse, DomainError> {
    use num_bigint::BigInt;
    use std::str::FromStr;

    let client = state
        .inner
        .run_cost_projector
        .as_ref()
        .ok_or_else(|| DomainError::DecisionStage("projector client not configured".into()))?;

    // Sum claims for the per-call reservation amount.
    let mut total = BigInt::from(0i64);
    for c in claims {
        if let Ok(v) = BigInt::from_str(&c.amount_atomic) {
            total += v;
        }
    }
    // Clamp to i64 — projector proto is int64. Per audit-extension §3.3
    // round-2 M5: NUMERIC values exceeding 2^63-1 are constraint-rejected
    // at audit_outbox INSERT; we cap silently here so the projector
    // doesn't see a wraparound.
    let this_call_atomic: i64 = i64::try_from(total).unwrap_or(i64::MAX);

    let budget_remaining = projector_budget_remaining_atomic(state, ctx, req, claims).await;
    // Signal 3 hint: passed through from DecisionRequest.planned_steps_hint
    // (SLICE_09 additive proto field; SLICE_12 SDK with_run_plan decorator
    // populates it).
    let planned_steps_hint = req.planned_steps_hint.max(0);

    let project_req = crate::proto::run_cost_projector::v1::ProjectRequest {
        tenant_id: ctx.tenant_id.clone(),
        run_id: enrichment.run_id.clone(),
        agent_id: enrichment.agent_id.clone(),
        model: enrichment.model_family.clone(),
        step_id: req
            .ids
            .as_ref()
            .map(|i| i.step_id.clone())
            .unwrap_or_default(),
        decision_id: projector_decision_id_from_idempotency_key(adapter_idempotency_key),
        this_call_reservation_atomic: this_call_atomic,
        unit_id: claims
            .first()
            .and_then(|c| c.unit.as_ref())
            .map(|u| u.unit_id.clone())
            .unwrap_or_default(),
        budget_remaining_atomic: budget_remaining,
        planned_steps_hint,
        planned_tools_hint: 0,
    };

    client.project(project_req).await
}

fn projector_decision_id_from_idempotency_key(adapter_idempotency_key: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"spendguard:run-cost-projector:decision:v1:");
    h.update(adapter_idempotency_key.as_bytes());
    hex::encode(h.finalize())
}

/// Extract the run projector's budget snapshot from DecisionRequest.
///
/// `projected_p90_atomic` is a risk-band hint, not remaining budget. The
/// only accepted budget snapshot is an explicit adapter metadata field.
/// Missing, malformed, or negative snapshots remain non-triggering so the
/// projector never invents a false `RUN_BUDGET_PROJECTION_EXCEEDED`.
async fn projector_budget_remaining_atomic(
    state: &SidecarState,
    ctx: &DecisionContext,
    req: &DecisionRequest,
    claims: &[BudgetClaim],
) -> i64 {
    if state.inner.allow_untrusted_budget_metadata {
        return projector_budget_remaining_from_runtime_metadata(req).unwrap_or(i64::MAX);
    }

    match authoritative_budget_remaining_atomic(state, ctx, claims).await {
        Some(value) => value,
        None => i64::MAX,
    }
}

fn projector_budget_remaining_from_runtime_metadata(req: &DecisionRequest) -> Option<i64> {
    req.inputs
        .as_ref()
        .and_then(|i| i.runtime_metadata.as_ref())
        .and_then(|m| {
            ["budget_remaining_atomic", "run_budget_remaining_atomic"]
                .iter()
                .find_map(|key| {
                    m.fields
                        .get(*key)
                        .and_then(nonnegative_i64_from_struct_value)
                })
        })
}

async fn authoritative_budget_remaining_atomic(
    state: &SidecarState,
    ctx: &DecisionContext,
    claims: &[BudgetClaim],
) -> Option<i64> {
    let mut min_remaining: Option<i64> = None;
    for claim in claims {
        if claim.direction == crate::proto::common::v1::budget_claim::Direction::Credit as i32 {
            continue;
        }
        let unit_id = claim.unit.as_ref()?.unit_id.clone();
        let now = Utc::now();
        let response = state
            .inner
            .ledger
            .query_budget_state(crate::proto::ledger::v1::QueryBudgetStateRequest {
                tenant_id: ctx.tenant_id.clone(),
                budget_id: claim.budget_id.clone(),
                window_instance_id: claim.window_instance_id.clone(),
                snapshot_at: Some(prost_types::Timestamp {
                    seconds: now.timestamp(),
                    nanos: now.timestamp_subsec_nanos() as i32,
                }),
                unit_id,
            })
            .await
            .ok()?;
        let remaining = nonnegative_i64_from_decimal_str(&response.available_atomic)?;
        min_remaining = Some(match min_remaining {
            Some(current) => current.min(remaining),
            None => remaining,
        });
    }
    min_remaining
}

fn nonnegative_i64_from_decimal_str(raw: &str) -> Option<i64> {
    raw.trim().parse::<i64>().ok().map(|value| value.max(0))
}

fn nonnegative_i64_from_struct_value(value: &prost_types::Value) -> Option<i64> {
    match value.kind.as_ref()? {
        prost_types::value::Kind::StringValue(raw) => {
            raw.trim().parse::<i64>().ok().filter(|v| *v >= 0)
        }
        prost_types::value::Kind::NumberValue(raw) => {
            if raw.is_finite() && raw.fract() == 0.0 && *raw >= 0.0 && *raw <= i64::MAX as f64 {
                Some(*raw as i64)
            } else {
                None
            }
        }
        _ => None,
    }
}

async fn replay_existing_decision_by_idempotency(
    state: &SidecarState,
    ctx: &DecisionContext,
    req: &DecisionRequest,
    adapter_idempotency_key: &str,
    request_fingerprint_hex: &str,
) -> Result<Option<DecisionOutput>, DomainError> {
    let response = state
        .inner
        .ledger
        .query_decision_outcome(QueryDecisionOutcomeRequest {
            tenant_id: ctx.tenant_id.clone(),
            decision_id: String::new(),
            idempotency_key: adapter_idempotency_key.to_string(),
        })
        .await?;
    let stage =
        QueryDecisionStage::try_from(response.stage).unwrap_or(QueryDecisionStage::Unspecified);
    if matches!(
        stage,
        QueryDecisionStage::NotFound | QueryDecisionStage::Unspecified
    ) {
        return Ok(None);
    }
    if response.request_fingerprint_hex.is_empty() {
        return Err(DomainError::IdempotencyConflict(format!(
            "ledger idempotency replay for key '{}' has no request fingerprint; refusing ambiguous replay before projector mutation",
            adapter_idempotency_key
        )));
    }
    if response.request_fingerprint_hex != request_fingerprint_hex {
        return Err(DomainError::IdempotencyConflict(format!(
            "DecisionRequest.idempotency.key reused with different request fingerprint (existing={}, current={})",
            response.request_fingerprint_hex, request_fingerprint_hex
        )));
    }

    let decision_id = parse_replay_uuid("decision_id", &response.decision_id)?;
    let audit_decision_event_id =
        parse_replay_uuid("audit_decision_event_id", &response.audit_decision_event_id)?;
    let decision = replay_decision_kind(
        &response.operation_kind,
        &response.final_decision,
        &response.run_code_triggered,
        &response.reason_codes,
    )?;
    let snapshot_hash = compute_snapshot_hash(req, &ctx.tenant_id);
    let effect_hash = compute_effect_hash(&snapshot_hash, decision);

    let reservation_set_id = if response.operation_kind == "reserve" {
        if response.operation_id.is_empty() {
            derive_reservation_set_id(&decision_id).to_string()
        } else {
            response.operation_id.clone()
        }
    } else {
        String::new()
    };
    let reservation_ids = if response.operation_kind == "reserve" {
        response.projection_ids.clone()
    } else {
        Vec::new()
    };

    Ok(Some(DecisionOutput {
        decision_id,
        audit_decision_event_id,
        effect_hash,
        decision,
        reservation_set_id,
        reservation_ids,
        ledger_transaction_id: response.ledger_transaction_id,
        approval_request_id: String::new(),
        ttl_expires_at: response.ttl_expires_at,
        matched_rule_ids: response.matched_rule_ids,
        reason_codes: response.reason_codes.clone(),
        run_code_triggered: if response.run_code_triggered.is_empty() {
            response
                .reason_codes
                .iter()
                .find(|code| code.starts_with("RUN_"))
                .cloned()
                .unwrap_or_default()
        } else {
            response.run_code_triggered
        },
        // D13: replays of BYOK decisions never carry a meter snapshot;
        // subscription-meter requests short-circuit before this point.
        subscription_meter: None,
    }))
}

fn parse_replay_uuid(field: &str, raw: &str) -> Result<Uuid, DomainError> {
    Uuid::parse_str(raw).map_err(|e| {
        DomainError::DecisionStage(format!(
            "ledger idempotency replay returned malformed {field} '{raw}': {e}"
        ))
    })
}

fn replay_decision_kind(
    operation_kind: &str,
    final_decision: &str,
    run_code_triggered: &str,
    reason_codes: &[String],
) -> Result<Decision, DomainError> {
    if operation_kind == "reserve" {
        return Ok(Decision::Continue);
    }
    if operation_kind != "denied_decision" {
        return Err(DomainError::DecisionStage(format!(
            "ledger idempotency replay returned unsupported operation_kind '{operation_kind}'"
        )));
    }
    let run_projection =
        !run_code_triggered.is_empty() || reason_codes.iter().any(|code| code.starts_with("RUN_"));
    match final_decision {
        "STOP" if run_projection => Ok(Decision::StopRunProjection),
        "STOP" => Ok(Decision::Stop),
        "REQUIRE_APPROVAL" => Ok(Decision::RequireApproval),
        "DEGRADE" => Ok(Decision::Degrade),
        "SKIP" => Ok(Decision::Skip),
        other => Err(DomainError::DecisionStage(format!(
            "ledger idempotency replay returned unsupported final_decision '{other}'"
        ))),
    }
}

pub fn idempotency_request_fingerprint_hex(ctx: &DecisionContext, req: &DecisionRequest) -> String {
    let mut h = Sha256::new();
    h.update(b"spendguard.sidecar.decision_idempotency_request.v1:");
    h.update((ctx.tenant_id.len() as u64).to_be_bytes());
    h.update(ctx.tenant_id.as_bytes());
    h.update((ctx.region.len() as u64).to_be_bytes());
    h.update(ctx.region.as_bytes());

    let mut encoded = Vec::with_capacity(req.encoded_len());
    req.encode(&mut encoded)
        .expect("DecisionRequest encoding to Vec cannot fail");
    h.update((encoded.len() as u64).to_be_bytes());
    h.update(&encoded);
    hex::encode(h.finalize())
}

/// Drive the decision transaction end-to-end through stage 4 (reserve +
/// atomic audit_decision). Stage 6 (publish_effect) is performed by the
/// adapter handler after this returns; that handler reads `effect_hash`
/// for idempotent re-publish on crash recovery.
pub async fn run_through_reserve(
    cfg: &Config,
    state: &SidecarState,
    ctx: &DecisionContext,
    req: &DecisionRequest,
) -> Result<DecisionOutput, DomainError> {
    if state.is_draining() {
        return Err(DomainError::Draining);
    }
    crate::bootstrap::catalog::enforce_freshness_gate(state, cfg)?;

    // D13 COV_61: subscription-meter short-circuit.
    //
    // When the egress proxy / SDK has classified the request as a
    // subscription-tier call (Claude Code Pro / Codex on ChatGPT Plus),
    // it sets `reservation_source = SUBSCRIPTION_METER`. The sidecar
    // MUST NOT open a ledger transaction for these rows — Anthropic /
    // OpenAI settle the flat fee internally and ledger_entries would
    // double-count it as a phantom dollar over the $20/mo plan.
    //
    // Instead we build an advisory `DecisionOutput` that carries the
    // meter snapshot in `subscription_meter` and returns CONTINUE (or
    // STOP when a hard-cap fires).  See
    // subscription_meter::route_decision_request for the full lane.
    if req.reservation_source
        == crate::proto::common::v1::ReservationSource::SubscriptionMeter as i32
    {
        return crate::subscription_meter::route_decision_request(cfg, state, ctx, req);
    }

    // Validate the adapter idempotency key before any mutating downstream
    // call. The run-cost projector Project RPC mutates per-run state, so
    // retries must carry a stable key that Project can use as its own
    // idempotency discriminator.
    let adapter_idempotency_key = req
        .idempotency
        .as_ref()
        .ok_or_else(|| DomainError::InvalidRequest("DecisionRequest.idempotency required".into()))?
        .key
        .clone();
    if adapter_idempotency_key.is_empty() {
        return Err(DomainError::InvalidRequest(
            "DecisionRequest.idempotency.key required".into(),
        ));
    }

    // Durable replay check before the run-cost projector. Project mutates
    // per-run state, so cache loss or sidecar restart must not let a retry
    // recompute a different RUN_* decision before the ledger can replay the
    // first decision for this adapter idempotency key.
    let request_fingerprint_hex = idempotency_request_fingerprint_hex(ctx, req);
    if let Some(replay) = replay_existing_decision_by_idempotency(
        state,
        ctx,
        req,
        &adapter_idempotency_key,
        &request_fingerprint_hex,
    )
    .await?
    {
        return Ok(replay);
    }

    // Stage 1: snapshot
    let _snapshot_id = Uuid::now_v7();
    let snapshot_hash = compute_snapshot_hash(req, &ctx.tenant_id);

    // Stage 2: evaluate (Phase 3 wedge — real contract evaluator).
    //
    // Reads parsed Contract DSL from the cached bundle and applies rules
    // to the incoming claims. Open-by-default: no rule matches → CONTINUE.
    // Restrictive rules opt-in via explicit when/then blocks (POC subset
    // of §6 / §7; CEL deferred).
    let claims = build_budget_claims(req)?;
    let bundle = state
        .inner
        .contract_bundle
        .read()
        .clone()
        .ok_or_else(|| DomainError::DecisionStage("no contract bundle loaded".into()))?;

    let eval_outcome = contract::evaluate(&bundle.parsed, &claims);

    // Cost Advisor P0.5 enrichment used early so projector call can carry
    // run_id / agent_id even on the DENY path (where enrichment is also
    // pulled inside run_record_denied_decision below — both call sites
    // extract from the same DecisionRequest so the values match).
    let enrichment = extract_enrichment(req);

    // ── SLICE_09 Phase E: run_cost_projector wire ──────────────────────
    //
    // Spec ref `run-cost-projector-spec-v1alpha1.md` §1.2 + §10.
    //
    // Call the projector right after the per-call evaluator returns.
    // The projector emits at most one RUN_* code; we project that onto a
    // v1alpha1 lattice decision per contract.prediction_policy
    // (contract-dsl-v1alpha2 §3.4 + §5.3 allowed-pairs table). The
    // projector outcome merges into eval_outcome via the standard
    // most-restrictive lattice (CONTINUE < SKIP < DEGRADE <
    // REQUIRE_APPROVAL < STOP).
    //
    // Failure modes per spec §10:
    //   * projector client = None (Helm not configured) → no projection;
    //     reason_codes unchanged; audit row gets -1 sentinel for
    //     `run_predicted_remaining_steps` (audit-chain-extension §3.3).
    //   * projector.Project Err (timeout, connect, validation) → same as
    //     above + metric `projector_unreachable` would be incremented
    //     (Phase F surfaces the metric counter).
    //
    // The "additive only" property means a v1alpha1 sidecar running the
    // legacy code path (no projector linked) produces byte-identical
    // audit rows to before Phase E — the only NEW field populated when
    // the projector IS wired is `reason_codes` (plus the 3 audit columns
    // wired to the same proto envelope).
    let projector_result = call_projector_safe(
        state,
        ctx,
        req,
        &claims,
        &enrichment,
        &adapter_idempotency_key,
    )
    .await;
    let projector_response_ref = projector_result.as_ref().ok();
    let combined_outcome = match projector_response_ref {
        Some(resp) if !resp.emitted_code.is_empty() => {
            match contract::apply_projector_code(&bundle.parsed, &resp.emitted_code) {
                Some(projector_outcome) => {
                    contract::merge_outcomes(eval_outcome, projector_outcome)
                }
                None => eval_outcome,
            }
        }
        _ => eval_outcome,
    };
    let decision_kind = combined_outcome.decision;
    let matched_rules = combined_outcome.matched_rule_ids;
    let reason_codes = combined_outcome.reason_codes;

    // SLICE_09 Phase E: surface the RUN_* code that drove the decision (if
    // any) on the wire-level DecisionResponse.run_code_triggered field
    // (tag 16; SLICE_02 added the field, SLICE_09 populates it).
    let run_code_triggered = projector_response_ref
        .map(|r| r.emitted_code.clone())
        .unwrap_or_default();

    // Stage 3: prepare_effect (pure)
    let effect_hash = compute_effect_hash(&snapshot_hash, decision_kind);

    // Stage 4: split on decision kind.
    let decision_id = Uuid::now_v7();
    let audit_decision_event_id = Uuid::now_v7();
    // Producer sequence is bootstrapped from ledger replay at startup so
    // a restart does NOT collide with previously-emitted audit_outbox rows
    // (Stage 2 §4.3 — UNIQUE per (tenant, workload_instance_id, sequence)).
    let producer_sequence = state.next_producer_sequence();

    let pricing = PricingFreeze {
        pricing_version: bundle.pricing_version.clone(),
        price_snapshot_hash: bundle.price_snapshot_hash.clone().into(),
        fx_rate_version: bundle.fx_rate_version.clone(),
        unit_conversion_version: bundle.unit_conversion_version.clone(),
    };

    let fencing_state = state
        .inner
        .fencing
        .read()
        .clone()
        .ok_or_else(|| DomainError::FencingAcquire("no active fencing scope".into()))?;
    let fencing = Fencing {
        epoch: fencing_state.epoch,
        scope_id: fencing_state.scope_id.to_string(),
        workload_instance_id: ctx.workload_instance_id.clone(),
    };
    let schema_bundle_id = active_schema_bundle_id(state);

    // Cost Advisor P0.5 enrichment: extracted ONCE above for projector
    // call. Same value flows into both CONTINUE + DENY audit.decision
    // emissions below per the pre-SLICE_09 invariant.

    // Phase 3 wedge: branch CONTINUE vs DENY before building the
    // reserve-specific payload. DENY skips Reserve entirely but still
    // emits an audit_decision row via Ledger.RecordDeniedDecision so
    // Contract §6.1 invariant 「無 audit 則無 effect」 holds.
    if decision_kind != Decision::Continue {
        // SLICE_10 Phase C: thread the egress_proxy/SDK ClaimEstimate
        // through the DENY lane so the audit row also carries the 17
        // prediction columns. None = legacy SDK wrapper path.
        let claim_estimate_for_deny = extract_claim_estimate(req);
        return run_record_denied_decision(
            state,
            ctx,
            &decision_id,
            &audit_decision_event_id,
            producer_sequence,
            &snapshot_hash,
            decision_kind,
            &matched_rules,
            &reason_codes,
            &claims,
            &pricing,
            &fencing,
            &bundle,
            &schema_bundle_id,
            &adapter_idempotency_key,
            &request_fingerprint_hex,
            effect_hash,
            &enrichment,
            projector_response_ref,
            claim_estimate_for_deny,
        )
        .await;
    }

    let idempotency = Idempotency {
        key: adapter_idempotency_key.clone(),
        // Leave empty so the ledger computes its canonical hash server-side
        // and uses THAT for replay verification (see
        // services/ledger/src/handlers/reserve_set.rs `canonical_request_hash`).
        // The ledger's canonical covers tenant + decision + audit_event +
        // claims + pricing + fencing + ttl + contract_bundle. Recomputing
        // it here would require re-implementing the same canonicalization;
        // empty signals "let server own this".
        request_hash: Vec::new().into(),
    };

    // SLICE_02 §6: thread the contract's prediction_policy into the
    // audit CloudEvent so canonical_ingest mirrors it onto the
    // audit_outbox.prediction_policy_used column. For v1alpha1
    // contracts default-filled to STRICT_CEILING this emits the
    // literal "STRICT_CEILING" (spec §4.1 conservative default).
    let prediction_policy_str = bundle.parsed.prediction_policy.as_str();
    // SLICE_10 Phase C: pick up egress_proxy / SDK supplied
    // ClaimEstimate (None if caller is legacy / pre-SLICE_10).
    let claim_estimate_ref = extract_claim_estimate(req);
    let mut cloudevent = build_audit_decision_cloudevent(
        ctx,
        &decision_id,
        &audit_decision_event_id,
        producer_sequence,
        &snapshot_hash,
        &matched_rules,
        &reason_codes,
        &enrichment,
        prediction_policy_str,
        &schema_bundle_id,
        &request_fingerprint_hex,
        projector_response_ref,
        claim_estimate_ref,
    );
    crate::audit::sign_cloudevent_in_place(&*state.inner.signer, &mut cloudevent).await?;

    let request = ReserveSetRequest {
        tenant_id: ctx.tenant_id.clone(),
        decision_id: decision_id.to_string(),
        audit_decision_event_id: audit_decision_event_id.to_string(),
        producer_sequence,
        idempotency: Some(idempotency),
        fencing: Some(fencing),
        claims,
        lock_order_token: None, // server derives
        // TTL from config (Codex TTL r1 P1.4). Default 600s; demo
        // ttl_sweep overrides to 5s. Phase 2 derives TTL from the
        // matched contract rule's `reservation.ttl` field (Contract §7).
        ttl_expires_at: Some(prost_types::Timestamp {
            seconds: (Utc::now() + chrono::Duration::seconds(state.inner.reservation_ttl_seconds))
                .timestamp(),
            nanos: 0,
        }),
        audit_event: Some(cloudevent),
        pricing: Some(pricing),
        contract_bundle: Some(ContractBundleRef {
            bundle_id: bundle.bundle_id.to_string(),
            bundle_hash: bundle.bundle_hash.clone().into(),
            bundle_signature: vec![].into(), // POC: omit sidecar-side bundle sig
            signing_key_id: bundle.signing_key_id.clone(),
        }),
    };

    let response: ReserveSetResponse = state.inner.ledger.reserve_set(request).await?;
    match response.outcome {
        Some(Outcome::Success(s)) => {
            // Mirror the server's TTL anchor (the value the ledger stored
            // alongside the reservations) instead of recomputing locally.
            // For concurrent same-key races this is what makes the loser's
            // Replay response byte-equivalent to the winner's Success
            // response (winner's TTL is what's stored; both branches mirror
            // from server).
            let ttl_from_server = s
                .reservations
                .first()
                .and_then(|r| r.ttl_expires_at.clone());

            // Phase 2B Step 7 — populate reservation_cache so the LLM_CALL_POST
            // commit path hits hot. Cache miss falls back to
            // Ledger.QueryReservationContext (durable recovery).
            populate_reservation_cache(
                state,
                ctx,
                &s.reservations,
                &fencing_state,
                &decision_id,
                &bundle,
            );

            Ok(DecisionOutput {
                decision_id,
                audit_decision_event_id,
                effect_hash,
                decision: decision_kind,
                reservation_set_id: s.reservation_set_id,
                reservation_ids: s
                    .reservations
                    .iter()
                    .map(|r| r.reservation_id.clone())
                    .collect(),
                ledger_transaction_id: s.ledger_transaction_id,
                approval_request_id: String::new(),
                ttl_expires_at: ttl_from_server,
                matched_rule_ids: matched_rules.clone(),
                reason_codes: reason_codes.clone(),
                run_code_triggered: run_code_triggered.clone(),
                subscription_meter: None,
            })
        }
        Some(Outcome::Replay(r)) => {
            // Replay variant MUST surface the ORIGINAL identifiers (per
            // Contract §6 / Ledger §7 idempotency — same idempotency_key
            // must yield same decision_id + audit chain across retries).
            // Sidecar mints fresh `decision_id` / `audit_decision_event_id`
            // on every call; on Replay we discard those and use the ledger
            // row's original values.
            //
            // Fail-closed on malformed UUIDs: silently falling back to the
            // freshly-minted ids (or Uuid::nil) would corrupt the audit
            // chain. The ledger writes both fields NOT-NULL on INSERT and
            // build_replay_response refuses to return Replay when either
            // is NULL, so well-formed input is guaranteed; we still parse
            // here to convert wire-format strings.
            let original_audit_id =
                uuid::Uuid::parse_str(&r.audit_decision_event_id).map_err(|e| {
                    DomainError::DecisionStage(format!(
                        "ledger replay returned malformed audit_decision_event_id '{}': {}",
                        r.audit_decision_event_id, e
                    ))
                })?;
            let original_decision_id = uuid::Uuid::parse_str(&r.decision_id).map_err(|e| {
                DomainError::DecisionStage(format!(
                    "ledger replay returned malformed decision_id '{}': {}",
                    r.decision_id, e
                ))
            })?;
            Ok(DecisionOutput {
                decision_id: original_decision_id,
                audit_decision_event_id: original_audit_id,
                effect_hash,
                decision: decision_kind,
                reservation_set_id: r.operation_id.clone(),
                // Original projection ids from the first call, in claim-
                // ordinal order — non-empty for ReserveSet replays.
                reservation_ids: r.projection_ids.clone(),
                ledger_transaction_id: r.ledger_transaction_id,
                approval_request_id: String::new(),
                // Original TTL anchor from first call (mirrored from the
                // server-stored value, not recomputed).
                ttl_expires_at: r.ttl_expires_at.clone(),
                matched_rule_ids: matched_rules.clone(),
                reason_codes: reason_codes.clone(),
                run_code_triggered: run_code_triggered.clone(),
                subscription_meter: None,
            })
        }
        Some(Outcome::Error(e)) => Err(DomainError::DecisionStage(format!(
            "ReserveSet error code={} msg={}",
            e.code, e.message
        ))),
        None => Err(DomainError::DecisionStage(
            "ReserveSet response empty oneof".into(),
        )),
    }
}

fn compute_snapshot_hash(req: &DecisionRequest, tenant_id: &str) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"snapshot:v1:");
    h.update(tenant_id.as_bytes());
    h.update(req.session_id.as_bytes());
    h.update(req.route.as_bytes());
    if let Some(ids) = &req.ids {
        h.update(ids.run_id.as_bytes());
        h.update(ids.step_id.as_bytes());
        h.update(ids.llm_call_id.as_bytes());
        h.update(ids.tool_call_id.as_bytes());
        h.update(ids.decision_id.as_bytes());
    }
    h.update(&[req.trigger as u8]);
    h.finalize().into()
}

fn compute_effect_hash(snapshot_hash: &[u8; 32], decision: Decision) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"effect:v1:");
    h.update(snapshot_hash);
    h.update(&[decision as u8]);
    h.finalize().into()
}

fn build_budget_claims(req: &DecisionRequest) -> Result<Vec<BudgetClaim>, DomainError> {
    let inputs = req
        .inputs
        .as_ref()
        .ok_or_else(|| DomainError::InvalidRequest("DecisionRequest.inputs required".into()))?;
    let mut out = Vec::with_capacity(inputs.projected_claims.len());
    for c in &inputs.projected_claims {
        out.push(c.clone());
    }
    if out.is_empty() {
        return Err(DomainError::InvalidRequest(
            "DecisionRequest.inputs.projected_claims must be non-empty".into(),
        ));
    }
    Ok(out)
}

/// SLICE_10 Phase C — read the egress_proxy / SDK supplied ClaimEstimate
/// off the DecisionRequest (additive proto field; absent on legacy
/// callers). Returns None when the caller did not supply a ClaimEstimate
/// — sidecar then falls back to the existing per-field defaults (proto3
/// zero = SQL NULL via prediction-mirror).
fn extract_claim_estimate(
    req: &DecisionRequest,
) -> Option<&crate::proto::sidecar_adapter::v1::ClaimEstimate> {
    req.inputs.as_ref().and_then(|i| i.claim_estimate.as_ref())
}

fn active_schema_bundle_id(state: &SidecarState) -> String {
    state
        .inner
        .schema_bundle
        .read()
        .as_ref()
        .map(|s| s.bundle_id.to_string())
        .unwrap_or_default()
}

fn insert_claim_estimate_payload_mirrors(payload: &mut serde_json::Value, est: &ClaimEstimate) {
    let Some(obj) = payload.as_object_mut() else {
        return;
    };
    if !est.model.is_empty() {
        obj.insert("model".into(), serde_json::Value::String(est.model.clone()));
    }
    if !est.prompt_class.is_empty() {
        obj.insert(
            "prompt_class".into(),
            serde_json::Value::String(est.prompt_class.clone()),
        );
    }
    if !est.prompt_class_fingerprint.is_empty() {
        obj.insert(
            "prompt_class_fingerprint".into(),
            serde_json::Value::String(est.prompt_class_fingerprint.clone()),
        );
    }
}

fn build_audit_decision_cloudevent(
    ctx: &DecisionContext,
    decision_id: &Uuid,
    audit_decision_event_id: &Uuid,
    producer_sequence: u64,
    snapshot_hash: &[u8; 32],
    matched_rules: &[String],
    reason_codes: &[String],
    enrichment: &AuditEnrichment,
    prediction_policy: &str,
    schema_bundle_id: &str,
    request_fingerprint_hex: &str,
    projector_response: Option<&crate::proto::run_cost_projector::v1::ProjectResponse>,
    claim_estimate: Option<&crate::proto::sidecar_adapter::v1::ClaimEstimate>,
) -> CloudEvent {
    let mut payload = serde_json::json!({
        "snapshot_hash":   hex::encode(snapshot_hash),
        "matched_rules":   matched_rules,
        "reason_codes":    reason_codes,
        "idempotency_request_fingerprint": request_fingerprint_hex,
        "session_id":      ctx.session_id,
        // Cost Advisor P0.5 enrichment fields. Empty strings indicate
        // the SDK adapter did not provide enrichment for this call —
        // rules treat empties as "not classified" and don't fire on
        // those rows. See audit-report §8.5.
        "agent_id":        enrichment.agent_id,
        "model_family":    enrichment.model_family,
        "prompt_hash":     enrichment.prompt_hash,
    });
    // GH #77 — emit the LiteLLM 12-field enrichment as a nested
    // `spendguard` sub-object iff the SDK sent any allowlisted keys.
    // Backward compat: legacy adapters that don't send `spendguard.*`
    // produce Null here, which we skip to keep the payload identical
    // to pre-GH#77 emissions (no signature drift on cached rows).
    if !enrichment.spendguard_context.is_null() {
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("spendguard".into(), enrichment.spendguard_context.clone());
        }
    }
    if let Some(est) = claim_estimate {
        insert_claim_estimate_payload_mirrors(&mut payload, est);
    }
    let payload_bytes =
        serde_json::to_vec(&payload).expect("snapshot json serialization is infallible");
    // SLICE_02 §6 audit-chain impact: populate prediction_policy_used
    // (CloudEvent proto tag 305) on every spendguard.audit.decision
    // emission. SLICE_01 added the audit_outbox column; SLICE_02 wires
    // the value at producer time so canonical_ingest's mirror path
    // (services/canonical_ingest/src/persistence/append.rs) writes it
    // to the column, the trigger function reject_audit_outbox_immutable_columns
    // protects it, and verify-chain confirms it matches the
    // cloudevent_payload_signature.
    //
    // For v1alpha1 contracts default-filled to STRICT_CEILING, this
    // emits the literal "STRICT_CEILING" — which is the byte-identical
    // expectation per spec §6.4 ("v1alpha1 contracts produce same
    // decision_id, reason_codes, mutation_patch_json under v1alpha2
    // evaluator"). NOTE: the audit-row prediction_policy_used VALUE
    // changes from NULL (pre-SLICE_02) to "STRICT_CEILING"
    // (post-SLICE_02). This is the intentional SLICE_02 boundary:
    // SLICE_01 deferred filling the column to SLICE_02. Per spec §8.2
    // the byte-identical regression compares the FIELDS spec calls
    // out (decision_id, reason_codes, mutation_patch_json) — not
    // every column, since `prediction_policy_used` going from NULL
    // to STRICT_CEILING is the entire point of this slice.
    let mut ce = CloudEvent {
        specversion: "1.0".into(),
        r#type: "spendguard.audit.decision".into(),
        source: format!("sidecar://{}/{}", ctx.region, ctx.workload_instance_id),
        id: audit_decision_event_id.to_string(),
        time: Some(prost_types::Timestamp {
            seconds: Utc::now().timestamp(),
            nanos: Utc::now().timestamp_subsec_nanos() as i32,
        }),
        datacontenttype: "application/json".into(),
        data: payload_bytes.into(),
        tenant_id: ctx.tenant_id.clone(),
        // Cost Advisor P0.5: was String::new(); now sourced from
        // SpendGuardIds.run_id. canonical_events.run_id COLUMN is
        // populated downstream by canonical_ingest from this envelope
        // field, unblocking run-scoped rule grouping.
        run_id: enrichment.run_id.clone(),
        decision_id: decision_id.to_string(),
        schema_bundle_id: schema_bundle_id.to_string(),
        producer_id: format!("sidecar:{}", ctx.workload_instance_id),
        producer_sequence,
        producer_signature: vec![].into(), // POC: signing TBD
        signing_key_id: String::new(),
        ..Default::default()
    };
    // SLICE_02: set tag 305 directly so the mirror path picks it up.
    // The rest of the tag 300+ prediction fields stay at proto3 default
    // (= SQL NULL via the prediction-mirror crate) until SLICE_06 wires
    // the predictor; spec §6 confirms only prediction_policy_used is
    // populated at this slice.
    ce.prediction_policy_used = prediction_policy.to_string();

    // SLICE_10 Phase C: ALL 17 prediction columns now populated from
    // the egress_proxy / SDK supplied ClaimEstimate. When the caller
    // doesn't supply one (legacy SDK wrapper, SLICE_09 path), the field
    // is None and we leave the CloudEvent fields at proto3 defaults
    // (= SQL NULL via the prediction-mirror crate) — backwards-compat
    // with pre-SLICE_10 producers.
    //
    // The tokenizer/output-predictor/cold-start columns come from
    // ClaimEstimate. The 3 run-level columns are authoritative from the
    // sidecar's projector_response when present because the sidecar is
    // the single mutating Project caller in hardened production wiring.
    // If the projector is not wired, keep ClaimEstimate's run fields for
    // legacy proxy callers; otherwise use the SLICE_09 unreachable
    // sentinels.
    if let Some(est) = claim_estimate {
        ce.predicted_a_tokens = est.predicted_a_tokens;
        ce.predicted_b_tokens = est.predicted_b_tokens;
        ce.predicted_c_tokens = est.predicted_c_tokens;
        ce.reserved_strategy = est.reserved_strategy.clone();
        ce.prediction_strategy_used = est.prediction_strategy_used.clone();
        // prediction_policy_used is authoritative from the loaded
        // contract bundle. ClaimEstimate is caller input and must not
        // rewrite the signed audit policy.
        ce.tokenizer_tier = est.tokenizer_tier.clone();
        ce.tokenizer_version_id = est.tokenizer_version_id.clone();
        ce.prediction_confidence = est.prediction_confidence;
        ce.prediction_sample_size = est.prediction_sample_size;
        ce.cold_start_layer_used = est.cold_start_layer_used.clone();

        ce.run_projection_at_decision_atomic = est.run_projection_at_decision_atomic;
        ce.run_predicted_remaining_steps = est.run_predicted_remaining_steps;
        ce.run_steps_completed_so_far = est.run_steps_completed_so_far;
    } else {
        // Projector unreachable / not wired — sentinel.
        ce.run_projection_at_decision_atomic = 0;
        ce.run_predicted_remaining_steps = -1;
        ce.run_steps_completed_so_far = 0;
    }
    if let Some(resp) = projector_response {
        ce.run_projection_at_decision_atomic = resp.run_projection_at_decision_atomic;
        ce.run_predicted_remaining_steps = resp.run_predicted_remaining_steps;
        ce.run_steps_completed_so_far = resp.run_steps_completed_so_far;
    }

    ce
}

// (Producer sequence now lives on SidecarState, initialized from ledger
// replay at startup so restarts don't collide with prior sequences.)

// =====================================================================
// Phase 3 wedge — DENY lane.
// =====================================================================

/// Map a `Decision` enum value to the canonical wire string used in the
/// DENY-lane `audit_decision.decision` column.
///
/// Per `docs/contract-dsl-spec-v1alpha2.md` §3.4 invariant: "v1alpha1
/// lattice + audit row decision field stays v1alpha1; new RUN_* codes
/// appear in reason_codes only." Therefore `Decision::StopRunProjection`
/// maps to the same `"STOP"` audit-row string as `Decision::Stop`; the
/// run-projection categorisation lives in `reason_codes` (RUN_* code),
/// not in the column string.
///
/// Returns `None` for variants that should never reach the DENY lane
/// (`Continue` is filtered out by the caller; `Unspecified` would be a
/// proto-default leak indicating an upstream bug).
///
/// Exposed at module visibility so the round-1 fix unit test can assert
/// the SLICE_02 invariant directly.
pub(super) fn denied_decision_label(decision_kind: Decision) -> Option<&'static str> {
    match decision_kind {
        Decision::Stop => Some("STOP"),
        // SLICE_02 §3.4: STOP_RUN_PROJECTION is the dashboard / SIEM
        // categorisation for a run-projection-driven stop; the audit
        // row `decision` column stays at the v1alpha1 lattice value
        // ("STOP"). The differentiator lives in `reason_codes` (RUN_*).
        Decision::StopRunProjection => Some("STOP"),
        Decision::RequireApproval => Some("REQUIRE_APPROVAL"),
        Decision::Degrade => Some("DEGRADE"),
        Decision::Skip => Some("SKIP"),
        // Continue is filtered out by caller; Unspecified should not flow.
        Decision::Continue | Decision::Unspecified => None,
    }
}

/// Stage 4 (DENY branch). Skips Reserve and writes only an audit_decision
/// row via Ledger.RecordDeniedDecision. Preserves Contract §6.1
/// invariant 「無 audit 則無 effect」 — every decision (even «no
/// effect») produces exactly one spendguard.audit.decision row.
#[allow(clippy::too_many_arguments)]
async fn run_record_denied_decision(
    state: &SidecarState,
    ctx: &DecisionContext,
    decision_id: &Uuid,
    audit_decision_event_id: &Uuid,
    producer_sequence: u64,
    snapshot_hash: &[u8; 32],
    decision_kind: Decision,
    matched_rules: &[String],
    reason_codes: &[String],
    claims: &[BudgetClaim],
    pricing: &PricingFreeze,
    fencing: &Fencing,
    bundle: &crate::domain::state::CachedContractBundle,
    schema_bundle_id: &str,
    adapter_idempotency_key: &str,
    request_fingerprint_hex: &str,
    effect_hash: [u8; 32],
    enrichment: &AuditEnrichment,
    projector_response: Option<&crate::proto::run_cost_projector::v1::ProjectResponse>,
    // SLICE_10 Phase C: ClaimEstimate flows into DENY-lane CloudEvent
    // identical to the ALLOW lane (per §6.4 byte-identical regression).
    claim_estimate: Option<&crate::proto::sidecar_adapter::v1::ClaimEstimate>,
) -> Result<DecisionOutput, DomainError> {
    // SLICE_09 Phase E: surface the RUN_* code that drove this DENY
    // (if any). Empty string = per-call DENY without projector input.
    let run_code_triggered_local: String = projector_response
        .map(|r| r.emitted_code.clone())
        .unwrap_or_default();

    let final_decision_str = denied_decision_label(decision_kind).ok_or_else(|| {
        DomainError::DecisionStage(format!(
            "run_record_denied_decision called with unsupported decision {:?}",
            decision_kind
        ))
    })?;

    // Use the adapter-supplied idempotency_key directly (no namespacing).
    // The new SP performs a cross-kind exclusivity check: if the same key
    // already won a `reserve` row, the SP refuses with IDEMPOTENCY_CONFLICT.
    // This prevents bundle hot-reload mid-retry from producing both a
    // reserve AND a denied_decision row for the same logical request
    // (Codex R1 P0). The reverse direction (DENY→CONTINUE retry) is the
    // companion gap deferred to GA — POC has no hot-reload.
    let denied_idempotency_key = adapter_idempotency_key.to_string();

    // Build CloudEvent payload. matched_rules + reason_codes + final
    // decision live inside `data` so canonical_events keeps the
    // forensics without schema changes.
    let mut payload = serde_json::json!({
        "snapshot_hash":     hex::encode(snapshot_hash),
        "matched_rules":     matched_rules,
        "reason_codes":      reason_codes,
        "idempotency_request_fingerprint": request_fingerprint_hex,
        "final_decision":    final_decision_str,
        "session_id":        ctx.session_id,
        "attempted_claims":  claims.iter().map(|c| serde_json::json!({
            "budget_id":          c.budget_id,
            "amount_atomic":      c.amount_atomic,
            "window_instance_id": c.window_instance_id,
            "unit_id":            c.unit.as_ref().map(|u| u.unit_id.clone()).unwrap_or_default(),
        })).collect::<Vec<_>>(),
        // Cost Advisor P0.5 enrichment (DENY path). Even denied
        // decisions need run_id + agent_id + prompt_hash so cost_advisor
        // can group retries that hit STOP/REQUIRE_APPROVAL — a runaway-
        // loop pattern that hammers the same prompt against a maxed-
        // out budget is still wasteful behavior worth flagging.
        "agent_id":          enrichment.agent_id,
        "model_family":      enrichment.model_family,
        "prompt_hash":       enrichment.prompt_hash,
    });
    // GH #77 (DENY path) — emit LiteLLM 12-field enrichment so deny
    // forensics see WHICH litellm_call / model / team_id was rejected.
    if !enrichment.spendguard_context.is_null() {
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("spendguard".into(), enrichment.spendguard_context.clone());
        }
    }
    if let Some(est) = claim_estimate {
        insert_claim_estimate_payload_mirrors(&mut payload, est);
    }
    let payload_bytes =
        serde_json::to_vec(&payload).expect("denied decision json serialization is infallible");
    let mut cloudevent = CloudEvent {
        specversion: "1.0".into(),
        r#type: "spendguard.audit.decision".into(),
        source: format!("sidecar://{}/{}", ctx.region, ctx.workload_instance_id),
        id: audit_decision_event_id.to_string(),
        time: Some(prost_types::Timestamp {
            seconds: Utc::now().timestamp(),
            nanos: Utc::now().timestamp_subsec_nanos() as i32,
        }),
        datacontenttype: "application/json".into(),
        data: payload_bytes.into(),
        tenant_id: ctx.tenant_id.clone(),
        // Cost Advisor P0.5 (DENY path): was String::new(); now
        // sourced from SpendGuardIds.run_id so canonical_events.run_id
        // is populated downstream.
        run_id: enrichment.run_id.clone(),
        decision_id: decision_id.to_string(),
        schema_bundle_id: schema_bundle_id.to_string(),
        producer_id: format!("sidecar:{}", ctx.workload_instance_id),
        producer_sequence,
        producer_signature: vec![].into(),
        signing_key_id: String::new(),
        ..Default::default()
    };
    // SLICE_02 §6: DENY-lane CloudEvent emits prediction_policy_used
    // identical to the ALLOW lane (per §6.4 byte-identical regression
    // requirement). v1alpha1 contracts get STRICT_CEILING; v1alpha2
    // contracts pass through whatever they declared.
    cloudevent.prediction_policy_used = bundle.parsed.prediction_policy.as_str().to_string();

    // SLICE_10 Phase C: DENY-lane CloudEvent emits all 17 prediction
    // columns identical to the ALLOW lane. STOP / REQUIRE_APPROVAL /
    // DEGRADE / SKIP rows still carry the predictor + projector
    // observations so calibration-report can backtest precision against
    // realized decisions. ClaimEstimate supplies tokenizer/output
    // predictor fields; the sidecar's projector_response overrides the
    // 3 run-level fields whenever it exists.
    if let Some(est) = claim_estimate {
        cloudevent.predicted_a_tokens = est.predicted_a_tokens;
        cloudevent.predicted_b_tokens = est.predicted_b_tokens;
        cloudevent.predicted_c_tokens = est.predicted_c_tokens;
        cloudevent.reserved_strategy = est.reserved_strategy.clone();
        cloudevent.prediction_strategy_used = est.prediction_strategy_used.clone();
        // prediction_policy_used is authoritative from the loaded
        // contract bundle. ClaimEstimate is caller input and must not
        // rewrite the signed audit policy.
        cloudevent.tokenizer_tier = est.tokenizer_tier.clone();
        cloudevent.tokenizer_version_id = est.tokenizer_version_id.clone();
        cloudevent.prediction_confidence = est.prediction_confidence;
        cloudevent.prediction_sample_size = est.prediction_sample_size;
        cloudevent.cold_start_layer_used = est.cold_start_layer_used.clone();

        cloudevent.run_projection_at_decision_atomic = est.run_projection_at_decision_atomic;
        cloudevent.run_predicted_remaining_steps = est.run_predicted_remaining_steps;
        cloudevent.run_steps_completed_so_far = est.run_steps_completed_so_far;
    } else {
        cloudevent.run_projection_at_decision_atomic = 0;
        cloudevent.run_predicted_remaining_steps = -1;
        cloudevent.run_steps_completed_so_far = 0;
    }
    if let Some(resp) = projector_response {
        cloudevent.run_projection_at_decision_atomic = resp.run_projection_at_decision_atomic;
        cloudevent.run_predicted_remaining_steps = resp.run_predicted_remaining_steps;
        cloudevent.run_steps_completed_so_far = resp.run_steps_completed_so_far;
    }

    crate::audit::sign_cloudevent_in_place(&*state.inner.signer, &mut cloudevent).await?;

    // Round-2 #9 producer SP. When the contract evaluator returns
    // REQUIRE_APPROVAL, build the decision_context + requested_effect
    // JSON blobs the ledger's `post_approval_required_decision` SP
    // needs to write an `approval_requests` row atomically with the
    // audit_outbox row. Sidecar's resume_after_approval handler later
    // reads these blobs back to rebuild a fresh ReserveSetRequest.
    //
    // Shape mirrors the SQL header comment in migration 0037 + the
    // `approval_resume_payload` parser in adapter_uds.rs.
    let (decision_context_json, requested_effect_json, approval_ttl_seconds) = if decision_kind
        == Decision::RequireApproval
    {
        let primary_claim = claims.first();
        let (unit_id, unit_kind_str, unit_token_kind) = match primary_claim
            .and_then(|c| c.unit.as_ref())
        {
            Some(u) => (
                u.unit_id.clone(),
                match u.kind {
                    x if x == crate::proto::common::v1::unit_ref::Kind::Monetary as i32 => {
                        "MONETARY"
                    }
                    x if x == crate::proto::common::v1::unit_ref::Kind::Token as i32 => "TOKEN",
                    x if x == crate::proto::common::v1::unit_ref::Kind::Credit as i32 => "CREDIT",
                    x if x == crate::proto::common::v1::unit_ref::Kind::NonMonetary as i32 => {
                        "NON_MONETARY"
                    }
                    _ => "MONETARY",
                },
                u.token_kind.clone(),
            ),
            None => (String::new(), "MONETARY", String::new()),
        };
        // Issue #59: capture the 4 pricing fields at REQUIRE_APPROVAL
        // time so resume() can rebuild the ReserveSetRequest with
        // frozen-at-PRE pricing, not the sidecar's live bundle
        // (which may have hot-reloaded between approval + resume).
        // Spec: docs/specs/issue-59-approval-resume-frozen-pricing.md §3.1.
        let decision_ctx = serde_json::json!({
            "tenant_id":                       ctx.tenant_id,
            "budget_id":                       primary_claim.map(|c| c.budget_id.clone()).unwrap_or_default(),
            "window_instance_id":              primary_claim.map(|c| c.window_instance_id.clone()).unwrap_or_default(),
            "fencing_scope_id":                fencing.scope_id,
            "fencing_epoch":                   fencing.epoch,
            "decision_id":                     decision_id.to_string(),
            "matched_rule_ids":                matched_rules,
            "reason_codes":                    reason_codes,
            "contract_bundle_id":              bundle.bundle_id.to_string(),
            "contract_bundle_hash_hex":        hex::encode(&bundle.bundle_hash),
            "schema_bundle_id":                state.inner.schema_bundle.read().as_ref().map(|s| s.bundle_id.to_string()).unwrap_or_default(),
            "schema_bundle_canonical_version": state.inner.schema_bundle.read().as_ref().map(|s| s.canonical_schema_version.clone()).unwrap_or_default(),
            "pricing_version":                 bundle.pricing_version,
            "price_snapshot_hash_hex":         hex::encode(&bundle.price_snapshot_hash),
            "fx_rate_version":                 bundle.fx_rate_version,
            "unit_conversion_version":         bundle.unit_conversion_version,
        });
        let amount = primary_claim
            .map(|c| c.amount_atomic.clone())
            .unwrap_or_default();
        let direction = match primary_claim.map(|c| c.direction) {
            Some(x) if x == crate::proto::common::v1::budget_claim::Direction::Credit as i32 => {
                "CREDIT"
            }
            _ => "DEBIT",
        };
        let requested_eff = serde_json::json!({
            "unit_id":         unit_id,
            "unit_kind":       unit_kind_str,
            "unit_token_kind": unit_token_kind,
            "amount_atomic":   amount,
            "direction":       direction,
        });
        (
            serde_json::to_vec(&decision_ctx).unwrap_or_default(),
            serde_json::to_vec(&requested_eff).unwrap_or_default(),
            3600_u32,
        )
    } else {
        (Vec::new(), Vec::new(), 0_u32)
    };

    let request = RecordDeniedDecisionRequest {
        tenant_id: ctx.tenant_id.clone(),
        decision_id: decision_id.to_string(),
        audit_decision_event_id: audit_decision_event_id.to_string(),
        producer_sequence,
        idempotency: Some(Idempotency {
            key: denied_idempotency_key,
            request_hash: Vec::new().into(),
        }),
        fencing: Some(fencing.clone()),
        attempted_claims: claims.to_vec(),
        matched_rule_ids: matched_rules.to_vec(),
        reason_codes: reason_codes.to_vec(),
        final_decision: final_decision_str.into(),
        audit_event: Some(cloudevent),
        contract_bundle: Some(ContractBundleRef {
            bundle_id: bundle.bundle_id.to_string(),
            bundle_hash: bundle.bundle_hash.clone().into(),
            bundle_signature: vec![].into(),
            signing_key_id: bundle.signing_key_id.clone(),
        }),
        pricing: Some(pricing.clone()),
        decision_context_json: decision_context_json.into(),
        requested_effect_json: requested_effect_json.into(),
        approval_ttl_seconds,
    };

    let response: RecordDeniedDecisionResponse =
        state.inner.ledger.record_denied_decision(request).await?;

    match response.outcome {
        Some(DeniedOutcome::Success(s)) => Ok(DecisionOutput {
            decision_id: *decision_id,
            audit_decision_event_id: *audit_decision_event_id,
            effect_hash,
            decision: decision_kind,
            reservation_set_id: String::new(),
            reservation_ids: vec![],
            ledger_transaction_id: s.ledger_transaction_id,
            approval_request_id: s.approval_id,
            ttl_expires_at: None,
            matched_rule_ids: matched_rules.to_vec(),
            reason_codes: reason_codes.to_vec(),
            run_code_triggered: run_code_triggered_local.clone(),
            subscription_meter: None,
        }),
        Some(DeniedOutcome::Replay(r)) => {
            // Codex R1 P1 — known POC gap: Replay path returns the
            // freshly-computed `decision_kind` from THIS call's
            // evaluator, not the kind stored on the original row.
            // Risk only triggers if a bundle hot-reload changed the
            // rule outcome between the original call and this retry.
            // POC has no hot-reload, so decision_kind is stable across
            // retries within a session. GA path: surface
            // final_decision in RecordDeniedDecisionResponse.Replay
            // and propagate through DecisionOutput.
            let original_decision_id = uuid::Uuid::parse_str(&r.decision_id).map_err(|e| {
                DomainError::DecisionStage(format!(
                    "ledger denied replay returned malformed decision_id '{}': {}",
                    r.decision_id, e
                ))
            })?;
            let original_audit_id =
                uuid::Uuid::parse_str(&r.audit_decision_event_id).map_err(|e| {
                    DomainError::DecisionStage(format!(
                        "ledger denied replay returned malformed audit_decision_event_id '{}': {}",
                        r.audit_decision_event_id, e
                    ))
                })?;
            Ok(DecisionOutput {
                decision_id: original_decision_id,
                audit_decision_event_id: original_audit_id,
                effect_hash,
                decision: decision_kind,
                reservation_set_id: String::new(),
                reservation_ids: vec![],
                ledger_transaction_id: r.ledger_transaction_id,
                // Round-2 #9 part 2 POC: shared Replay proto doesn't
                // carry approval_id; the resume path can recover it
                // via the decision_id index when needed. GA work item
                // is to extend Replay with operation_id semantics for
                // approval kind.
                approval_request_id: String::new(),
                ttl_expires_at: None,
                matched_rule_ids: matched_rules.to_vec(),
                reason_codes: reason_codes.to_vec(),
                run_code_triggered: run_code_triggered_local.clone(),
                subscription_meter: None,
            })
        }
        Some(DeniedOutcome::Error(e)) => Err(DomainError::DecisionStage(format!(
            "RecordDeniedDecision error code={} msg={}",
            e.code, e.message
        ))),
        None => Err(DomainError::DecisionStage(
            "RecordDeniedDecision response empty oneof".into(),
        )),
    }
}

/// Build a DecisionResponse for the adapter. Wraps the ledger output
/// with effect_hash + decision kind + reservation context. Phase 3:
/// surfaces matched_rule_ids + reason_codes from the contract evaluator
/// so the adapter sees which rules fired and why.
pub fn build_response(out: DecisionOutput) -> DecisionResponse {
    DecisionResponse {
        decision_id: out.decision_id.to_string(),
        audit_decision_event_id: out.audit_decision_event_id.to_string(),
        decision: out.decision as i32,
        reason_codes: out.reason_codes,
        matched_rule_ids: out.matched_rule_ids,
        mutation_patch_json: String::new(),
        effect_hash: out.effect_hash.to_vec().into(),
        ledger_transaction_id: out.ledger_transaction_id,
        reservation_ids: out.reservation_ids,
        ttl_expires_at: out.ttl_expires_at,
        approval_request_id: out.approval_request_id,
        approval_ttl: None,
        approver_role: String::new(),
        // SLICE_02 §3.4: STOP_RUN_PROJECTION is wire-semantically
        // identical to STOP ("STOP semantics completely equivalent;
        // STOP_RUN_PROJECTION is dashboard / SIEM categorisation
        // only"). The adapter response's `terminal` boolean must
        // therefore treat both arms identically — otherwise SLICE_09
        // run-projection stops would leak `terminal=false` and adapter
        // callers (egress proxy, LiteLLM hook) would attempt to
        // forward the call instead of halting the run.
        terminal: matches!(out.decision, Decision::Stop | Decision::StopRunProjection),
        error: None,
        // SLICE_09 Phase E: surface the RUN_* code (if any) that drove
        // this decision. Empty string for per-call decisions. v1alpha1
        // clients deserializing this response see proto3 default empty
        // string, identical to pre-bump behavior.
        run_code_triggered: out.run_code_triggered,
        // D13 COV_61 — meter snapshot (None when not subscription).
        subscription_meter: out.subscription_meter,
    }
}

// Silence unused warnings for types kept for vertical-slice expansion.
#[allow(dead_code)]
fn _retain_types(_a: UnitRef, _b: LockOrderToken) {}

// =====================================================================
// Stage 7 — commit_or_release (Phase 2B Step 7, CommitEstimated lane).
// =====================================================================

#[derive(Debug)]
pub struct CommitEstimatedOutput {
    pub ledger_transaction_id: String,
    pub reservation_id: String,
    pub delta_to_reserved_atomic: String,
}

/// Drive Stage 7 commit lane via CommitEstimated. Routing is decided
/// against `LlmCallPostPayload`:
///   * estimated_amount_atomic non-empty + outcome=SUCCESS  -> CommitEstimated
///   * provider_reported_amount_atomic non-empty            -> deferred (UNIMPLEMENTED)
///   * outcome != SUCCESS                                   -> deferred (Release path B2)
pub async fn run_commit_estimated(
    cfg: &crate::config::Config,
    state: &SidecarState,
    ctx: &DecisionContext,
    payload: &LlmCallPostPayload,
) -> Result<CommitEstimatedOutput, DomainError> {
    use crate::proto::sidecar_adapter::v1::llm_call_post_payload::Outcome as LlmOutcome;

    // 1) Routing validation (per A3 design).
    let outcome = LlmOutcome::try_from(payload.outcome).unwrap_or(LlmOutcome::Unspecified);
    if outcome != LlmOutcome::Success {
        return Err(DomainError::InvalidRequest(format!(
            "LLM_CALL_POST outcome={:?} requires Release path (deferred this slice)",
            outcome
        )));
    }
    if !payload.provider_reported_amount_atomic.is_empty()
        && !payload.estimated_amount_atomic.is_empty()
    {
        return Err(DomainError::InvalidRequest(
            "estimated_amount_atomic and provider_reported_amount_atomic are mutually exclusive"
                .into(),
        ));
    }
    if !payload.provider_reported_amount_atomic.is_empty() {
        return Err(DomainError::InvalidRequest(
            "ProviderReport path is deferred to a future slice; emit estimated_amount_atomic instead".into(),
        ));
    }
    if payload.estimated_amount_atomic.is_empty() {
        return Err(DomainError::InvalidRequest(
            "LLM_CALL_POST.SUCCESS missing estimated_amount_atomic".into(),
        ));
    }

    let reservation_uuid = uuid::Uuid::parse_str(&payload.reservation_id)
        .map_err(|e| DomainError::InvalidRequest(format!("reservation_id parse: {e}")))?;

    // 2) Recover reservation context (cache -> ledger query).
    let resv = recover_reservation_ctx(state, &ctx.tenant_id, &reservation_uuid).await?;

    // Codex round 2 challenge P2.4: short-circuit on non-`reserved` states.
    // The SP enforces this too, but failing fast at the sidecar produces a
    // typed error (vs round-trip + SP exception) and avoids burning a
    // producer_sequence on a doomed call.
    if resv.current_state != "reserved" {
        return Err(DomainError::ReservationStateConflict(format!(
            "reservation {} current_state={} (expected reserved)",
            reservation_uuid, resv.current_state
        )));
    }

    // 3) Validate fencing epoch == reserve-time epoch (DD5 C1).
    //
    // Known POC limitation (Codex round 2 challenge P2.1): the demo
    // bootstraps the active epoch from a static env var, so a sidecar
    // restart does not actually advance the epoch — restart-then-commit
    // would slip past this gate. The full mitigation is a ledger CAS
    // acquire/recover at startup (acknowledged-deferred to a future
    // slice; user task brief explicitly defers fencing scope acquire RPC
    // / hot-reload bundles to GA). Once the acquire RPC lands, restart
    // increments the epoch and this check rejects stale-owner commits.
    let fencing_state = state
        .inner
        .fencing
        .read()
        .clone()
        .ok_or_else(|| DomainError::FencingAcquire("no active fencing scope".into()))?;
    if fencing_state.epoch != resv.fencing_epoch_at_post {
        return Err(DomainError::FencingEpochStale(format!(
            "current epoch {} differs from reserve-time epoch {} (sidecar restart between reserve and commit; reservation will TTL-release)",
            fencing_state.epoch, resv.fencing_epoch_at_post
        )));
    }

    // 4) Validate unit + pricing match (sanity; SP also validates).
    if let Some(u) = &payload.unit {
        if u.unit_id != resv.unit_id.to_string() {
            return Err(DomainError::InvalidRequest(format!(
                "payload.unit_id {} does not match reservation {}",
                u.unit_id, resv.unit_id
            )));
        }
    }
    if let Some(p) = &payload.pricing {
        if p.pricing_version != resv.pricing_version
            || p.price_snapshot_hash.as_ref() != resv.price_snapshot_hash.as_slice()
            || p.fx_rate_version != resv.fx_rate_version
            || p.unit_conversion_version != resv.unit_conversion_version
        {
            return Err(DomainError::PricingFreezeMismatch(
                "payload pricing tuple differs from original reservation".into(),
            ));
        }
    }

    // 5) Validate 0 < estimated <= original_reserved.
    use num_bigint::BigInt;
    let estimated = payload
        .estimated_amount_atomic
        .parse::<BigInt>()
        .map_err(|e| DomainError::InvalidRequest(format!("estimated_amount_atomic parse: {e}")))?;
    let original = resv
        .original_reserved_amount_atomic
        .parse::<BigInt>()
        .map_err(|e| DomainError::Internal(anyhow::anyhow!("ctx original amount parse: {e}")))?;
    if estimated.sign() != num_bigint::Sign::Plus {
        return Err(DomainError::InvalidRequest(
            "estimated_amount_atomic must be > 0".into(),
        ));
    }
    if estimated > original {
        return Err(DomainError::OverrunReservation(format!(
            "estimated {} exceeds original_reserved {}",
            estimated, original
        )));
    }

    // 6) Single producer_sequence allocation (Codex round 2 N2.4).
    let producer_seq = state.next_producer_sequence();

    // 7) New audit_outcome_event_id (CloudEvent.id) — paired with original
    //    decision_id at audit_outbox per Stage 2 §4.8.
    let audit_outcome_event_id = uuid::Uuid::now_v7();

    let mut ce_payload = serde_json::json!({
        "kind": "commit_estimated",
        "reservation_id": reservation_uuid.to_string(),
        "estimated_amount_atomic": payload.estimated_amount_atomic,
        "decision_id": resv.decision_id.to_string(),
    });
    if let Some(v) = payload.actual_input_tokens {
        ce_payload["actual_input_tokens"] = serde_json::json!(v);
    }
    if let Some(v) = payload.actual_output_tokens {
        ce_payload["actual_output_tokens"] = serde_json::json!(v);
    }
    if let Some(v) = payload.delta_b_ratio.filter(|v| v.is_finite() && *v >= 0.0) {
        ce_payload["delta_b_ratio"] = serde_json::json!(v);
    }
    if let Some(v) = payload.delta_c_ratio.filter(|v| v.is_finite() && *v >= 0.0) {
        ce_payload["delta_c_ratio"] = serde_json::json!(v);
    }
    let mut cloudevent = CloudEvent {
        specversion: "1.0".into(),
        r#type: "spendguard.audit.outcome".into(),
        source: format!("sidecar://{}/{}", ctx.region, ctx.workload_instance_id),
        id: audit_outcome_event_id.to_string(),
        time: Some(prost_types::Timestamp {
            seconds: Utc::now().timestamp(),
            nanos: Utc::now().timestamp_subsec_nanos() as i32,
        }),
        datacontenttype: "application/json".into(),
        data: serde_json::to_vec(&ce_payload)
            .expect("payload json")
            .into(),
        tenant_id: ctx.tenant_id.clone(),
        run_id: String::new(),
        decision_id: resv.decision_id.to_string(),
        schema_bundle_id: String::new(),
        producer_id: format!("sidecar:{}", ctx.workload_instance_id),
        producer_sequence: producer_seq,
        producer_signature: vec![].into(),
        signing_key_id: String::new(),
        // SLICE_02: tag 300+ prediction columns default to SQL-NULL
        // sentinels (per audit-chain-prediction-extension-v1alpha1.md
        // §3.3). spendguard.audit.outcome events only populate the
        // commit-side actuals (tags 314-317) once the predictor
        // wires through; SLICE_02 leaves them at proto3 default.
        ..Default::default()
    };
    if let Some(v) = payload.actual_input_tokens {
        cloudevent.actual_input_tokens = v;
    }
    if let Some(v) = payload.actual_output_tokens {
        cloudevent.actual_output_tokens = v;
    }
    if let Some(v) = payload.delta_b_ratio.filter(|v| v.is_finite() && *v >= 0.0) {
        cloudevent.delta_b_ratio = v;
    }
    if let Some(v) = payload.delta_c_ratio.filter(|v| v.is_finite() && *v >= 0.0) {
        cloudevent.delta_c_ratio = v;
    }
    crate::audit::sign_cloudevent_in_place(&*state.inner.signer, &mut cloudevent).await?;

    let request = CommitEstimatedRequest {
        tenant_id: ctx.tenant_id.clone(),
        reservation_id: reservation_uuid.to_string(),
        estimated_amount_atomic: payload.estimated_amount_atomic.clone(),
        unit: Some(UnitRef {
            unit_id: resv.unit_id.to_string(),
            ..Default::default()
        }),
        idempotency: Some(Idempotency {
            key: format!("commit_estimated:{}:1", reservation_uuid),
            request_hash: Vec::new().into(),
        }),
        fencing: Some(Fencing {
            epoch: fencing_state.epoch,
            scope_id: resv.fencing_scope_id.to_string(),
            workload_instance_id: ctx.workload_instance_id.clone(),
        }),
        pricing: Some(PricingFreeze {
            pricing_version: resv.pricing_version.clone(),
            price_snapshot_hash: resv.price_snapshot_hash.clone().into(),
            fx_rate_version: resv.fx_rate_version.clone(),
            unit_conversion_version: resv.unit_conversion_version.clone(),
        }),
        audit_event: Some(cloudevent),
        decision_id: resv.decision_id.to_string(),
        producer_sequence: producer_seq,
    };

    let _ = cfg; // currently unused; kept for parity with run_through_reserve
    let response: CommitEstimatedResponse = state.inner.ledger.commit_estimated(request).await?;
    match response.outcome {
        Some(CommitOutcome::Success(s)) => {
            state
                .inner
                .reservation_cache
                .lock()
                .remove(&reservation_uuid);
            // Step 7.5 P2.2: also evict decision_id index so a stray
            // ConfirmPublishOutcome.APPLY_FAILED for an already-committed
            // decision doesn't route to run_release with stale state.
            state
                .inner
                .decision_id_to_reservation
                .lock()
                .remove(&resv.decision_id);
            Ok(CommitEstimatedOutput {
                ledger_transaction_id: s.ledger_transaction_id,
                reservation_id: s.reservation_id,
                delta_to_reserved_atomic: s.delta_to_reserved_atomic,
            })
        }
        Some(CommitOutcome::Replay(r)) => {
            state
                .inner
                .reservation_cache
                .lock()
                .remove(&reservation_uuid);
            state
                .inner
                .decision_id_to_reservation
                .lock()
                .remove(&resv.decision_id);
            Ok(CommitEstimatedOutput {
                ledger_transaction_id: r.ledger_transaction_id,
                reservation_id: reservation_uuid.to_string(),
                delta_to_reserved_atomic: String::new(),
            })
        }
        Some(CommitOutcome::Error(e)) => map_proto_error_to_domain(e.code, e.message),
        None => Err(DomainError::DecisionStage(
            "CommitEstimated response empty oneof".into(),
        )),
    }
}

// =====================================================================
// Stage 7 — release lane (Phase 2B Step 7.5).
// Triggered by:
//   * LLM_CALL_POST.outcome ∈ {PROVIDER_ERROR, CLIENT_TIMEOUT, RUN_ABORTED}
//     (reservation_id from LlmCallPostPayload.reservation_id)
//   * ConfirmPublishOutcome.APPLY_FAILED
//     (decision_id from PublishOutcomeRequest; resolved via
//      decision_id_to_reservation index)
// =====================================================================

#[derive(Debug)]
pub struct ReleaseOutput {
    pub ledger_transaction_id: String,
    pub released_reservation_ids: Vec<String>,
    /// Detached signature of the audit.release CloudEvent emitted by
    /// this release. Lets explicit ASP callers (ReleaseReservation RPC)
    /// pin the response to the receipt without re-fetching from the
    /// audit chain. Implicit callers (APPLY_FAILED, EmitTraceEvents
    /// drivers) currently ignore it; that's fine — the field is
    /// additive.
    pub audit_event_signature: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
pub enum ReleaseReason {
    RuntimeError,
    RunAborted,
    Explicit,
}

impl ReleaseReason {
    fn to_proto(self) -> ReleaseReasonProto {
        match self {
            ReleaseReason::RuntimeError => ReleaseReasonProto::RuntimeError,
            ReleaseReason::RunAborted => ReleaseReasonProto::RunAborted,
            ReleaseReason::Explicit => ReleaseReasonProto::Explicit,
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            ReleaseReason::RuntimeError => "RUNTIME_ERROR",
            ReleaseReason::RunAborted => "RUN_ABORTED",
            ReleaseReason::Explicit => "EXPLICIT",
        }
    }
}

fn release_signature_cache_ttl_seconds(state: &SidecarState) -> i64 {
    state.inner.reservation_ttl_seconds.clamp(
        RELEASE_SIGNATURE_CACHE_MIN_TTL_SECONDS,
        RELEASE_SIGNATURE_CACHE_MAX_TTL_SECONDS,
    )
}

fn release_signature_cache_key(tenant_id: &str, ledger_idempotency_key: &str) -> String {
    format!("{tenant_id}:{ledger_idempotency_key}")
}

fn store_release_signature(
    cache: &mut HashMap<String, CachedReleaseSignature>,
    key: String,
    signature: &[u8],
    now: chrono::DateTime<Utc>,
    ttl_seconds: i64,
) {
    if signature.is_empty() {
        return;
    }
    cache.retain(|_, cached| cached.expires_at > now);
    cache.insert(
        key,
        CachedReleaseSignature {
            signature: signature.to_vec(),
            expires_at: now + chrono::Duration::seconds(ttl_seconds.max(1)),
        },
    );
}

fn lookup_release_signature(
    cache: &mut HashMap<String, CachedReleaseSignature>,
    key: &str,
    now: chrono::DateTime<Utc>,
) -> Option<Vec<u8>> {
    match cache.get(key) {
        Some(cached) if cached.expires_at > now => Some(cached.signature.clone()),
        Some(_) => {
            cache.remove(key);
            None
        }
        None => None,
    }
}

#[cfg(test)]
mod release_signature_cache_tests {
    use super::*;

    #[test]
    fn release_replay_cache_returns_original_signature() {
        let mut cache = HashMap::new();
        let key = release_signature_cache_key("tenant-a", "release:reservation-1:key-1");
        let now = Utc::now();

        store_release_signature(&mut cache, key.clone(), b"original-signature", now, 600);
        let replay_signature =
            lookup_release_signature(&mut cache, &key, now + chrono::Duration::seconds(1));

        assert_eq!(replay_signature, Some(b"original-signature".to_vec()));
    }

    #[test]
    fn release_replay_cache_expires_without_fabricating_signature() {
        let mut cache = HashMap::new();
        let key = release_signature_cache_key("tenant-a", "release:reservation-1:key-1");
        let now = Utc::now();

        store_release_signature(&mut cache, key.clone(), b"original-signature", now, 1);
        let replay_signature =
            lookup_release_signature(&mut cache, &key, now + chrono::Duration::seconds(2));

        assert_eq!(replay_signature, None);
        assert!(!cache.contains_key(&key));
    }

    #[test]
    fn release_replay_cache_is_tenant_scoped() {
        let key_a = release_signature_cache_key("tenant-a", "release:reservation-1:key-1");
        let key_b = release_signature_cache_key("tenant-b", "release:reservation-1:key-1");

        assert_ne!(key_a, key_b);
    }
}

pub async fn run_release(
    cfg: &crate::config::Config,
    state: &SidecarState,
    ctx: &DecisionContext,
    reservation_uuid: uuid::Uuid,
    reason: ReleaseReason,
    payload_metadata: Option<&str>,
    // Adapter-supplied idempotency key for the ledger. None → use the
    // built-in `release:{uuid}:1` key (the legacy implicit-path
    // behavior). Some(k) → forward the adapter's key so the
    // (reservation_id, adapter_key) dedup contract documented in
    // adapter.proto::ReleaseReservationRequest holds end-to-end.
    idempotency_key_override: Option<&str>,
    // Adapter-supplied hash of the raw request body (e.g. SHA-256 of
    // the joined reason_codes). Lets the ledger detect "same key,
    // different body" as IdempotencyConflict instead of replaying
    // the original outcome. None → empty hash (legacy implicit-path
    // behavior — implicit callers don't have a stable body to hash).
    request_body_hash: Option<&[u8]>,
) -> Result<ReleaseOutput, DomainError> {
    let _ = cfg;

    // 1) Recover reservation context (cache → ledger query).
    let resv = recover_reservation_ctx(state, &ctx.tenant_id, &reservation_uuid).await?;

    // 2) Short-circuit on non-`reserved` states (Codex round 1 P1.4
    //    state-check ordering; SP also enforces, but failing fast at
    //    sidecar avoids burning a producer_sequence).
    //
    //    `released` is intentionally let through: an explicit
    //    ReleaseReservation retry with the same (reservation_id,
    //    idempotency_key) MUST be able to reach the ledger so its
    //    Replay branch can fire and return the original outcome (per
    //    adapter.proto::ReleaseReservationRequest dedup contract).
    //    The ledger remains the source of truth — if the retry's
    //    idempotency key differs from the original, the ledger
    //    returns RESERVATION_STATE_CONFLICT which maps to the
    //    "different idempotency_key against terminal reservation"
    //    error from ASP Draft-01 §3.3.
    if resv.current_state != "reserved" && resv.current_state != "released" {
        return Err(DomainError::ReservationStateConflict(format!(
            "reservation {} current_state={} (expected reserved or released for release)",
            reservation_uuid, resv.current_state
        )));
    }

    // 3) Fencing epoch parity (DD5 C1).
    //
    //    For state == "released" (replay-attempt path) we skip the
    //    fencing comparison: the ledger SP checks idempotency BEFORE
    //    fencing, so a same-key retry will replay correctly even if
    //    the sidecar's fencing lease was renewed/lost between the
    //    original release and the retry. Forcing a fresh fencing
    //    check here would turn a legitimate idempotent retry into
    //    FencingEpochStale instead of the documented Replay outcome.
    //    We still need a fencing_state to populate the request proto;
    //    the ledger ignores it on the replay path.
    let fencing_state = state
        .inner
        .fencing
        .read()
        .clone()
        .ok_or_else(|| DomainError::FencingAcquire("no active fencing scope".into()))?;
    let is_replay_attempt = resv.current_state == "released";
    if !is_replay_attempt {
        crate::fencing::check_active(state)?;
        if fencing_state.epoch != resv.fencing_epoch_at_post {
            return Err(DomainError::FencingEpochStale(format!(
                "current epoch {} differs from reserve-time epoch {}; reservation will TTL-release",
                fencing_state.epoch, resv.fencing_epoch_at_post
            )));
        }
    }

    // 4) Single producer_sequence allocation.
    let producer_seq = state.next_producer_sequence();

    // 5) New audit_outcome_event_id.
    let audit_outcome_event_id = uuid::Uuid::now_v7();

    // 6) Derive reservation_set_id (matches ledger's Rust derivation in
    //    reserve_set.rs::derive_reservation_set_id; SP doesn't verify
    //    canonical form per round 2 M1.1 fix — opaque wire identity).
    let reservation_set_id = derive_reservation_set_id(&resv.decision_id);

    let payload_metadata_str = payload_metadata.unwrap_or("");

    let ce_payload = serde_json::json!({
        "kind": "release",
        "reservation_id": reservation_uuid.to_string(),
        "reservation_set_id": reservation_set_id.to_string(),
        "decision_id": resv.decision_id.to_string(),
        "reason": reason.as_str(),
        "metadata": payload_metadata_str,
    });

    let mut cloudevent = CloudEvent {
        specversion: "1.0".into(),
        r#type: "spendguard.audit.outcome".into(),
        source: format!("sidecar://{}/{}", ctx.region, ctx.workload_instance_id),
        id: audit_outcome_event_id.to_string(),
        time: Some(prost_types::Timestamp {
            seconds: Utc::now().timestamp(),
            nanos: Utc::now().timestamp_subsec_nanos() as i32,
        }),
        datacontenttype: "application/json".into(),
        data: serde_json::to_vec(&ce_payload)
            .expect("payload json")
            .into(),
        tenant_id: ctx.tenant_id.clone(),
        run_id: String::new(),
        decision_id: resv.decision_id.to_string(),
        schema_bundle_id: String::new(),
        producer_id: format!("sidecar:{}", ctx.workload_instance_id),
        producer_sequence: producer_seq,
        producer_signature: vec![].into(),
        signing_key_id: String::new(),
        // SLICE_02: see commit_estimated CloudEvent comment above —
        // tag 300+ prediction columns left at proto3 default for the
        // release-lane spendguard.audit.outcome event.
        ..Default::default()
    };
    crate::audit::sign_cloudevent_in_place(&*state.inner.signer, &mut cloudevent).await?;

    // Snapshot the audit signature now so we can return it to explicit
    // ASP callers regardless of which ledger outcome branch fires.
    let audit_event_signature: Vec<u8> = cloudevent.producer_signature.to_vec();

    // Namespace the adapter-supplied key by reservation_uuid so the
    // ledger sees a unique key per (reservation, adapter_key) pair.
    // The ledger's idempotency dedup is scoped by (tenant, op_kind),
    // not by reservation_id, so passing the raw adapter key would let
    // two unrelated reservations using the same retry key collide.
    // Wrapping in `release:{uuid}:{key}` preserves the documented
    // Draft-01 (reservation_id, idempotency_key) dedup contract.
    let ledger_idempotency_key = idempotency_key_override
        .map(|k| format!("release:{}:{}", reservation_uuid, k))
        .unwrap_or_else(|| format!("release:{}:1", reservation_uuid));
    let signature_cache_key = release_signature_cache_key(&ctx.tenant_id, &ledger_idempotency_key);

    let request_hash_vec: Vec<u8> = request_body_hash.map(|h| h.to_vec()).unwrap_or_default();

    let request = ReleaseRequest {
        tenant_id: ctx.tenant_id.clone(),
        reservation_set_id: reservation_set_id.to_string(),
        idempotency: Some(Idempotency {
            key: ledger_idempotency_key.clone(),
            request_hash: request_hash_vec.into(),
        }),
        fencing: Some(Fencing {
            epoch: fencing_state.epoch,
            scope_id: resv.fencing_scope_id.to_string(),
            workload_instance_id: ctx.workload_instance_id.clone(),
        }),
        reason: reason.to_proto() as i32,
        audit_event: Some(cloudevent),
        decision_id: resv.decision_id.to_string(),
        producer_sequence: producer_seq,
    };

    let response: ReleaseResponse = state.inner.ledger.release(request).await?;
    match response.outcome {
        Some(ReleaseOutcome::Success(s)) => {
            // Evict caches.
            state
                .inner
                .reservation_cache
                .lock()
                .remove(&reservation_uuid);
            state
                .inner
                .decision_id_to_reservation
                .lock()
                .remove(&resv.decision_id);
            // The ledger's `released_reservation_ids` lists
            // reservation_set_ids, not the caller's reservation_id.
            // The Replay branch below returns vec![reservation_uuid].
            // Normalize Success to match so callers see the same
            // identifier shape across first-success and retry-replay
            // for the same operation.
            let ttl_seconds = release_signature_cache_ttl_seconds(state);
            let mut signature_cache = state.inner.release_signature_cache.lock();
            store_release_signature(
                &mut signature_cache,
                signature_cache_key,
                &audit_event_signature,
                Utc::now(),
                ttl_seconds,
            );
            Ok(ReleaseOutput {
                ledger_transaction_id: s.ledger_transaction_id,
                released_reservation_ids: vec![reservation_uuid.to_string()],
                audit_event_signature,
            })
        }
        Some(ReleaseOutcome::Replay(r)) => {
            state
                .inner
                .reservation_cache
                .lock()
                .remove(&reservation_uuid);
            state
                .inner
                .decision_id_to_reservation
                .lock()
                .remove(&resv.decision_id);
            let mut signature_cache = state.inner.release_signature_cache.lock();
            let replay_signature =
                lookup_release_signature(&mut signature_cache, &signature_cache_key, Utc::now())
                    .unwrap_or_default();
            Ok(ReleaseOutput {
                ledger_transaction_id: r.ledger_transaction_id,
                released_reservation_ids: vec![reservation_uuid.to_string()],
                // Never return the freshly-generated retry signature:
                // the ledger replay branch did not persist that event.
                // Use only the original signature captured after the
                // first success. Cache miss (expiry/restart) degrades to
                // empty bytes rather than fabricating audit evidence.
                audit_event_signature: replay_signature,
            })
        }
        Some(ReleaseOutcome::Error(e)) => map_proto_error_to_domain(e.code, e.message),
        None => Err(DomainError::DecisionStage(
            "Release response empty oneof".into(),
        )),
    }
}

/// Mirrors services/ledger/src/handlers/reserve_set.rs::derive_reservation_set_id.
fn derive_reservation_set_id(decision_id: &uuid::Uuid) -> uuid::Uuid {
    let mut h = Sha256::new();
    h.update(decision_id.as_bytes());
    h.update(b":reservation_set");
    let bytes: [u8; 32] = h.finalize().into();
    let mut buf = [0u8; 16];
    buf.copy_from_slice(&bytes[..16]);
    buf[6] = (buf[6] & 0x0f) | 0x40;
    buf[8] = (buf[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(buf)
}

fn map_proto_error_to_domain<T>(code: i32, message: String) -> Result<T, DomainError> {
    use crate::proto::common::v1::error::Code as PC;
    let pc = PC::try_from(code).unwrap_or(PC::Unspecified);
    Err(match pc {
        PC::FencingEpochStale => DomainError::FencingEpochStale(message),
        PC::ReservationStateConflict => DomainError::ReservationStateConflict(message),
        PC::ReservationTtlExpired => DomainError::ReservationTtlExpired(message),
        PC::PricingFreezeMismatch => DomainError::PricingFreezeMismatch(message),
        PC::OverrunReservation => DomainError::OverrunReservation(message),
        PC::MultiReservationCommitDeferred => DomainError::MultiReservationCommitDeferred(message),
        PC::IdempotencyConflict => DomainError::IdempotencyConflict(message),
        _ => DomainError::DecisionStage(format!("ledger error code={code} msg={message}")),
    })
}

#[cfg(test)]
mod release_error_mapping_tests {
    use super::*;
    use crate::proto::common::v1::error::Code as ProtoCode;

    #[test]
    fn idempotency_conflict_maps_to_failed_precondition_domain_error() {
        let err = map_proto_error_to_domain::<()>(
            ProtoCode::IdempotencyConflict as i32,
            "idempotency_key reused with different request_hash".into(),
        )
        .unwrap_err();

        let status_code = err.to_status().code();
        match err {
            DomainError::IdempotencyConflict(message) => {
                assert!(message.contains("idempotency_key reused"));
                assert_eq!(status_code, tonic::Code::FailedPrecondition);
            }
            other => panic!("expected IdempotencyConflict, got {other:?}"),
        }
    }

    #[test]
    fn unknown_proto_code_still_maps_to_internal_domain_error() {
        let err = map_proto_error_to_domain::<()>(9999, "unexpected".into()).unwrap_err();
        assert!(matches!(err, DomainError::DecisionStage(_)));
    }
}

async fn recover_reservation_ctx(
    state: &SidecarState,
    tenant_id: &str,
    reservation_id: &uuid::Uuid,
) -> Result<ReservationCtx, DomainError> {
    if let Some(ctx) = state
        .inner
        .reservation_cache
        .lock()
        .get(reservation_id)
        .cloned()
    {
        return Ok(ctx);
    }
    // Cold path: query ledger.
    let req = QueryReservationContextRequest {
        tenant_id: tenant_id.to_string(),
        reservation_id: reservation_id.to_string(),
    };
    let resp = state.inner.ledger.query_reservation_context(req).await?;
    match resp.outcome {
        Some(QrcOutcome::Context(c)) => {
            let parsed = ReservationCtx {
                tenant_id: c.tenant_id,
                budget_id: uuid::Uuid::parse_str(&c.budget_id)
                    .map_err(|e| DomainError::Internal(anyhow::anyhow!("budget_id: {e}")))?,
                window_instance_id: uuid::Uuid::parse_str(&c.window_instance_id).map_err(|e| {
                    DomainError::Internal(anyhow::anyhow!("window_instance_id: {e}"))
                })?,
                unit_id: uuid::Uuid::parse_str(
                    c.unit.as_ref().map(|u| u.unit_id.as_str()).unwrap_or(""),
                )
                .map_err(|e| DomainError::Internal(anyhow::anyhow!("unit_id: {e}")))?,
                original_reserved_amount_atomic: c.original_reserved_amount_atomic,
                pricing_version: c
                    .pricing
                    .as_ref()
                    .map(|p| p.pricing_version.clone())
                    .unwrap_or_default(),
                price_snapshot_hash: c
                    .pricing
                    .as_ref()
                    .map(|p| p.price_snapshot_hash.to_vec())
                    .unwrap_or_default(),
                fx_rate_version: c
                    .pricing
                    .as_ref()
                    .map(|p| p.fx_rate_version.clone())
                    .unwrap_or_default(),
                unit_conversion_version: c
                    .pricing
                    .as_ref()
                    .map(|p| p.unit_conversion_version.clone())
                    .unwrap_or_default(),
                fencing_scope_id: uuid::Uuid::parse_str(&c.fencing_scope_id)
                    .map_err(|e| DomainError::Internal(anyhow::anyhow!("fencing_scope_id: {e}")))?,
                fencing_epoch_at_post: c.fencing_epoch_at_post,
                decision_id: uuid::Uuid::parse_str(&c.decision_id)
                    .map_err(|e| DomainError::Internal(anyhow::anyhow!("decision_id: {e}")))?,
                ttl_expires_at: c
                    .ttl_expires_at
                    .map(|t| {
                        chrono::DateTime::<chrono::Utc>::from_timestamp(t.seconds, t.nanos as u32)
                            .unwrap_or_default()
                    })
                    .unwrap_or_default(),
                current_state: c.current_state,
            };
            // Memoize for subsequent calls in this process.
            state
                .inner
                .reservation_cache
                .lock()
                .insert(*reservation_id, parsed.clone());
            Ok(parsed)
        }
        Some(QrcOutcome::Error(e)) => map_proto_error_to_domain(e.code, e.message),
        None => Err(DomainError::DecisionStage(
            "QueryReservationContext empty oneof".into(),
        )),
    }
}

fn populate_reservation_cache(
    state: &SidecarState,
    ctx: &DecisionContext,
    reservations: &[crate::proto::ledger::v1::Reservation],
    fencing_state: &crate::domain::state::ActiveFencing,
    decision_id: &uuid::Uuid,
    bundle: &crate::domain::state::CachedContractBundle,
) {
    // Step 7.5: populate decision_id -> reservation_id index alongside
    // the cache so ConfirmPublishOutcome.APPLY_FAILED can route to the
    // right reservation (PublishOutcomeRequest carries only decision_id).
    if let Some(first) = reservations.first() {
        if let Ok(rid) = uuid::Uuid::parse_str(&first.reservation_id) {
            state
                .inner
                .decision_id_to_reservation
                .lock()
                .insert(*decision_id, rid);
        }
    }

    let mut cache = state.inner.reservation_cache.lock();
    for r in reservations {
        let Ok(rid) = uuid::Uuid::parse_str(&r.reservation_id) else {
            continue;
        };
        let Ok(budget_id) = uuid::Uuid::parse_str(&r.budget_id) else {
            continue;
        };
        let Ok(window_instance_id) = uuid::Uuid::parse_str(&r.window_instance_id) else {
            continue;
        };
        let unit_id = match r
            .unit
            .as_ref()
            .and_then(|u| uuid::Uuid::parse_str(&u.unit_id).ok())
        {
            Some(u) => u,
            None => continue,
        };
        let ttl = r
            .ttl_expires_at
            .as_ref()
            .and_then(|t| {
                chrono::DateTime::<chrono::Utc>::from_timestamp(t.seconds, t.nanos as u32)
            })
            .unwrap_or_else(chrono::Utc::now);
        cache.insert(
            rid,
            ReservationCtx {
                tenant_id: ctx.tenant_id.clone(),
                budget_id,
                window_instance_id,
                unit_id,
                original_reserved_amount_atomic: r.amount_atomic.clone(),
                pricing_version: bundle.pricing_version.clone(),
                price_snapshot_hash: bundle.price_snapshot_hash.clone(),
                fx_rate_version: bundle.fx_rate_version.clone(),
                unit_conversion_version: bundle.unit_conversion_version.clone(),
                fencing_scope_id: fencing_state.scope_id,
                fencing_epoch_at_post: fencing_state.epoch,
                decision_id: *decision_id,
                ttl_expires_at: ttl,
                current_state: "reserved".to_string(),
            },
        );
    }
}

// =====================================================================
// Slice C1 R1 — extract_enrichment unit tests
// =====================================================================
//
// Architect + Code Reviewer Staff panel R1 P1: irreversible signed
// CloudEvent payloads (DESIGN NG2) require unit coverage on the
// allowlist/coercion logic BEFORE merge. Tests cover:
//   - BoolValue / NullValue coercion (R1 P0 fix)
//   - Empty runtime_metadata → spendguard_context = Null
//   - Unknown-key drop
//   - All-allowlisted-keys round-trip
// =====================================================================

#[cfg(test)]
mod enrichment_tests {
    use super::*;
    use crate::proto::sidecar_adapter::v1::decision_request::Inputs as DecisionInputs;
    use prost_types::{value::Kind as PtKind, Struct as PtStruct, Value as PtValue};
    use std::collections::BTreeMap;

    fn _str_value(s: &str) -> PtValue {
        PtValue {
            kind: Some(PtKind::StringValue(s.into())),
        }
    }
    fn _bool_value(b: bool) -> PtValue {
        PtValue {
            kind: Some(PtKind::BoolValue(b)),
        }
    }
    fn _null_value() -> PtValue {
        PtValue {
            kind: Some(PtKind::NullValue(0)),
        }
    }
    fn _request_with_metadata(fields: BTreeMap<String, PtValue>) -> DecisionRequest {
        DecisionRequest {
            inputs: Some(DecisionInputs {
                runtime_metadata: Some(PtStruct { fields }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn empty_runtime_metadata_yields_null_context() {
        let req = _request_with_metadata(BTreeMap::new());
        let enr = extract_enrichment(&req);
        assert!(
            enr.spendguard_context.is_null(),
            "empty fields → Null, got {:?}",
            enr.spendguard_context
        );
    }

    #[test]
    fn missing_runtime_metadata_yields_null_context() {
        let req = DecisionRequest {
            inputs: None,
            ..Default::default()
        };
        let enr = extract_enrichment(&req);
        assert!(enr.spendguard_context.is_null());
    }

    #[test]
    fn bool_value_coerced_to_json_bool() {
        // Slice C1 R1 P0 (Backend Architect): SDK sends `stream: bool`
        // as proto BoolValue. Pre-fix: silently dropped. Post-fix:
        // emitted as JSON boolean.
        let mut fields = BTreeMap::new();
        fields.insert("stream".into(), _bool_value(true));
        let req = _request_with_metadata(fields);
        let enr = extract_enrichment(&req);
        let obj = enr
            .spendguard_context
            .as_object()
            .expect("expected Object, got Null");
        assert_eq!(obj.get("stream"), Some(&serde_json::Value::Bool(true)));
    }

    #[test]
    fn null_value_preserved_as_json_null() {
        // SDK sends `team_id: None` when user_api_key_dict.team_id is
        // missing → proto NullValue. Architect mandate: preserve as
        // explicit null in payload (not dropped).
        let mut fields = BTreeMap::new();
        fields.insert("team_id".into(), _null_value());
        let req = _request_with_metadata(fields);
        let enr = extract_enrichment(&req);
        let obj = enr
            .spendguard_context
            .as_object()
            .expect("expected Object, got Null");
        assert_eq!(obj.get("team_id"), Some(&serde_json::Value::Null));
    }

    #[test]
    fn unknown_key_dropped_not_in_payload() {
        // Architect NG2 PII smuggling guard: keys outside the 12-field
        // allowlist must not flow into the signed payload.
        let mut fields = BTreeMap::new();
        fields.insert("integration".into(), _str_value("litellm"));
        fields.insert("evil_pii".into(), _str_value("ssn:123-45-6789"));
        let req = _request_with_metadata(fields);
        let enr = extract_enrichment(&req);
        let obj = enr.spendguard_context.as_object().unwrap();
        assert!(obj.contains_key("integration"));
        assert!(
            !obj.contains_key("evil_pii"),
            "PII key must NOT leak into signed payload"
        );
    }

    #[test]
    fn all_twelve_keys_round_trip() {
        let mut fields = BTreeMap::new();
        for &k in SPENDGUARD_ENRICHMENT_ALLOWLIST {
            fields.insert(k.into(), _str_value(&format!("val-{}", k)));
        }
        let req = _request_with_metadata(fields);
        let enr = extract_enrichment(&req);
        let obj = enr.spendguard_context.as_object().unwrap();
        assert_eq!(obj.len(), SPENDGUARD_ENRICHMENT_ALLOWLIST.len());
        for &k in SPENDGUARD_ENRICHMENT_ALLOWLIST {
            assert_eq!(
                obj.get(k),
                Some(&serde_json::Value::String(format!("val-{}", k))),
                "key {k} missing or wrong value",
            );
        }
    }

    // Slice C2 — DENY path parity test. The CloudEvent merge logic for
    // the DENY branch (`run_record_denied_decision`, line 657-ish) is
    // identical to the ALLOW branch (`build_audit_decision_cloudevent`,
    // line 545-ish): both check `!enrichment.spendguard_context.is_null()`
    // and `obj.insert("spendguard", clone)`. This test asserts the
    // clone yields the same JSON Map shape both times — a regression
    // guard against future drift where ALLOW and DENY diverge in
    // their merge logic.
    #[test]
    fn enrichment_clone_stable_for_both_emit_paths() {
        let mut fields = BTreeMap::new();
        fields.insert("integration".into(), _str_value("litellm"));
        fields.insert("model".into(), _str_value("gpt-4o-mini"));
        fields.insert("stream".into(), _bool_value(true));
        let req = _request_with_metadata(fields);
        let enr = extract_enrichment(&req);
        // Simulate both emit paths cloning the same context.
        let allow_clone = enr.spendguard_context.clone();
        let deny_clone = enr.spendguard_context.clone();
        assert_eq!(
            allow_clone, deny_clone,
            "ALLOW and DENY clones must produce identical JSON"
        );
        // Both must be Objects (not Null) with the same key set.
        let a = allow_clone.as_object().expect("ALLOW clone is Object");
        let d = deny_clone.as_object().expect("DENY clone is Object");
        assert_eq!(a.len(), d.len());
        assert_eq!(a.get("integration"), d.get("integration"));
        assert_eq!(a.get("stream"), d.get("stream"));
        // Verify mixed-type coercion survives both clones.
        assert_eq!(a.get("stream"), Some(&serde_json::Value::Bool(true)));
    }
}

// =====================================================================
// SLICE_02 round-1 fix — exhaustive Decision match coverage tests.
//
// B1: build_response.terminal must collapse Stop + StopRunProjection
//      into the same `true` arm (per spec §3.4 wire-semantic identity).
//      Pre-fix `matches!(out.decision, Decision::Stop)` returned `false`
//      for StopRunProjection, leaking a non-terminal response that adapter
//      callers (egress proxy, LiteLLM hook) would forward instead of halt.
//
// B2: denied_decision_label must categorise StopRunProjection as the
//      v1alpha1 audit-row string `"STOP"` (per spec §3.4 invariant: the
//      audit row `decision` column stays v1alpha1; RUN_* lives in
//      reason_codes). Pre-fix the function returned Err on
//      StopRunProjection, which would have surfaced as
//      `DecisionStage` errors when SLICE_09 starts emitting the variant.
// =====================================================================

#[cfg(test)]
mod slice_02_decision_match_tests {
    use super::*;
    use crate::proto::sidecar_adapter::v1::decision_request::Inputs;
    use uuid::Uuid;

    fn _fixture_output(decision: Decision) -> DecisionOutput {
        DecisionOutput {
            decision_id: Uuid::nil(),
            audit_decision_event_id: Uuid::nil(),
            effect_hash: [0u8; 32],
            decision,
            reservation_set_id: String::new(),
            reservation_ids: vec![],
            ledger_transaction_id: String::new(),
            approval_request_id: String::new(),
            ttl_expires_at: None,
            matched_rule_ids: vec![],
            reason_codes: vec![],
            run_code_triggered: String::new(),
            subscription_meter: None,
        }
    }

    #[test]
    fn build_response_terminal_true_for_stop() {
        let out = _fixture_output(Decision::Stop);
        let resp = build_response(out);
        assert!(resp.terminal, "Decision::Stop must produce terminal=true");
    }

    #[test]
    fn build_response_terminal_true_for_stop_run_projection() {
        // SLICE_02 §3.4: STOP_RUN_PROJECTION is wire-semantically
        // identical to STOP. The adapter response's terminal boolean
        // must therefore be `true`. Pre-round-1: this returned `false`,
        // which would silently leak to adapter callers in SLICE_09.
        let out = _fixture_output(Decision::StopRunProjection);
        let resp = build_response(out);
        assert!(
            resp.terminal,
            "Decision::StopRunProjection must produce terminal=true \
             per spec §3.4 STOP-equivalent invariant"
        );
    }

    #[test]
    fn build_response_terminal_false_for_continue_and_other_non_stop() {
        for kind in [
            Decision::Continue,
            Decision::Degrade,
            Decision::Skip,
            Decision::RequireApproval,
        ] {
            let resp = build_response(_fixture_output(kind));
            assert!(
                !resp.terminal,
                "Decision::{:?} must produce terminal=false",
                kind
            );
        }
    }

    #[test]
    fn denied_decision_label_maps_stop() {
        assert_eq!(denied_decision_label(Decision::Stop), Some("STOP"));
    }

    #[test]
    fn denied_decision_label_maps_stop_run_projection_to_v1alpha1_stop() {
        // SLICE_02 §3.4 invariant: audit_decision.decision column stays
        // at v1alpha1 lattice values ("STOP"); the RUN_* differentiator
        // lives in reason_codes. Pre-round-1 the underlying match arm
        // would have returned Err(DecisionStage(...)) on this variant,
        // breaking the DENY-lane path when SLICE_09 emits it.
        assert_eq!(
            denied_decision_label(Decision::StopRunProjection),
            Some("STOP"),
            "StopRunProjection must categorise as v1alpha1 'STOP' string"
        );
    }

    #[test]
    fn denied_decision_label_maps_other_deny_lane_variants() {
        assert_eq!(
            denied_decision_label(Decision::RequireApproval),
            Some("REQUIRE_APPROVAL")
        );
        assert_eq!(denied_decision_label(Decision::Degrade), Some("DEGRADE"));
        assert_eq!(denied_decision_label(Decision::Skip), Some("SKIP"));
    }

    #[test]
    fn denied_decision_label_returns_none_for_non_deny_variants() {
        // Continue is filtered by caller; Unspecified is a proto-default
        // leak that should not flow. Both yield None so the caller
        // surfaces DecisionStage error.
        assert_eq!(denied_decision_label(Decision::Continue), None);
        assert_eq!(denied_decision_label(Decision::Unspecified), None);
    }

    #[test]
    fn projector_budget_remaining_unknown_is_non_triggering() {
        let req = DecisionRequest {
            inputs: Some(Inputs {
                projected_p90_atomic: String::new(),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            projector_budget_remaining_from_runtime_metadata(&req),
            None,
            "missing budget snapshot must not synthesize budget=0 and trigger RUN_BUDGET_PROJECTION_EXCEEDED"
        );
    }

    #[test]
    fn projector_budget_remaining_invalid_or_negative_is_non_triggering() {
        for budget_remaining_atomic in ["not-a-number", "-1"] {
            let mut fields = std::collections::BTreeMap::new();
            fields.insert(
                "budget_remaining_atomic".to_string(),
                prost_types::Value {
                    kind: Some(prost_types::value::Kind::StringValue(
                        budget_remaining_atomic.to_string(),
                    )),
                },
            );
            let req = DecisionRequest {
                inputs: Some(Inputs {
                    runtime_metadata: Some(prost_types::Struct { fields }),
                    ..Default::default()
                }),
                ..Default::default()
            };
            assert_eq!(projector_budget_remaining_from_runtime_metadata(&req), None);
        }
    }

    #[test]
    fn projector_budget_remaining_ignores_projected_p90_hint() {
        let req = DecisionRequest {
            inputs: Some(Inputs {
                projected_p90_atomic: "12345".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            projector_budget_remaining_from_runtime_metadata(&req),
            None,
            "projected_p90_atomic is a risk-band hint, not remaining budget"
        );
    }

    #[test]
    fn projector_budget_remaining_uses_explicit_runtime_snapshot() {
        let mut fields = std::collections::BTreeMap::new();
        fields.insert(
            "budget_remaining_atomic".to_string(),
            prost_types::Value {
                kind: Some(prost_types::value::Kind::StringValue("999".into())),
            },
        );
        let req = DecisionRequest {
            inputs: Some(Inputs {
                projected_p90_atomic: "12345".into(),
                runtime_metadata: Some(prost_types::Struct { fields }),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            projector_budget_remaining_from_runtime_metadata(&req),
            Some(999)
        );
    }

    #[test]
    fn authoritative_budget_remaining_clamps_negative_available_to_zero() {
        assert_eq!(nonnegative_i64_from_decimal_str("-1"), Some(0));
        assert_eq!(nonnegative_i64_from_decimal_str("123"), Some(123));
        assert_eq!(nonnegative_i64_from_decimal_str("not-a-number"), None);
    }

    #[test]
    fn idempotency_replay_decision_kind_preserves_projection_stop() {
        let reason_codes = vec!["RUN_BUDGET_PROJECTION_EXCEEDED".to_string()];
        assert_eq!(
            replay_decision_kind("denied_decision", "STOP", "", &reason_codes).unwrap(),
            Decision::StopRunProjection
        );
        assert_eq!(
            replay_decision_kind("reserve", "", "", &[]).unwrap(),
            Decision::Continue
        );
    }

    #[test]
    fn idempotency_request_fingerprint_changes_with_claims_and_ids() {
        let ctx = DecisionContext {
            session_id: "session-test".to_string(),
            workload_instance_id: "sidecar-test".to_string(),
            tenant_id: Uuid::nil().to_string(),
            region: "test-region".to_string(),
        };
        let mut req = DecisionRequest {
            session_id: "session-test".into(),
            route: "llm.openai.chat".into(),
            ids: Some(crate::proto::common::v1::SpendGuardIds {
                run_id: "run-1".into(),
                step_id: "step-1".into(),
                llm_call_id: "call-1".into(),
                decision_id: "decision-1".into(),
                ..Default::default()
            }),
            inputs: Some(Inputs {
                projected_claims: vec![BudgetClaim {
                    budget_id: "budget-1".into(),
                    amount_atomic: "100".into(),
                    window_instance_id: "window-1".into(),
                    unit: Some(UnitRef {
                        unit_id: "usd-micros".into(),
                        ..Default::default()
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            idempotency: Some(Idempotency {
                key: "idem-1".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let original = idempotency_request_fingerprint_hex(&ctx, &req);

        req.ids.as_mut().unwrap().decision_id = "decision-2".into();
        assert_ne!(original, idempotency_request_fingerprint_hex(&ctx, &req));

        req.ids.as_mut().unwrap().decision_id = "decision-1".into();
        req.inputs.as_mut().unwrap().projected_claims[0].amount_atomic = "101".into();
        assert_ne!(original, idempotency_request_fingerprint_hex(&ctx, &req));
    }

    #[test]
    fn claim_estimate_payload_mirrors_include_model_prompt_class_and_fingerprint() {
        let mut payload = serde_json::json!({
            "snapshot_hash": "00",
            "reason_codes": ["RUN_BUDGET_PROJECTION_EXCEEDED"],
        });
        let est = ClaimEstimate {
            model: "gpt-4o-mini".into(),
            prompt_class: "support_triage".into(),
            prompt_class_fingerprint: "pcfp_123".into(),
            ..Default::default()
        };

        insert_claim_estimate_payload_mirrors(&mut payload, &est);

        assert_eq!(
            payload.get("model"),
            Some(&serde_json::json!("gpt-4o-mini"))
        );
        assert_eq!(
            payload.get("prompt_class"),
            Some(&serde_json::json!("support_triage"))
        );
        assert_eq!(
            payload.get("prompt_class_fingerprint"),
            Some(&serde_json::json!("pcfp_123"))
        );
    }

    #[test]
    fn projector_decision_id_is_stable_bounded_hash() {
        let a = projector_decision_id_from_idempotency_key("adapter-key-1");
        let b = projector_decision_id_from_idempotency_key("adapter-key-1");
        let c = projector_decision_id_from_idempotency_key("adapter-key-2");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|ch| ch.is_ascii_hexdigit()));
    }

    #[test]
    fn projector_response_overrides_claim_estimate_run_fields() {
        let ctx = DecisionContext {
            session_id: "session-test".to_string(),
            workload_instance_id: "sidecar-test".to_string(),
            tenant_id: Uuid::nil().to_string(),
            region: "test-region".to_string(),
        };
        let enrichment = AuditEnrichment {
            run_id: Uuid::nil().to_string(),
            agent_id: "agent-test".to_string(),
            model_family: "model-test".to_string(),
            prompt_hash: "prompt-test".to_string(),
            spendguard_context: serde_json::Value::Null,
        };
        let claim = ClaimEstimate {
            predicted_a_tokens: 12,
            run_projection_at_decision_atomic: 1,
            run_predicted_remaining_steps: 2,
            run_steps_completed_so_far: 3,
            ..Default::default()
        };
        let projector = crate::proto::run_cost_projector::v1::ProjectResponse {
            run_projection_at_decision_atomic: 900,
            run_predicted_remaining_steps: 8,
            run_steps_completed_so_far: 4,
            ..Default::default()
        };

        let ce = build_audit_decision_cloudevent(
            &ctx,
            &Uuid::nil(),
            &Uuid::nil(),
            1,
            &[0u8; 32],
            &[],
            &[],
            &enrichment,
            "STRICT_CEILING",
            "schema-bundle-test",
            "test-fingerprint",
            Some(&projector),
            Some(&claim),
        );

        assert_eq!(ce.predicted_a_tokens, 12);
        assert_eq!(ce.run_projection_at_decision_atomic, 900);
        assert_eq!(ce.run_predicted_remaining_steps, 8);
        assert_eq!(ce.run_steps_completed_so_far, 4);
    }

    #[test]
    fn claim_estimate_cannot_override_contract_prediction_policy() {
        let ctx = DecisionContext {
            session_id: "session-test".to_string(),
            workload_instance_id: "sidecar-test".to_string(),
            tenant_id: Uuid::nil().to_string(),
            region: "test-region".to_string(),
        };
        let enrichment = AuditEnrichment {
            run_id: Uuid::nil().to_string(),
            agent_id: "agent-test".to_string(),
            model_family: "model-test".to_string(),
            prompt_hash: "prompt-test".to_string(),
            spendguard_context: serde_json::Value::Null,
        };
        let claim = ClaimEstimate {
            prediction_policy_used: "STRICT_CEILING".into(),
            ..Default::default()
        };

        let ce = build_audit_decision_cloudevent(
            &ctx,
            &Uuid::nil(),
            &Uuid::nil(),
            1,
            &[0u8; 32],
            &[],
            &[],
            &enrichment,
            "ADAPTIVE_CEILING",
            "schema-bundle-test",
            "test-fingerprint",
            None,
            Some(&claim),
        );

        assert_eq!(ce.prediction_policy_used, "ADAPTIVE_CEILING");
    }

    #[test]
    fn allow_path_audit_payload_includes_reason_codes() {
        let ctx = DecisionContext {
            session_id: "session-test".to_string(),
            workload_instance_id: "sidecar-test".to_string(),
            tenant_id: Uuid::nil().to_string(),
            region: "test-region".to_string(),
        };
        let enrichment = AuditEnrichment {
            run_id: "run-test".to_string(),
            agent_id: "agent-test".to_string(),
            model_family: "model-test".to_string(),
            prompt_hash: "prompt-test".to_string(),
            spendguard_context: serde_json::Value::Null,
        };
        let matched_rules = vec!["run-drift-alert".to_string()];
        let reason_codes = vec!["RUN_DRIFT_DETECTED".to_string()];

        let ce = build_audit_decision_cloudevent(
            &ctx,
            &Uuid::nil(),
            &Uuid::nil(),
            1,
            &[0u8; 32],
            &matched_rules,
            &reason_codes,
            &enrichment,
            "STRICT_CEILING",
            "schema-bundle-test",
            "test-fingerprint",
            None,
            None,
        );
        let payload: serde_json::Value =
            serde_json::from_slice(ce.data.as_ref()).expect("audit decision payload JSON");

        assert_eq!(
            payload.get("reason_codes"),
            Some(&serde_json::json!(["RUN_DRIFT_DETECTED"]))
        );
        assert_eq!(ce.schema_bundle_id, "schema-bundle-test");
    }
}
