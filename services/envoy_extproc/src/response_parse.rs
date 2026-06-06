//! SLICE 4 — provider response usage extractor.
//!
//! Maps the upstream LLM provider's response body → typed
//! [`ProviderUsage`] for SLICE 4's `LLM_CALL_POST.SUCCESS` audit emit.
//!
//! Two provider shapes are handled in v1 per design §3.5 (chat /
//! completions + messages scope):
//!
//!   * **OpenAI** — `{"usage": {"prompt_tokens": N, "completion_tokens": M}}`
//!   * **Anthropic** — `{"usage": {"input_tokens": N, "output_tokens": M}}`
//!
//! Unknown providers (Bedrock / Vertex / Azure / mis-routed) attempt
//! both shapes — whichever matches first wins — and fall back to
//! `tokens_unknown: true` when neither yields a number.
//!
//! ## Defense in depth
//!
//! The body is hard-capped at 1 MiB before JSON parsing. The actual
//! gateway hot path streams chunks; SLICE 4's Response-Body phase
//! commits at end-of-stream and the body we receive here is the final
//! chunk. A misbehaving upstream sending a 100 MB JSON blob will not
//! OOM the gateway; instead we return [`ParseError::TooLarge`] and the
//! caller falls back to a `tokens_unknown` audit emit (HARDEN_03 pattern).
//!
//! Streaming SSE bodies are explicitly out of scope for SLICE 4 per
//! review-standards §5.2 — the conformance fixtures land in SLICE 5.
//!
//! Spec refs:
//!   - docs/specs/coverage/D01_envoy_extproc/design.md §3.5 (wire-format scope)
//!   - docs/specs/coverage/D01_envoy_extproc/review-standards.md §5.1, §5.2
//!   - docs/slices/COV_04_envoy_extproc_audit_emit.md

use thiserror::Error;

/// Provider hint stashed on `StreamState` during Request-Headers /
/// Request-Body — drives which usage shape we try first. Falling back
/// to `Unknown` is safe (tries both shapes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderHint {
    OpenAi,
    Anthropic,
    /// Bedrock / Vertex / Azure / mis-routed. SLICE 4 still attempts
    /// both shapes; SLICE 5 conformance will pin per-provider mappings.
    Unknown,
}

impl ProviderHint {
    /// Derive a [`ProviderHint`] from `ParsedRequest.provider_str`. SLICE
    /// 4 only distinguishes OpenAI / Anthropic; everything else maps to
    /// [`ProviderHint::Unknown`].
    pub fn from_provider_str(provider: &str) -> Self {
        match provider {
            "openai" | "azure_openai" => Self::OpenAi,
            "anthropic" | "bedrock" => Self::Anthropic,
            _ => Self::Unknown,
        }
    }
}

/// Provider-reported usage extracted from the response body. The
/// `tokens_unknown` flag is `true` when neither shape yielded numbers —
/// in that case the caller MUST still emit `LLM_CALL_POST.SUCCESS` per
/// review-standards §5.2 fallback rule, but the audit row carries a
/// calibration-friendly "unknown" marker rather than a silent zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    /// True when neither OpenAI nor Anthropic usage shape parsed. The
    /// caller emits the audit event anyway so the audit chain stays
    /// complete; the LLM_CALL_POST payload's `actual_input_tokens` /
    /// `actual_output_tokens` are left unset and the audit row's
    /// calibration-ratio fields stay NULL.
    pub tokens_unknown: bool,
}

impl ProviderUsage {
    pub fn unknown() -> Self {
        Self {
            input_tokens: None,
            output_tokens: None,
            tokens_unknown: true,
        }
    }
}

/// Parse errors. The caller maps every variant to "tokens unknown"
/// behaviour on the LLM_CALL_POST audit emit — the audit row still
/// lands so the chain stays complete.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    /// Body exceeded the 1 MiB cap. Defense against an OOM via a
    /// hostile upstream; SLICE 5 streaming work is the proper fix.
    #[error("response body exceeded {limit_bytes} byte cap (got {actual_bytes})")]
    TooLarge {
        limit_bytes: usize,
        actual_bytes: usize,
    },

    /// JSON parse failed. We surface a typed variant so the caller can
    /// log a structured field; the inner detail is internal-only.
    #[error("response body is not valid JSON: {detail}")]
    Malformed {
        // Internal-only — never propagated to the wire (matches
        // review-standards §4.1.3 info-disclosure rule).
        detail: String,
    },
}

