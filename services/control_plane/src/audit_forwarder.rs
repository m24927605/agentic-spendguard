use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use prost::Message as _;
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};
use tokio::time::sleep;
use tonic::transport::{Channel, Endpoint};
use tracing::{debug, info, warn};
use uuid::Uuid;

use spendguard_signing::Signer;

use crate::proto::canonical_ingest::v1::{
    append_events_request::Route, canonical_ingest_client::CanonicalIngestClient,
    event_result::Status as EventStatus, AppendEventsRequest, AppendEventsResponse,
};
use crate::proto::common::v1::{CloudEvent, SchemaBundleRef};

#[derive(Debug, Clone)]
pub struct AuditForwarderConfig {
    pub canonical_ingest_url: String,
    pub schema_bundle: SchemaBundleRef,
    pub poll_interval_seconds: u64,
    pub batch_size: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForwardSummary {
    pub forwarded: usize,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct PendingOutboxRow {
    audit_outbox_id: Uuid,
    tenant_id: Uuid,
    event_type: String,
    cloudevent_payload: Value,
    producer_sequence: i64,
}

pub async fn build_canonical_client(
    endpoint: &str,
) -> Result<CanonicalIngestClient<Channel>, anyhow::Error> {
    let channel = Endpoint::from_shared(endpoint.to_owned())
        .with_context(|| format!("invalid canonical_ingest_url `{endpoint}`"))?
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(5))
        .connect()
        .await
        .with_context(|| format!("connect canonical_ingest `{endpoint}`"))?;
    Ok(CanonicalIngestClient::new(channel))
}

pub fn spawn_audit_forwarder(
    pool: PgPool,
    signer: Arc<dyn Signer>,
    cfg: AuditForwarderConfig,
    mut client: CanonicalIngestClient<Channel>,
) {
    tokio::spawn(async move {
        info!(
            canonical_ingest_url = %cfg.canonical_ingest_url,
            poll_interval_seconds = cfg.poll_interval_seconds,
            batch_size = cfg.batch_size,
            producer_id = %signer.producer_identity(),
            "control-plane audit forwarder started"
        );
        let poll = Duration::from_secs(cfg.poll_interval_seconds.max(1));
        loop {
            match forward_once(&pool, signer.as_ref(), &cfg, &mut client).await {
                Ok(summary) if summary.forwarded > 0 => {
                    info!(
                        forwarded = summary.forwarded,
                        "control-plane audit outbox forwarded"
                    );
                }
                Ok(_) => debug!("control-plane audit outbox empty"),
                Err(e) => warn!(error = ?e, "control-plane audit forwarder batch failed"),
            }
            sleep(poll).await;
        }
    });
}

pub async fn forward_once(
    pool: &PgPool,
    signer: &dyn Signer,
    cfg: &AuditForwarderConfig,
    client: &mut CanonicalIngestClient<Channel>,
) -> Result<ForwardSummary, anyhow::Error> {
    let mut tx = pool.begin().await.context("begin audit forward tx")?;
    let rows = fetch_pending_rows(&mut tx, cfg.batch_size).await?;
    if rows.is_empty() {
        tx.commit().await?;
        return Ok(ForwardSummary { forwarded: 0 });
    }

    let mut forwarded = 0usize;
    for row in rows {
        let (req, signature) = build_signed_append_request(&row, signer, cfg).await?;
        let resp = client
            .append_events(req)
            .await
            .context("canonical_ingest AppendEvents for control-plane audit")?
            .into_inner();
        ensure_append_accepted(resp)?;
        mark_forwarded(&mut tx, row.audit_outbox_id, &signature).await?;
        forwarded += 1;
    }

    tx.commit().await.context("commit audit forward tx")?;
    Ok(ForwardSummary { forwarded })
}

async fn build_signed_append_request(
    row: &PendingOutboxRow,
    signer: &dyn Signer,
    cfg: &AuditForwarderConfig,
) -> Result<(AppendEventsRequest, Vec<u8>), anyhow::Error> {
    let mut event = row_to_cloudevent(row, signer)?;
    let mut canonical = Vec::new();
    event
        .encode(&mut canonical)
        .context("encode unsigned control-plane CloudEvent")?;
    let sig = signer
        .sign(&canonical)
        .await
        .map_err(|e| anyhow::anyhow!("sign control-plane audit event: {e}"))?;
    event.producer_signature = sig.bytes.clone();
    event.signing_key_id = sig.key_id.clone();

    Ok((
        AppendEventsRequest {
            producer_id: signer.producer_identity().to_string(),
            batch_max_producer_sequence: row.producer_sequence.max(0) as u64,
            batch_signature: Vec::new(),
            signing_key_id: sig.key_id.clone(),
            schema_bundle: Some(cfg.schema_bundle.clone()),
            events: vec![event],
            route: Route::Observability as i32,
        },
        sig.bytes,
    ))
}

async fn fetch_pending_rows(
    tx: &mut Transaction<'_, Postgres>,
    batch_size: i64,
) -> Result<Vec<PendingOutboxRow>, anyhow::Error> {
    sqlx::query_as::<_, PendingOutboxRow>(
        r#"
        SELECT audit_outbox_id, tenant_id, event_type, cloudevent_payload, producer_sequence
          FROM control_plane_audit_outbox
         WHERE forwarded_at IS NULL
         ORDER BY tenant_id, producer_sequence
         LIMIT $1
         FOR UPDATE SKIP LOCKED
        "#,
    )
    .bind(batch_size.max(1))
    .fetch_all(&mut **tx)
    .await
    .context("fetch pending control-plane audit outbox rows")
}

async fn mark_forwarded(
    tx: &mut Transaction<'_, Postgres>,
    audit_outbox_id: Uuid,
    signature: &[u8],
) -> Result<(), anyhow::Error> {
    sqlx::query(
        r#"
        UPDATE control_plane_audit_outbox
           SET cloudevent_payload_signature_hex = $2,
               forwarded_at = clock_timestamp()
         WHERE audit_outbox_id = $1
           AND forwarded_at IS NULL
        "#,
    )
    .bind(audit_outbox_id)
    .bind(hex::encode(signature))
    .execute(&mut **tx)
    .await
    .context("mark control-plane audit outbox forwarded")?;
    Ok(())
}

fn row_to_cloudevent(
    row: &PendingOutboxRow,
    signer: &dyn Signer,
) -> Result<CloudEvent, anyhow::Error> {
    let payload = &row.cloudevent_payload;
    let id = required_str(payload, "id")?.to_owned();
    let source = required_str(payload, "source")?.to_owned();
    let specversion = required_str(payload, "specversion")?.to_owned();
    let data = payload.get("data").cloned().unwrap_or(Value::Null);
    let decision_id = data
        .get("decision_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();

    Ok(CloudEvent {
        specversion,
        r#type: row.event_type.clone(),
        source,
        id,
        time: Some(prost_types::Timestamp {
            seconds: chrono::Utc::now().timestamp(),
            nanos: 0,
        }),
        datacontenttype: "application/json".to_string(),
        data: serde_json::to_vec(&data)?,
        tenant_id: row.tenant_id.to_string(),
        run_id: String::new(),
        decision_id,
        schema_bundle_id: String::new(),
        producer_id: signer.producer_identity().to_string(),
        producer_sequence: row.producer_sequence.max(0) as u64,
        producer_signature: Vec::new(),
        signing_key_id: signer.key_id().to_string(),
        ..Default::default()
    })
}

fn required_str<'a>(payload: &'a Value, key: &str) -> Result<&'a str, anyhow::Error> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow::anyhow!("control-plane audit payload missing `{key}`"))
}

