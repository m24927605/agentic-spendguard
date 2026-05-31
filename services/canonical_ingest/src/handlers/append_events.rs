//! `CanonicalIngest::AppendEvents` handler.
//!
//! Per Stage 2 §8.2.2 + Trace §10.1 / §10.2 / §13:
//!   * Verify producer signature (TODO: keys come from sidecar handshake).
//!   * Verify schema_bundle exists + hash matches.
//!   * For each event in batch:
//!       1) Dedupe by event_id; on collision return `DEDUPED`.
//!       2) Classify storage_class from event_type (Trace §10.2).
//!       3) For audit.outcome:
//!           - check matching audit.decision exists; if not → quarantine,
//!             return `AWAITING_PRECEDING_DECISION`.
//!       4) For all other events: append atomically to canonical_events +
//!          canonical_events_global_keys.
//!   * On backpressure (quarantine depth > threshold): enforcement-route
//!     events fail_closed; observability-route events still buffer.
//!
//! POC simplifications:
//!   * Producer signature verification is stubbed (TODO: integrate with
//!     sidecar Producer Trust §13).
//!   * Each event runs in its own Postgres transaction. Batch atomicity is
//!     not required by spec; per-event independent results are.

use bigdecimal::BigDecimal;
use chrono::Utc;
use prost_types::Timestamp;
use sha2::{Digest, Sha256};
use spendguard_signing::{Verifier, VerifyFailure};
use sqlx::PgPool;
use std::str::FromStr;
use tracing::{debug, instrument, warn};
use uuid::Uuid;

use crate::{
    config::Config,
    domain::{error::DomainError, event_routing::classify},
    metrics::{IngestMetrics, QuarantineReason, Route as MetricsRoute},
    persistence::{
        append::{self, AppendInput, AppendOutcome},
        query, schema_bundle, signature_quarantine,
    },
    proto::{
        canonical_ingest::v1::{
            append_events_request::Route, event_result::Status as EventStatus, AppendEventsRequest,
            AppendEventsResponse, EventResult, IngestPosition,
        },
        common::v1::{error::Code as ProtoCode, CloudEvent, Error as ProtoError},
    },
    verifier::canonical_bytes,
};

#[instrument(skip(pool, cfg, verifier, metrics, req), fields(
    producer_id = %req.producer_id,
    event_count = req.events.len(),
    route = ?req.route()
))]
pub async fn handle(
    pool: &PgPool,
    cfg: &Config,
    verifier: Option<&dyn Verifier>,
    metrics: &IngestMetrics,
    req: AppendEventsRequest,
) -> Result<AppendEventsResponse, tonic::Status> {
    // Validate batch envelope.
    if req.producer_id.is_empty() {
        return Err(tonic::Status::invalid_argument("producer_id required"));
    }
    if req.events.is_empty() {
        return Err(tonic::Status::invalid_argument("events must not be empty"));
    }
    let bundle_ref = req
        .schema_bundle
        .as_ref()
        .ok_or_else(|| tonic::Status::invalid_argument("schema_bundle required"))?;

    // Verify schema bundle existence + hash.
    let bundle_id = parse_uuid(
        &bundle_ref.schema_bundle_id,
        "schema_bundle.schema_bundle_id",
    )
    .map_err(|e| e.to_status())?;
    let bundle = match schema_bundle::lookup(pool, bundle_id, &bundle_ref.schema_bundle_hash).await
    {
        Ok(Some(b)) => b,
        Ok(None) => return Err(DomainError::SchemaBundleUnknown(bundle_id.to_string()).to_status()),
        Err(e) => return Err(e.to_status()),
    };

    // Phase 5 GA hardening S8: producer signature verification (per
    // Trace §13). Strict mode requires a verifier and rejects /
    // quarantines events that fail. Non-strict mode still records
    // outcomes via metrics so operators can prepare to flip the flag.
    if cfg.strict_signatures && verifier.is_none() {
        return Err(tonic::Status::failed_precondition(
            "strict_signatures=true but no trust store configured; \
             set SPENDGUARD_CANONICAL_INGEST_TRUST_STORE_DIR or flip \
             strict_signatures=false",
        ));
    }

    // Reject ROUTE_UNSPECIFIED — fail-closed default per Stage 2 §8.2.2.
    let route = req.route();
    if route == Route::Unspecified {
        return Err(tonic::Status::invalid_argument(
            "route is unspecified; clients MUST set ENFORCEMENT or OBSERVABILITY",
        ));
    }

    // Backpressure check (best_effort_with_backpressure per Trace §10.1).
    let depth = match query::approximate_backpressure_depth(pool).await {
        Ok(d) => d as u64,
        Err(_) => 0, // soft fail
    };
    let backpressure_active = depth > cfg.backpressure_threshold;

    let mut results = Vec::with_capacity(req.events.len());
    for evt in req.events {
        let res = process_one(
            pool,
            cfg,
            verifier,
            metrics,
            &evt,
            &bundle,
            route,
            backpressure_active,
        )
        .await;
        results.push(res);
    }

    Ok(AppendEventsResponse { results })
}

