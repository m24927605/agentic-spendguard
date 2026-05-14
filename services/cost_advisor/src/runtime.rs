//! P1 runtime: rule registry + evaluation + cost_findings UPSERT.
//!
//! Orchestrates rule execution for a (tenant, date) bucket:
//!   1. Build registry from compiled-in rules (registers only those
//!      whose `is_ready()` returns true so placeholders never run).
//!   2. For each ready rule, execute its SQL against the appropriate
//!      pool (P1 idle_reservation_rate reads ledger; P1.5 rules read
//!      canonical) via `rule.target_db()`.
//!   3. Decode result into [`FindingEvidence`] proto + JSONB shape.
//!   4. UPSERT via `cost_findings_upsert()` SP on the ledger DB
//!      (CA-P1.6: cost_findings moved from canonical to ledger to get
//!      a real Postgres FK with approval_requests; see spec §0.3 +
//!      integration-doc §9).
//!
//! P1 ships exactly one rule (`idle_reservation_rate_v1`). The
//! registry pattern lets P1.5 add the other 3 rules without touching
//! this file.

use anyhow::{anyhow, Context, Result};
use bigdecimal::BigDecimal;
use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use sqlx::{postgres::PgRow, PgPool, Row};
use uuid::Uuid;

use crate::fingerprint;
use crate::proto::cost_advisor::v1::{
    FindingCategory, FindingEvidence, FindingScope, Metric, MetricUnit, PiiClassification,
    ScopeType, Severity, WasteConfidence, WasteEstimate, WasteMethod,
};
use crate::rule::{CostRule, TargetDb};
use crate::rules::{failed_retry_burn, idle_reservation_rate, runaway_loop};
use crate::sql_rule::SqlCostRule;

/// Single emitted finding row returned to a CLI / future REST caller.
///
/// `estimated_waste_micros_usd` is `Option<i64>` so consumers can
/// distinguish "USD estimate pending" (None / null in JSON) from
/// "$0 of verified waste" (Some(0)). Codex CA-P1 r3 caught that the
/// earlier i64-with-unwrap_or(0) flattened these two states into
/// indistinguishable zeros.
#[derive(Debug, Serialize)]
pub struct EmittedFinding {
    pub outcome: String,
    pub finding_id: String,
    pub rule_id: String,
    pub severity: String,
    pub confidence: f64,
    pub estimated_waste_micros_usd: Option<i64>,
    pub evidence: serde_json::Value,
    pub proposed_dsl_patch: Option<serde_json::Value>,
}

/// Build the P1 rule registry. Returns only rules whose
/// `is_ready()` returns true so the runtime never invokes a
/// placeholder.
pub fn build_registry() -> Vec<SqlCostRule> {
    [
        idle_reservation_rate::descriptor(),
        failed_retry_burn::descriptor(),
        runaway_loop::descriptor(),
    ]
    .into_iter()
    .filter(|r| r.is_ready())
    .collect()
}

/// Single-tenant evaluation entry point. Runs every registered rule
/// against the (tenant, date) bucket, UPSERTs findings, optionally
/// emits proposed DSL patches.
///
/// `propose_patches=true` returns the RFC-6902 patch in the result
/// row so a CLI caller can show it; it does NOT write an
/// `approval_requests` row in P1 (gated on owner-ack #53/#54).
///
/// CA-P1.6: cost_findings now lives in `spendguard_ledger` (issue
/// #56 — moved to get a real FK with approval_requests). All
/// WRITES (cost_findings_upsert SP) go to the ledger pool. READS
/// still dispatch by rule's target_db() because P1.5 rules read
/// canonical_events from the canonical pool.
pub async fn evaluate_tenant_day(
    ledger: &PgPool,
    canonical: &PgPool,
    tenant_id: Uuid,
    bucket_date: NaiveDate,
    propose_patches: bool,
) -> Result<Vec<EmittedFinding>> {
    let mut emitted = Vec::new();

    for rule in build_registry() {
        // Per-rule READ pool (codex CA-P1.5 r1 P1).
        let target_pool = match rule.target_db() {
            TargetDb::Ledger => ledger,
            TargetDb::Canonical => canonical,
        };
        let Some(finding) = run_rule(target_pool, &rule, tenant_id, bucket_date).await? else {
            continue;
        };

        // WRITE pool is always ledger (CA-P1.6: cost_findings is
        // ledger-resident; cost_findings_upsert lives in ledger DB).
        let outcome = upsert_finding(ledger, tenant_id, &finding).await?;

        let proposed_patch = if propose_patches {
            build_proposed_patch_for_rule(rule.rule_id(), &finding)
        } else {
            None
        };

        emitted.push(EmittedFinding {
            outcome: outcome.outcome,
            finding_id: outcome.finding_id.to_string(),
            rule_id: finding.proto.rule_id.clone(),
            severity: severity_str(finding.proto.severity()).to_string(),
            confidence: finding.confidence,
            // Codex CA-P1 r3 P2: Option preserves "USD estimate
            // pending" semantics — None / null in JSON. Some(n)
            // means a real quantified figure from waste_estimate.
            estimated_waste_micros_usd: finding
                .proto
                .waste_estimate
                .as_ref()
                .map(|w| w.micros_usd),
            evidence: finding.proto_json.clone(),
            proposed_dsl_patch: proposed_patch,
        });
    }

    Ok(emitted)
}

