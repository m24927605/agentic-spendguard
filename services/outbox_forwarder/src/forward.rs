//! Build AppendEventsRequest, call canonical_ingest, update audit_outbox.
//!
//! Happy path: APPENDED/DEDUPED → mark forwarded.
//! All other statuses + decode errors + batch failures: keep pending +
//! increment forward_attempts + log error.

use chrono::{DateTime, NaiveDate, Utc};
use sqlx::QueryBuilder;
use std::collections::{HashMap, HashSet};
use tracing::{info, warn};
use uuid::Uuid;

use crate::{
    decode::strict_decode,
    poll::OutboxRow,
    proto::{
        canonical_ingest::v1::{
            append_events_request::Route, event_result::Status as EventStatus,
            AppendEventsRequest, EventResult,
        },
        common::v1::SchemaBundleRef,
    },
    state::AppState,
};

#[derive(Debug, Clone)]
struct UpdateRow {
    recorded_month: NaiveDate,
    audit_outbox_id: Uuid,
    pending_forward: bool,
    forwarded_at: Option<DateTime<Utc>>,
    last_forward_error: Option<String>,
}

pub async fn forward_batch(state: &mut AppState) -> anyhow::Result<usize> {
    let rows = crate::poll::fetch_pending(&state.pg, state.config.batch_size).await?;
    if rows.is_empty() {
        return Ok(0);
    }
    let total = rows.len();
    info!(count = total, "fetched pending audit_outbox rows");

    let mut decoded: Vec<(OutboxRow, crate::proto::common::v1::CloudEvent)> = Vec::new();
    let mut updates: Vec<UpdateRow> = Vec::new();

    for row in rows {
        match strict_decode(&row) {
            Ok(ce) => decoded.push((row, ce)),
            Err(e) => {
                warn!(audit_outbox_id = %row.audit_outbox_id, error = ?e, "strict_decode failed");
                updates.push(UpdateRow {
                    recorded_month: row.recorded_month,
                    audit_outbox_id: row.audit_outbox_id,
                    pending_forward: true,
                    forwarded_at: None,
                    last_forward_error: Some(format!("DECODE_FAILED:{:?}", e)),
                });
            }
        }
    }

    if !decoded.is_empty() {
        let event_id_to_pk: HashMap<String, (NaiveDate, Uuid)> = decoded
            .iter()
            .map(|(r, e)| (e.id.clone(), (r.recorded_month, r.audit_outbox_id)))
            .collect();

        let max_seq = decoded
            .iter()
            .map(|(_, e)| e.producer_sequence)
            .max()
            .unwrap_or(0);

        let req = AppendEventsRequest {
            producer_id: format!("outbox-forwarder:{}", state.config.workload_instance_id),
            batch_max_producer_sequence: max_seq,
            batch_signature: vec![].into(),
            signing_key_id: "outbox-forwarder:demo:v1".into(),
            schema_bundle: Some(SchemaBundleRef {
                schema_bundle_id: state.config.schema_bundle_id.clone(),
                schema_bundle_hash: hex::decode(&state.config.schema_bundle_hash_hex)?.into(),
                canonical_schema_version: "spendguard.v1alpha1".into(),
            }),
            events: decoded.iter().map(|(_, e)| e.clone()).collect(),
            route: Route::Enforcement as i32,
        };

        match state.canonical_client.append_events(req).await {
            Ok(resp) => {
                let results: Vec<EventResult> = resp.into_inner().results;
                let mut seen_ids: HashSet<String> = HashSet::new();

                for r in &results {
                    if !seen_ids.insert(r.event_id.clone()) {
                        warn!(event_id = %r.event_id, "duplicate event_id in CI response");
                        continue;
                    }
                    let pk = match event_id_to_pk.get(&r.event_id) {
                        Some(p) => p,
                        None => {
                            warn!(event_id = %r.event_id, "result for unknown event_id");
                            continue;
                        }
                    };
                    let status = EventStatus::try_from(r.status).unwrap_or(EventStatus::Unspecified);
                    match status {
                        EventStatus::Appended | EventStatus::Deduped => {
                            updates.push(UpdateRow {
                                recorded_month: pk.0,
                                audit_outbox_id: pk.1,
                                pending_forward: false,
                                forwarded_at: Some(Utc::now()),
                                last_forward_error: None,
                            });
                        }
                        other => {
                            updates.push(UpdateRow {
                                recorded_month: pk.0,
                                audit_outbox_id: pk.1,
                                pending_forward: true,
                                forwarded_at: None,
                                last_forward_error: Some(format!("{:?}", other)),
                            });
                        }
                    }
                }

                // Detect orphan: sent but no result returned (Codex r2 V2.5).
                for (event_id, pk) in &event_id_to_pk {
                    if !seen_ids.contains(event_id) {
                        updates.push(UpdateRow {
                            recorded_month: pk.0,
                            audit_outbox_id: pk.1,
                            pending_forward: true,
                            forwarded_at: None,
                            last_forward_error: Some("NO_RESULT_RETURNED".into()),
                        });
                    }
                }
            }
            Err(grpc_status) => {
                warn!(status = %grpc_status, "canonical_ingest.AppendEvents batch failed");
                for (_, pk) in &event_id_to_pk {
                    updates.push(UpdateRow {
                        recorded_month: pk.0,
                        audit_outbox_id: pk.1,
                        pending_forward: true,
                        forwarded_at: None,
                        last_forward_error: Some(format!("BATCH_GRPC: {}", grpc_status.code())),
                    });
                }
            }
        }
    }

    if !updates.is_empty() {
        apply_updates(state, &updates).await?;
    }

    Ok(total)
}

async fn apply_updates(state: &AppState, updates: &[UpdateRow]) -> anyhow::Result<()> {
    let mut qb: QueryBuilder<sqlx::Postgres> = QueryBuilder::new(
        "UPDATE audit_outbox AS a \
            SET pending_forward    = v.pending_forward, \
                forwarded_at       = v.forwarded_at, \
                forward_attempts   = a.forward_attempts + 1, \
                last_forward_error = v.last_forward_error \
           FROM (",
    );
    qb.push_values(updates, |mut b, u| {
        b.push_bind(u.recorded_month)
            .push_bind(u.audit_outbox_id)
            .push_bind(u.pending_forward)
            .push_bind(u.forwarded_at)
            .push_bind(u.last_forward_error.clone());
    });
    qb.push(
        ") AS v(recorded_month, audit_outbox_id, pending_forward, forwarded_at, last_forward_error) \
          WHERE a.recorded_month = v.recorded_month AND a.audit_outbox_id = v.audit_outbox_id",
    );

    qb.build().execute(&state.pg).await?;
    info!(count = updates.len(), "applied batch update to audit_outbox");
    Ok(())
}