#[allow(clippy::too_many_arguments)]
async fn process_one(
    pool: &PgPool,
    cfg: &Config,
    verifier: Option<&dyn Verifier>,
    metrics: &IngestMetrics,
    evt: &CloudEvent,
    bundle: &crate::persistence::schema_bundle::CachedBundle,
    route: Route,
    backpressure_active: bool,
) -> EventResult {
    // Per-event validation.
    if let Err(e) = validate_envelope(evt) {
        return error_result(&evt.id, EventStatus::Quarantined, e);
    }

    // Dedup: parse event_id.
    let event_id = match Uuid::parse_str(&evt.id) {
        Ok(id) => id,
        Err(e) => {
            return error_result(
                &evt.id,
                EventStatus::Quarantined,
                DomainError::InvalidRequest(format!("event_id: {}", e)),
            )
        }
    };
    let tenant_id = match Uuid::parse_str(&evt.tenant_id) {
        Ok(id) => id,
        Err(e) => {
            return error_result(
                &evt.id,
                EventStatus::Quarantined,
                DomainError::InvalidRequest(format!("tenant_id: {}", e)),
            )
        }
    };

    let decision_id = if evt.decision_id.is_empty() {
        None
    } else {
        match Uuid::parse_str(&evt.decision_id) {
            Ok(id) => Some(id),
            Err(e) => {
                return error_result(
                    &evt.id,
                    EventStatus::Quarantined,
                    DomainError::InvalidRequest(format!("decision_id: {}", e)),
                )
            }
        }
    };
    // Parse run_id strictly — silently dropping malformed run_ids would break
    // QueryAuditChain by run_id later.
    let run_id = if evt.run_id.is_empty() {
        None
    } else {
        match Uuid::parse_str(&evt.run_id) {
            Ok(id) => Some(id),
            Err(e) => {
                return error_result(
                    &evt.id,
                    EventStatus::Quarantined,
                    DomainError::InvalidRequest(format!("run_id: {}", e)),
                )
            }
        }
    };

    // Per-event schema_bundle_id MUST match the batch-level bundle (Trace §12).
    if !evt.schema_bundle_id.is_empty()
        && evt.schema_bundle_id != bundle.schema_bundle_id.to_string()
    {
        return error_result(
            &evt.id,
            EventStatus::Quarantined,
            DomainError::InvalidRequest(format!(
                "event.schema_bundle_id ({}) != batch.schema_bundle_id ({})",
                evt.schema_bundle_id, bundle.schema_bundle_id
            )),
        );
    }

    // producer_sequence: reject overflow.
    let producer_sequence: i64 = match i64::try_from(evt.producer_sequence) {
        Ok(v) => v,
        Err(_) => {
            return error_result(
                &evt.id,
                EventStatus::Quarantined,
                DomainError::InvalidRequest(format!(
                    "producer_sequence {} overflows i64",
                    evt.producer_sequence
                )),
            )
        }
    };

    // Phase 5 GA hardening S8: producer signature verification.
    // Returns either Continue (event passes; admit) or a typed
    // EventResult that the caller surfaces directly (reject /
    // quarantine).
    if let Some(early) = verify_or_handle(pool, cfg, verifier, metrics, evt, route).await {
        return early;
    }

    // Backpressure on enforcement route.
    if backpressure_active && route == Route::Enforcement {
        return EventResult {
            event_id: evt.id.clone(),
            status: EventStatus::Backpressure as i32,
            ingest_position: None,
            error: Some(ProtoError {
                code: ProtoCode::Unspecified as i32,
                message: "ingest backpressure on enforcement route".to_string(),
                details: Default::default(),
            }),
        };
    }

    // Storage class.
    let storage_class = classify(&evt.r#type);

    // event_time: reject invalid nanos / out-of-range timestamps; do NOT
    // silently fall back to Utc::now() (mutates the canonical event).
    let event_time = match &evt.time {
        Some(Timestamp { seconds, nanos }) => {
            if *nanos < 0 || *nanos >= 1_000_000_000 {
                return error_result(
                    &evt.id,
                    EventStatus::Quarantined,
                    DomainError::InvalidRequest(format!(
                        "event_time.nanos {} out of [0, 1_000_000_000)",
                        nanos
                    )),
                );
            }
            match chrono::DateTime::<Utc>::from_timestamp(*seconds, *nanos as u32) {
                Some(t) => t,
                None => {
                    return error_result(
                        &evt.id,
                        EventStatus::Quarantined,
                        DomainError::InvalidRequest(format!(
                            "event_time seconds {} out of range",
                            seconds
                        )),
                    )
                }
            }
        }
        None => {
            return error_result(
                &evt.id,
                EventStatus::Quarantined,
                DomainError::InvalidRequest("event_time required".into()),
            )
        }
    };

    // Build append input.
    let payload_json =
        serde_json::to_value(cloudevent_to_json(evt)).unwrap_or(serde_json::Value::Null);
    let event_hash = Sha256::digest(canonical_bytes(evt)).to_vec();

    // Cost Advisor P1.5 (issue #51, spec §5.1.2): classify audit
    // .outcome events into one of the 9 failure_class enum values.
    // For non-outcome events the classifier returns None and we
    // persist NULL in canonical_events.failure_class. The classifier
    // is fault-tolerant: malformed data_b64 maps to FailureClass
    // ::Unknown rather than aborting the INSERT.
    let decoded_data = crate::classify::decode_payload_data(&payload_json);
    let failure_class = crate::classify::classify_audit_outcome(&evt.r#type, decoded_data.as_ref())
        .map(|c| c.as_db_str());
    let aggregator = aggregator_mirrors_from_event(evt, decoded_data.as_ref(), run_id);

    let input = AppendInput {
        event_id,
        tenant_id,
        decision_id,
        run_id,
        event_type: &evt.r#type,
        storage_class,
        producer_id: &evt.producer_id,
        producer_sequence,
        producer_signature: &evt.producer_signature,
        signing_key_id: &evt.signing_key_id,
        event_hash: &event_hash,
        schema_bundle_id: bundle.schema_bundle_id,
        schema_bundle_hash: &bundle.schema_bundle_hash,
        specversion: &evt.specversion,
        source: &evt.source,
        event_time,
        datacontenttype: &evt.datacontenttype,
        payload_json,
        payload_blob_ref: None,
        region_id: &cfg.region,
        ingest_shard_id: &cfg.ingest_shard_id,
        failure_class,
        model: aggregator.model,
        agent_id: aggregator.agent_id,
        run_id_mirror: aggregator.run_id_mirror,
        prompt_class: aggregator.prompt_class,
        prompt_class_fingerprint: aggregator.prompt_class_fingerprint,
        prediction: prediction_columns_from_event(evt),
    };

    // Per-decision sequence enforcement: audit.outcome with no preceding decision.
    if evt.r#type == "spendguard.audit.outcome" {
        let dec_id = match decision_id {
            Some(d) => d,
            None => {
                return error_result(
                    &evt.id,
                    EventStatus::Quarantined,
                    DomainError::InvalidRequest("audit.outcome missing decision_id".into()),
                )
            }
        };

        match append::has_preceding_decision(pool, tenant_id, dec_id).await {
            Ok(true) => { /* fall through to normal append */ }
            Ok(false) => {
                let orphan_after =
                    Utc::now() + chrono::Duration::seconds(cfg.orphan_after_seconds as i64);
                if let Err(e) =
                    append::quarantine_audit_outcome(pool, input.clone(), orphan_after).await
                {
                    return error_result(&evt.id, EventStatus::Quarantined, e);
                }
                return EventResult {
                    event_id: evt.id.clone(),
                    status: EventStatus::AwaitingPrecedingDecision as i32,
                    ingest_position: None,
                    error: None,
                };
            }
            Err(e) => return error_result(&evt.id, EventStatus::Quarantined, e),
        }
    }

    // Normal append.
    match append::append_event(pool, input).await {
        Ok(AppendOutcome::Appended { ingest_log_offset }) => {
            debug!(
                event_id = %evt.id,
                offset = ingest_log_offset,
                storage_class = ?storage_class,
                "appended"
            );

            // After committing an audit.decision row, release any
            // quarantined audit.outcome rows for the same decision_id.
            // Release function uses original quarantine metadata (NOT this
            // decision's bundle / datacontenttype).
            if evt.r#type == "spendguard.audit.decision" {
                if let Some(dec) = decision_id {
                    if let Err(e) = append::release_quarantined_outcomes(
                        pool,
                        tenant_id,
                        dec,
                        &cfg.region,
                        &cfg.ingest_shard_id,
                    )
                    .await
                    {
                        warn!(
                            decision_id = %dec,
                            err = %e,
                            "release_quarantined_outcomes failed; reaper will retry"
                        );
                    }
                }
            }

            EventResult {
                event_id: evt.id.clone(),
                status: EventStatus::Appended as i32,
                ingest_position: Some(IngestPosition {
                    region_id: cfg.region.clone(),
                    ingest_shard_id: cfg.ingest_shard_id.clone(),
                    ingest_log_offset: ingest_log_offset as u64,
                }),
                error: None,
            }
        }
        Ok(AppendOutcome::Deduped) => EventResult {
            event_id: evt.id.clone(),
            status: EventStatus::Deduped as i32,
            ingest_position: None,
            error: None,
        },
        Err(DomainError::Duplicate(msg)) => EventResult {
            event_id: evt.id.clone(),
            status: EventStatus::Duplicate as i32,
            ingest_position: None,
            error: Some(ProtoError {
                code: ProtoCode::DuplicateDecisionEvent as i32,
                message: msg,
                details: Default::default(),
            }),
        },
        Err(DomainError::AwaitingPrecedingDecision(msg)) => {
            // Defense-in-depth: trigger fired despite our check above; redirect
            // to quarantine. This can happen if a concurrent caller deleted
            // the decision (which our triggers forbid, but kept here for
            // safety).
            warn!(event_id = %evt.id, reason = %msg, "trigger-side quarantine fallback");
            let orphan_after =
                Utc::now() + chrono::Duration::seconds(cfg.orphan_after_seconds as i64);
            // Rebuild input — original was moved.
            let payload_json =
                serde_json::to_value(cloudevent_to_json(evt)).unwrap_or(serde_json::Value::Null);
            // Re-classify for the quarantine path's audit row.
            let decoded_data = crate::classify::decode_payload_data(&payload_json);
            let failure_class =
                crate::classify::classify_audit_outcome(&evt.r#type, decoded_data.as_ref())
                    .map(|c| c.as_db_str());
            let aggregator = aggregator_mirrors_from_event(evt, decoded_data.as_ref(), run_id);
            let input = AppendInput {
                event_id,
                tenant_id,
                decision_id,
                run_id,
                event_type: &evt.r#type,
                storage_class,
                producer_id: &evt.producer_id,
                producer_sequence,
                producer_signature: &evt.producer_signature,
                signing_key_id: &evt.signing_key_id,
                event_hash: &event_hash,
                schema_bundle_id: bundle.schema_bundle_id,
                schema_bundle_hash: &bundle.schema_bundle_hash,
                specversion: &evt.specversion,
                source: &evt.source,
                event_time,
                datacontenttype: &evt.datacontenttype,
                payload_json,
                payload_blob_ref: None,
                region_id: &cfg.region,
                ingest_shard_id: &cfg.ingest_shard_id,
                failure_class,
                model: aggregator.model,
                agent_id: aggregator.agent_id,
                run_id_mirror: aggregator.run_id_mirror,
                prompt_class: aggregator.prompt_class,
                prompt_class_fingerprint: aggregator.prompt_class_fingerprint,
                prediction: prediction_columns_from_event(evt),
            };
            if let Err(e) = append::quarantine_audit_outcome(pool, input, orphan_after).await {
                return error_result(&evt.id, EventStatus::Quarantined, e);
            }
            EventResult {
                event_id: evt.id.clone(),
                status: EventStatus::AwaitingPrecedingDecision as i32,
                ingest_position: None,
                error: None,
            }
        }
        Err(e) => error_result(&evt.id, EventStatus::Quarantined, e),
    }
}

fn validate_envelope(evt: &CloudEvent) -> Result<(), DomainError> {
    if evt.specversion != "1.0" {
        return Err(DomainError::InvalidRequest(format!(
            "specversion must be '1.0', got '{}'",
            evt.specversion
        )));
    }
    if evt.r#type.is_empty() {
        return Err(DomainError::InvalidRequest("type required".into()));
    }
    if evt.id.is_empty() {
        return Err(DomainError::InvalidRequest("id required".into()));
    }
    if evt.tenant_id.is_empty() {
        return Err(DomainError::InvalidRequest("tenant_id required".into()));
    }
    if evt.producer_id.is_empty() {
        return Err(DomainError::InvalidRequest("producer_id required".into()));
    }
    // decision_id is REQUIRED for audit chain events. Without it, the
    // partial UNIQUE indexes on (tenant_id, decision_id) cannot enforce
    // per-decision uniqueness because Postgres treats multiple NULLs as
    // distinct.
    if (evt.r#type == "spendguard.audit.decision" || evt.r#type == "spendguard.audit.outcome")
        && evt.decision_id.is_empty()
    {
        return Err(DomainError::InvalidRequest(format!(
            "{} requires non-empty decision_id",
            evt.r#type
        )));
    }
    Ok(())
}

fn cloudevent_to_json(evt: &CloudEvent) -> serde_json::Value {
    use base64::Engine as _;
    serde_json::json!({
        "specversion":     evt.specversion,
        "type":            evt.r#type,
        "source":          evt.source,
        "id":              evt.id,
        "time_seconds":    evt.time.as_ref().map(|t| t.seconds).unwrap_or_default(),
        "time_nanos":      evt.time.as_ref().map(|t| t.nanos).unwrap_or_default(),
        "datacontenttype": evt.datacontenttype,
        "data_b64":        base64::engine::general_purpose::STANDARD.encode(&evt.data),
        "tenantid":        evt.tenant_id,
        "runid":           evt.run_id,
        "decisionid":      evt.decision_id,
        "schema_bundle_id": evt.schema_bundle_id,
        "producer_id":     evt.producer_id,
        "producer_sequence": evt.producer_sequence,
        "signing_key_id":  evt.signing_key_id,
        "predicted_a_tokens": evt.predicted_a_tokens,
        "predicted_b_tokens": evt.predicted_b_tokens,
        "predicted_c_tokens": evt.predicted_c_tokens,
        "reserved_strategy": evt.reserved_strategy,
        "prediction_strategy_used": evt.prediction_strategy_used,
        "prediction_policy_used": evt.prediction_policy_used,
        "tokenizer_tier": evt.tokenizer_tier,
        "tokenizer_version_id": evt.tokenizer_version_id,
        "prediction_confidence": evt.prediction_confidence,
        "prediction_sample_size": evt.prediction_sample_size,
        "cold_start_layer_used": evt.cold_start_layer_used,
        "run_projection_at_decision_atomic": evt.run_projection_at_decision_atomic,
        "run_predicted_remaining_steps": evt.run_predicted_remaining_steps,
        "run_steps_completed_so_far": evt.run_steps_completed_so_far,
        "actual_input_tokens": evt.actual_input_tokens,
        "actual_output_tokens": evt.actual_output_tokens,
        "delta_b_ratio": evt.delta_b_ratio,
        "delta_c_ratio": evt.delta_c_ratio,
    })
}

#[derive(Debug, Clone, Copy, Default)]
struct AggregatorMirrors<'a> {
    model: Option<&'a str>,
    agent_id: Option<&'a str>,
    run_id_mirror: Option<Uuid>,
    prompt_class: Option<&'a str>,
    prompt_class_fingerprint: Option<&'a str>,
}