struct DecodedFinding {
    proto: FindingEvidence,
    proto_json: serde_json::Value,
    confidence: f64,
    finding_id: Uuid,
    detected_at: DateTime<Utc>,
    sample_decision_ids: Vec<Uuid>,
}

async fn run_rule(
    ledger: &PgPool,
    rule: &SqlCostRule,
    tenant_id: Uuid,
    bucket_date: NaiveDate,
) -> Result<Option<DecodedFinding>> {
    let row_opt: Option<PgRow> = sqlx::query(rule.sql())
        .bind(tenant_id)
        .bind(bucket_date)
        .fetch_optional(ledger)
        .await
        .with_context(|| format!("execute rule {}", rule.rule_id()))?;

    let Some(row) = row_opt else {
        return Ok(None);
    };

    match rule.rule_id() {
        "idle_reservation_rate_v1" => {
            decode_idle_reservation_rate(rule, row, tenant_id, bucket_date).map(Some)
        }
        "failed_retry_burn_v1" => {
            decode_failed_retry_burn(rule, row, tenant_id, bucket_date).map(Some)
        }
        "runaway_loop_v1" => {
            decode_runaway_loop(rule, row, tenant_id, bucket_date).map(Some)
        }
        other => Err(anyhow!("no decoder registered for rule {}", other)),
    }
}

fn decode_failed_retry_burn(
    rule: &SqlCostRule,
    row: PgRow,
    tenant_id: Uuid,
    bucket_date: NaiveDate,
) -> Result<DecodedFinding> {
    let affected: i64 = row.try_get("affected_run_prompt_groups")?;
    let total_attempts: i64 = row.try_get("total_attempts")?;
    let total_billed_failures: i64 = row.try_get("total_billed_failures")?;
    let total_atomic_sum_bd: Option<bigdecimal::BigDecimal> =
        row.try_get("total_failed_atomic_sum").ok();
    // Codex CA-P1.5 r1 P2: surface the atomic total in metrics.
    // BigDecimal → f64 loses precision for huge values but matches
    // Metric.value's wire type. Range fine for realistic atomic
    // sums (< 2^53).
    use bigdecimal::ToPrimitive;
    let total_atomic_sum_f = total_atomic_sum_bd
        .as_ref()
        .and_then(|bd| bd.to_f64())
        .unwrap_or(0.0);
    let sample_ids: Vec<Uuid> = row
        .try_get::<Option<Vec<Uuid>>, _>("sample_decision_ids")?
        .unwrap_or_default();

    let time_bucket = bucket_date.format("%Y-%m-%d").to_string();
    let scope = FindingScope {
        scope_type: ScopeType::Run as i32,
        agent_id: String::new(),
        run_id: String::new(), // tenant-bucket aggregate across runs
        tool_name: String::new(),
        model_family: String::new(),
    };
    let fingerprint_hex =
        fingerprint::compute(rule.rule_id(), &tenant_id.to_string(), &scope, &time_bucket);

    let metrics = vec![
        metric("affected_run_prompt_groups", affected as f64, MetricUnit::Count, "derived: COUNT(DISTINCT run_id, prompt_hash) in step3"),
        metric("total_attempts", total_attempts as f64, MetricUnit::Calls, "derived: SUM(attempt_count) across groups"),
        metric("total_billed_failures", total_billed_failures as f64, MetricUnit::Calls, "derived: SUM(billed_failure_count) across groups"),
        // Atomic-units waste exposed so operators see magnitude even
        // before baseline_refresher converts to USD. Unit is the
        // ledger's per-budget atomic (e.g. tokens × scale_factor);
        // pricing snapshot converts to USD during P2.
        metric("total_failed_atomic_sum", total_atomic_sum_f, MetricUnit::Count, "derived: SUM(estimated_amount_atomic) of wasted attempts; ledger-unit-atomic"),
    ];

    // P1 contract: estimated_amount_atomic is unit-atomic, not USD
    // micros. Same nullable-waste pattern as idle_reservation_rate_v1
    // — emit WasteEstimate only when baseline_refresher computes a
    // real USD figure. P1.5 first cut returns NULL waste.
    let waste_estimate: Option<WasteEstimate> = None;

    let proto = FindingEvidence {
        rule_id: rule.rule_id().into(),
        rule_version: rule.rule_version(),
        fingerprint: fingerprint_hex,
        category: FindingCategory::DetectedWaste as i32,
        scope: Some(scope),
        metrics,
        decision_refs: sample_ids.iter().map(|u| u.to_string()).collect(),
        waste_estimate,
        severity: Severity::Warn as i32,
        time_bucket,
        co_observed_rules: Vec::new(),
    };

    let proto_json = build_proto_json(&proto, "warn");

    Ok(DecodedFinding {
        proto,
        proto_json,
        confidence: 0.85, // high-medium: billed failures are deterministic
        finding_id: Uuid::now_v7(),
        detected_at: Utc::now(),
        sample_decision_ids: sample_ids,
    })
}

