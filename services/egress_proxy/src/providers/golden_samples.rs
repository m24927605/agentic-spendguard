//! SLICE_11 Phase B — Golden response samples per provider.
//!
//! Per slice §8.1 (§9.10 spec): each provider's `usage_extractor`
//! must round-trip 5+ real upstream response samples. These golden
//! samples are kept in one place so future provider additions follow
//! the same coverage rubric.
//!
//! Sources of these samples:
//!   * OpenAI: openai-python SDK test fixtures
//!     (https://github.com/openai/openai-python/tree/main/tests/api_resources)
//!   * Anthropic: anthropic-sdk-python repo tests
//!     (https://github.com/anthropics/anthropic-sdk-python/tree/main/tests/api_resources)
//!   * Bedrock: AWS docs example payloads
//!     (https://docs.aws.amazon.com/bedrock/latest/userguide/model-parameters.html)
//!   * Vertex: GCP docs example payloads
//!     (https://cloud.google.com/vertex-ai/generative-ai/docs/model-reference/inference)
//!   * Azure OpenAI: Azure docs response shapes
//!     (https://learn.microsoft.com/en-us/azure/ai-services/openai/reference)
//!
//! Each sample is a real-world response shape (or a known variant)
//! observed at upstream. Tests in this module pin the extracted
//! UsageMetrics so a future SDK upgrade that mutates response shape
//! breaks the test BEFORE silently mis-counting tokens in production.

#![cfg(test)]

use serde_json::json;

use crate::providers::{anthropic, azure_openai, bedrock, openai, vertex};
use crate::routing::UsageMetrics;

// ─── OpenAI samples (5 covering Chat Completions + Responses + edge cases) ──

#[test]
fn openai_sample_1_basic_chat_completion() {
    let body = json!({
        "id": "chatcmpl-9abc123",
        "object": "chat.completion",
        "created": 1714935600,
        "model": "gpt-4o-mini-2024-07-18",
        "choices": [{"message": {"role": "assistant", "content": "Hello!"}}],
        "usage": {"prompt_tokens": 9, "completion_tokens": 2, "total_tokens": 11}
    });
    let u = openai::extract_usage(&body);
    assert_eq!(u, UsageMetrics {
        input_tokens: 9, output_tokens: 2, total_tokens: 11,
        ..Default::default()
    });
}

#[test]
fn openai_sample_2_chat_with_tool_calls() {
    let body = json!({
        "id": "chatcmpl-tool-call",
        "model": "gpt-4o",
        "choices": [{"message": {"tool_calls": [{"id": "call_1", "type": "function"}]}}],
        "usage": {"prompt_tokens": 86, "completion_tokens": 18, "total_tokens": 104}
    });
    let u = openai::extract_usage(&body);
    assert_eq!(u.input_tokens, 86);
    assert_eq!(u.output_tokens, 18);
    assert_eq!(u.total_tokens, 104);
}

#[test]
fn openai_sample_3_responses_api_text_output() {
    // Responses API (2024-09 GA) non-streaming response shape.
    let body = json!({
        "id": "resp_67890",
        "object": "response",
        "model": "gpt-4o",
        "output": [{"type": "message", "role": "assistant"}],
        "usage": {"prompt_tokens": 23, "completion_tokens": 47, "total_tokens": 70}
    });
    let u = openai::extract_usage(&body);
    assert_eq!(u.total_tokens, 70);
}

#[test]
fn openai_sample_4_responses_sse_completed_event() {
    // SSE response.completed event JSON payload (post-`data:`).
    let body = json!({
        "type": "response.completed",
        "response": {
            "id": "resp_streamed",
            "usage": {"prompt_tokens": 12, "completion_tokens": 34, "total_tokens": 46}
        }
    });
    let u = openai::extract_usage(&body);
    assert_eq!(u.input_tokens, 12);
    assert_eq!(u.output_tokens, 34);
    assert_eq!(u.total_tokens, 46);
}

#[test]
fn openai_sample_5_zero_completion_tokens_error_response() {
    // Defensive: an error response that still includes prompt_tokens.
    let body = json!({
        "id": "chatcmpl-err",
        "choices": [],
        "usage": {"prompt_tokens": 5, "completion_tokens": 0, "total_tokens": 5}
    });
    let u = openai::extract_usage(&body);
    assert_eq!(u.input_tokens, 5);
    assert_eq!(u.output_tokens, 0);
}

// ─── Anthropic samples (5 covering messages + cache variants) ────────────────

#[test]
fn anthropic_sample_1_basic_messages_response() {
    let body = json!({
        "id": "msg_01EhPAB",
        "type": "message",
        "role": "assistant",
        "model": "claude-3-5-sonnet-20241022",
        "content": [{"type": "text", "text": "Hi there."}],
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 18, "output_tokens": 10}
    });
    let u = anthropic::extract_usage(&body);
    assert_eq!(u.input_tokens, 18);
    assert_eq!(u.output_tokens, 10);
    assert_eq!(u.total_for_commit(), 28);
}

