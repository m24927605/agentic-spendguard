//! Cost Advisor P1.5 — failure classification per spec §5.1.2.
//!
//! Maps an `spendguard.audit.outcome` event's CloudEvent payload to
//! one of the 9 `canonical_events.failure_class` enum values:
//!
//!   * `unknown`             — classifier could not match
//!   * `provider_5xx`        — HTTP 500/502/503/504; billed
//!   * `provider_4xx_billed` — HTTP 400 with `usage` field present
//!   * `provider_4xx_unbilled` — HTTP 400/401/429 without `usage`
//!   * `tool_error`          — framework `ToolException` / `ToolCallError`
//!   * `malformed_json_response` — LLM returned unparseable JSON
//!   * `timeout_billed`      — partial response then timeout; `usage` set
//!   * `timeout_unbilled`    — client-side timeout pre-response
//!   * `retry_then_success`  — eventually succeeded after N failed attempts
//!
//! The classification is computed once per audit.outcome event at
//! `AppendEvents` INSERT time and written to
//! `canonical_events.failure_class`. Pre-CA-P1.5 rows stay NULL and
//! rules treat NULL as "not classified" (skip).
//!
//! `CLASSIFIER_VERSION` is bumped when the rules change. A separate
//! operational procedure may re-classify recent rows by temporarily
//! dropping the immutability trigger (see migration 0011 header).
//!
//! `decode_payload_data` lives here too — the audit_outbox /
//! canonical_events `payload_json` carries a base64-encoded
//! CloudEvent `data` field per CA-P0 audit-report §0.2. The
//! classifier needs the decoded JSON to inspect kind/outcome/usage.

use serde::{Deserialize, Serialize};

pub const CLASSIFIER_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    Unknown,
    Provider5xx,
    Provider4xxBilled,
    Provider4xxUnbilled,
    ToolError,
    MalformedJsonResponse,
    TimeoutBilled,
    TimeoutUnbilled,
    RetryThenSuccess,
}

impl FailureClass {
    /// Wire form for the `canonical_events.failure_class` TEXT column.
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Provider5xx => "provider_5xx",
            Self::Provider4xxBilled => "provider_4xx_billed",
            Self::Provider4xxUnbilled => "provider_4xx_unbilled",
            Self::ToolError => "tool_error",
            Self::MalformedJsonResponse => "malformed_json_response",
            Self::TimeoutBilled => "timeout_billed",
            Self::TimeoutUnbilled => "timeout_unbilled",
            Self::RetryThenSuccess => "retry_then_success",
        }
    }
}

