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

use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;
use spendguard_tokenizer::EncoderKind;
use tracing::warn;

use crate::routing::UsageMetrics;

/// SLICE_04 R2 B1 — Anthropic Claude 3 / 3.5 Bedrock dispatch
/// (cross-region prefix permitted).
static BEDROCK_ANTHROPIC_3_5: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^(?:[a-z][a-z0-9-]*\.)?anthropic\.claude-3-5-(sonnet|haiku|opus)(-\d{8})?-v\d+:\d+$")
        .expect("bedrock anthropic-3-5 regex")
});
static BEDROCK_ANTHROPIC_3: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^(?:[a-z][a-z0-9-]*\.)?anthropic\.claude-3-(sonnet|haiku|opus)(-\d{8})?-v\d+:\d+$")
        .expect("bedrock anthropic-3 regex")
});
/// SLICE_04 R2 B1 — Cohere Command R Bedrock dispatch
/// (cross-region prefix permitted). Note that the upstream Cohere
/// encoder is feature-gated in the tokenizer crate (`cohere` Cargo
/// feature); when off, model IDs fall to Tier 3.
static BEDROCK_COHERE_COMMAND_R: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^(?:[a-z][a-z0-9-]*\.)?cohere\.command(-r)?(-plus)?-v\d+:\d+$")
        .expect("bedrock cohere regex")
});
/// SLICE_04 R2 B1 — Meta Llama 3.x Bedrock dispatch
/// (cross-region prefix permitted). Pre-Llama-3 (`meta.llama2-*`)
/// intentionally falls to Tier 3 per SLICE_04 R2 B2 narrow Option A.
static BEDROCK_META_LLAMA3: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^(?:[a-z][a-z0-9-]*\.)?meta\.llama3(-\d+)?-\d+b-instruct-v\d+:\d+$")
        .expect("bedrock llama3 regex")
});

/// Bedrock-specific model dispatch — picks the correct tokenizer
/// kind for a Bedrock model id.
///
/// Returns:
///
///   * `Some(EncoderKind::Anthropic)` for anthropic.claude-3 / 3.5 models
///     (with or without cross-region prefix).
///   * `Some(EncoderKind::Cohere)` for cohere.command-r / -r-plus models
///     (with or without cross-region prefix). Note: the tokenizer
///     library's `cohere` feature must be enabled for this to load a
///     real encoder; otherwise it falls to Tier 3 inside the tokenizer.
///   * `Some(EncoderKind::Llama)` for meta.llama3-* models.
///   * `None` for unknown vendors / pre-Claude-3 / pre-Llama-3 /
///     Cohere embed models. Caller emits the
///     `tokenizer_unknown_model` metric per spec §3.3.
///
/// Per SLICE_04 R2 B2 — narrow Option A: silent under-count vs the
/// conservative 5% Tier 3 margin makes Tier 3 the safer default for
/// any unknown variant.
pub fn dispatch_tokenizer_kind(model_id: &str) -> Option<EncoderKind> {
    // 3.5 BEFORE 3 (first-match-wins; SLICE_04 ordering rule).
    if BEDROCK_ANTHROPIC_3_5.is_match(model_id) {
        return Some(EncoderKind::Anthropic);
    }
    if BEDROCK_ANTHROPIC_3.is_match(model_id) {
        return Some(EncoderKind::Anthropic);
    }
    if BEDROCK_COHERE_COMMAND_R.is_match(model_id) {
        return Some(EncoderKind::Cohere);
    }
    if BEDROCK_META_LLAMA3.is_match(model_id) {
        return Some(EncoderKind::Llama);
    }
    // Unknown — caller emits tokenizer_unknown_model metric per spec §3.3.
    warn!(model_id = %model_id, "bedrock unknown model; falling to Tier 3");
    None
}

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
        assert_eq!(
            dispatch_tokenizer_kind("meta.llama2-70b-chat-v1"),
            None
        );
    }

    #[test]
    fn dispatch_cohere_embed_falls_to_tier3() {
        // Different vocab from command-r.
        assert_eq!(
            dispatch_tokenizer_kind("cohere.embed-english-v3"),
            None
        );
        assert_eq!(
            dispatch_tokenizer_kind("cohere.embed-multilingual-v3"),
            None
        );
    }

    #[test]
    fn dispatch_unknown_vendor_falls_to_tier3() {
        assert_eq!(dispatch_tokenizer_kind("amazon.titan-text-express-v1"), None);
        assert_eq!(dispatch_tokenizer_kind("ai21.j2-ultra-v1"), None);
        assert_eq!(dispatch_tokenizer_kind("mistral.mistral-7b-instruct-v0:2"), None);
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
