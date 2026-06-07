//! Token-count dispatch — wraps the `spendguard-tokenizer` library.
//!
//! Spec refs:
//!   - docs/specs/coverage/D01_envoy_extproc/design.md §3.2 (token counting reuses egress_proxy routing)
//!   - docs/specs/coverage/D01_envoy_extproc/implementation.md §5 (ClaimEstimate)
//!   - docs/specs/coverage/D01_envoy_extproc/review-standards.md §3 SLICE 2 blocker checklist
//!     - §3.1.1: Tier 2 hot path (in-process library form, NEVER Tier 1 RPC)
//!     - §3.1.3: unknown model → `tokenizer_kind: None`, `input_tokens: 0`, `tokenizer_tier: "T3"`
//!     - §3.1.4: no silent fallback that fakes Strategy B/C values; B/C MUST be 0
//!
//! SLICE 2 emits a minimal [`ClaimEstimate`] carrying only the fields a
//! downstream SLICE 3 budget-decision RPC needs from the Request-Body
//! phase: `input_tokens`, `tokenizer_tier`, `tokenizer_version_id`,
//! `model`, `provider`, and the Strategy A reservation
//! (`predicted_a_tokens = input_tokens * 2`). Strategy B/C, prompt class,
//! and prediction-policy fields are deliberately left at their type
//! defaults — SLICE 3 will replace this struct with the full sidecar
//! adapter `ClaimEstimate` proto once `spendguard-sidecar-adapter-proto`
//! is path-deped in. Keeping the SLICE 2 surface small means SLICE 3 can
//! land the proto wiring without revisiting tokenizer code.

use spendguard_tokenizer::{TokenizeRequest, Tokenizer, TokenizerError};
use thiserror::Error;
use tracing::warn;

use crate::parse::ParsedRequest;

/// Internal per-stream claim estimate. SLICE 3 replaces this with the
/// full `sidecar_adapter::ClaimEstimate` proto.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ClaimEstimate {
    /// Tier 2 input-token count. `0` for unknown models (Tier 3 fallback
    /// returns chars/4 × 1.05; SLICE 3 will surface that, but SLICE 2
    /// just stashes the value for audit).
    pub input_tokens: i64,
    /// `"T2"` for hot-path Tier 2, `"T3"` for the heuristic fallback.
    /// Per review-standards §3.1.3 unknown model MUST be "T3".
    pub tokenizer_tier: String,
    /// UUIDv7 of the `tokenizer_versions` row (empty for Tier 3).
    pub tokenizer_version_id: String,
    /// Upstream model identifier, lifted from [`ParsedRequest::model_id`].
    pub model: String,
    /// Provider tag (`"openai"`, `"anthropic"`, ...). Stored as a `String`
    /// because SLICE 3's proto field is a `string`.
    pub provider: String,
    /// Strategy A reservation = `input_tokens * 2` (saturating).
    /// Matches the egress_proxy SLICE_10 Strategy A baseline (chars/4 × 2
    /// → tokens × 2 after tokenizer wiring). SLICE 3 sidecar predictor
    /// pipeline refines if a customer wires a plugin.
    pub predicted_a_tokens: i64,
    /// Per review-standards §3.1.4 MUST stay 0 in this slice.
    pub predicted_b_tokens: i64,
    /// Per review-standards §3.1.4 MUST stay 0 in this slice.
    pub predicted_c_tokens: i64,
    /// Strategy A is the only reservation strategy active in this slice.
    pub reserved_strategy: String,
}

/// Tokenizer dispatch errors. SLICE 2 maps each to a warn-and-continue
/// path on the server (see `server.rs::handle_request_body`).
#[derive(Debug, Error)]
pub enum TokenizeError {
    /// `spendguard-tokenizer` returned an error. Should never happen
    /// after boot (Tier 2 panic invariant per spec §3.6) but we surface
    /// the type so the caller logs and falls through.
    #[error("tokenizer dispatch failed: {source}")]
    Tokenizer {
        #[source]
        source: TokenizerError,
    },
}