fn decode_runaway_loop(
    rule: &SqlCostRule,
    row: PgRow,
    tenant_id: Uuid,
    bucket_date: NaiveDate,
) -> Result<DecodedFinding> {
    let affected: i64 = row.try_get("affected_run_prompt_groups")?;
    let total_calls: i64 = row.try_get("total_calls")?;
    let max_depth: i64 = row.try_get("max_loop_depth")?;
    let total_atomic_sum_bd: Option<bigdecimal::BigDecimal> =
        row.try_get("total_atomic_sum").ok();
    use bigdecimal::ToPrimitive;
    let total_atomic_sum_f = total_atomic_sum_bd
        .as_ref()
        .and_then(|bd| bd.to_f64())
        .unwrap_or(0.0);
    let sample_ids: Vec<Uuid> = row
        .try_get::<Option<Vec<Uuid>>, _>("sample_decision_ids")?
        .unwrap_or_default();

    let time_bucket = bucket_date.format("%Y-%m-%d").to_string();
    let scope = FindingScope {
        scope_type: ScopeType::Run as i32,
        agent_id: String::new(),
        run_id: String::new(),
        tool_name: String::new(),
        model_family: String::new(),
    };
    let fingerprint_hex =
        fingerprint::compute(rule.rule_id(), &tenant_id.to_string(), &scope, &time_bucket);

    let metrics = vec![
        metric("affected_run_prompt_groups", affected as f64, MetricUnit::Count, "derived: COUNT(DISTINCT run_id, prompt_hash) in step3"),
        metric("total_calls", total_calls as f64, MetricUnit::Calls, "derived: SUM(call_count) across groups"),
        metric("max_loop_depth", max_depth as f64, MetricUnit::Count, "derived: MAX(call_count) across groups"),
        // Codex CA-P1.5 r1 P2: surface atomic-units magnitude so
        // operators see the size of the loop's spend.
        metric("total_atomic_sum", total_atomic_sum_f, MetricUnit::Count, "derived: SUM(estimated_amount_atomic) across looping calls; ledger-unit-atomic"),
    ];

    let waste_estimate: Option<WasteEstimate> = None;

    let proto = FindingEvidence {
        rule_id: rule.rule_id().into(),
        rule_version: rule.rule_version(),
        fingerprint: fingerprint_hex,
        category: FindingCategory::DetectedWaste as i32,
        scope: Some(scope),
        metrics,
        decision_refs: sample_ids.iter().map(|u| u.to_string()).collect(),
        waste_estimate,
        severity: Severity::Warn as i32,
        time_bucket,
        co_observed_rules: Vec::new(),
    };

    let proto_json = build_proto_json(&proto, "warn");

    Ok(DecodedFinding {
        proto,
        proto_json,
        confidence: 0.65, // loops are heuristic: tight retry could be intentional
        finding_id: Uuid::now_v7(),
        detected_at: Utc::now(),
        sample_decision_ids: sample_ids,
    })
}

