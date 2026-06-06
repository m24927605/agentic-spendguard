//! SLICE_11 — AWS Bedrock InvokeModel provider implementation.
//!
//! Inbound: `POST /model/{model_id}/invoke` (the proxy preserves the
//! path on the upstream Bedrock call). The `model_id` segment can be
//! one of:
//!
//!   * `anthropic.claude-3-(5-)?(sonnet|haiku|opus)-YYYYMMDD-vN:M`
//!   * `cohere.command(-r|-light)?(-plus)?-vN:M`
//!   * `meta.llama3(-X)?-Yb-instruct-vN:M`
//!
//! Or with a SLICE_04 R2 B1 cross-region prefix:
//!
//!   * `(us|eu|apac|us-gov).anthropic.claude-3-...-vN:M`
//!   * `(us|eu|apac|us-gov).meta.llama3-...-vN:M`
//!   * `(us|eu|apac|us-gov).cohere.command-...-vN:M`
//!
//! Per `tokenizer-service-spec-v1alpha1.md` §3.1 + SLICE_04 R2 B1 the
//! cross-region prefix is a permissive `(?:[a-z][a-z0-9-]*\.)?` to
//! admit future region additions without code changes.
//!
//! ## §3.3 — unknown model fallback
//!
//! Bedrock model IDs that don't match any of the narrow vendor
//! patterns fall to Tier 3 (5% conservative margin) + the
//! `tokenizer_unknown_model` metric. Per `tokenizer-service-spec`
//! §3.4 we deliberately route to Tier 3 instead of guessing because
//! routing the wrong vocab silently under-counts ~5-20%.
//!
//! ## Usage shape
//!
//! Bedrock surfaces usage in two shapes depending on vendor:
//!
//!   anthropic.claude-*:
//!     {"usage": {"input_tokens": N, "output_tokens": M}}
//!     (Bedrock passes the Anthropic Messages shape through; no
//!      cache tokens because Bedrock doesn't expose prompt caching.)
//!
//!   cohere.command-*, meta.llama*:
//!     {"prompt_tokens": N, "completion_tokens": M}  (flat, no nesting)
//!
//! v1 handles both shapes via shape sniffing.

use serde_json::Value;

use crate::routing::UsageMetrics;

// COV_01: dispatch moved to crates/spendguard-provider-routing so that
// services/envoy_extproc can share it. We re-export the shared symbol so
// the existing `providers::bedrock::dispatch_tokenizer_kind` call sites
// in this crate (and the egress_proxy unit tests below) keep compiling
// byte-identical.
#[allow(unused_imports)]
pub use spendguard_provider_routing::bedrock::dispatch_tokenizer_kind;

#[cfg(test)]
use spendguard_tokenizer::EncoderKind;

