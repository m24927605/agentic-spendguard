//! SLICE_11 — Vertex AI generateContent provider implementation.
//!
//! Inbound: `POST /v1/projects/{project}/locations/{location}/publishers/google/models/{model}:generateContent`
//!
//! Usage shape (per Vertex AI docs):
//!
//!   {
//!     "usageMetadata": {
//!       "promptTokenCount": N,
//!       "candidatesTokenCount": M,
//!       "totalTokenCount": N+M,
//!       "cachedContentTokenCount": K  // optional, context caching
//!     }
//!   }
//!
//! camelCase field names are GCP convention. The proxy returns
//! cached tokens in `cache_read_input_tokens` (no separate "creation"
//! event for Gemini context caching — the cache is created out-of-band
//! via the cachedContents.create API).

use serde_json::Value;

use crate::routing::UsageMetrics;

/// Pull `(input, output, total, cached)` token counts from a Vertex
/// generateContent JSON response.
pub fn extract_usage(body: &Value) -> UsageMetrics {
    let usage = match body.get("usageMetadata").and_then(|u| u.as_object()) {
        Some(u) => u,
        None => return UsageMetrics::default(),
    };
    let input = get_i64(usage, "promptTokenCount");
    let output = get_i64(usage, "candidatesTokenCount");
    let total = get_i64(usage, "totalTokenCount");
    let cached = get_i64(usage, "cachedContentTokenCount");
    UsageMetrics {
        input_tokens: input,
        output_tokens: output,
        total_tokens: if total > 0 {
            total
        } else {
            input.saturating_add(output)
        },
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: cached,
    }
}

fn get_i64(obj: &serde_json::Map<String, Value>, key: &str) -> i64 {
    obj.get(key).and_then(|v| v.as_i64()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_basic_vertex_usage() {
        let body = json!({
            "candidates": [{"content": {"parts": [{"text": "Hi"}]}}],
            "usageMetadata": {
                "promptTokenCount": 10,
                "candidatesTokenCount": 20,
                "totalTokenCount": 30,
            }
        });
        let u = extract_usage(&body);
        assert_eq!(u.input_tokens, 10);
        assert_eq!(u.output_tokens, 20);
        assert_eq!(u.total_tokens, 30);
    }

    #[test]
    fn extracts_context_cached_tokens() {
        let body = json!({
            "usageMetadata": {
                "promptTokenCount": 100,
                "candidatesTokenCount": 50,
                "totalTokenCount": 150,
                "cachedContentTokenCount": 80,
            }
        });
        let u = extract_usage(&body);
        assert_eq!(u.input_tokens, 100);
        assert_eq!(u.output_tokens, 50);
        assert_eq!(u.total_tokens, 150);
        assert_eq!(u.cache_read_input_tokens, 80);
        assert_eq!(u.cache_creation_input_tokens, 0);
    }

    #[test]
    fn missing_total_falls_back_to_sum() {
        // Some Vertex responses omit totalTokenCount on partial responses.
        let body = json!({
            "usageMetadata": {
                "promptTokenCount": 7,
                "candidatesTokenCount": 14,
            }
        });
        let u = extract_usage(&body);
        assert_eq!(u.input_tokens, 7);
        assert_eq!(u.output_tokens, 14);
        assert_eq!(u.total_tokens, 21);
    }

    #[test]
    fn missing_usage_metadata_returns_zeros() {
        let body = json!({"candidates": []});
        let u = extract_usage(&body);
        assert_eq!(u.input_tokens, 0);
        assert_eq!(u.output_tokens, 0);
    }

    #[test]
    fn handles_safety_blocked_response() {
        // When Vertex blocks for safety, candidates is empty but
        // usageMetadata still reports prompt tokens (no candidates output).
        let body = json!({
            "promptFeedback": {"blockReason": "SAFETY"},
            "usageMetadata": {
                "promptTokenCount": 50,
                "totalTokenCount": 50,
            }
        });
        let u = extract_usage(&body);
        assert_eq!(u.input_tokens, 50);
        assert_eq!(u.output_tokens, 0);
        assert_eq!(u.total_tokens, 50);
    }
}