/// Common JSON builder for FindingEvidence — same shape as the
/// idle_reservation_rate decoder's inline json! but factored so all
/// rule decoders use one canonical layout.
fn build_proto_json(proto: &FindingEvidence, severity: &str) -> serde_json::Value {
    let scope_type_str = match proto.scope.as_ref().map(|s| s.scope_type).unwrap_or(0) {
        x if x == ScopeType::TenantGlobal as i32 => "tenant_global",
        x if x == ScopeType::Run as i32 => "run",
        x if x == ScopeType::Agent as i32 => "agent",
        x if x == ScopeType::Tool as i32 => "tool",
        _ => "unspecified",
    };
    serde_json::json!({
        "rule_id": proto.rule_id,
        "rule_version": proto.rule_version,
        "fingerprint": proto.fingerprint,
        "category": "detected_waste",
        "scope": { "scope_type": scope_type_str },
        "metrics": proto.metrics.iter().map(|m| serde_json::json!({
            "name": m.name,
            "value": m.value,
            "unit": metric_unit_str(m.unit),
            "source_field": m.source_field,
            "pii_classification": "none",
            "derivation": m.derivation,
        })).collect::<Vec<_>>(),
        "decision_refs": proto.decision_refs,
        "waste_estimate": proto.waste_estimate.as_ref().map(|w| serde_json::json!({
            "micros_usd": w.micros_usd,
            "method": waste_method_str(w.method),
            "confidence": waste_confidence_str(w.confidence),
            "explanation": w.explanation,
        })),
        "severity": severity,
        "time_bucket": proto.time_bucket,
    })
}

fn decode_idle_reservation_rate(
    rule: &SqlCostRule,
    row: PgRow,
    tenant_id: Uuid,
    bucket_date: NaiveDate,
) -> Result<DecodedFinding> {
    let total: i64 = row.try_get("total_reservations")?;
    let ttl_expired: i64 = row.try_get("ttl_expired_count")?;
    let median_ttl: i32 = row.try_get("median_ttl_seconds")?;
    let p95_ttl: i32 = row.try_get("p95_ttl_seconds")?;
    // Codex CA-P1 r1 P1: rule SQL now samples decision_ids (not
    // reservation_ids) so the dashboard "view raw evidence" link
    // points at canonical_events.decision_id rows.
    let sample_ids: Vec<Uuid> = row
        .try_get::<Option<Vec<Uuid>>, _>("sample_decision_ids")?
        .unwrap_or_default();
    // Codex CA-P1 r1 P2: rule SQL returns NULL waste until the P2
    // baseline_refresher computes a real per-tenant figure. Map
    // NULL → None and surface heuristic/low confidence.
    let waste_micros_opt: Option<i64> = row.try_get("estimated_waste_micros_usd")?;

    let idle_ratio = if total > 0 {
        ttl_expired as f64 / total as f64
    } else {
        0.0
    };
    let time_bucket = bucket_date.format("%Y-%m-%d").to_string();

    let scope = FindingScope {
        scope_type: ScopeType::TenantGlobal as i32,
        agent_id: String::new(),
        run_id: String::new(),
        tool_name: String::new(),
        model_family: String::new(),
    };
    // Codex CA-P1 r1 P2: tenant_id is now part of fingerprint input
    // so tenant_global findings on the same day for different tenants
    // produce DISTINCT fingerprints.
    let fingerprint_hex =
        fingerprint::compute(rule.rule_id(), &tenant_id.to_string(), &scope, &time_bucket);

    let metrics = vec![
        metric("total_reservations", total as f64, MetricUnit::Count, "reservations_with_ttl_status_v1.reservation_id"),
        metric("ttl_expired_count", ttl_expired as f64, MetricUnit::Count, "reservations_with_ttl_status_v1.derived_state"),
        Metric {
            name: "idle_ratio".into(),
            value: idle_ratio,
            unit: MetricUnit::Ratio as i32,
            source_field: "derived: ttl_expired_count / total_reservations".into(),
            pii_classification: PiiClassification::None as i32,
            derivation: "ttl_expired_count / total_reservations".into(),
            ci95_low: None,
            ci95_high: None,
        },
        metric("median_ttl_seconds", median_ttl as f64, MetricUnit::Seconds, "derived: PERCENTILE_CONT(0.5) of ttl_seconds"),
        metric("p95_ttl_seconds", p95_ttl as f64, MetricUnit::Seconds, "derived: PERCENTILE_CONT(0.95) of ttl_seconds"),
    ];

    // Codex CA-P1 r2 P2: when the rule SQL returns NULL waste (P1
    // path; baseline_refresher lands in P2), emit NO WasteEstimate
    // at all. Proto §4.0 makes WasteEstimate nullable
    // ("Optional; null for unquantifiable hypothesis findings");
    // emitting micros_usd=0 silently sums to "no waste detected" in
    // any consumer that totals findings. Only when a real USD
    // figure flows from baseline_refresher (P2) do we attach a
    // WasteEstimate (method=baseline_excess + confidence=medium).
    let waste_estimate = waste_micros_opt.map(|usd| WasteEstimate {
        micros_usd: usd,
        method: WasteMethod::BaselineExcess as i32,
        confidence: WasteConfidence::Medium as i32,
        explanation: format!(
            "{} of {} reservations TTL'd (idle ratio {:.0}%); median TTL {}s — contract reservation TTL is held longer than the workload's typical commit latency.",
            ttl_expired, total, idle_ratio * 100.0, median_ttl
        ),
    });

    let severity = Severity::Warn;
    let finding_id = Uuid::now_v7();
    let detected_at = Utc::now();
    let decision_refs: Vec<String> = sample_ids.iter().map(|u| u.to_string()).collect();

    let proto = FindingEvidence {
        rule_id: rule.rule_id().into(),
        rule_version: rule.rule_version(),
        fingerprint: fingerprint_hex,
        category: FindingCategory::DetectedWaste as i32,
        scope: Some(scope),
        metrics,
        decision_refs,
        waste_estimate,
        severity: severity as i32,
        time_bucket,
        co_observed_rules: Vec::new(),
    };

    // Codex CA-P1 r1 P1: spec §4.0 JSONSchema uses LOWERCASE enum
    // values (detected_waste, tenant_global, baseline_excess, etc.).
    // Earlier draft emitted SCREAMING_SNAKE proto-style names which
    // would break any §4.0 schema validator + downstream consumers.
    let proto_json = serde_json::json!({
        "rule_id": proto.rule_id,
        "rule_version": proto.rule_version,
        "fingerprint": proto.fingerprint,
        "category": "detected_waste",
        "scope": { "scope_type": "tenant_global" },
        "metrics": proto.metrics.iter().map(|m| serde_json::json!({
            "name": m.name,
            "value": m.value,
            "unit": metric_unit_str(m.unit),
            "source_field": m.source_field,
            "pii_classification": "none",
            "derivation": m.derivation,
        })).collect::<Vec<_>>(),
        "decision_refs": proto.decision_refs,
        "waste_estimate": proto.waste_estimate.as_ref().map(|w| serde_json::json!({
            "micros_usd": w.micros_usd,
            "method": waste_method_str(w.method),
            "confidence": waste_confidence_str(w.confidence),
            "explanation": w.explanation,
        })),
        "severity": severity_str(severity),
        "time_bucket": proto.time_bucket,
    });

    Ok(DecodedFinding {
        proto,
        proto_json,
        confidence: 0.75,
        finding_id,
        detected_at,
        sample_decision_ids: sample_ids,
    })
}

