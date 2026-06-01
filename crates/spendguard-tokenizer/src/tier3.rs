//! Tier 3 heuristic fallback.
//!
//! Spec ref `tokenizer-service-spec-v1alpha1.md` §5.
//!
//! ## Formula (§5.1)
//!
//! ```text
//! tier3_input_tokens = ceil(total_chars / 4 × 1.05)
//! ```
//!
//! ## Why 1.05 (not 2.0)?
//!
//! Per §5.1 commentary:
//!
//! > 1.05 是 5% conservative margin —— 故意比 v1alpha1 之前的 17 行
//! > heuristic (`chars / 4 × 2`) 窄；因為 Tier 3 只在 unknown model
//! > 時觸發，operator 應該主動 PR 補 dispatch entry，不該長期靠
//! > Tier 3 撐。寬 margin 會讓 operator 缺乏動機修。
//!
//! The legacy `chars / 4 × 2` was 100% margin — comfortable enough
//! that nobody bothered improving the predictor. v1alpha1 narrows it
//! to 5% so any Tier 3 hit produces a visible
//! `tokenizer_unknown_model` metric AND a noticeably tight
//! reservation, both of which force the operator to file a PR
//! adding the missing dispatch entry.
//!
//! Per §5.3 health invariant the goal is
//! `tier3_hits / total_calls < 0.001` (0.1%). The control plane
//! alerts when this breaks; SLICE-extra ships the dashboard.

use crate::{TokenizeRequest, TokenizeResponse};

#[cfg(test)]
use crate::Message;

/// 5% margin per spec §5.1.
pub const TIER3_MARGIN_RATIO: f32 = 1.05;

/// Characters per token assumption — well-known approximation; per
/// §5.1 we deliberately do not refine this. The 5% margin absorbs
/// minor language / model variation.
pub const TIER3_CHARS_PER_TOKEN: f32 = 4.0;

/// Compute the Tier 3 fallback for a request the dispatch table did
/// not match. Per §5.1 input-side fallback only — `max_tokens`
/// output projection is handled by `output_predictor` in SLICE_06.
pub fn tier3_fallback(req: &TokenizeRequest) -> TokenizeResponse {
    let total_chars = total_chars(req);

    // Per §5.1: ceil(chars / 4 × 1.05).
    let tokens_float = (total_chars as f32) / TIER3_CHARS_PER_TOKEN * TIER3_MARGIN_RATIO;
    let input_tokens = tokens_float.ceil() as i64;

    TokenizeResponse {
        input_tokens,
        tier: "T3".to_string(),
        // Per audit-chain extension §2.1: Tier 3 → tokenizer_version_id NULL
        // (empty string at the proto / library boundary; the mirror crate
        // translates to SQL NULL).
        tokenizer_version_id: crate::versions::TIER3_NULL_SENTINEL_VERSION_ID.to_string(),
        kind: "HEURISTIC".to_string(),
        fallback_char_count: total_chars as i64,
        fallback_margin_ratio: TIER3_MARGIN_RATIO,
        latency_ns: 0, // populated by Tokenizer::tokenize wrapper
    }
}

/// Sum character counts across all parts of a request.
///
/// We sum `raw_text` + each message's `content` + every `tool_call`
/// (`name` and `arguments_json`). Empty parts contribute zero. We do
/// NOT count envelope tokens in Tier 3 — the 5% margin absorbs them.
fn total_chars(req: &TokenizeRequest) -> usize {
    let mut total = req.raw_text.chars().count();
    for msg in &req.messages {
        total += msg.role.chars().count();
        total += msg.content.chars().count();
        for tc in &msg.tool_calls {
            total += tc.name.chars().count();
            total += tc.arguments_json.chars().count();
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier3_response_shape() {
        let req = TokenizeRequest {
            model: "unknown-finetune-2026".to_string(),
            raw_text: "abcd".to_string(),
            ..Default::default()
        };
        let resp = tier3_fallback(&req);
        assert_eq!(resp.tier, "T3");
        assert_eq!(resp.kind, "HEURISTIC");
        assert!(resp.tokenizer_version_id.is_empty());
        assert_eq!(resp.fallback_char_count, 4);
        assert_eq!(resp.fallback_margin_ratio, 1.05);
        // ceil(4 / 4 * 1.05) = ceil(1.05) = 2
        assert_eq!(resp.input_tokens, 2);
    }

    #[test]
    fn tier3_formula_matches_spec_5_1() {
        // 80 chars / 4 = 20 → × 1.05 = 21 → ceil = 21.
        let req = TokenizeRequest {
            model: "unknown".to_string(),
            raw_text: "x".repeat(80),
            ..Default::default()
        };
        let resp = tier3_fallback(&req);
        assert_eq!(resp.fallback_char_count, 80);
        assert_eq!(resp.input_tokens, 21);
    }

    #[test]
    fn tier3_counts_messages_and_tool_calls() {
        let req = TokenizeRequest {
            model: "unknown".to_string(),
            messages: vec![
                Message {
                    role: "user".to_string(),     // 4 chars
                    content: "hello".to_string(), // 5 chars
                    tool_calls: vec![],
                },
                Message {
                    role: "assistant".to_string(), // 9 chars
                    content: String::new(),
                    tool_calls: vec![crate::ToolCall {
                        name: "get_weather".to_string(),                // 11 chars
                        arguments_json: r#"{"city":"sf"}"#.to_string(), // 13 chars
                    }],
                },
            ],
            raw_text: "extra".to_string(), // 5 chars
            ..Default::default()
        };
        let resp = tier3_fallback(&req);
        // 4 + 5 + 9 + 11 + 13 + 5 = 47
        assert_eq!(resp.fallback_char_count, 47);
        // ceil(47 / 4 * 1.05) = ceil(12.3375) = 13
        assert_eq!(resp.input_tokens, 13);
    }

    #[test]
    fn tier3_empty_request_returns_zero() {
        let req = TokenizeRequest::default();
        let resp = tier3_fallback(&req);
        assert_eq!(resp.fallback_char_count, 0);
        assert_eq!(resp.input_tokens, 0);
    }

    #[test]
    fn tier3_counts_multibyte_characters_not_bytes() {
        // Each CJK char = 1 .chars() unit but 3 UTF-8 bytes; spec
        // §5.1 uses chars (graphemes approximated by codepoints).
        // 4 CJK chars / 4 * 1.05 = 1.05 → ceil = 2.
        let req = TokenizeRequest {
            model: "unknown".to_string(),
            raw_text: "あいうえ".to_string(),
            ..Default::default()
        };
        let resp = tier3_fallback(&req);
        assert_eq!(resp.fallback_char_count, 4);
        assert_eq!(resp.input_tokens, 2);
    }
}