/// Classify an audit.outcome event by inspecting its (decoded) CloudEvent
/// data payload. Returns None when the event isn't an audit.outcome —
/// callers persist NULL in that case.
///
/// `decoded_data` is the JSON object parsed from
/// `cloudevent_payload->>'data_b64'` after base64 decode + UTF-8.
/// Pass `None` when decode failed (corrupt payload); classifier
/// returns `Some(FailureClass::Unknown)` to preserve a valid row.
pub fn classify_audit_outcome(
    event_type: &str,
    decoded_data: Option<&serde_json::Value>,
) -> Option<FailureClass> {
    if event_type != "spendguard.audit.outcome" {
        return None;
    }
    let Some(data) = decoded_data else {
        return Some(FailureClass::Unknown);
    };

    // `kind` discriminates the outcome family. release/commit_estimated
    // are SpendGuard-internal lifecycle events; they don't carry
    // failure semantics. Map them to Unknown so they neither fire nor
    // suppress the billed-failure rule SQL.
    let kind = data.get("kind").and_then(|v| v.as_str());

    // Check for explicit framework signatures FIRST (independent of
    // HTTP status). Precedence rationale (codex CA-P1.5 r1 P3):
    // when a framework error fires AND the underlying provider also
    // returned a status code, the framework error is the PROXIMAL
    // cause — what the agent actually saw. Classifying as tool_error
    // here keeps the rule SQL's billed/unbilled accounting correct
    // (tool_error is conditional waste, not unconditional billed
    // waste). Provider HTTP status checks come AFTER this block.
    //
    // Codex CA-P1.5 r1 P3: match a closed allowlist of fully-
    // qualified framework class names instead of suffix `ends_with`
    // (which would misclassify app-specific classes like
    // CustomToolException). Update this list as new adapters land.
    if let Some(err_type) = data.get("error_type").and_then(|v| v.as_str()) {
        const TOOL_ERROR_TYPES: &[&str] = &[
            "langchain_core.exceptions.ToolException",
            "openai.agents.ToolCallError",
            "pydantic_ai.tools.ToolException",
        ];
        const MALFORMED_JSON_TYPES: &[&str] = &[
            "MalformedJsonError",
            "JSONDecodeError",
            "json.JSONDecodeError",
            "langchain_core.exceptions.OutputParserException",
        ];
        if TOOL_ERROR_TYPES.contains(&err_type) {
            return Some(FailureClass::ToolError);
        }
        if MALFORMED_JSON_TYPES.contains(&err_type) {
            return Some(FailureClass::MalformedJsonResponse);
        }
    }

    // outcome=SUCCESS after retries: retry_then_success. The retried
    // earlier attempts are the wasted ones; this rule fires on the
    // SUCCESS row to anchor the run.
    if let Some(retries) = data.get("retry_count").and_then(|v| v.as_i64()) {
        if retries > 0 && kind == Some("success") {
            return Some(FailureClass::RetryThenSuccess);
        }
    }

    // Provider response classification: HTTP status × usage field.
    let http_status = data
        .get("provider_http_status")
        .and_then(|v| v.as_i64());
    let has_usage = data
        .get("usage")
        .map(|v| !v.is_null())
        .unwrap_or(false);

    if let Some(status) = http_status {
        if (500..600).contains(&status) {
            return Some(FailureClass::Provider5xx);
        }
        if status == 400 {
            return Some(if has_usage {
                FailureClass::Provider4xxBilled
            } else {
                FailureClass::Provider4xxUnbilled
            });
        }
        if matches!(status, 401 | 403 | 429) {
            return Some(FailureClass::Provider4xxUnbilled);
        }
    }

    // Timeouts: kind='timeout' OR error_type is an exact-match
    // timeout class name. Codex CA-P1.5 r1 P3: `contains("Timeout")`
    // would misclassify names like `OperationTimeoutLog` or
    // `TimeoutConfig`. Use allowlisted exact names instead.
    const TIMEOUT_ERROR_TYPES: &[&str] = &[
        "TimeoutError",
        "asyncio.TimeoutError",
        "httpx.TimeoutException",
        "httpx.ReadTimeout",
        "httpx.ConnectTimeout",
        "concurrent.futures.TimeoutError",
        "openai.APITimeoutError",
        "anthropic.APITimeoutError",
    ];
    let is_timeout = kind == Some("timeout")
        || data
            .get("error_type")
            .and_then(|v| v.as_str())
            .map(|s| TIMEOUT_ERROR_TYPES.contains(&s))
            .unwrap_or(false);
    if is_timeout {
        return Some(if has_usage {
            FailureClass::TimeoutBilled
        } else {
            FailureClass::TimeoutUnbilled
        });
    }

    // No signal we recognize → Unknown. Better than NULL: rules know
    // we LOOKED and didn't find anything.
    Some(FailureClass::Unknown)
}

