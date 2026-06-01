//! Google Gemini 1.5 / 2.0 BPE encoder (community approximation).
//!
//! Spec refs:
//!   - `tokenizer-service-spec-v1alpha1.md` §3.1 (dispatch table entry
//!     `gemini-1.5-bpe`), §4.2 (drift threshold 1%), §7.1 (vendored
//!     source + license).
//!
//! ## Important — community approximation, not vendor-official
//!
//! Google's official Gemini tokenizer is exposed only as a REST
//! endpoint: `POST /v1/models/{model}:countTokens`. There is no
//! vendor-published BPE merges file we can vendor for Tier 2 use.
//!
//! Instead we use the open-source Gemma tokenizer (released by
//! Google AI under Apache 2.0) as the closest publicly available
//! approximation.
//!
//! ### R2 M5 honest disclosure (2026-05-30)
//!
//! The R1 doc-comment claimed `< 1% delta per Google's published
//! parity table (Gemma 1.0/2.0 release notes)`. This was an
//! unsupported assertion — Google does NOT publish a Gemma-vs-Gemini
//! parity table. The actual gap between the Gemma vocab and the
//! Gemini `countTokens` semantics is **unknown** until SLICE_05
//! shadow worker measures it in production.
//!
//! Spec §4.2 prescribes a 0.01 (1%) drift threshold for the Gemini
//! kind to absorb the approximation gap. SLICE_05 ships the shadow
//! worker that calls the actual Gemini `countTokens` API at 1%
//! sampling and emits drift_alert events when the cumulative gap
//! exceeds threshold; if production data shows the gap is wider than
//! the spec threshold, we will:
//!   1. Tighten the spec threshold (operator response).
//!   2. Switch Gemini to a Tier 1-only strategy if Tier 2
//!      approximation cannot meet the SpendGuard accuracy promise.
//!
//! For now we ship the Gemma approximation as Tier 2 source of truth;
//! reservation accuracy is bounded by the 1% threshold and any
//! drift becomes operator-visible via the shadow worker. The §4.2
//! Gemini row rationale carries the same disclosure for cross-spec
//! consistency.
//!
//! ## Envelope rules (SLICE_04 R2 M3 amendment)
//!
//! Gemini's API takes a `contents` array where role is a structured
//! field, NOT a token in the prompt. There is no per-message envelope
//! to tokenize. Per spec §3.4 amendment:
//!
//! ```text
//! ChatEnvelope { per_message: 0, per_turn_boundary: 0, reply_priming: 0 }
//! ```
//!
//! Gemini also has no BOS token in the Gemma vocab (Google's official
//! `countTokens` API does not include one):
//!
//! ```text
//! bos_token_count() = 0
//! ```
//!
//! ## Vendoring
//!
//! Source URL (pinned in `LICENSE_NOTICES.md`):
//!   https://huggingface.co/Xenova/gemma-tokenizer
//!
//! License: MIT (mirror) / Apache 2.0 (Google upstream).

use crate::encoders::{ChatEnvelope, EncodeResult, Encoder, EncoderKind};
use crate::error::TokenizerError;
use crate::versions::GEMINI_15_VERSION_ID;
use crate::{TokenizeRequest, ToolCall};
use sha2::{Digest, Sha256};
use tokenizers::Tokenizer;

const ASSET_BYTES: &[u8] = include_bytes!("../../data/gemini-1.5/tokenizer.json");

const CROSS_CHECK_FIXTURE: &str = "spendguard-cross-check-fixture-v1alpha1";

/// Expected token vector when [`CROSS_CHECK_FIXTURE`] is encoded with
/// the pinned Xenova Gemma tokenizer.json revision.
const EXPECTED_GEMINI_FIXTURE: &[u32] = &[
    120479, 14413, 235290, 16100, 235290, 3534, 235290, 35693, 235290, 235272, 235274, 4705, 235274,
];

pub struct GeminiEncoder {
    tokenizer: Tokenizer,
}

impl GeminiEncoder {
    pub fn new() -> Result<Self, TokenizerError> {
        verify_asset_sha256("gemini-1.5", ASSET_BYTES, crate::asset_sha256::GEMINI_15)?;

        let tokenizer =
            Tokenizer::from_bytes(ASSET_BYTES).map_err(|e| TokenizerError::AssetLoadFailed {
                encoder: "gemini-1.5",
                message: format!("Tokenizer::from_bytes failed: {e}"),
            })?;

        cross_check(&tokenizer, EXPECTED_GEMINI_FIXTURE)?;

        Ok(Self { tokenizer })
    }
}

