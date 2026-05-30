//! SLICE_11 — Anthropic Messages API provider implementation.
//!
//! Inbound: `POST /v1/messages` (proxy preserves the path on the
//! upstream call to `api.anthropic.com`).
//!
//! Usage shape (per Messages API docs):
//!
//!   {
//!     "usage": {
//!       "input_tokens": N,
//!       "output_tokens": M,
//!       "cache_creation_input_tokens": K,   // optional, prompt caching
//!       "cache_read_input_tokens": R        // optional, prompt caching
//!     }
//!   }
//!
//! No `total_tokens` field — the proxy computes total =
//! input + output via `UsageMetrics::total_for_commit`.
//!
//! ## §6 — cache token accounting (slice doc)
//!
//! Anthropic prompt caching surfaces two extra counters:
//!
//!   * `cache_creation_input_tokens`: tokens that BUILT the cache for
//!     a later read; billed at 1.25x the input rate.
//!   * `cache_read_input_tokens`: tokens served FROM cache; billed at
//!     0.1x the input rate.
//!
//! Both count as "input" for context-window accounting but at
//! different prices. The slice surfaces them separately so a future
//! audit-row extension can split them out without re-extracting.

use serde_json::Value;

use crate::routing::UsageMetrics;

/// Pull `(input, output, cache_creation, cache_read)` token counts
/// from an Anthropic Messages API JSON response.
pub fn extract_usage(body: &Value) -> UsageMetrics {
    let usage = match body.get("usage").and_then(|u| u.as_object()) {
        Some(u) => u,
        None => return UsageMetrics::default(),
    };
    let input = get_i64(usage, "input_tokens");
    let output = get_i64(usage, "output_tokens");
    let cache_creation = get_i64(usage, "cache_creation_input_tokens");
    let cache_read = get_i64(usage, "cache_read_input_tokens");
    UsageMetrics {
        input_tokens: input,
        output_tokens: output,
        total_tokens: input.saturating_add(output),
        cache_creation_input_tokens: cache_creation,
        cache_read_input_tokens: cache_read,
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
    fn extracts_basic_messages_usage() {
        let body = json!({
            "id": "msg_abc",
            "type": "message",
            "role": "assistant",
            "model": "claude-3-5-sonnet-20240620",
            "usage": {
                "input_tokens": 13,
                "output_tokens": 42,
            }
        });
        let u = extract_usage(&body);
        assert_eq!(u.input_tokens, 13);
        assert_eq!(u.output_tokens, 42);
        // Anthropic doesn't report total; computed by total_for_commit.
        assert_eq!(u.total_for_commit(), 55);
        assert_eq!(u.cache_creation_input_tokens, 0);
        assert_eq!(u.cache_read_input_tokens, 0);
    }

    #[test]
    fn extracts_cache_creation_and_read_separately() {
        // Per slice doc §6 — cache tokens surface separately so audit
        // can bill them at the right rate.
        let body = json!({
            "usage": {
                "input_tokens": 100,
                "output_tokens": 50,
                "cache_creation_input_tokens": 1000,
                "cache_read_input_tokens": 500,
            }
        });
        let u = extract_usage(&body);
        assert_eq!(u.input_tokens, 100);
        assert_eq!(u.output_tokens, 50);
        assert_eq!(u.cache_creation_input_tokens, 1000);
        assert_eq!(u.cache_read_input_tokens, 500);
        // total_for_commit uses ONLY input + output for the reservation
        // amount (cache tokens are surfaced separately for downstream
        // billing).
        assert_eq!(u.total_for_commit(), 150);
    }

    #[test]
    fn missing_usage_returns_zeros() {
        let body = json!({"id": "msg_abc"});
        let u = extract_usage(&body);
        assert_eq!(u.input_tokens, 0);
        assert_eq!(u.output_tokens, 0);
    }

    #[test]
    fn handles_partial_usage_with_only_input() {
        // Defensive: stream early termination might give us only input.
        let body = json!({
            "usage": {
                "input_tokens": 25,
            }
        });
        let u = extract_usage(&body);
        assert_eq!(u.input_tokens, 25);
        assert_eq!(u.output_tokens, 0);
        assert_eq!(u.total_for_commit(), 25);
    }

    #[test]
    fn cache_read_only_no_cache_creation() {
        // Real-world: subsequent calls read from cache without creating new.
        let body = json!({
            "usage": {
                "input_tokens": 50,
                "output_tokens": 100,
                "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 2000,
            }
        });
        let u = extract_usage(&body);
        assert_eq!(u.cache_creation_input_tokens, 0);
        assert_eq!(u.cache_read_input_tokens, 2000);
    }

    #[test]
    fn ignores_non_numeric_usage_field() {
        // Defensive against malformed responses.
        let body = json!({
            "usage": {
                "input_tokens": "thirteen",
                "output_tokens": 42,
            }
        });
        let u = extract_usage(&body);
        assert_eq!(u.input_tokens, 0);
        assert_eq!(u.output_tokens, 42);
    }
}