fn metric(name: &str, value: f64, unit: MetricUnit, source_field: &str) -> Metric {
    Metric {
        name: name.into(),
        value,
        unit: unit as i32,
        source_field: source_field.into(),
        pii_classification: PiiClassification::None as i32,
        derivation: String::new(),
        ci95_low: None,
        ci95_high: None,
    }
}

struct UpsertOutcome {
    outcome: String,
    finding_id: Uuid,
}

async fn upsert_finding(
    canonical: &PgPool,
    tenant_id: Uuid,
    finding: &DecodedFinding,
) -> Result<UpsertOutcome> {
    let confidence_bd: BigDecimal = finding
        .confidence
        .to_string()
        .parse()
        .context("convert confidence to BigDecimal")?;

    let row: PgRow = sqlx::query(
        r#"
        SELECT outcome, finding_id, finding_detected_at
          FROM cost_findings_upsert(
            $1::uuid, $2::char(64), $3::uuid, $4::timestamptz,
            $5::text, $6::int, $7::text, $8::text, $9::numeric,
            $10::text, $11::text, $12::text,
            $13::jsonb, $14::bigint, $15::uuid[]
          )
        "#,
    )
    .bind(finding.finding_id)
    .bind(&finding.proto.fingerprint)
    .bind(tenant_id)
    .bind(finding.detected_at)
    .bind(&finding.proto.rule_id)
    .bind(finding.proto.rule_version as i32)
    .bind("detected_waste")
    .bind(severity_str(finding.proto.severity()))
    .bind(confidence_bd)
    .bind::<Option<String>>(None)
    .bind::<Option<String>>(None)
    .bind::<Option<String>>(None)
    .bind(&finding.proto_json)
    // Codex CA-P1 r3 P2: bind Option<i64> so NULL flows into
    // cost_findings.estimated_waste_micros_usd (nullable per spec
    // §4.1). Earlier .unwrap_or(0) coerced unquantified findings
    // to a stored zero — indistinguishable from a real $0
    // verified-waste row when consumers SUM the column.
    .bind(
        finding
            .proto
            .waste_estimate
            .as_ref()
            .map(|w| w.micros_usd),
    )
    .bind(&finding.sample_decision_ids)
    .fetch_one(canonical)
    .await
    .context("cost_findings_upsert SP call")?;

    Ok(UpsertOutcome {
        outcome: row.try_get("outcome")?,
        finding_id: row.try_get("finding_id")?,
    })
}

