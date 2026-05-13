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

use chrono::Utc;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    config::Config,
    contract,
    domain::{
        error::DomainError,
        state::{ReservationCtx, SidecarState},
    },
    proto::{
        common::v1::{
            BudgetClaim, CloudEvent, ContractBundleRef, Fencing, Idempotency, LockOrderToken,
            PricingFreeze, UnitRef,
        },
        ledger::v1::{
            commit_estimated_response::Outcome as CommitOutcome,
            query_reservation_context_response::Outcome as QrcOutcome,
            record_denied_decision_response::Outcome as DeniedOutcome,
            release_request::Reason as ReleaseReasonProto,
            release_response::Outcome as ReleaseOutcome,
            reserve_set_response::Outcome, CommitEstimatedRequest, CommitEstimatedResponse,
            QueryReservationContextRequest, RecordDeniedDecisionRequest,
            RecordDeniedDecisionResponse, ReleaseRequest, ReleaseResponse, ReserveSetRequest,
            ReserveSetResponse,
        },
        sidecar_adapter::v1::{
            decision_response::Decision, DecisionRequest, DecisionResponse, LlmCallPostPayload,
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
}

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

    AuditEnrichment {
        run_id,
        agent_id,
        model_family,
        prompt_hash,
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
    /// Reservation TTL — adapter MUST commit/release before this deadline.
    /// None when decision != CONTINUE (no reservation).
    pub ttl_expires_at: Option<prost_types::Timestamp>,
    /// Phase 3 wedge: contract evaluator outputs. Carried through to
    /// DecisionResponse so adapter sees which rules fired and why.
    pub matched_rule_ids: Vec<String>,
    pub reason_codes: Vec<String>,
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
    let decision_kind = eval_outcome.decision;
    let matched_rules = eval_outcome.matched_rule_ids;
    let reason_codes = eval_outcome.reason_codes;

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

    // Use the adapter-supplied idempotency key directly so retries from
    // the same logical trigger collapse via ledger UNIQUE
    // (tenant_id, operation_kind, idempotency_key) — even after a sidecar
    // process restart that wipes the local IdempotencyCache.
    let adapter_idempotency = req.idempotency.as_ref().ok_or_else(|| {
        DomainError::InvalidRequest("DecisionRequest.idempotency required".into())
    })?;
    if adapter_idempotency.key.is_empty() {
        return Err(DomainError::InvalidRequest(
            "DecisionRequest.idempotency.key required".into(),
        ));
    }

    // Cost Advisor P0.5 enrichment: extract run_id / agent_id /
    // model_family / prompt_hash from the request ONCE; thread into
    // both CONTINUE + DENY audit.decision emissions below.
    let enrichment = extract_enrichment(req);

    // Phase 3 wedge: branch CONTINUE vs DENY before building the
    // reserve-specific payload. DENY skips Reserve entirely but still
    // emits an audit_decision row via Ledger.RecordDeniedDecision so
    // Contract §6.1 invariant 「無 audit 則無 effect」 holds.
    if decision_kind != Decision::Continue {
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
            &adapter_idempotency.key,
            effect_hash,
            &enrichment,
        )
        .await;
    }

    let idempotency = Idempotency {
        key: adapter_idempotency.key.clone(),
        // Leave empty so the ledger computes its canonical hash server-side
        // and uses THAT for replay verification (see
        // services/ledger/src/handlers/reserve_set.rs `canonical_request_hash`).
        // The ledger's canonical covers tenant + decision + audit_event +
        // claims + pricing + fencing + ttl + contract_bundle. Recomputing
        // it here would require re-implementing the same canonicalization;
        // empty signals "let server own this".
        request_hash: Vec::new().into(),
    };

    let mut cloudevent = build_audit_decision_cloudevent(
        ctx,
        &decision_id,
        &audit_decision_event_id,
        producer_sequence,
        &snapshot_hash,
        &matched_rules,
        &enrichment,
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
            seconds: (Utc::now()
                + chrono::Duration::seconds(state.inner.reservation_ttl_seconds))
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
                reservation_ids: s.reservations.iter().map(|r| r.reservation_id.clone()).collect(),
                ledger_transaction_id: s.ledger_transaction_id,
                ttl_expires_at: ttl_from_server,
                matched_rule_ids: matched_rules.clone(),
                reason_codes: reason_codes.clone(),
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
            let original_audit_id = uuid::Uuid::parse_str(&r.audit_decision_event_id)
                .map_err(|e| DomainError::DecisionStage(format!(
                    "ledger replay returned malformed audit_decision_event_id '{}': {}",
                    r.audit_decision_event_id, e
                )))?;
            let original_decision_id = uuid::Uuid::parse_str(&r.decision_id)
                .map_err(|e| DomainError::DecisionStage(format!(
                    "ledger replay returned malformed decision_id '{}': {}",
                    r.decision_id, e
                )))?;
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
                // Original TTL anchor from first call (mirrored from the
                // server-stored value, not recomputed).
                ttl_expires_at: r.ttl_expires_at.clone(),
                matched_rule_ids: matched_rules.clone(),
                reason_codes: reason_codes.clone(),
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

fn build_audit_decision_cloudevent(
    ctx: &DecisionContext,
    decision_id: &Uuid,
    audit_decision_event_id: &Uuid,
    producer_sequence: u64,
    snapshot_hash: &[u8; 32],
    matched_rules: &[String],
    enrichment: &AuditEnrichment,
) -> CloudEvent {
    let payload = serde_json::json!({
        "snapshot_hash":   hex::encode(snapshot_hash),
        "matched_rules":   matched_rules,
        "session_id":      ctx.session_id,
        // Cost Advisor P0.5 enrichment fields. Empty strings indicate
        // the SDK adapter did not provide enrichment for this call —
        // rules treat empties as "not classified" and don't fire on
        // those rows. See audit-report §8.5.
        "agent_id":        enrichment.agent_id,
        "model_family":    enrichment.model_family,
        "prompt_hash":     enrichment.prompt_hash,
    });
    let payload_bytes =
        serde_json::to_vec(&payload).expect("snapshot json serialization is infallible");
    CloudEvent {
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
        schema_bundle_id: String::new(),
        producer_id: format!("sidecar:{}", ctx.workload_instance_id),
        producer_sequence,
        producer_signature: vec![].into(), // POC: signing TBD
        signing_key_id: String::new(),
    }
}

// (Producer sequence now lives on SidecarState, initialized from ledger
// replay at startup so restarts don't collide with prior sequences.)

// =====================================================================
// Phase 3 wedge — DENY lane.
// =====================================================================

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
    adapter_idempotency_key: &str,
    effect_hash: [u8; 32],
    enrichment: &AuditEnrichment,
) -> Result<DecisionOutput, DomainError> {
    let final_decision_str = match decision_kind {
        Decision::Stop => "STOP",
        Decision::RequireApproval => "REQUIRE_APPROVAL",
        Decision::Degrade => "DEGRADE",
        Decision::Skip => "SKIP",
        // Continue is filtered out by caller; Unspecified should not flow.
        _ => {
            return Err(DomainError::DecisionStage(format!(
                "run_record_denied_decision called with unsupported decision {:?}",
                decision_kind
            )))
        }
    };

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
    let payload = serde_json::json!({
        "snapshot_hash":     hex::encode(snapshot_hash),
        "matched_rules":     matched_rules,
        "reason_codes":      reason_codes,
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
    let payload_bytes = serde_json::to_vec(&payload)
        .expect("denied decision json serialization is infallible");
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
        schema_bundle_id: String::new(),
        producer_id: format!("sidecar:{}", ctx.workload_instance_id),
        producer_sequence,
        producer_signature: vec![].into(),
        signing_key_id: String::new(),
    };
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
    let (decision_context_json, requested_effect_json, approval_ttl_seconds) =
        if decision_kind == Decision::RequireApproval {
            let primary_claim = claims.first();
            let (unit_id, unit_kind_str, unit_token_kind) = match primary_claim.and_then(|c| c.unit.as_ref()) {
                Some(u) => (
                    u.unit_id.clone(),
                    match u.kind {
                        x if x == crate::proto::common::v1::unit_ref::Kind::Monetary as i32 => "MONETARY",
                        x if x == crate::proto::common::v1::unit_ref::Kind::Token as i32 => "TOKEN",
                        x if x == crate::proto::common::v1::unit_ref::Kind::Credit as i32 => "CREDIT",
                        x if x == crate::proto::common::v1::unit_ref::Kind::NonMonetary as i32 => "NON_MONETARY",
                        _ => "MONETARY",
                    },
                    u.token_kind.clone(),
                ),
                None => (String::new(), "MONETARY", String::new()),
            };
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
            });
            let amount = primary_claim.map(|c| c.amount_atomic.clone()).unwrap_or_default();
            let direction = match primary_claim.map(|c| c.direction) {
                Some(x) if x == crate::proto::common::v1::budget_claim::Direction::Credit as i32 => "CREDIT",
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
            ttl_expires_at: None,
            matched_rule_ids: matched_rules.to_vec(),
            reason_codes: reason_codes.to_vec(),
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
            let original_decision_id = uuid::Uuid::parse_str(&r.decision_id)
                .map_err(|e| DomainError::DecisionStage(format!(
                    "ledger denied replay returned malformed decision_id '{}': {}",
                    r.decision_id, e
                )))?;
            let original_audit_id = uuid::Uuid::parse_str(&r.audit_decision_event_id)
                .map_err(|e| DomainError::DecisionStage(format!(
                    "ledger denied replay returned malformed audit_decision_event_id '{}': {}",
                    r.audit_decision_event_id, e
                )))?;
            Ok(DecisionOutput {
                decision_id: original_decision_id,
                audit_decision_event_id: original_audit_id,
                effect_hash,
                decision: decision_kind,
                reservation_set_id: String::new(),
                reservation_ids: vec![],
                ledger_transaction_id: r.ledger_transaction_id,
                ttl_expires_at: None,
                matched_rule_ids: matched_rules.to_vec(),
                reason_codes: reason_codes.to_vec(),
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
        approval_request_id: String::new(),
        approval_ttl: None,
        approver_role: String::new(),
        terminal: matches!(out.decision, Decision::Stop),
        error: None,
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
            "estimated_amount_atomic and provider_reported_amount_atomic are mutually exclusive".into(),
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

    let reservation_uuid = uuid::Uuid::parse_str(&payload.reservation_id).map_err(|e| {
        DomainError::InvalidRequest(format!("reservation_id parse: {e}"))
    })?;

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
        .map_err(|e| {
            DomainError::Internal(anyhow::anyhow!("ctx original amount parse: {e}"))
        })?;
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

    let ce_payload = serde_json::json!({
        "kind": "commit_estimated",
        "reservation_id": reservation_uuid.to_string(),
        "estimated_amount_atomic": payload.estimated_amount_atomic,
        "decision_id": resv.decision_id.to_string(),
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
        data: serde_json::to_vec(&ce_payload).expect("payload json").into(),
        tenant_id: ctx.tenant_id.clone(),
        run_id: String::new(),
        decision_id: resv.decision_id.to_string(),
        schema_bundle_id: String::new(),
        producer_id: format!("sidecar:{}", ctx.workload_instance_id),
        producer_sequence: producer_seq,
        producer_signature: vec![].into(),
        signing_key_id: String::new(),
    };
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
    let response: CommitEstimatedResponse =
        state.inner.ledger.commit_estimated(request).await?;
    match response.outcome {
        Some(CommitOutcome::Success(s)) => {
            state.inner.reservation_cache.lock().remove(&reservation_uuid);
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
            state.inner.reservation_cache.lock().remove(&reservation_uuid);
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
        Some(CommitOutcome::Error(e)) => {
            map_proto_error_to_domain(e.code, e.message)
        }
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

pub async fn run_release(
    cfg: &crate::config::Config,
    state: &SidecarState,
    ctx: &DecisionContext,
    reservation_uuid: uuid::Uuid,
    reason: ReleaseReason,
    payload_metadata: Option<&str>,
) -> Result<ReleaseOutput, DomainError> {
    let _ = cfg;

    // 1) Recover reservation context (cache → ledger query).
    let resv = recover_reservation_ctx(state, &ctx.tenant_id, &reservation_uuid).await?;

    // 2) Short-circuit on non-`reserved` states (Codex round 1 P1.4
    //    state-check ordering; SP also enforces, but failing fast at
    //    sidecar avoids burning a producer_sequence).
    if resv.current_state != "reserved" {
        return Err(DomainError::ReservationStateConflict(format!(
            "reservation {} current_state={} (expected reserved for release)",
            reservation_uuid, resv.current_state
        )));
    }

    // 3) Fencing epoch parity (DD5 C1).
    let fencing_state = state
        .inner
        .fencing
        .read()
        .clone()
        .ok_or_else(|| DomainError::FencingAcquire("no active fencing scope".into()))?;
    if fencing_state.epoch != resv.fencing_epoch_at_post {
        return Err(DomainError::FencingEpochStale(format!(
            "current epoch {} differs from reserve-time epoch {}; reservation will TTL-release",
            fencing_state.epoch, resv.fencing_epoch_at_post
        )));
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
        data: serde_json::to_vec(&ce_payload).expect("payload json").into(),
        tenant_id: ctx.tenant_id.clone(),
        run_id: String::new(),
        decision_id: resv.decision_id.to_string(),
        schema_bundle_id: String::new(),
        producer_id: format!("sidecar:{}", ctx.workload_instance_id),
        producer_sequence: producer_seq,
        producer_signature: vec![].into(),
        signing_key_id: String::new(),
    };
    crate::audit::sign_cloudevent_in_place(&*state.inner.signer, &mut cloudevent).await?;

    let request = ReleaseRequest {
        tenant_id: ctx.tenant_id.clone(),
        reservation_set_id: reservation_set_id.to_string(),
        idempotency: Some(Idempotency {
            key: format!("release:{}:1", reservation_uuid),
            request_hash: Vec::new().into(),
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
            state.inner.reservation_cache.lock().remove(&reservation_uuid);
            state
                .inner
                .decision_id_to_reservation
                .lock()
                .remove(&resv.decision_id);
            Ok(ReleaseOutput {
                ledger_transaction_id: s.ledger_transaction_id,
                released_reservation_ids: s.released_reservation_ids,
            })
        }
        Some(ReleaseOutcome::Replay(r)) => {
            state.inner.reservation_cache.lock().remove(&reservation_uuid);
            state
                .inner
                .decision_id_to_reservation
                .lock()
                .remove(&resv.decision_id);
            Ok(ReleaseOutput {
                ledger_transaction_id: r.ledger_transaction_id,
                released_reservation_ids: vec![reservation_uuid.to_string()],
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

fn map_proto_error_to_domain<T>(
    code: i32,
    message: String,
) -> Result<T, DomainError> {
    use crate::proto::common::v1::error::Code as PC;
    let pc = PC::try_from(code).unwrap_or(PC::Unspecified);
    Err(match pc {
        PC::FencingEpochStale => DomainError::FencingEpochStale(message),
        PC::ReservationStateConflict => DomainError::ReservationStateConflict(message),
        PC::ReservationTtlExpired => DomainError::ReservationTtlExpired(message),
        PC::PricingFreezeMismatch => DomainError::PricingFreezeMismatch(message),
        PC::OverrunReservation => DomainError::OverrunReservation(message),
        PC::MultiReservationCommitDeferred => DomainError::MultiReservationCommitDeferred(message),
        _ => DomainError::DecisionStage(format!("ledger error code={code} msg={message}")),
    })
}

async fn recover_reservation_ctx(
    state: &SidecarState,
    tenant_id: &str,
    reservation_id: &uuid::Uuid,
) -> Result<ReservationCtx, DomainError> {
    if let Some(ctx) = state.inner.reservation_cache.lock().get(reservation_id).cloned() {
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
                budget_id: uuid::Uuid::parse_str(&c.budget_id).map_err(|e| {
                    DomainError::Internal(anyhow::anyhow!("budget_id: {e}"))
                })?,
                window_instance_id: uuid::Uuid::parse_str(&c.window_instance_id).map_err(
                    |e| DomainError::Internal(anyhow::anyhow!("window_instance_id: {e}")),
                )?,
                unit_id: uuid::Uuid::parse_str(c.unit.as_ref().map(|u| u.unit_id.as_str()).unwrap_or(""))
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
                fencing_scope_id: uuid::Uuid::parse_str(&c.fencing_scope_id).map_err(|e| {
                    DomainError::Internal(anyhow::anyhow!("fencing_scope_id: {e}"))
                })?,
                fencing_epoch_at_post: c.fencing_epoch_at_post,
                decision_id: uuid::Uuid::parse_str(&c.decision_id).map_err(|e| {
                    DomainError::Internal(anyhow::anyhow!("decision_id: {e}"))
                })?,
                ttl_expires_at: c
                    .ttl_expires_at
                    .map(|t| chrono::DateTime::<chrono::Utc>::from_timestamp(t.seconds, t.nanos as u32).unwrap_or_default())
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
        let Ok(rid) = uuid::Uuid::parse_str(&r.reservation_id) else { continue };
        let Ok(budget_id) = uuid::Uuid::parse_str(&r.budget_id) else { continue };
        let Ok(window_instance_id) = uuid::Uuid::parse_str(&r.window_instance_id) else { continue };
        let unit_id = match r.unit.as_ref().and_then(|u| uuid::Uuid::parse_str(&u.unit_id).ok()) {
            Some(u) => u,
            None => continue,
        };
        let ttl = r
            .ttl_expires_at
            .as_ref()
            .and_then(|t| chrono::DateTime::<chrono::Utc>::from_timestamp(t.seconds, t.nanos as u32))
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
