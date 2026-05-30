use base64::Engine as _;
use serde_json::{json, Value};

use crate::proto::common::v1::CloudEvent;

pub(crate) fn cloudevent_payload(evt: &CloudEvent) -> Value {
    json!({
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