fn aggregator_mirrors_from_event<'a>(
    evt: &CloudEvent,
    decoded_data: Option<&'a serde_json::Value>,
    run_id: Option<Uuid>,
) -> AggregatorMirrors<'a> {
    if evt.r#type != "spendguard.audit.decision" && evt.r#type != "spendguard.audit.outcome" {
        return AggregatorMirrors::default();
    }
    let Some(data) = decoded_data.and_then(|v| v.as_object()) else {
        return AggregatorMirrors::default();
    };
    let model = json_nonempty_str(data.get("model"))
        .or_else(|| json_nonempty_str(data.get("model_family")));
    AggregatorMirrors {
        model,
        agent_id: json_nonempty_str(data.get("agent_id")),
        run_id_mirror: run_id,
        prompt_class: json_nonempty_str(data.get("prompt_class")),
        prompt_class_fingerprint: json_nonempty_str(data.get("prompt_class_fingerprint")),
    }
}

fn json_nonempty_str(value: Option<&serde_json::Value>) -> Option<&str> {
    value
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn prediction_columns_from_event(evt: &CloudEvent) -> append::PredictionColumns<'_> {
    let is_decision = evt.r#type == "spendguard.audit.decision";
    let is_outcome = evt.r#type == "spendguard.audit.outcome";
    let projector_unreachable = evt.run_projection_at_decision_atomic == 0
        && evt.run_predicted_remaining_steps == -1
        && evt.run_steps_completed_so_far == 0;
    let projector_absent_default = evt.run_projection_at_decision_atomic == 0
        && evt.run_predicted_remaining_steps == 0
        && evt.run_steps_completed_so_far == 0;
    let no_projector = !is_decision || projector_unreachable || projector_absent_default;

    append::PredictionColumns {
        predicted_a_tokens: is_decision
            .then(|| nonzero_i64(evt.predicted_a_tokens))
            .flatten(),
        predicted_b_tokens: is_decision
            .then(|| nonzero_i64(evt.predicted_b_tokens))
            .flatten(),
        predicted_c_tokens: is_decision
            .then(|| nonzero_i64(evt.predicted_c_tokens))
            .flatten(),
        reserved_strategy: is_decision
            .then(|| nonempty(&evt.reserved_strategy))
            .flatten(),
        prediction_strategy_used: is_decision
            .then(|| nonempty(&evt.prediction_strategy_used))
            .flatten(),
        prediction_policy_used: is_decision
            .then(|| nonempty(&evt.prediction_policy_used))
            .flatten(),
        tokenizer_tier: is_decision.then(|| nonempty(&evt.tokenizer_tier)).flatten(),
        tokenizer_version_id: is_decision
            .then(|| uuid_nonempty(&evt.tokenizer_version_id))
            .flatten(),
        prediction_confidence: is_decision
            .then(|| nonzero_f32_decimal(evt.prediction_confidence))
            .flatten(),
        prediction_sample_size: is_decision
            .then(|| nonzero_i64(evt.prediction_sample_size))
            .flatten(),
        cold_start_layer_used: is_decision
            .then(|| nonempty(&evt.cold_start_layer_used))
            .flatten(),
        run_projection_at_decision_atomic: if no_projector {
            None
        } else {
            nonzero_i64_decimal(evt.run_projection_at_decision_atomic)
        },
        run_predicted_remaining_steps: if no_projector || evt.run_predicted_remaining_steps == -1 {
            None
        } else {
            Some(evt.run_predicted_remaining_steps)
        },
        run_steps_completed_so_far: if no_projector {
            None
        } else {
            Some(evt.run_steps_completed_so_far)
        },
        actual_input_tokens: outcome_actual_token(
            evt,
            "actual_input_tokens",
            evt.actual_input_tokens,
        ),
        actual_output_tokens: outcome_actual_token(
            evt,
            "actual_output_tokens",
            evt.actual_output_tokens,
        ),
        delta_b_ratio: is_outcome.then(|| nonzero_f32(evt.delta_b_ratio)).flatten(),
        delta_c_ratio: is_outcome.then(|| nonzero_f32(evt.delta_c_ratio)).flatten(),
    }
}

