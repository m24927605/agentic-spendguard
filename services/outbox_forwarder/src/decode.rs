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
    let opt_i64 = |key: &str, default: i64| -> Result<i64, DecodeError> {
        match p.get(key) {
            Some(v) => v.as_i64().ok_or_else(|| DecodeError::BadNumber(key.into())),
            None => Ok(default),
        }
    };
    let opt_i32 = |key: &str, default: i32| -> Result<i32, DecodeError> {
        let value = opt_i64(key, default as i64)?;
        i32::try_from(value).map_err(|_| DecodeError::BadNumber(key.into()))
    };
    let opt_f32 = |key: &str, default: f32| -> Result<f32, DecodeError> {
        match p.get(key) {
            Some(v) => v
                .as_f64()
                .map(|n| n as f32)
                .ok_or_else(|| DecodeError::BadNumber(key.into())),
            None => Ok(default),
        }
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
        predicted_a_tokens: opt_i64("predicted_a_tokens", 0)?,
        predicted_b_tokens: opt_i64("predicted_b_tokens", 0)?,
        predicted_c_tokens: opt_i64("predicted_c_tokens", 0)?,
        reserved_strategy: opt_str("reserved_strategy", ""),
        prediction_strategy_used: opt_str("prediction_strategy_used", ""),
        prediction_policy_used: opt_str("prediction_policy_used", ""),
        tokenizer_tier: opt_str("tokenizer_tier", ""),
        tokenizer_version_id: opt_str("tokenizer_version_id", ""),
        prediction_confidence: opt_f32("prediction_confidence", 0.0)?,
        prediction_sample_size: opt_i64("prediction_sample_size", 0)?,
        cold_start_layer_used: opt_str("cold_start_layer_used", ""),
        run_projection_at_decision_atomic: opt_i64("run_projection_at_decision_atomic", 0)?,
        run_predicted_remaining_steps: opt_i32("run_predicted_remaining_steps", 0)?,
        run_steps_completed_so_far: opt_i64("run_steps_completed_so_far", 0)?,
        // Keep proto3 defaults here so canonical_ingest verifies the
        // producer's original canonical bytes. It derives SQL presence for
        // zero actuals from signed inner payload keys before mirroring.
        actual_input_tokens: opt_i64("actual_input_tokens", 0)?,
        actual_output_tokens: opt_i64("actual_output_tokens", 0)?,
        delta_b_ratio: opt_f32("delta_b_ratio", 0.0)?,
        delta_c_ratio: opt_f32("delta_c_ratio", 0.0)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;

    fn minimal_row() -> OutboxRow {
        let id = Uuid::now_v7();
        let decision_id = Uuid::now_v7();
        let tenant_id = Uuid::now_v7();
        OutboxRow {
            recorded_month: Utc::now().date_naive(),
            audit_outbox_id: id,
            audit_decision_event_id: id,
            decision_id,
            tenant_id,
            event_type: "spendguard.audit.decision".into(),
            cloudevent_payload: json!({
                "specversion": "1.0",
                "type": "spendguard.audit.decision",
                "source": "spendguard://sidecar/test",
                "id": id.to_string(),
                "time_seconds": 1,
                "time_nanos": 2,
                "datacontenttype": "application/json",
                "data_b64": "e30=",
                "tenantid": tenant_id.to_string(),
                "runid": "run-1",
                "decisionid": decision_id.to_string(),
                "schema_bundle_id": "schema-1",
                "producer_id": "sidecar:test",
                "producer_sequence": 7,
                "signing_key_id": "key-1"
            }),
            cloudevent_payload_signature: vec![1, 2, 3],
            producer_sequence: 7,
            recorded_at: Utc::now(),
        }
    }

    #[test]
    fn strict_decode_defaults_prediction_extension_fields() {
        let event = strict_decode(&minimal_row()).expect("decode minimal row");

        assert_eq!(event.predicted_a_tokens, 0);
        assert_eq!(event.reserved_strategy, "");
        assert_eq!(event.run_projection_at_decision_atomic, 0);
        assert_eq!(event.actual_input_tokens, 0);
        assert_eq!(event.delta_b_ratio, 0.0);
    }

    #[test]
    fn strict_decode_preserves_prediction_extension_fields() {
        let mut row = minimal_row();
        let payload = row.cloudevent_payload.as_object_mut().unwrap();
        payload.insert("predicted_a_tokens".into(), json!(4096));
        payload.insert("predicted_b_tokens".into(), json!(3072));
        payload.insert("predicted_c_tokens".into(), json!(2048));
        payload.insert("reserved_strategy".into(), json!("A"));
        payload.insert("prediction_strategy_used".into(), json!("B"));
        payload.insert("prediction_policy_used".into(), json!("STRICT_CEILING"));
        payload.insert("tokenizer_tier".into(), json!("T2"));
        payload.insert(
            "tokenizer_version_id".into(),
            json!("01918000-0000-7c10-8c10-000000000001"),
        );
        payload.insert("prediction_confidence".into(), json!(0.875));
        payload.insert("prediction_sample_size".into(), json!(42));
        payload.insert("cold_start_layer_used".into(), json!("L2"));
        payload.insert("run_projection_at_decision_atomic".into(), json!(9999));
        payload.insert("run_predicted_remaining_steps".into(), json!(-1));
        payload.insert("run_steps_completed_so_far".into(), json!(7));
        payload.insert("actual_input_tokens".into(), json!(123));
        payload.insert("actual_output_tokens".into(), json!(456));
        payload.insert("delta_b_ratio".into(), json!(1.25));
        payload.insert("delta_c_ratio".into(), json!(1.5));

        let event = strict_decode(&row).expect("decode row");

        assert_eq!(event.predicted_a_tokens, 4096);
        assert_eq!(event.predicted_b_tokens, 3072);
        assert_eq!(event.predicted_c_tokens, 2048);
        assert_eq!(event.reserved_strategy, "A");
        assert_eq!(event.prediction_strategy_used, "B");
        assert_eq!(event.prediction_policy_used, "STRICT_CEILING");
        assert_eq!(event.tokenizer_tier, "T2");
        assert_eq!(
            event.tokenizer_version_id,
            "01918000-0000-7c10-8c10-000000000001"
        );
        assert!((event.prediction_confidence - 0.875).abs() < f32::EPSILON);
        assert_eq!(event.prediction_sample_size, 42);
        assert_eq!(event.cold_start_layer_used, "L2");
        assert_eq!(event.run_projection_at_decision_atomic, 9999);
        assert_eq!(event.run_predicted_remaining_steps, -1);
        assert_eq!(event.run_steps_completed_so_far, 7);
        assert_eq!(event.actual_input_tokens, 123);
        assert_eq!(event.actual_output_tokens, 456);
        assert!((event.delta_b_ratio - 1.25).abs() < f32::EPSILON);
        assert!((event.delta_c_ratio - 1.5).abs() < f32::EPSILON);
    }
}
