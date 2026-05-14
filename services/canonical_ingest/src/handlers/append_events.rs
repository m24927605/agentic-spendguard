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

use chrono::Utc;
use prost_types::Timestamp;
use sqlx::PgPool;
use spendguard_signing::{VerifyFailure, Verifier};
use tracing::{debug, instrument, warn};
use uuid::Uuid;

use crate::{
    config::Config,
    domain::{
        error::DomainError,
        event_routing::classify,
    },
    metrics::{IngestMetrics, QuarantineReason, Route as MetricsRoute},
    persistence::{
        append::{self, AppendInput, AppendOutcome},
        query, schema_bundle, signature_quarantine,
    },
    proto::{
        canonical_ingest::v1::{
            event_result::Status as EventStatus, AppendEventsRequest, AppendEventsResponse,
            EventResult, IngestPosition, append_events_request::Route,
        },
        common::v1::{CloudEvent, Error as ProtoError, error::Code as ProtoCode},
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
    let bundle_id = parse_uuid(&bundle_ref.schema_bundle_id, "schema_bundle.schema_bundle_id")
        .map_err(|e| e.to_status())?;
    let bundle = match schema_bundle::lookup(pool, bundle_id, &bundle_ref.schema_bundle_hash).await
    {
        Ok(Some(b)) => b,
        Ok(None) => {
            return Err(DomainError::SchemaBundleUnknown(bundle_id.to_string()).to_status())
        }
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
    if !evt.schema_bundle_id.is_empty() && evt.schema_bundle_id
        != bundle.schema_bundle_id.to_string()
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

    // Cost Advisor P1.5 (issue #51, spec §5.1.2): classify audit
    // .outcome events into one of the 9 failure_class enum values.
    // For non-outcome events the classifier returns None and we
    // persist NULL in canonical_events.failure_class. The classifier
    // is fault-tolerant: malformed data_b64 maps to FailureClass
    // ::Unknown rather than aborting the INSERT.
    let decoded_data = crate::classify::decode_payload_data(&payload_json);
    let failure_class = crate::classify::classify_audit_outcome(
        &evt.r#type,
        decoded_data.as_ref(),
    )
    .map(|c| c.as_db_str());

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
                let orphan_after = Utc::now() + chrono::Duration::seconds(cfg.orphan_after_seconds as i64);
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
            let payload_json = serde_json::to_value(cloudevent_to_json(evt))
                .unwrap_or(serde_json::Value::Null);
            // Re-classify for the quarantine path's audit row.
            let decoded_data = crate::classify::decode_payload_data(&payload_json);
            let failure_class = crate::classify::classify_audit_outcome(
                &evt.r#type,
                decoded_data.as_ref(),
            )
            .map(|c| c.as_db_str());
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
    if (evt.r#type == "spendguard.audit.decision"
        || evt.r#type == "spendguard.audit.outcome")
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
    })
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
            DomainError::InvalidRequest("oversized canonical bytes; dropped at quarantine boundary".into()),
        );
    }
    if let Err(e) = signature_quarantine::insert(pool, evt, &canonical, reason).await {
        warn!(event_id = %evt.id, err = %e, "audit_signature_quarantine insert failed");
    }
    error_result(
        &evt.id,
        EventStatus::Quarantined,
        DomainError::InvalidRequest(format!(
            "signature verification failed ({reason})"
        )),
    )
}
