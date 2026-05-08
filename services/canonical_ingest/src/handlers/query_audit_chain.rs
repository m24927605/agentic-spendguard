//! `CanonicalIngest::QueryAuditChain` server-streaming handler.

use prost::bytes::Bytes;
use prost_types::Timestamp;
use sqlx::PgPool;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Response, Status};
use tracing::warn;
use uuid::Uuid;

use crate::{
    persistence::query::{self, ChainRow},
    proto::canonical_ingest::v1::{
        query_audit_chain_request::Anchor as ProtoAnchor,
        query_audit_chain_request::StorageClass as ProtoStorageClass, AuditChainEvent,
        IngestPosition, QueryAuditChainRequest,
    },
    proto::common::v1::CloudEvent,
};

pub async fn handle(
    pool: PgPool,
    req: QueryAuditChainRequest,
) -> Result<Response<ReceiverStream<Result<AuditChainEvent, Status>>>, Status> {
    let tenant_id = Uuid::parse_str(&req.tenant_id)
        .map_err(|e| Status::invalid_argument(format!("tenant_id: {}", e)))?;

    let storage_classes: Vec<&'static str> = if req.storage_classes.is_empty() {
        vec!["immutable_audit_log", "canonical_raw_log"]
    } else {
        req.storage_classes
            .iter()
            .filter_map(|c| match ProtoStorageClass::try_from(*c) {
                Ok(ProtoStorageClass::ImmutableAuditLog) => Some("immutable_audit_log"),
                Ok(ProtoStorageClass::CanonicalRawLog) => Some("canonical_raw_log"),
                Ok(ProtoStorageClass::ProfilePayloadBlob) => Some("profile_payload_blob"),
                _ => None,
            })
            .collect()
    };

    let (tx, rx) = mpsc::channel::<Result<AuditChainEvent, Status>>(64);

    tokio::spawn(async move {
        let rows_result = match req.anchor {
            Some(ProtoAnchor::DecisionId(s)) => match Uuid::parse_str(&s) {
                Ok(decision_id) => {
                    query::query_chain_by_decision(&pool, tenant_id, decision_id, Some(storage_classes))
                        .await
                        .map_err(|e| e.to_status())
                }
                Err(e) => Err(Status::invalid_argument(format!("decision_id: {}", e))),
            },
            Some(ProtoAnchor::RunId(s)) => match Uuid::parse_str(&s) {
                Ok(run_id) => {
                    query::query_chain_by_run(&pool, tenant_id, run_id, Some(storage_classes))
                        .await
                        .map_err(|e| e.to_status())
                }
                Err(e) => Err(Status::invalid_argument(format!("run_id: {}", e))),
            },
            Some(ProtoAnchor::ReservationId(_)) => Err(Status::unimplemented(
                "ReservationId anchor: requires denorm column; vertical slice expansion",
            )),
            None => Err(Status::invalid_argument("anchor required")),
        };

        match rows_result {
            Ok(rows) => {
                for row in rows {
                    let cloudevent = match decode_cloudevent_from_row(&row) {
                        Ok(ev) => ev,
                        Err(e) => {
                            warn!(
                                event_id = %row.event_id,
                                err = %e,
                                "skipping row with undecodable CloudEvent payload"
                            );
                            continue;
                        }
                    };
                    let storage_class_enum = match row.storage_class.as_str() {
                        "immutable_audit_log" => ProtoStorageClass::ImmutableAuditLog,
                        "canonical_raw_log" => ProtoStorageClass::CanonicalRawLog,
                        "profile_payload_blob" => ProtoStorageClass::ProfilePayloadBlob,
                        _ => ProtoStorageClass::Unspecified,
                    };
                    let chain_event = AuditChainEvent {
                        event: Some(cloudevent),
                        ingest_position: Some(IngestPosition {
                            region_id: row.region_id.clone(),
                            ingest_shard_id: row.ingest_shard_id.clone(),
                            ingest_log_offset: row.ingest_log_offset as u64,
                        }),
                        ingested_at: Some(Timestamp {
                            seconds: row.ingest_at.timestamp(),
                            nanos: row.ingest_at.timestamp_subsec_nanos() as i32,
                        }),
                        storage_class: storage_class_enum as i32,
                    };
                    if tx.send(Ok(chain_event)).await.is_err() {
                        return;
                    }
                }
            }
            Err(s) => {
                let _ = tx.send(Err(s)).await;
            }
        }
    });

    Ok(Response::new(ReceiverStream::new(rx)))
}

fn decode_cloudevent_from_row(row: &ChainRow) -> Result<CloudEvent, anyhow::Error> {
    use base64::Engine as _;
    let payload = row
        .payload_json
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("payload_json absent"))?;
    let get_str = |k: &str| -> String {
        payload
            .get(k)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_default()
    };
    let get_u64 =
        |k: &str| -> u64 { payload.get(k).and_then(|v| v.as_u64()).unwrap_or_default() };
    let get_i64 =
        |k: &str| -> i64 { payload.get(k).and_then(|v| v.as_i64()).unwrap_or_default() };
    let get_i32 = |k: &str| -> i32 { get_i64(k) as i32 };

    let data_b64 = get_str("data_b64");
    let data: Bytes = if data_b64.is_empty() {
        Bytes::new()
    } else {
        base64::engine::general_purpose::STANDARD
            .decode(data_b64)
            .map_err(|e| anyhow::anyhow!("data_b64 decode: {}", e))?
            .into()
    };

    Ok(CloudEvent {
        specversion: get_str("specversion"),
        r#type: get_str("type"),
        source: get_str("source"),
        id: get_str("id"),
        time: Some(Timestamp {
            seconds: get_i64("time_seconds"),
            nanos: get_i32("time_nanos"),
        }),
        datacontenttype: get_str("datacontenttype"),
        data,
        tenant_id: get_str("tenantid"),
        run_id: get_str("runid"),
        decision_id: get_str("decisionid"),
        schema_bundle_id: get_str("schema_bundle_id"),
        producer_id: get_str("producer_id"),
        producer_sequence: get_u64("producer_sequence"),
        producer_signature: row.producer_signature.clone().into(),
        signing_key_id: get_str("signing_key_id"),
    })
}
