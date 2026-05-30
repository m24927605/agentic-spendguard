//! SLICE_11 — OpenAI provider implementation.
//!
//! Covers both Chat Completions (`/v1/chat/completions`) and the
//! Responses API (`/v1/responses`). Both return usage in the same
//! shape:
//!
//!   {
//!     "usage": {
//!       "prompt_tokens": N,
//!       "completion_tokens": M,
//!       "total_tokens": N+M
//!     }
//!   }
//!
//! The Responses API also nests this under `"response": { "usage": ... }`
//! on the SSE `response.completed` event, but the non-streaming JSON
//! response surfaces usage at the top level (verified against the
//! 2024-09 Responses API GA spec).

use serde_json::Value;

use crate::routing::UsageMetrics;

/// Pull `(input, output, total)` token counts from an OpenAI JSON
/// response. Accepts both:
///
///   * Chat Completions: top-level `usage.{prompt,completion,total}_tokens`
///   * Responses API non-streaming: same top-level shape
///   * Responses API SSE final event: `response.usage.{...}_tokens`
///
/// Spec invariant: returns `UsageMetrics::default()` (all zeros) when
/// the response body is missing usage. The proxy's commit lane treats
/// zero as the "release with no usage" path → reservation TTL release.
pub fn extract_usage(body: &Value) -> UsageMetrics {
    // Try top-level first (Chat Completions + Responses non-streaming).
    if let Some(usage) = body.get("usage").and_then(|u| u.as_object()) {
        return UsageMetrics {
            input_tokens: get_i64(usage, "prompt_tokens"),
            output_tokens: get_i64(usage, "completion_tokens"),
            total_tokens: get_i64(usage, "total_tokens"),
            // OpenAI doesn't expose cache_creation / cache_read; left 0.
            ..Default::default()
        };
    }
    // Fallback: Responses API SSE-final shape (rare in non-streaming
    // path; defensive).
    if let Some(usage) = body
        .get("response")
        .and_then(|r| r.get("usage"))
        .and_then(|u| u.as_object())
    {
        return UsageMetrics {
            input_tokens: get_i64(usage, "prompt_tokens"),
            output_tokens: get_i64(usage, "completion_tokens"),
            total_tokens: get_i64(usage, "total_tokens"),
            ..Default::default()
        };
    }
    UsageMetrics::default()
}

fn get_i64(obj: &serde_json::Map<String, Value>, key: &str) -> i64 {
    obj.get(key).and_then(|v| v.as_i64()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_chat_completions_usage() {
        let body = json!({
            "id": "chatcmpl-abc",
            "object": "chat.completion",
            "usage": {
                "prompt_tokens": 13,
                "completion_tokens": 42,
                "total_tokens": 55,
            }
        });
        let u = extract_usage(&body);
        assert_eq!(u.input_tokens, 13);
        assert_eq!(u.output_tokens, 42);
        assert_eq!(u.total_tokens, 55);
    }

    #[test]
    fn extracts_responses_api_usage() {
        // Responses API non-streaming response (GA shape).
        let body = json!({
            "id": "resp_abc",
            "object": "response",
            "usage": {
                "prompt_tokens": 7,
                "completion_tokens": 14,
                "total_tokens": 21,
            }
        });
        let u = extract_usage(&body);
        assert_eq!(u.input_tokens, 7);
        assert_eq!(u.output_tokens, 14);
        assert_eq!(u.total_tokens, 21);
    }

    #[test]
    fn extracts_responses_api_sse_final_event() {
        // Defensive: Responses API SSE final event shape.
        let body = json!({
            "type": "response.completed",
            "response": {
                "id": "resp_xyz",
                "usage": {
                    "prompt_tokens": 100,
                    "completion_tokens": 200,
                    "total_tokens": 300,
                }
            }
        });
        let u = extract_usage(&body);
        assert_eq!(u.input_tokens, 100);
        assert_eq!(u.output_tokens, 200);
        assert_eq!(u.total_tokens, 300);
    }

    #[test]
    fn missing_usage_returns_zeros() {
        let body = json!({"id": "chatcmpl-abc"});
        let u = extract_usage(&body);
        assert_eq!(u.input_tokens, 0);
        assert_eq!(u.output_tokens, 0);
        assert_eq!(u.total_tokens, 0);
    }

    #[test]
    fn partial_usage_block() {
        // Some providers report only prompt_tokens. Sum is computed
        // by UsageMetrics::total_for_commit (caller).
        let body = json!({
            "usage": {
                "prompt_tokens": 50,
            }
        });
        let u = extract_usage(&body);
        assert_eq!(u.input_tokens, 50);
        assert_eq!(u.output_tokens, 0);
        assert_eq!(u.total_tokens, 0);
        assert_eq!(u.total_for_commit(), 50);
    }
}