/// Gemini chat envelope per spec §3.4 (R2 M3): role is a structured
/// API field, not a prompt token. No per-message overhead.
const GEMINI_ENVELOPE: ChatEnvelope = ChatEnvelope {
    per_message: 0,
    per_turn_boundary: 0,
    reply_priming: 0,
};

/// Gemini BOS per spec §3.4 (R2 M4): Gemma vocab has no BOS in the
/// `countTokens` semantics. SLICE_05 shadow worker measures residual.
const GEMINI_BOS_COUNT: usize = 0;

impl Encoder for GeminiEncoder {
    fn kind(&self) -> EncoderKind {
        EncoderKind::Gemini
    }

    fn version_id(&self) -> &'static str {
        GEMINI_15_VERSION_ID
    }

    fn encoder_name(&self) -> &'static str {
        "gemini-1.5-bpe"
    }

    fn envelope_overhead(&self) -> ChatEnvelope {
        GEMINI_ENVELOPE
    }

    fn bos_token_count(&self) -> usize {
        GEMINI_BOS_COUNT
    }

    fn count_tokens_request(&self, req: &TokenizeRequest) -> Result<EncodeResult, TokenizerError> {
        let input_tokens = count_tokens_gemini(&self.tokenizer, req)?;
        Ok(EncodeResult {
            input_tokens,
            tokenizer_version_id: GEMINI_15_VERSION_ID,
            kind: EncoderKind::Gemini,
        })
    }
}

fn count_tokens_gemini(
    tokenizer: &Tokenizer,
    req: &TokenizeRequest,
) -> Result<usize, TokenizerError> {
    let mut total: usize = 0;

    if !req.messages.is_empty() {
        let env = GEMINI_ENVELOPE;
        for msg in &req.messages {
            total += env.per_message;
            total += env.per_turn_boundary;
            total += encode_count(tokenizer, &msg.role)?;
            total += encode_count(tokenizer, &msg.content)?;
            for tc in &msg.tool_calls {
                total += tool_call_tokens(tokenizer, tc)?;
            }
        }
        total += env.reply_priming;
    }

    if !req.raw_text.is_empty() {
        total += encode_count(tokenizer, &req.raw_text)?;
        // R2 M4: Gemini BOS=0; no BOS added.
        total += GEMINI_BOS_COUNT;
    }

    Ok(total)
}

fn tool_call_tokens(tokenizer: &Tokenizer, tc: &ToolCall) -> Result<usize, TokenizerError> {
    const TOOL_CALL_OVERHEAD: usize = 1;
    Ok(TOOL_CALL_OVERHEAD
        + encode_count(tokenizer, &tc.name)?
        + encode_count(tokenizer, &tc.arguments_json)?)
}

fn encode_count(tokenizer: &Tokenizer, text: &str) -> Result<usize, TokenizerError> {
    if text.is_empty() {
        return Ok(0);
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tokenizer.encode(text, false)
    }));
    match result {
        Ok(Ok(enc)) => Ok(enc.get_ids().len()),
        Ok(Err(e)) => Err(TokenizerError::EncoderInternal {
            kind: "gemini-1.5-bpe",
            message: format!("tokenizers encode error: {e}"),
        }),
        Err(_) => Err(TokenizerError::EncoderInternal {
            kind: "gemini-1.5-bpe",
            message: "tokenizers encode panicked on input".to_string(),
        }),
    }
}

fn verify_asset_sha256(
    encoder: &'static str,
    bytes: &[u8],
    expected: &'static str,
) -> Result<(), TokenizerError> {
    use subtle::ConstantTimeEq;

    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let actual_bytes: [u8; 32] = hasher.finalize().into();
    let actual_hex = hex::encode(actual_bytes);

    let expected_vec = match hex::decode(expected) {
        Ok(v) if v.len() == 32 => v,
        _ => {
            return Err(TokenizerError::AssetSignatureMismatch {
                encoder,
                expected,
                actual: format!("expected-const-malformed (got {actual_hex})"),
            });
        }
    };

    if actual_bytes.as_slice().ct_eq(&expected_vec).into() {
        Ok(())
    } else {
        Err(TokenizerError::AssetSignatureMismatch {
            encoder,
            expected,
            actual: actual_hex,
        })
    }
}