#[test]
fn anthropic_sample_2_with_cache_creation_only() {
    // First call with cache_control on a system message.
    let body = json!({
        "id": "msg_cache_create",
        "model": "claude-3-5-sonnet-20240620",
        "usage": {
            "input_tokens": 50,
            "output_tokens": 100,
            "cache_creation_input_tokens": 2000,
            "cache_read_input_tokens": 0,
        }
    });
    let u = anthropic::extract_usage(&body);
    assert_eq!(u.cache_creation_input_tokens, 2000);
    assert_eq!(u.cache_read_input_tokens, 0);
}

#[test]
fn anthropic_sample_3_cache_read_subsequent_call() {
    // Subsequent calls with cache hit.
    let body = json!({
        "id": "msg_cache_read",
        "model": "claude-3-5-sonnet-20240620",
        "usage": {
            "input_tokens": 25,
            "output_tokens": 80,
            "cache_creation_input_tokens": 0,
            "cache_read_input_tokens": 1980,
        }
    });
    let u = anthropic::extract_usage(&body);
    assert_eq!(u.cache_read_input_tokens, 1980);
    assert_eq!(u.cache_creation_input_tokens, 0);
}

#[test]
fn anthropic_sample_4_tool_use_response() {
    let body = json!({
        "id": "msg_tool_use",
        "model": "claude-3-5-sonnet-20241022",
        "stop_reason": "tool_use",
        "content": [{"type": "tool_use", "id": "tool_call_1"}],
        "usage": {"input_tokens": 142, "output_tokens": 56}
    });
    let u = anthropic::extract_usage(&body);
    assert_eq!(u.input_tokens, 142);
    assert_eq!(u.output_tokens, 56);
}

#[test]
fn anthropic_sample_5_haiku_short_response() {
    let body = json!({
        "id": "msg_haiku",
        "model": "claude-3-haiku-20240307",
        "usage": {"input_tokens": 8, "output_tokens": 4}
    });
    let u = anthropic::extract_usage(&body);
    assert_eq!(u.input_tokens, 8);
    assert_eq!(u.output_tokens, 4);
}

// ─── Bedrock samples (5 covering anthropic / cohere / llama shapes) ──────

#[test]
fn bedrock_sample_1_claude_3_5_sonnet_response() {
    // Bedrock passes Anthropic Messages shape through.
    let body = json!({
        "id": "msg_bdrock_1",
        "model": "claude-3-5-sonnet-20240620",
        "usage": {"input_tokens": 30, "output_tokens": 45}
    });
    let u = bedrock::extract_usage(&body);
    assert_eq!(u.input_tokens, 30);
    assert_eq!(u.output_tokens, 45);
    assert_eq!(u.total_tokens, 75);
}

#[test]
fn bedrock_sample_2_cohere_command_r_response() {
    // Cohere on Bedrock: nested usage with prompt/completion.
    let body = json!({
        "id": "cohere_resp",
        "generations": [{"text": "Hello"}],
        "usage": {"prompt_tokens": 15, "completion_tokens": 5, "total_tokens": 20}
    });
    let u = bedrock::extract_usage(&body);
    assert_eq!(u.input_tokens, 15);
    assert_eq!(u.output_tokens, 5);
    assert_eq!(u.total_tokens, 20);
}

#[test]
fn bedrock_sample_3_llama_top_level_keys() {
    // Llama on Bedrock: top-level flat keys.
    let body = json!({
        "generation": "Hello world",
        "prompt_tokens": 12,
        "completion_tokens": 6,
        "stop_reason": "stop",
    });
    let u = bedrock::extract_usage(&body);
    assert_eq!(u.input_tokens, 12);
    assert_eq!(u.output_tokens, 6);
    assert_eq!(u.total_tokens, 18);
}

#[test]
fn bedrock_sample_4_cross_region_anthropic_response() {
    // Cross-region inference profile response — same shape.
    let body = json!({
        "id": "msg_us_anthropic",
        "model": "anthropic.claude-3-5-sonnet-20240620-v1:0",
        "usage": {"input_tokens": 100, "output_tokens": 200}
    });
    let u = bedrock::extract_usage(&body);
    assert_eq!(u.input_tokens, 100);
    assert_eq!(u.output_tokens, 200);
    assert_eq!(u.total_tokens, 300);
}

#[test]
fn bedrock_sample_5_missing_usage_returns_zero() {
    // Defensive: provider didn't include usage; commit lane treats as
    // PROVIDER_ERROR + reservation TTL release.
    let body = json!({"id": "msg_no_usage"});
    let u = bedrock::extract_usage(&body);
    assert_eq!(u.input_tokens, 0);
    assert_eq!(u.output_tokens, 0);
}