fn nonempty(value: &str) -> Option<&str> {
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn uuid_nonempty(value: &str) -> Option<Uuid> {
    if value.is_empty() {
        None
    } else {
        Uuid::parse_str(value).ok()
    }
}

fn nonzero_i64(value: i64) -> Option<i64> {
    if value == 0 {
        None
    } else {
        Some(value)
    }
}

fn nonzero_i64_decimal(value: i64) -> Option<BigDecimal> {
    nonzero_i64(value).map(BigDecimal::from)
}

fn outcome_actual_token(evt: &CloudEvent, key: &str, extension_value: i64) -> Option<i64> {
    if evt.r#type != "spendguard.audit.outcome" {
        return None;
    }
    if let Ok(serde_json::Value::Object(data)) =
        serde_json::from_slice::<serde_json::Value>(&evt.data)
    {
        if let Some(value) = data.get(key).and_then(json_i64) {
            return (value >= 0).then_some(value);
        }
    }
    (extension_value > 0).then_some(extension_value)
}

fn json_i64(value: &serde_json::Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|s| s.parse::<i64>().ok()))
}

fn nonzero_f32(value: f32) -> Option<f32> {
    if value == 0.0 {
        None
    } else {
        Some(value)
    }
}

fn nonzero_f32_decimal(value: f32) -> Option<BigDecimal> {
    nonzero_f32(value).and_then(|v| BigDecimal::from_str(&format!("{v:.3}")).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prediction_columns_preserve_first_run_step_zero() {
        let evt = CloudEvent {
            r#type: "spendguard.audit.decision".to_string(),
            run_projection_at_decision_atomic: 42,
            run_predicted_remaining_steps: 3,
            run_steps_completed_so_far: 0,
            ..Default::default()
        };

        let cols = prediction_columns_from_event(&evt);

        assert_eq!(
            cols.run_steps_completed_so_far,
            Some(0),
            "step zero is a valid first decision when projector data is present"
        );
        assert_eq!(
            cols.run_projection_at_decision_atomic
                .map(|v| v.to_string()),
            Some("42".to_string())
        );
        assert_eq!(cols.run_predicted_remaining_steps, Some(3));
    }

    #[test]
    fn prediction_columns_keep_unreachable_projector_sentinel_null() {
        let evt = CloudEvent {
            r#type: "spendguard.audit.decision".to_string(),
            run_projection_at_decision_atomic: 0,
            run_predicted_remaining_steps: -1,
            run_steps_completed_so_far: 0,
            ..Default::default()
        };

        let cols = prediction_columns_from_event(&evt);

        assert_eq!(cols.run_projection_at_decision_atomic, None);
        assert_eq!(cols.run_predicted_remaining_steps, None);
        assert_eq!(cols.run_steps_completed_so_far, None);
    }

    #[test]
    fn prediction_columns_keep_default_decision_projection_null() {
        let evt = CloudEvent {
            r#type: "spendguard.audit.decision".to_string(),
            run_projection_at_decision_atomic: 0,
            run_predicted_remaining_steps: 0,
            run_steps_completed_so_far: 0,
            ..Default::default()
        };

        let cols = prediction_columns_from_event(&evt);

        assert_eq!(cols.run_projection_at_decision_atomic, None);
        assert_eq!(cols.run_predicted_remaining_steps, None);
        assert_eq!(cols.run_steps_completed_so_far, None);
    }

    #[test]
    fn prediction_columns_do_not_invent_run_projection_for_outcomes() {
        let evt = CloudEvent {
            r#type: "spendguard.audit.outcome".to_string(),
            run_projection_at_decision_atomic: 0,
            run_predicted_remaining_steps: 0,
            run_steps_completed_so_far: 0,
            actual_input_tokens: 10,
            actual_output_tokens: 20,
            ..Default::default()
        };

        let cols = prediction_columns_from_event(&evt);

        assert_eq!(cols.run_projection_at_decision_atomic, None);
        assert_eq!(cols.run_predicted_remaining_steps, None);
        assert_eq!(cols.run_steps_completed_so_far, None);
        assert_eq!(cols.actual_input_tokens, Some(10));
        assert_eq!(cols.actual_output_tokens, Some(20));
    }

    #[test]
    fn prediction_columns_preserve_zero_actual_tokens_for_outcomes() {
        let evt = CloudEvent {
            r#type: "spendguard.audit.outcome".to_string(),
            data: serde_json::json!({
                "actual_input_tokens": 0,
                "actual_output_tokens": 0
            })
            .to_string()
            .into_bytes()
            .into(),
            actual_input_tokens: 0,
            actual_output_tokens: 0,
            ..Default::default()
        };

        let cols = prediction_columns_from_event(&evt);

        assert_eq!(cols.actual_input_tokens, Some(0));
        assert_eq!(cols.actual_output_tokens, Some(0));
    }

    #[test]
    fn prediction_columns_keep_missing_actual_tokens_null_for_outcomes() {
        let evt = CloudEvent {
            r#type: "spendguard.audit.outcome".to_string(),
            data: serde_json::json!({
                "estimated_amount_atomic": "42"
            })
            .to_string()
            .into_bytes()
            .into(),
            actual_input_tokens: 0,
            actual_output_tokens: 0,
            ..Default::default()
        };

        let cols = prediction_columns_from_event(&evt);

        assert_eq!(cols.actual_input_tokens, None);
        assert_eq!(cols.actual_output_tokens, None);
    }

    #[test]
    fn decision_payload_populates_aggregator_mirror_columns() {
        let run_id = Uuid::parse_str("00000000-0000-7000-8000-000000000001").unwrap();
        let evt = CloudEvent {
            r#type: "spendguard.audit.decision".to_string(),
            ..Default::default()
        };
        let data = serde_json::json!({
            "model": "gpt-4o-mini",
            "agent_id": "agent-alpha",
            "prompt_class": "support_triage",
            "prompt_class_fingerprint": "pcfp_123"
        });

        let mirrors = aggregator_mirrors_from_event(&evt, Some(&data), Some(run_id));

        assert_eq!(mirrors.model, Some("gpt-4o-mini"));
        assert_eq!(mirrors.agent_id, Some("agent-alpha"));
        assert_eq!(mirrors.run_id_mirror, Some(run_id));
        assert_eq!(mirrors.prompt_class, Some("support_triage"));
        assert_eq!(mirrors.prompt_class_fingerprint, Some("pcfp_123"));
    }

    #[test]
    fn decision_payload_model_family_fallback_populates_model_mirror() {
        let evt = CloudEvent {
            r#type: "spendguard.audit.decision".to_string(),
            ..Default::default()
        };
        let data = serde_json::json!({
            "model_family": "claude-3-5-sonnet",
            "agent_id": "agent-beta",
            "prompt_class": "code_gen",
            "prompt_class_fingerprint": "pcfp_456"
        });

        let mirrors = aggregator_mirrors_from_event(&evt, Some(&data), None);

        assert_eq!(mirrors.model, Some("claude-3-5-sonnet"));
        assert_eq!(mirrors.agent_id, Some("agent-beta"));
        assert_eq!(mirrors.prompt_class, Some("code_gen"));
        assert_eq!(mirrors.prompt_class_fingerprint, Some("pcfp_456"));
    }

    #[test]
    fn outcome_payload_populates_aggregator_mirror_columns() {
        let run_id = Uuid::parse_str("00000000-0000-7000-8000-000000000002").unwrap();
        let evt = CloudEvent {
            r#type: "spendguard.audit.outcome".to_string(),
            ..Default::default()
        };
        let data = serde_json::json!({
            "model": "gpt-4o-mini",
            "agent_id": "agent-alpha",
            "prompt_class": "support_triage",
            "prompt_class_fingerprint": "pcfp_outcome"
        });

        let mirrors = aggregator_mirrors_from_event(&evt, Some(&data), Some(run_id));

        assert_eq!(mirrors.model, Some("gpt-4o-mini"));
        assert_eq!(mirrors.agent_id, Some("agent-alpha"));
        assert_eq!(mirrors.run_id_mirror, Some(run_id));
        assert_eq!(mirrors.prompt_class, Some("support_triage"));
        assert_eq!(mirrors.prompt_class_fingerprint, Some("pcfp_outcome"));
    }
}

fn parse_uuid(s: &str, field: &str) -> Result<Uuid, DomainError> {
    Uuid::parse_str(s).map_err(|e| DomainError::InvalidRequest(format!("{}: {}", field, e)))
}

fn error_result(event_id: &str, status: EventStatus, err: DomainError) -> EventResult {
    EventResult {
        event_id: event_id.to_string(),
        status: status as i32,
        ingest_position: None,
        error: Some(ProtoError {
            code: ProtoCode::Unspecified as i32,
            message: err.to_string(),
            details: Default::default(),
        }),
    }
}

fn route_to_metric(route: Route) -> MetricsRoute {
    match route {
        Route::Enforcement => MetricsRoute::Enforcement,
        _ => MetricsRoute::Observability,
    }
}

/// Triage signature verification result. Returns:
///   * `None` — event passes verification (or is admitted in non-strict
///     mode); caller continues to append.
///   * `Some(EventResult)` — caller MUST surface this result instead of
///     appending. Includes the quarantine-write side effect.
async fn verify_or_handle(
    pool: &PgPool,
    cfg: &Config,
    verifier: Option<&dyn Verifier>,
    metrics: &IngestMetrics,
    evt: &CloudEvent,
    route: Route,
) -> Option<EventResult> {
    // No verifier configured + non-strict mode → fully bypass (POC
    // path). We still increment "accepted" so the metric is honest.
    let v = match verifier {
        Some(v) => v,
        None => {
            metrics.inc_accepted(route_to_metric(route));
            return None;
        }
    };

    match crate::verifier::verify_cloudevent(v, evt) {
        Ok(()) => {
            metrics.inc_accepted(route_to_metric(route));
            None
        }
        Err(VerifyFailure::PreS6) => {
            // Pre-S6 backfill row. Strict mode → quarantine. Non-strict
            // → admit but bump a counter so operators can monitor the
            // tail of the pre-S6 backlog draining.
            if cfg.strict_signatures {
                metrics.inc_quarantined(QuarantineReason::PreS6);
                Some(write_quarantine(pool, evt, "pre_s6", metrics).await)
            } else {
                metrics.inc_pre_s6_admitted();
                None
            }
        }
        Err(VerifyFailure::Disabled) => {
            // Demo-profile row signed with the disabled signer. Strict
            // mode quarantines (production must never see this). Non-
            // strict admits but counts.
            if cfg.strict_signatures {
                metrics.inc_quarantined(QuarantineReason::Disabled);
                Some(write_quarantine(pool, evt, "disabled", metrics).await)
            } else {
                metrics.inc_disabled_admitted();
                None
            }
        }
        Err(VerifyFailure::UnknownKey) => {
            // Codex P2#3: non-strict mode is "audit-only" per the
            // verifier docstring. Quarantining unknown-key in
            // non-strict contradicts that contract; mirror the
            // PreS6/Disabled pattern (admit + counter, no quarantine).
            // Strict mode (production-mandated by Helm gate) still
            // quarantines.
            if cfg.strict_signatures {
                metrics.inc_quarantined(QuarantineReason::UnknownKey);
                Some(write_quarantine(pool, evt, "unknown_key", metrics).await)
            } else {
                metrics.inc_unknown_key_admitted();
                None
            }
        }
        Err(VerifyFailure::InvalidSignature) => {
            // Codex P2#3: see UnknownKey arm. The
            // rejected_invalid_signature counter still fires so
            // operators can detect tamper attempts even in non-strict
            // mode — only the quarantine + rejection is gated.
            metrics.inc_rejected_invalid_sig(route_to_metric(route));
            if cfg.strict_signatures {
                metrics.inc_quarantined(QuarantineReason::InvalidSignature);
                Some(write_quarantine(pool, evt, "invalid_signature", metrics).await)
            } else {
                metrics.inc_invalid_signature_admitted();
                None
            }
        }
        // S7: per-key validity-window failures. Always quarantine
        // regardless of strict mode — these are unambiguous policy
        // violations (key was past its window or operator-revoked).
        Err(VerifyFailure::KeyExpired) => {
            metrics.inc_quarantined(QuarantineReason::KeyExpired);
            Some(write_quarantine(pool, evt, "key_expired", metrics).await)
        }
        Err(VerifyFailure::KeyNotYetValid) => {
            metrics.inc_quarantined(QuarantineReason::KeyNotYetValid);
            Some(write_quarantine(pool, evt, "key_not_yet_valid", metrics).await)
        }
        Err(VerifyFailure::KeyRevoked) => {
            metrics.inc_quarantined(QuarantineReason::KeyRevoked);
            Some(write_quarantine(pool, evt, "key_revoked", metrics).await)
        }
    }
}

/// Persist the offending event in `audit_signature_quarantine` and
/// return the EventResult the handler surfaces. On insert failure we
/// fail-open with a Quarantined status — the audit invariant is
/// preserved (the row was rejected from the canonical log) even if the
/// quarantine write fails (operator sees the gRPC status + the metric
/// counter; row is dropped, which is acceptable at the quarantine
/// boundary).
async fn write_quarantine(
    pool: &PgPool,
    evt: &CloudEvent,
    reason: &str,
    metrics: &IngestMetrics,
) -> EventResult {
    let canonical = canonical_bytes(evt);
    if canonical.len() > 1_048_576 {
        metrics.inc_quarantined(QuarantineReason::Oversized);
        warn!(
            event_id = %evt.id,
            len = canonical.len(),
            "quarantine: canonical bytes too large; dropping"
        );
        return error_result(
            &evt.id,
            EventStatus::Quarantined,
            DomainError::InvalidRequest(
                "oversized canonical bytes; dropped at quarantine boundary".into(),
            ),
        );
    }
    if let Err(e) = signature_quarantine::insert(pool, evt, &canonical, reason).await {
        warn!(event_id = %evt.id, err = %e, "audit_signature_quarantine insert failed");
    }
    error_result(
        &evt.id,
        EventStatus::Quarantined,
        DomainError::InvalidRequest(format!("signature verification failed ({reason})")),
    )
}
