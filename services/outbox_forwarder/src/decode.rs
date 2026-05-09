//! Strict CloudEvent decode (Codex r1 P2.1 + r2 V2.3 fixes).
//! Required fields fail closed; only truly optional fields default.

use base64::Engine as _;
use prost_types::Timestamp;
use thiserror::Error;
use uuid::Uuid;

use crate::{poll::OutboxRow, proto::common::v1::CloudEvent};

#[derive(Debug, Error)]
pub enum DecodeError {
    #[error("missing required field: {0}")]
    Missing(String),
    #[error("bad UUID at {0}")]
    BadUuid(String),
    #[error("bad number at {0}")]
    BadNumber(String),
    #[error("bad enum at {0}: {1}")]
    BadEnum(String, String),
    #[error("bad base64 at {0}")]
    BadBase64(String),
    #[error("payload not object")]
    NotObject,
}

pub fn strict_decode(row: &OutboxRow) -> Result<CloudEvent, DecodeError> {
    let p = row
        .cloudevent_payload
        .as_object()
        .ok_or(DecodeError::NotObject)?;

    let req_str = |key: &str| -> Result<String, DecodeError> {
        p.get(key)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .ok_or_else(|| DecodeError::Missing(key.into()))
    };
    let opt_str = |key: &str, default: &str| -> String {
        p.get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| default.to_string())
    };
    let req_uuid = |key: &str| -> Result<Uuid, DecodeError> {
        Uuid::parse_str(&req_str(key)?).map_err(|_| DecodeError::BadUuid(key.into()))
    };
    let req_i64 = |key: &str| -> Result<i64, DecodeError> {
        p.get(key)
            .and_then(|v| v.as_i64())
            .ok_or_else(|| DecodeError::BadNumber(key.into()))
    };
    let req_u64 = |key: &str| -> Result<u64, DecodeError> {
        p.get(key)
            .and_then(|v| v.as_u64())
            .ok_or_else(|| DecodeError::BadNumber(key.into()))
    };

    // --- core CloudEvent envelope (Codex r2 V2.3: stricter on these) ---
    let specversion = req_str("specversion")?;
    if specversion != "1.0" {
        return Err(DecodeError::BadEnum("specversion".into(), specversion));
    }
    let event_type = req_str("type")?;
    if event_type != "spendguard.audit.decision" && event_type != "spendguard.audit.outcome" {
        return Err(DecodeError::BadEnum("type".into(), event_type));
    }
    let source = req_str("source")?;
    let datacontenttype = req_str("datacontenttype")?;
    let id = req_str("id")?;
    let _ = Uuid::parse_str(&id).map_err(|_| DecodeError::BadUuid("id".into()))?;
    let time_seconds = req_i64("time_seconds")?;
    let time_nanos = req_i64("time_nanos")?;

    // --- audit-specific required fields ---
    let _ = req_uuid("tenantid")?;
    let _ = req_uuid("decisionid")?;
    let producer_id = req_str("producer_id")?;
    let producer_sequence = req_u64("producer_sequence")?;

    // --- payload data (required) ---
    let data_b64 = req_str("data_b64")?;
    let data: Vec<u8> = base64::engine::general_purpose::STANDARD
        .decode(&data_b64)
        .map_err(|_| DecodeError::BadBase64("data_b64".into()))?;

    // --- truly optional ---
    let run_id = opt_str("runid", "");
    let schema_bundle_id_field = opt_str("schema_bundle_id", "");
    let signing_key_id = opt_str("signing_key_id", "");

    Ok(CloudEvent {
        specversion,
        r#type: event_type,
        source,
        id,
        time: Some(Timestamp {
            seconds: time_seconds,
            nanos: time_nanos as i32,
        }),
        datacontenttype,
        data: data.into(),
        tenant_id: p
            .get("tenantid")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        run_id,
        decision_id: p
            .get("decisionid")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        schema_bundle_id: schema_bundle_id_field,
        producer_id,
        producer_sequence,
        signing_key_id,
        producer_signature: row.cloudevent_payload_signature.clone().into(),
    })
}