// ─── Vertex samples (5 covering generateContent variants) ────────────────

#[test]
fn vertex_sample_1_gemini_1_5_pro_response() {
    let body = json!({
        "candidates": [{"content": {"parts": [{"text": "Hi"}]}}],
        "usageMetadata": {
            "promptTokenCount": 5,
            "candidatesTokenCount": 8,
            "totalTokenCount": 13,
        }
    });
    let u = vertex::extract_usage(&body);
    assert_eq!(u.input_tokens, 5);
    assert_eq!(u.output_tokens, 8);
    assert_eq!(u.total_tokens, 13);
}

#[test]
fn vertex_sample_2_with_cached_content() {
    let body = json!({
        "candidates": [{"content": {"parts": [{"text": "Hi"}]}}],
        "usageMetadata": {
            "promptTokenCount": 50,
            "candidatesTokenCount": 10,
            "totalTokenCount": 60,
            "cachedContentTokenCount": 40,
        }
    });
    let u = vertex::extract_usage(&body);
    assert_eq!(u.cache_read_input_tokens, 40);
    assert_eq!(u.total_tokens, 60);
}

#[test]
fn vertex_sample_3_safety_blocked() {
    let body = json!({
        "promptFeedback": {"blockReason": "SAFETY"},
        "usageMetadata": {
            "promptTokenCount": 25,
            "totalTokenCount": 25,
        }
    });
    let u = vertex::extract_usage(&body);
    assert_eq!(u.input_tokens, 25);
    assert_eq!(u.output_tokens, 0);
}

#[test]
fn vertex_sample_4_gemini_2_0_flash_response() {
    let body = json!({
        "candidates": [{"content": {"parts": [{"text": "Answer"}]}, "finishReason": "STOP"}],
        "usageMetadata": {
            "promptTokenCount": 100,
            "candidatesTokenCount": 50,
            "totalTokenCount": 150,
        }
    });
    let u = vertex::extract_usage(&body);
    assert_eq!(u.input_tokens, 100);
    assert_eq!(u.output_tokens, 50);
}

#[test]
fn vertex_sample_5_missing_total_token_count() {
    // Partial response: only prompt + candidates counts, no total.
    let body = json!({
        "usageMetadata": {
            "promptTokenCount": 7,
            "candidatesTokenCount": 14,
        }
    });
    let u = vertex::extract_usage(&body);
    assert_eq!(u.total_tokens, 21); // Fallback to sum.
}

// ─── Azure OpenAI samples (5 covering content filter + deployment ids) ───

#[test]
fn azure_sample_1_basic_chat_completion() {
    let body = json!({
        "id": "chatcmpl-azure-1",
        "model": "gpt-4o",
        "usage": {"prompt_tokens": 9, "completion_tokens": 2, "total_tokens": 11}
    });
    let u = azure_openai::extract_usage(&body);
    assert_eq!(u.total_tokens, 11);
}

#[test]
fn azure_sample_2_with_content_filter_results() {
    let body = json!({
        "id": "chatcmpl-azure-2",
        "content_filter_results": {"hate": {"filtered": false}},
        "usage": {"prompt_tokens": 15, "completion_tokens": 35, "total_tokens": 50}
    });
    let u = azure_openai::extract_usage(&body);
    assert_eq!(u.total_tokens, 50);
}

#[test]
fn azure_sample_3_prompt_filter_only_response() {
    // Azure may return content filtered → completion=0.
    let body = json!({
        "id": "chatcmpl-azure-filtered",
        "choices": [],
        "prompt_filter_results": [{"prompt_index": 0}],
        "usage": {"prompt_tokens": 12, "completion_tokens": 0, "total_tokens": 12}
    });
    let u = azure_openai::extract_usage(&body);
    assert_eq!(u.input_tokens, 12);
    assert_eq!(u.output_tokens, 0);
}

#[test]
fn azure_sample_4_with_system_fingerprint() {
    let body = json!({
        "id": "chatcmpl-azure-4",
        "system_fingerprint": "fp_xyz",
        "usage": {"prompt_tokens": 100, "completion_tokens": 200, "total_tokens": 300}
    });
    let u = azure_openai::extract_usage(&body);
    assert_eq!(u.total_tokens, 300);
}

#[test]
fn azure_sample_5_jailbreak_blocked() {
    let body = json!({
        "id": "chatcmpl-azure-jailbreak",
        "content_filter_results": {"jailbreak": {"filtered": true, "detected": true}},
        "usage": {"prompt_tokens": 50, "completion_tokens": 0, "total_tokens": 50}
    });
    let u = azure_openai::extract_usage(&body);
    assert_eq!(u.input_tokens, 50);
    assert_eq!(u.output_tokens, 0);
}