/// Build the optional `proposed_dsl_patch` for a rule's emitted finding.
///
/// Codex CA-P1 r2 P1: proposed_dsl_patch is consumed downstream by
/// bundle_registry's apply pipeline as a real RFC-6902 DSL delta.
/// Emitting a non-mutating `test` op against a non-existent metadata
/// path would FAIL when applied. For tenant-global scope findings
/// (no specific budget identified), there is no safe RFC-6902 patch
/// in P1 because:
///   * The contract DSL's addressable-path schema is unresolved
///     (owner-ack #55).
///   * Tenant-global rules don't pick a specific budget — the
///     operator must do that before any patch is applicable.
///
/// So P1 returns None for tenant-global scope. The
/// EmittedFinding.evidence carries the human-readable
/// recommendation in waste_estimate.explanation + metrics; a future
/// P3.5 owner-ack resolution unlocks real-patch emission for
/// budget/agent-scoped rules.
///
/// Returns None for tenant_global scope. Returns Some(patch) for
/// budget/agent/run/tool-scoped findings (none of which exist in P1
/// — placeholder for P1.5).
fn build_proposed_patch_for_rule(
    _rule_id: &str,
    finding: &DecodedFinding,
) -> Option<serde_json::Value> {
    let scope_type = finding
        .proto
        .scope
        .as_ref()
        .map(|s| s.scope_type)
        .unwrap_or(0);
    let is_tenant_global = ScopeType::try_from(scope_type)
        .map(|s| matches!(s, ScopeType::TenantGlobal))
        .unwrap_or(true);
    if is_tenant_global {
        // No safe patch in P1; the EmittedFinding's evidence /
        // explanation guides the operator to manually review the
        // affected budgets.
        return None;
    }

    // Reserved for P1.5: rules with agent/run/tool scope can emit
    // budget-specific patches. None of those ship in P1; this branch
    // never fires today.
    None
}

// ---- enum → string helpers --------------------------------------

fn metric_unit_str(unit: i32) -> &'static str {
    match MetricUnit::try_from(unit).unwrap_or(MetricUnit::Unspecified) {
        MetricUnit::Unspecified => "METRIC_UNIT_UNSPECIFIED",
        MetricUnit::MicrosUsd => "micros_usd",
        MetricUnit::Usd => "usd",
        MetricUnit::Tokens => "tokens",
        MetricUnit::Calls => "calls",
        MetricUnit::Seconds => "seconds",
        MetricUnit::Ratio => "ratio",
        MetricUnit::Count => "count",
        MetricUnit::Percent => "percent",
        MetricUnit::Multiplier => "multiplier",
    }
}

fn severity_str(s: Severity) -> &'static str {
    match s {
        Severity::Critical => "critical",
        Severity::Warn => "warn",
        Severity::Info => "info",
        Severity::Unspecified => "warn",
    }
}

fn waste_method_str(m: i32) -> &'static str {
    match WasteMethod::try_from(m).unwrap_or(WasteMethod::Heuristic) {
        WasteMethod::CounterfactualDiff => "counterfactual_diff",
        WasteMethod::BaselineExcess => "baseline_excess",
        WasteMethod::RedundantCallSum => "redundant_call_sum",
        WasteMethod::Heuristic => "heuristic",
        WasteMethod::Unspecified => "heuristic",
    }
}

fn waste_confidence_str(c: i32) -> &'static str {
    match WasteConfidence::try_from(c).unwrap_or(WasteConfidence::Low) {
        WasteConfidence::High => "high",
        WasteConfidence::Medium => "medium",
        WasteConfidence::Low => "low",
        WasteConfidence::Unspecified => "low",
    }
}