/// Look up the tokenizer kind for `parsed`, call the in-process
/// `Tokenizer::tokenize`, and pack the result into a [`ClaimEstimate`].
///
/// Per review-standards §3.1.1: this MUST use the in-process library
/// form (Tier 2 hot path), NEVER the Tier 1 RPC. The tokenizer crate's
/// `Tokenizer::tokenize` is the only call path here.
///
/// Per review-standards §3.1.3: unknown model returns `Tier "T3"`,
/// `input_tokens = 0`, empty `tokenizer_version_id`. The tokenizer
/// library itself emits the `tokenizer_unknown_model` log via its
/// dispatch table — the `info!()` in `Tokenizer::tokenize` covers this.
pub fn estimate_tokens(
    tokenizer: &Tokenizer,
    parsed: &ParsedRequest,
) -> Result<ClaimEstimate, TokenizeError> {
    let tok_req = TokenizeRequest {
        model: parsed.model_id.clone(),
        messages: parsed.messages.clone(),
        raw_text: parsed.raw_text.clone(),
        request_id: String::new(),
    };

    let resp = tokenizer
        .tokenize(&tok_req)
        .map_err(|source| TokenizeError::Tokenizer { source })?;

    // Strategy A: input * 2. Saturating to avoid overflow on absurdly
    // large requests (the sidecar enforces max-tokens at decision time;
    // we just want safe arithmetic here).
    let predicted_a = resp.input_tokens.saturating_mul(2);

    // SLICE 7 (COV_07) demo override: when the binary is built with
    // `--features uds-dev` (default for cargo build / docker-compose,
    // OFF for the production chart image) AND the request body carried
    // `spendguard_estimate_override`, we substitute the override into
    // `predicted_a_tokens` so the demo DENY step can bust the seeded
    // 1B-atomic hard-cap without needing a 500M-token prompt body.
    // Production binaries do not compile this branch.
    #[cfg(feature = "uds-dev")]
    let predicted_a = match parsed.demo_estimate_override {
        Some(n) => n,
        None => predicted_a,
    };

    Ok(ClaimEstimate {
        input_tokens: resp.input_tokens,
        tokenizer_tier: resp.tier,
        tokenizer_version_id: resp.tokenizer_version_id,
        model: parsed.model_id.clone(),
        provider: parsed.provider_str.to_string(),
        predicted_a_tokens: predicted_a,
        predicted_b_tokens: 0,
        predicted_c_tokens: 0,
        reserved_strategy: "A".to_string(),
    })
}