fn cross_check(tokenizer: &Tokenizer, expected: &[u32]) -> Result<(), TokenizerError> {
    let enc = tokenizer.encode(CROSS_CHECK_FIXTURE, false).map_err(|e| {
        TokenizerError::AssetSignatureMismatch {
            encoder: "gemini-1.5",
            expected: "cross_check_fixture_vector",
            actual: format!("fixture-encode-error: {e}"),
        }
    })?;
    let actual = enc.get_ids();
    if actual != expected {
        let expected_summary: String = expected
            .iter()
            .take(6)
            .map(|t| t.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let actual_summary: String = actual
            .iter()
            .take(6)
            .map(|t| t.to_string())
            .collect::<Vec<_>>()
            .join(",");
        return Err(TokenizerError::AssetSignatureMismatch {
            encoder: "gemini-1.5",
            expected: "cross_check_fixture_vector",
            actual: format!(
                "fixture-vector-mismatch: expected first 6 tokens=[{expected_summary}], got=[{actual_summary}]"
            ),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Message;

    #[test]
    fn gemini_loads_and_passes_integrity() {
        let _enc = GeminiEncoder::new().expect("Gemini encoder boots clean");
    }

    #[test]
    fn gemini_rejects_tampered_asset_bytes() {
        let mut tampered = ASSET_BYTES.to_vec();
        tampered[2048] ^= 0x01;

        let err = verify_asset_sha256("gemini-1.5", &tampered, crate::asset_sha256::GEMINI_15)
            .expect_err("tamper must fail");

        assert!(matches!(
            err,
            TokenizerError::AssetSignatureMismatch {
                encoder: "gemini-1.5",
                ..
            }
        ));
    }

    #[test]
    fn gemini_kind_and_version_id() {
        let enc = GeminiEncoder::new().expect("boot");
        assert_eq!(enc.kind(), EncoderKind::Gemini);
        assert_eq!(enc.version_id(), GEMINI_15_VERSION_ID);
        assert_eq!(enc.encoder_name(), "gemini-1.5-bpe");
    }

    #[test]
    fn gemini_encodes_hello_world_to_2_tokens_no_bos() {
        // R2 M4: Gemini BOS=0 so raw_text count unchanged from R1.
        let enc = GeminiEncoder::new().expect("boot");
        let req = TokenizeRequest {
            model: "gemini-1.5-flash".to_string(),
            raw_text: "hello world".to_string(),
            ..Default::default()
        };
        let r = enc.count_tokens_request(&req).unwrap();
        assert_eq!(r.input_tokens, 2);
        assert_eq!(r.kind, EncoderKind::Gemini);
    }

    #[test]
    fn gemini_chat_envelope_only_adds_role_token() {
        // R2 M3: Gemini envelope is all-zero (role is a structured API
        // field, not a prompt token). The only delta between raw_text
        // and 1-message chat is the role string being tokenized.
        let enc = GeminiEncoder::new().expect("boot");
        let raw_req = TokenizeRequest {
            model: "gemini-1.5-flash".to_string(),
            raw_text: "hello".to_string(),
            ..Default::default()
        };
        let chat_req = TokenizeRequest {
            model: "gemini-1.5-flash".to_string(),
            messages: vec![Message {
                role: "user".to_string(),
                content: "hello".to_string(),
                tool_calls: vec![],
            }],
            ..Default::default()
        };
        let raw_n = enc.count_tokens_request(&raw_req).unwrap().input_tokens;
        let chat_n = enc.count_tokens_request(&chat_req).unwrap().input_tokens;
        // Both contain "hello" content; chat adds "user" role tokens.
        // The role string is non-empty so chat_n must be > raw_n by at
        // least 1 (role_tokens), with no envelope overhead added.
        assert!(
            chat_n > raw_n,
            "chat must exceed raw by role-token count; got raw={raw_n} chat={chat_n}"
        );
    }

    #[test]
    fn gemini_envelope_and_bos_constants_match_trait() {
        let enc = GeminiEncoder::new().expect("boot");
        let env = enc.envelope_overhead();
        assert_eq!(env.per_message, 0);
        assert_eq!(env.per_turn_boundary, 0);
        assert_eq!(env.reply_priming, 0);
        assert_eq!(enc.bos_token_count(), 0);
    }

    #[test]
    fn gemini_empty_request_is_0() {
        let enc = GeminiEncoder::new().expect("boot");
        let r = enc
            .count_tokens_request(&TokenizeRequest::default())
            .unwrap();
        assert_eq!(r.input_tokens, 0);
    }

    #[test]
    fn gemini_drift_threshold_documents_approximation_gap() {
        // Spec §4.2 — Gemini approximation gap is bounded by the
        // 0.01 (1%) drift threshold defined on the EncoderKind enum.
        // This test is a structural reminder for future maintainers:
        // if the threshold widens, document why in this file's
        // module-level comment.
        assert_eq!(EncoderKind::Gemini.drift_threshold(), 0.01);
    }
}