/// Pull usage metrics from a Bedrock InvokeModel JSON response.
///
/// Bedrock's response body shape passes through to the underlying
/// vendor; v1 sniffs both forms (Anthropic-shape and OpenAI-style flat
/// keys for Cohere / Llama).
pub fn extract_usage(body: &Value) -> UsageMetrics {
    // ── Anthropic shape (nested under "usage") ──────────────────
    if let Some(usage) = body.get("usage").and_then(|u| u.as_object()) {
        // Anthropic Messages-on-Bedrock: input_tokens / output_tokens.
        let input = get_i64(usage, "input_tokens");
        let output = get_i64(usage, "output_tokens");
        if input > 0 || output > 0 {
            return UsageMetrics {
                input_tokens: input,
                output_tokens: output,
                total_tokens: input.saturating_add(output),
                ..Default::default()
            };
        }
        // OpenAI-style nested usage block (older Bedrock variants).
        let prompt = get_i64(usage, "prompt_tokens");
        let completion = get_i64(usage, "completion_tokens");
        if prompt > 0 || completion > 0 {
            return UsageMetrics {
                input_tokens: prompt,
                output_tokens: completion,
                total_tokens: prompt.saturating_add(completion),
                ..Default::default()
            };
        }
    }

    // ── Flat-key shape (Cohere / Llama on Bedrock often) ────────
    if let Some(obj) = body.as_object() {
        let prompt = get_i64(obj, "prompt_tokens");
        let completion = get_i64(obj, "completion_tokens");
        if prompt > 0 || completion > 0 {
            return UsageMetrics {
                input_tokens: prompt,
                output_tokens: completion,
                total_tokens: prompt.saturating_add(completion),
                ..Default::default()
            };
        }
        // Cohere on Bedrock uses "input_tokens" / "output_tokens" at top level too.
        let input = get_i64(obj, "input_tokens");
        let output = get_i64(obj, "output_tokens");
        if input > 0 || output > 0 {
            return UsageMetrics {
                input_tokens: input,
                output_tokens: output,
                total_tokens: input.saturating_add(output),
                ..Default::default()
            };
        }
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

    // ─── dispatch_tokenizer_kind ─────────────────────────────────────

    #[test]
    fn dispatch_anthropic_claude_3_5_sonnet() {
        assert_eq!(
            dispatch_tokenizer_kind("anthropic.claude-3-5-sonnet-20240620-v1:0"),
            Some(EncoderKind::Anthropic)
        );
    }

    #[test]
    fn dispatch_anthropic_claude_3_5_haiku() {
        assert_eq!(
            dispatch_tokenizer_kind("anthropic.claude-3-5-haiku-20241022-v1:0"),
            Some(EncoderKind::Anthropic)
        );
    }

    #[test]
    fn dispatch_anthropic_claude_3_opus() {
        assert_eq!(
            dispatch_tokenizer_kind("anthropic.claude-3-opus-20240229-v1:0"),
            Some(EncoderKind::Anthropic)
        );
    }

    #[test]
    fn dispatch_anthropic_cross_region_prefix() {
        // SLICE_04 R2 B1 — cross-region inference profile prefix.
        assert_eq!(
            dispatch_tokenizer_kind("us.anthropic.claude-3-5-sonnet-20240620-v1:0"),
            Some(EncoderKind::Anthropic)
        );
        assert_eq!(
            dispatch_tokenizer_kind("eu.anthropic.claude-3-haiku-20240307-v1:0"),
            Some(EncoderKind::Anthropic)
        );
        assert_eq!(
            dispatch_tokenizer_kind("apac.anthropic.claude-3-5-sonnet-20241022-v1:0"),
            Some(EncoderKind::Anthropic)
        );
        assert_eq!(
            dispatch_tokenizer_kind("us-gov.anthropic.claude-3-5-sonnet-20240620-v1:0"),
            Some(EncoderKind::Anthropic)
        );
    }

    #[test]
    fn dispatch_cohere_command_r() {
        assert_eq!(
            dispatch_tokenizer_kind("cohere.command-r-v1:0"),
            Some(EncoderKind::Cohere)
        );
        assert_eq!(
            dispatch_tokenizer_kind("cohere.command-r-plus-v1:0"),
            Some(EncoderKind::Cohere)
        );
    }

    #[test]
    fn dispatch_cohere_cross_region() {
        assert_eq!(
            dispatch_tokenizer_kind("us.cohere.command-r-v1:0"),
            Some(EncoderKind::Cohere)
        );
    }

    #[test]
    fn dispatch_meta_llama3() {
        assert_eq!(
            dispatch_tokenizer_kind("meta.llama3-1-8b-instruct-v1:0"),
            Some(EncoderKind::Llama)
        );
        assert_eq!(
            dispatch_tokenizer_kind("meta.llama3-1-70b-instruct-v1:0"),
            Some(EncoderKind::Llama)
        );
        assert_eq!(
            dispatch_tokenizer_kind("meta.llama3-2-1b-instruct-v1:0"),
            Some(EncoderKind::Llama)
        );
    }

    #[test]
    fn dispatch_meta_llama3_cross_region() {
        assert_eq!(
            dispatch_tokenizer_kind("us.meta.llama3-1-70b-instruct-v1:0"),
            Some(EncoderKind::Llama)
        );
    }

    #[test]
    fn dispatch_pre_claude_3_falls_to_tier3() {
        // Pre-Claude-3 uses different vocab — must NOT silently match
        // (would silently under-count by ~5-20% per SLICE_04 R2 B2).
        assert_eq!(dispatch_tokenizer_kind("anthropic.claude-instant-v1"), None);
        assert_eq!(dispatch_tokenizer_kind("anthropic.claude-v2"), None);
        assert_eq!(dispatch_tokenizer_kind("anthropic.claude-v2:1"), None);
    }

    #[test]
    fn dispatch_pre_llama_3_falls_to_tier3() {
        assert_eq!(dispatch_tokenizer_kind("meta.llama2-70b-chat-v1"), None);
    }

    #[test]
    fn dispatch_cohere_embed_falls_to_tier3() {
        // Different vocab from command-r.
        assert_eq!(dispatch_tokenizer_kind("cohere.embed-english-v3"), None);
        assert_eq!(
            dispatch_tokenizer_kind("cohere.embed-multilingual-v3"),
            None
        );
    }

    #[test]
    fn dispatch_unknown_vendor_falls_to_tier3() {
        assert_eq!(
            dispatch_tokenizer_kind("amazon.titan-text-express-v1"),
            None
        );
        assert_eq!(dispatch_tokenizer_kind("ai21.j2-ultra-v1"), None);
        assert_eq!(
            dispatch_tokenizer_kind("mistral.mistral-7b-instruct-v0:2"),
            None
        );
    }

    // ─── extract_usage ───────────────────────────────────────────────

    #[test]
    fn extracts_anthropic_shape() {
        let body = json!({
            "id": "msg_xyz",
            "usage": {
                "input_tokens": 50,
                "output_tokens": 75,
            }
        });
        let u = extract_usage(&body);
        assert_eq!(u.input_tokens, 50);
        assert_eq!(u.output_tokens, 75);
        assert_eq!(u.total_tokens, 125);
    }

    #[test]
    fn extracts_flat_prompt_completion_shape() {
        let body = json!({
            "id": "resp_abc",
            "prompt_tokens": 30,
            "completion_tokens": 60,
        });
        let u = extract_usage(&body);
        assert_eq!(u.input_tokens, 30);
        assert_eq!(u.output_tokens, 60);
        assert_eq!(u.total_tokens, 90);
    }

    #[test]
    fn extracts_nested_openai_style_shape() {
        let body = json!({
            "usage": {
                "prompt_tokens": 20,
                "completion_tokens": 40,
                "total_tokens": 60,
            }
        });
        let u = extract_usage(&body);
        assert_eq!(u.input_tokens, 20);
        assert_eq!(u.output_tokens, 40);
        assert_eq!(u.total_tokens, 60);
    }

    #[test]
    fn extracts_flat_input_output_shape() {
        // Cohere on Bedrock variant.
        let body = json!({
            "input_tokens": 15,
            "output_tokens": 30,
        });
        let u = extract_usage(&body);
        assert_eq!(u.input_tokens, 15);
        assert_eq!(u.output_tokens, 30);
        assert_eq!(u.total_tokens, 45);
    }

    #[test]
    fn missing_usage_returns_zeros() {
        let body = json!({"id": "msg_abc"});
        let u = extract_usage(&body);
        assert_eq!(u.input_tokens, 0);
        assert_eq!(u.output_tokens, 0);
    }
}