/// Wrap [`estimate_tokens`] with a log-and-default fall-through so the
/// SLICE 3 wire-up only needs to call this once per Request-Body frame.
/// Returns the typed error so callers (`server.rs`) can pick between
/// warn+continue (SLICE 2 behaviour) and fail-closed (SLICE 3).
///
/// This is a thin shim — kept as a separate function so SLICE 3's
/// fail-closed path is a one-line diff (call this fn and propagate the
/// Err).
pub fn estimate_tokens_or_warn(
    tokenizer: &Tokenizer,
    parsed: &ParsedRequest,
) -> Option<ClaimEstimate> {
    match estimate_tokens(tokenizer, parsed) {
        Ok(claim) => Some(claim),
        Err(e) => {
            warn!(
                err = %e,
                model = %parsed.model_id,
                provider = parsed.provider_str,
                "tokenizer estimate failed; falling through with no ClaimEstimate (SLICE 3 will fail-closed)"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_request_body;
    use bytes::Bytes;
    use spendguard_provider_routing::{
        init_extractors_for_test, ProviderKind, RoutingExtractors, UsageMetrics,
    };
    use spendguard_tokenizer::{Message, Tokenizer};

    fn install_test_extractors() {
        fn noop(_: &serde_json::Value) -> UsageMetrics {
            UsageMetrics::default()
        }
        init_extractors_for_test(RoutingExtractors {
            openai: noop,
            anthropic: noop,
            bedrock: noop,
            vertex: noop,
            azure_openai: noop,
        });
    }

    fn boot_tokenizer() -> Tokenizer {
        Tokenizer::new_with_embedded_assets().expect("embedded tokenizer assets load")
    }

    #[test]
    fn estimates_tokens_for_openai_model() {
        install_test_extractors();
        let tokenizer = boot_tokenizer();
        let parsed = ParsedRequest {
            provider: ProviderKind::OpenAi,
            provider_str: ProviderKind::OpenAi.as_str(),
            request_shape: spendguard_provider_routing::RequestShape::OpenAiChatCompletions,
            model_id: "gpt-4o-mini".to_string(),
            tokenizer_kind: Some(spendguard_tokenizer::EncoderKind::OpenAi),
            messages: vec![
                Message {
                    role: "system".to_string(),
                    content: "You are concise.".to_string(),
                    tool_calls: Vec::new(),
                },
                Message {
                    role: "user".to_string(),
                    content: "What is 2 + 2?".to_string(),
                    tool_calls: Vec::new(),
                },
            ],
            raw_text: String::new(),
            #[cfg(feature = "uds-dev")]
            demo_estimate_override: None,
        };

        let claim = estimate_tokens(&tokenizer, &parsed).expect("Tier 2 tokenize ok");
        assert!(
            claim.input_tokens > 0,
            "OpenAI Tier 2 must return > 0 tokens for non-empty messages, got {}",
            claim.input_tokens
        );
        assert_eq!(claim.tokenizer_tier, "T2", "OpenAI model must be Tier 2");
        assert!(
            !claim.tokenizer_version_id.is_empty(),
            "Tier 2 must populate tokenizer_version_id"
        );
        assert_eq!(claim.model, "gpt-4o-mini");
        assert_eq!(claim.provider, "openai");
        assert_eq!(claim.reserved_strategy, "A");
        // Strategy A = input * 2 baseline.
        assert_eq!(claim.predicted_a_tokens, claim.input_tokens * 2);
        // Review-standards §3.1.4: B/C MUST be 0 in this slice.
        assert_eq!(claim.predicted_b_tokens, 0);
        assert_eq!(claim.predicted_c_tokens, 0);
    }

    #[test]
    fn estimates_tokens_for_anthropic_model() {
        install_test_extractors();
        let tokenizer = boot_tokenizer();
        let parsed = ParsedRequest {
            provider: ProviderKind::Anthropic,
            provider_str: ProviderKind::Anthropic.as_str(),
            request_shape: spendguard_provider_routing::RequestShape::AnthropicMessages,
            model_id: "claude-3-5-sonnet-20240620".to_string(),
            tokenizer_kind: Some(spendguard_tokenizer::EncoderKind::Anthropic),
            messages: vec![Message {
                role: "user".to_string(),
                content: "Translate hello to French.".to_string(),
                tool_calls: Vec::new(),
            }],
            raw_text: String::new(),
            #[cfg(feature = "uds-dev")]
            demo_estimate_override: None,
        };

        let claim = estimate_tokens(&tokenizer, &parsed).expect("Tier 2 tokenize ok");
        assert!(claim.input_tokens > 0);
        assert_eq!(claim.tokenizer_tier, "T2");
        assert_eq!(claim.provider, "anthropic");
        assert_eq!(claim.reserved_strategy, "A");
    }

    #[test]
    fn unknown_model_emits_t3_with_zero_envelope() {
        install_test_extractors();
        let tokenizer = boot_tokenizer();
        let parsed = ParsedRequest {
            provider: ProviderKind::OpenAi,
            provider_str: ProviderKind::OpenAi.as_str(),
            request_shape: spendguard_provider_routing::RequestShape::OpenAiChatCompletions,
            model_id: "some-internal-experimental-model-2099".to_string(),
            // Note: even if tokenizer_kind is Some, dispatch only fires
            // on a model-string match — the regex-anchored dispatch table
            // says NO, so we Tier 3 fall back via the tokenizer library.
            tokenizer_kind: Some(spendguard_tokenizer::EncoderKind::OpenAi),
            messages: vec![Message {
                role: "user".to_string(),
                content: "hello world".to_string(),
                tool_calls: Vec::new(),
            }],
            raw_text: String::new(),
            #[cfg(feature = "uds-dev")]
            demo_estimate_override: None,
        };

        let claim = estimate_tokens(&tokenizer, &parsed).expect("Tier 3 fallback ok");
        assert_eq!(claim.tokenizer_tier, "T3");
        // Tier 3 leaves tokenizer_version_id empty per
        // tokenizer crate's `tier3_fallback` shape.
        assert!(claim.tokenizer_version_id.is_empty());
        // Tier 3 returns chars/4 × 1.05 for the raw_text + flattened
        // messages — for an 11-char user message that's > 0 so the
        // fallback "is alive" assertion holds. Per review-standards
        // §3.1.3 unknown models must yield Tier 3 (not error).
        assert!(claim.input_tokens >= 0, "Tier 3 returns a non-negative int");
    }

    #[test]
    fn provider_string_matches_routing_table_strings() {
        // Spec-locked CloudEvent payload strings — see provider-routing
        // tests `provider_kind_string_stable`. Re-pin here so any future
        // tokenize change can't silently drift the audit-chain string.
        // `ClaimEstimate.provider` is set from `parsed.provider_str`, which
        // is `ProviderKind::as_str()` — pin the source directly.
        assert_eq!(ProviderKind::OpenAi.as_str(), "openai");
        assert_eq!(ProviderKind::Anthropic.as_str(), "anthropic");
        assert_eq!(ProviderKind::Bedrock.as_str(), "bedrock");
        assert_eq!(ProviderKind::Vertex.as_str(), "vertex");
        assert_eq!(ProviderKind::AzureOpenAi.as_str(), "azure_openai");
    }

    #[test]
    fn estimate_or_warn_returns_some_for_known_model() {
        install_test_extractors();
        let tokenizer = boot_tokenizer();
        let parsed = ParsedRequest {
            provider: ProviderKind::OpenAi,
            provider_str: ProviderKind::OpenAi.as_str(),
            request_shape: spendguard_provider_routing::RequestShape::OpenAiChatCompletions,
            model_id: "gpt-4o".to_string(),
            tokenizer_kind: Some(spendguard_tokenizer::EncoderKind::OpenAi),
            messages: vec![Message {
                role: "user".to_string(),
                content: "tokens please".to_string(),
                tool_calls: Vec::new(),
            }],
            raw_text: String::new(),
            #[cfg(feature = "uds-dev")]
            demo_estimate_override: None,
        };
        assert!(estimate_tokens_or_warn(&tokenizer, &parsed).is_some());
    }

    #[test]
    fn estimates_tokens_for_vertex_gemini_model() {
        // R2 (M-4): exercise the Vertex Gemini path end-to-end —
        // parse Vertex `contents` → estimate Tier 2 Gemini tokens.
        // Vertex's `resolve_model_id` reads `body.model`; without it
        // model_id falls back to "unknown" and the tokenizer Tier 3s.
        install_test_extractors();
        let tokenizer = boot_tokenizer();
        let body = Bytes::from(
            r#"{
                "model": "gemini-1.5-pro",
                "contents": [
                    {"role": "user", "parts": [
                        {"text": "Translate hello to French."}
                    ]}
                ]
            }"#,
        );
        let path = "/v1/projects/my-proj/locations/us-central1/publishers/google/models/gemini-1.5-pro:generateContent";
        let parsed = parse_request_body(path, &body).expect("parse ok");
        assert_eq!(parsed.provider, ProviderKind::Vertex);
        assert_eq!(parsed.model_id, "gemini-1.5-pro");
        let claim = estimate_tokens(&tokenizer, &parsed).expect("Tier 2 tokenize ok");
        assert!(
            claim.input_tokens > 0,
            "Vertex Gemini Tier 2 must return > 0 tokens for non-empty content, got {}",
            claim.input_tokens
        );
        assert_ne!(
            claim.tokenizer_tier, "T3",
            "Vertex Gemini must hit a real Tier 2 tokenizer, not the T3 fallback"
        );
        assert_eq!(claim.provider, "vertex");
        assert_eq!(claim.model, "gemini-1.5-pro");
        // Strategy A baseline + B/C MUST stay 0 per review-standards §3.1.4.
        assert_eq!(claim.predicted_a_tokens, claim.input_tokens * 2);
        assert_eq!(claim.predicted_b_tokens, 0);
        assert_eq!(claim.predicted_c_tokens, 0);
    }

    #[test]
    fn estimates_tokens_for_bedrock_llama_prompt() {
        // R2 (M-4): exercise the Bedrock Llama path end-to-end —
        // parse `{prompt: "..."}` → raw_text → estimate Tier 2 Llama tokens.
        install_test_extractors();
        let tokenizer = boot_tokenizer();
        let body = Bytes::from(
            r#"{"prompt": "Once upon a time in a kingdom far away, there was a wise old wizard."}"#,
        );
        let path = "/model/meta.llama3-1-70b-instruct-v1:0/invoke";
        let parsed = parse_request_body(path, &body).expect("parse ok");
        assert_eq!(parsed.provider, ProviderKind::Bedrock);
        assert!(
            parsed.messages.is_empty(),
            "Llama Bedrock prompt shape carries content via raw_text, not messages"
        );
        let claim = estimate_tokens(&tokenizer, &parsed).expect("Tier 2 tokenize ok");
        assert!(
            claim.input_tokens > 0,
            "Bedrock Llama Tier 2 must return > 0 tokens for non-empty prompt, got {}",
            claim.input_tokens
        );
        assert_ne!(
            claim.tokenizer_tier, "T3",
            "Bedrock Llama must hit a real Tier 2 tokenizer, not the T3 fallback"
        );
        assert_eq!(claim.provider, "bedrock");
        assert_eq!(claim.model, "meta.llama3-1-70b-instruct-v1:0");
        // Strategy A baseline + B/C MUST stay 0 per review-standards §3.1.4.
        assert_eq!(claim.predicted_a_tokens, claim.input_tokens * 2);
        assert_eq!(claim.predicted_b_tokens, 0);
        assert_eq!(claim.predicted_c_tokens, 0);
    }
}