pub fn ensure_append_accepted(resp: AppendEventsResponse) -> Result<(), anyhow::Error> {
    if resp.results.len() != 1 {
        anyhow::bail!(
            "AppendEvents control-plane audit returned {} results for one event",
            resp.results.len()
        );
    }
    let result = &resp.results[0];
    let status = EventStatus::try_from(result.status).unwrap_or(EventStatus::Unspecified);
    match status {
        EventStatus::Appended | EventStatus::Deduped => Ok(()),
        other => {
            let error_message = result
                .error
                .as_ref()
                .map(|err| err.message.as_str())
                .filter(|msg| !msg.is_empty())
                .unwrap_or("canonical_ingest returned no error detail");
            anyhow::bail!(
                "AppendEvents control-plane audit rejected event_id={} status={:?}: {}",
                result.event_id,
                other,
                error_message
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::canonical_ingest::v1::{event_result::Status, EventResult};
    use spendguard_signing::DisabledSigner;

    fn fixture_row() -> PendingOutboxRow {
        let tenant_id = Uuid::parse_str("01918000-0000-7c10-8c10-000000000601").unwrap();
        PendingOutboxRow {
            audit_outbox_id: Uuid::parse_str("01918000-0000-7c10-8c10-000000000602").unwrap(),
            tenant_id,
            event_type: "spendguard.audit.plugin_registered.v1alpha1".to_string(),
            producer_sequence: 7,
            cloudevent_payload: serde_json::json!({
                "specversion": "1.0",
                "type": "spendguard.audit.plugin_registered.v1alpha1",
                "id": "01918000-0000-7c10-8c10-000000000603",
                "source": "spendguard-control-plane",
                "tenantid": tenant_id.to_string(),
                "data": {
                    "tenant_id": tenant_id.to_string(),
                    "endpoint_url": "https://plugin.example.invalid",
                    "server_cert_fingerprint": "00".repeat(32)
                }
            }),
        }
    }

    fn fixture_cfg() -> AuditForwarderConfig {
        AuditForwarderConfig {
            canonical_ingest_url: "http://127.0.0.1:50052".into(),
            schema_bundle: SchemaBundleRef {
                schema_bundle_id: "22222222-2222-4222-8222-222222222222".into(),
                schema_bundle_hash: vec![0xcc; 32],
                canonical_schema_version: "spendguard.v1alpha1".into(),
            },
            poll_interval_seconds: 5,
            batch_size: 32,
        }
    }

    #[tokio::test]
    async fn signed_append_request_carries_full_observability_envelope() {
        let signer = DisabledSigner::for_test("control-plane:test".into());
        let (req, signature) = build_signed_append_request(&fixture_row(), &signer, &fixture_cfg())
            .await
            .expect("build signed request");

        assert_eq!(req.producer_id, "control-plane:test");
        assert_eq!(req.signing_key_id, "disabled:control-plane:test");
        assert_eq!(req.schema_bundle, Some(fixture_cfg().schema_bundle));
        assert_eq!(req.route, Route::Observability as i32);
        assert_eq!(req.batch_max_producer_sequence, 7);
        assert_eq!(req.events.len(), 1);
        assert_eq!(req.events[0].producer_id, "control-plane:test");
        assert_eq!(req.events[0].producer_sequence, 7);
        assert_eq!(
            req.events[0].r#type,
            "spendguard.audit.plugin_registered.v1alpha1"
        );
        assert_eq!(req.events[0].producer_signature, signature);
    }

    #[test]
    fn append_response_requires_durable_success() {
        assert!(ensure_append_accepted(AppendEventsResponse {
            results: vec![EventResult {
                event_id: "evt".into(),
                status: Status::Appended as i32,
                ingest_position: None,
                error: None,
            }],
        })
        .is_ok());

        assert!(ensure_append_accepted(AppendEventsResponse {
            results: vec![EventResult {
                event_id: "evt".into(),
                status: Status::SignatureInvalid as i32,
                ingest_position: None,
                error: None,
            }],
        })
        .is_err());
    }
}