/// 1 MiB hard cap on the body we'll attempt to JSON-decode. Mirrors the
/// POST_GA_03 4 MiB cap on the Request-Body side; the response is
/// usually smaller (no system prompt), 1 MiB is more than enough for
/// any non-streaming completion.
pub const MAX_BODY_BYTES: usize = 1024 * 1024;

/// Extract `ProviderUsage` from a response body. On any non-fatal
/// shape mismatch we return `ProviderUsage::unknown()` so the audit
/// chain stays complete; the only hard errors are oversized or invalid
/// JSON.
pub fn extract_provider_usage(
    body: &[u8],
    provider: ProviderHint,
) -> Result<ProviderUsage, ParseError> {
    if body.len() > MAX_BODY_BYTES {
        return Err(ParseError::TooLarge {
            limit_bytes: MAX_BODY_BYTES,
            actual_bytes: body.len(),
        });
    }
    if body.is_empty() {
        // Zero-length body — treat as unknown so the caller still emits
        // SUCCESS (caller decides if HTTP status is 2xx vs 5xx).
        return Ok(ProviderUsage::unknown());
    }
    let value: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| ParseError::Malformed {
            detail: e.to_string(),
        })?;
    Ok(match provider {
        ProviderHint::OpenAi => parse_openai(&value).unwrap_or_else(ProviderUsage::unknown),
        ProviderHint::Anthropic => parse_anthropic(&value).unwrap_or_else(ProviderUsage::unknown),
        ProviderHint::Unknown => parse_openai(&value)
            .or_else(|| parse_anthropic(&value))
            .unwrap_or_else(ProviderUsage::unknown),
    })
}

/// OpenAI `usage.prompt_tokens` + `usage.completion_tokens`.
fn parse_openai(value: &serde_json::Value) -> Option<ProviderUsage> {
    let usage = value.get("usage")?;
    let input_tokens = usage.get("prompt_tokens").and_then(|v| v.as_u64());
    let output_tokens = usage.get("completion_tokens").and_then(|v| v.as_u64());
    if input_tokens.is_none() && output_tokens.is_none() {
        return None;
    }
    Some(ProviderUsage {
        input_tokens,
        output_tokens,
        tokens_unknown: false,
    })
}