/// Helper: decode the base64 `data_b64` field on a stored
/// `cloudevent_payload` JSONB into a parsed JSON object. Returns
/// None on any decode failure (matches the lenient pattern in
/// `services/ledger/migrations/0039_*.sql`'s
/// `cost_advisor_safe_release_reason`).
pub fn decode_payload_data(payload_json: &serde_json::Value) -> Option<serde_json::Value> {
    let data_b64 = payload_json.get("data_b64")?.as_str()?;
    let bytes = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        data_b64,
    )
    .ok()?;
    let text = std::str::from_utf8(&bytes).ok()?;
    serde_json::from_str(text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn non_outcome_returns_none() {
        let data = json!({"kind": "reserve"});
        assert!(classify_audit_outcome("spendguard.audit.decision", Some(&data)).is_none());
    }

    #[test]
    fn outcome_missing_data_returns_unknown() {
        let c = classify_audit_outcome("spendguard.audit.outcome", None);
        assert_eq!(c, Some(FailureClass::Unknown));
    }

    #[test]
    fn provider_500_billed() {
        let data = json!({"kind": "error", "provider_http_status": 503, "usage": {"input_tokens": 100}});
        let c = classify_audit_outcome("spendguard.audit.outcome", Some(&data));
        assert_eq!(c, Some(FailureClass::Provider5xx));
    }

    #[test]
    fn provider_400_with_usage_billed() {
        let data = json!({"provider_http_status": 400, "usage": {"input_tokens": 200}});
        let c = classify_audit_outcome("spendguard.audit.outcome", Some(&data));
        assert_eq!(c, Some(FailureClass::Provider4xxBilled));
    }

    #[test]
    fn provider_400_without_usage_unbilled() {
        let data = json!({"provider_http_status": 400});
        let c = classify_audit_outcome("spendguard.audit.outcome", Some(&data));
        assert_eq!(c, Some(FailureClass::Provider4xxUnbilled));
    }

    #[test]
    fn provider_429_unbilled() {
        let data = json!({"provider_http_status": 429});
        let c = classify_audit_outcome("spendguard.audit.outcome", Some(&data));
        assert_eq!(c, Some(FailureClass::Provider4xxUnbilled));
    }

    #[test]
    fn tool_exception_classified() {
        let data = json!({"error_type": "langchain_core.exceptions.ToolException"});
        let c = classify_audit_outcome("spendguard.audit.outcome", Some(&data));
        assert_eq!(c, Some(FailureClass::ToolError));
    }

    #[test]
    fn openai_agents_tool_call_error_classified() {
        let data = json!({"error_type": "openai.agents.ToolCallError"});
        let c = classify_audit_outcome("spendguard.audit.outcome", Some(&data));
        assert_eq!(c, Some(FailureClass::ToolError));
    }

    #[test]
    fn custom_tool_exception_NOT_classified_as_tool_error() {
        // Codex CA-P1.5 r1 P3 fix: exact-match allowlist prevents
        // app-specific class names from being misclassified.
        let data = json!({"error_type": "my_app.weird.CustomToolException"});
        let c = classify_audit_outcome("spendguard.audit.outcome", Some(&data));
        assert_eq!(c, Some(FailureClass::Unknown));
    }

    #[test]
    fn malformed_json_classified() {
        let data = json!({"error_type": "MalformedJsonError"});
        let c = classify_audit_outcome("spendguard.audit.outcome", Some(&data));
        assert_eq!(c, Some(FailureClass::MalformedJsonResponse));
    }

    #[test]
    fn timeout_with_usage_billed() {
        let data = json!({"kind": "timeout", "usage": {"input_tokens": 50}});
        let c = classify_audit_outcome("spendguard.audit.outcome", Some(&data));
        assert_eq!(c, Some(FailureClass::TimeoutBilled));
    }

    #[test]
    fn timeout_no_usage_unbilled() {
        let data = json!({"kind": "timeout"});
        let c = classify_audit_outcome("spendguard.audit.outcome", Some(&data));
        assert_eq!(c, Some(FailureClass::TimeoutUnbilled));
    }

    #[test]
    fn timeout_lookalike_NOT_classified() {
        // Codex CA-P1.5 r1 P3 fix: contains() would have matched
        // "OperationTimeoutLog" or "TimeoutConfig" as timeout.
        // Exact-allowlist rejects them.
        let data = json!({"error_type": "OperationTimeoutLog"});
        let c = classify_audit_outcome("spendguard.audit.outcome", Some(&data));
        assert_eq!(c, Some(FailureClass::Unknown));
    }

    #[test]
    fn precedence_tool_error_beats_provider_status() {
        // When both fire, tool_error wins (proximal cause that the
        // agent observed). Codex CA-P1.5 r1 P3 noted this is a
        // judgment call; lock it via test.
        let data = json!({
            "error_type": "langchain_core.exceptions.ToolException",
            "provider_http_status": 500,
            "usage": {"input_tokens": 100}
        });
        let c = classify_audit_outcome("spendguard.audit.outcome", Some(&data));
        assert_eq!(c, Some(FailureClass::ToolError));
    }

    #[test]
    fn retry_then_success_classified() {
        let data = json!({"kind": "success", "retry_count": 3});
        let c = classify_audit_outcome("spendguard.audit.outcome", Some(&data));
        assert_eq!(c, Some(FailureClass::RetryThenSuccess));
    }

    #[test]
    fn release_kind_returns_unknown_not_failure() {
        // release events are SpendGuard-internal lifecycle, not provider
        // failures. They should NOT match any of the billed-failure
        // classes that drive failed_retry_burn_v1.
        let data = json!({"kind": "release", "reason": "TTL_EXPIRED"});
        let c = classify_audit_outcome("spendguard.audit.outcome", Some(&data));
        assert_eq!(c, Some(FailureClass::Unknown));
    }

    #[test]
    fn empty_data_returns_unknown() {
        let data = json!({});
        let c = classify_audit_outcome("spendguard.audit.outcome", Some(&data));
        assert_eq!(c, Some(FailureClass::Unknown));
    }

    #[test]
    fn decode_payload_data_handles_valid_base64() {
        let inner = json!({"kind": "success", "retry_count": 2});
        let inner_str = serde_json::to_string(&inner).unwrap();
        let b64 = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            inner_str.as_bytes(),
        );
        let envelope = json!({"data_b64": b64});
        let decoded = decode_payload_data(&envelope).unwrap();
        assert_eq!(decoded.get("kind").unwrap(), "success");
    }

    #[test]
    fn decode_payload_data_handles_invalid_base64() {
        let envelope = json!({"data_b64": "!!!not-base64@@@"});
        assert!(decode_payload_data(&envelope).is_none());
    }

    #[test]
    fn decode_payload_data_handles_missing_field() {
        let envelope = json!({"specversion": "1.0"});
        assert!(decode_payload_data(&envelope).is_none());
    }
}