/// Anthropic `usage.input_tokens` + `usage.output_tokens`.
fn parse_anthropic(value: &serde_json::Value) -> Option<ProviderUsage> {
    let usage = value.get("usage")?;
    let input_tokens = usage.get("input_tokens").and_then(|v| v.as_u64());
    let output_tokens = usage.get("output_tokens").and_then(|v| v.as_u64());
    if input_tokens.is_none() && output_tokens.is_none() {
        return None;
    }
    Some(ProviderUsage {
        input_tokens,
        output_tokens,
        tokens_unknown: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_openai_usage_shape() {
        let body = br#"{
            "id": "chatcmpl-abc",
            "object": "chat.completion",
            "model": "gpt-4o-mini",
            "usage": {
                "prompt_tokens": 17,
                "completion_tokens": 42,
                "total_tokens": 59
            }
        }"#;
        let got = extract_provider_usage(body, ProviderHint::OpenAi).expect("must parse");
        assert_eq!(got.input_tokens, Some(17));
        assert_eq!(got.output_tokens, Some(42));
        assert!(!got.tokens_unknown);
    }

    #[test]
    fn extracts_anthropic_usage_shape() {
        let body = br#"{
            "id": "msg_01ABC",
            "type": "message",
            "model": "claude-3-5-sonnet-20240620",
            "usage": {
                "input_tokens": 23,
                "output_tokens": 100
            }
        }"#;
        let got = extract_provider_usage(body, ProviderHint::Anthropic).expect("must parse");
        assert_eq!(got.input_tokens, Some(23));
        assert_eq!(got.output_tokens, Some(100));
        assert!(!got.tokens_unknown);
    }

    #[test]
    fn unknown_provider_tries_both_shapes_openai_wins() {
        // Unknown provider, OpenAI shape present.
        let body = br#"{"usage": {"prompt_tokens": 5, "completion_tokens": 6}}"#;
        let got = extract_provider_usage(body, ProviderHint::Unknown).expect("must parse");
        assert_eq!(got.input_tokens, Some(5));
        assert_eq!(got.output_tokens, Some(6));
        assert!(!got.tokens_unknown);
    }

    #[test]
    fn unknown_provider_falls_back_to_anthropic_when_openai_misses() {
        let body = br#"{"usage": {"input_tokens": 7, "output_tokens": 8}}"#;
        let got = extract_provider_usage(body, ProviderHint::Unknown).expect("must parse");
        assert_eq!(got.input_tokens, Some(7));
        assert_eq!(got.output_tokens, Some(8));
        assert!(!got.tokens_unknown);
    }

    #[test]
    fn missing_usage_field_returns_tokens_unknown() {
        // Valid JSON but no `usage` key — Bedrock InvokeModel shape that
        // SLICE 4 doesn't yet model. Audit chain still completes.
        let body = br#"{"id": "msg", "stop_reason": "end_turn"}"#;
        let got = extract_provider_usage(body, ProviderHint::OpenAi).expect("must parse");
        assert!(got.tokens_unknown);
        assert!(got.input_tokens.is_none());
        assert!(got.output_tokens.is_none());
    }

    #[test]
    fn malformed_json_returns_parse_error() {
        let body = b"{ not valid json at all";
        let err = extract_provider_usage(body, ProviderHint::OpenAi).expect_err("must error");
        assert!(matches!(err, ParseError::Malformed { .. }));
    }

    #[test]
    fn oversized_body_returns_too_large() {
        // Build a 1 MiB + 1 body so we trip the cap deterministically.
        let body = vec![b'a'; MAX_BODY_BYTES + 1];
        let err = extract_provider_usage(&body, ProviderHint::OpenAi).expect_err("must error");
        match err {
            ParseError::TooLarge {
                limit_bytes,
                actual_bytes,
            } => {
                assert_eq!(limit_bytes, MAX_BODY_BYTES);
                assert_eq!(actual_bytes, MAX_BODY_BYTES + 1);
            }
            other => panic!("expected TooLarge, got {other:?}"),
        }
    }

    #[test]
    fn empty_body_returns_tokens_unknown() {
        let got = extract_provider_usage(b"", ProviderHint::OpenAi).expect("must parse");
        assert!(got.tokens_unknown);
    }

    #[test]
    fn anthropic_partial_usage_only_input_tokens() {
        // Anthropic can return input_tokens with output still streaming.
        // SLICE 4 isn't called mid-stream (caller enforces end-of-stream)
        // but defensive parsing keeps the field populated either way.
        let body = br#"{"usage": {"input_tokens": 11}}"#;
        let got = extract_provider_usage(body, ProviderHint::Anthropic).expect("must parse");
        assert_eq!(got.input_tokens, Some(11));
        assert_eq!(got.output_tokens, None);
        assert!(!got.tokens_unknown);
    }

    #[test]
    fn provider_hint_derives_from_provider_str() {
        assert_eq!(
            ProviderHint::from_provider_str("openai"),
            ProviderHint::OpenAi
        );
        assert_eq!(
            ProviderHint::from_provider_str("azure_openai"),
            ProviderHint::OpenAi
        );
        assert_eq!(
            ProviderHint::from_provider_str("anthropic"),
            ProviderHint::Anthropic
        );
        assert_eq!(
            ProviderHint::from_provider_str("bedrock"),
            ProviderHint::Anthropic
        );
        assert_eq!(
            ProviderHint::from_provider_str("vertex"),
            ProviderHint::Unknown
        );
        assert_eq!(ProviderHint::from_provider_str(""), ProviderHint::Unknown);
    }
}
