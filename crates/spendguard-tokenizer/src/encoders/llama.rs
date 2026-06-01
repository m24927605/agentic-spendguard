//! Meta Llama 3.1 tokenizer (BPE-style via SentencePiece config).
//!
//! Spec refs:
//!   - `tokenizer-service-spec-v1alpha1.md` §3.1 (dispatch table entry
//!     `llama-sentencepiece`), §4.2 (drift threshold 0.5%), §7.1
//!     (vendored source + license).
//!
//! ## Implementation note — "SentencePiece" terminology
//!
//! Meta's Llama 3.1 ships with a `tokenizer.json` that the
//! HuggingFace `tokenizers` crate parses as a tiktoken-style BPE
//! configuration (the underlying merges are SentencePiece-derived but
//! the runtime config exposes them as BPE for compatibility with the
//! `transformers` Python loader). Our spec §3.1 calls the kind
//! `SENTENCEPIECE_LLAMA` for the kind enum / SQL CHECK string — we
//! preserve that name even though the runtime path is a BPE encode.
//!
//! The `tokenizer.json` is binary-equivalent to what Meta ships in
//! `tokenizer.model` (SentencePiece protobuf); the format conversion
//! happens upstream at Hugging Face's mirror, and the encode output
//! is byte-exact against `meta-llama/Llama-3.1-8B`'s `transformers`
//! API. SLICE_05 shadow worker will validate via Bedrock count_tokens
//! sampling.
//!
//! Spec §4.2 0.005 (0.5%) drift threshold is the tightest of any
//! SLICE_04 encoder because SentencePiece is configuration-precise —
//! any drift > 0.5% indicates either a vendor model bump (legitimate
//! refresh trigger) or a config corruption (refuse-to-start).
//!
//! ## Vendoring
//!
//! Source URL (pinned in `LICENSE_NOTICES.md`):
//!   https://huggingface.co/Xenova/Meta-Llama-3.1-Tokenizer
//!
//! License: MIT (mirror) / Llama 3.1 Community License (Meta).
//!
//! ## Envelope rules (SLICE_04 R2 M3 amendment)
//!
//! Llama 3.1 chat uses the full header template
//! `<|begin_of_text|><|start_header_id|>{role}<|end_header_id|>\n\n
//! {content}<|eot_id|>` (≈ 5 tokens per turn for the header markers).
//! Per spec §3.4 amendment:
//!
//! ```text
//! ChatEnvelope { per_message: 5, per_turn_boundary: 0, reply_priming: 0 }
//! ```
//!
//! ## BOS token (SLICE_04 R2 M4 amendment)
//!
//! Bedrock invokeModel prepends `<|begin_of_text|>` (1 token).
//! `bos_token_count() = 1`.

use crate::encoders::{ChatEnvelope, EncodeResult, Encoder, EncoderKind};
use crate::error::TokenizerError;
use crate::versions::LLAMA_31_VERSION_ID;
use crate::{TokenizeRequest, ToolCall};
use sha2::{Digest, Sha256};
use tokenizers::Tokenizer;

const ASSET_BYTES: &[u8] = include_bytes!("../../data/llama-3.1/tokenizer.json");

const CROSS_CHECK_FIXTURE: &str = "spendguard-llama-cross-check-v1alpha1 你好 llama-ñ";

/// Expected token vector when [`CROSS_CHECK_FIXTURE`] is encoded with
/// the pinned Xenova Meta-Llama-3.1-Tokenizer revision.
///
/// POST_GA_03 / #119: the fixture intentionally includes non-ASCII
/// bytes so it cannot accidentally match OpenAI cl100k's ASCII-subset
/// vector and mask a wrong asset.
const EXPECTED_LLAMA_FIXTURE: &[u32] = &[
    2203, 408, 27190, 12, 657, 3105, 77529, 16313, 8437, 16, 7288, 16, 118195, 53901, 94776, 12,
    5771,
];

pub struct LlamaEncoder {
    tokenizer: Tokenizer,
}

impl LlamaEncoder {
    pub fn new() -> Result<Self, TokenizerError> {
        verify_asset_sha256("llama-3.1", ASSET_BYTES, crate::asset_sha256::LLAMA_31)?;

        let tokenizer =
            Tokenizer::from_bytes(ASSET_BYTES).map_err(|e| TokenizerError::AssetLoadFailed {
                encoder: "llama-3.1",
                message: format!("Tokenizer::from_bytes failed: {e}"),
            })?;

        cross_check(&tokenizer, EXPECTED_LLAMA_FIXTURE)?;

        Ok(Self { tokenizer })
    }
}

/// Llama chat envelope per spec §3.4 (R2 M3): 5 tokens per message
/// for the `<|start_header_id|>{role}<|end_header_id|>\n\n{content}
/// <|eot_id|>` template header markers (excludes role + content which
/// are encoded separately).
const LLAMA_ENVELOPE: ChatEnvelope = ChatEnvelope {
    per_message: 5,
    per_turn_boundary: 0,
    reply_priming: 0,
};

/// Llama BOS per spec §3.4 (R2 M4): Bedrock invokeModel prepends
/// `<|begin_of_text|>` to the prompt.
const LLAMA_BOS_COUNT: usize = 1;

impl Encoder for LlamaEncoder {
    fn kind(&self) -> EncoderKind {
        EncoderKind::Llama
    }

    fn version_id(&self) -> &'static str {
        LLAMA_31_VERSION_ID
    }

    fn encoder_name(&self) -> &'static str {
        "llama-sentencepiece"
    }

    fn envelope_overhead(&self) -> ChatEnvelope {
        LLAMA_ENVELOPE
    }

    fn bos_token_count(&self) -> usize {
        LLAMA_BOS_COUNT
    }

    fn count_tokens_request(&self, req: &TokenizeRequest) -> Result<EncodeResult, TokenizerError> {
        let input_tokens = count_tokens_llama(&self.tokenizer, req)?;
        Ok(EncodeResult {
            input_tokens,
            tokenizer_version_id: LLAMA_31_VERSION_ID,
            kind: EncoderKind::Llama,
        })
    }
}

fn count_tokens_llama(
    tokenizer: &Tokenizer,
    req: &TokenizeRequest,
) -> Result<usize, TokenizerError> {
    let mut total: usize = 0;

    if !req.messages.is_empty() {
        let env = LLAMA_ENVELOPE;
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
        // R2 M4: BOS prepended by Bedrock invokeModel.
        total += LLAMA_BOS_COUNT;
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
            kind: "llama-sentencepiece",
            message: format!("tokenizers encode error: {e}"),
        }),
        Err(_) => Err(TokenizerError::EncoderInternal {
            kind: "llama-sentencepiece",
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
            encoder: "llama-3.1",
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
            encoder: "llama-3.1",
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
    fn llama_loads_and_passes_integrity() {
        let _enc = LlamaEncoder::new().expect("Llama encoder boots clean");
    }

    #[test]
    fn llama_rejects_tampered_asset_bytes() {
        let mut tampered = ASSET_BYTES.to_vec();
        tampered[2048] ^= 0x01;

        let err = verify_asset_sha256("llama-3.1", &tampered, crate::asset_sha256::LLAMA_31)
            .expect_err("tamper must fail");

        assert!(matches!(
            err,
            TokenizerError::AssetSignatureMismatch {
                encoder: "llama-3.1",
                ..
            }
        ));
    }

    #[test]
    fn llama_kind_and_version_id() {
        let enc = LlamaEncoder::new().expect("boot");
        assert_eq!(enc.kind(), EncoderKind::Llama);
        assert_eq!(enc.version_id(), LLAMA_31_VERSION_ID);
        assert_eq!(enc.encoder_name(), "llama-sentencepiece");
    }

    #[test]
    fn llama_encodes_hello_world_raw_with_bos() {
        // R2 M4: BOS=1 added on raw_text. "hello world" = 2 + 1 = 3.
        let enc = LlamaEncoder::new().expect("boot");
        let req = TokenizeRequest {
            model: "meta.llama3-1-8b-instruct-v1:0".to_string(),
            raw_text: "hello world".to_string(),
            ..Default::default()
        };
        let r = enc.count_tokens_request(&req).unwrap();
        assert_eq!(r.input_tokens, 3, "expected 2 vocab + 1 BOS = 3");
        assert_eq!(r.kind, EncoderKind::Llama);
    }

    #[test]
    fn llama_chat_envelope_5_per_message() {
        // R2 M3: Llama envelope = `per_message=5, per_turn_boundary=0,
        // reply_priming=0`. For 1-message chat ("user" / "hello"):
        //   chat_n = 5 + role_tokens + content_tokens
        //   raw_n  = content_tokens + 1 (BOS)
        // Therefore chat_n - raw_n = 5 + role_tokens - 1.
        let enc = LlamaEncoder::new().expect("boot");
        let raw_req = TokenizeRequest {
            model: "meta.llama3-1-8b-instruct-v1:0".to_string(),
            raw_text: "hello".to_string(),
            ..Default::default()
        };
        let chat_req = TokenizeRequest {
            model: "meta.llama3-1-8b-instruct-v1:0".to_string(),
            messages: vec![Message {
                role: "user".to_string(),
                content: "hello".to_string(),
                tool_calls: vec![],
            }],
            ..Default::default()
        };
        let raw_n = enc.count_tokens_request(&raw_req).unwrap().input_tokens;
        let chat_n = enc.count_tokens_request(&chat_req).unwrap().input_tokens;
        assert!(chat_n > raw_n);
        assert!(
            chat_n >= raw_n + 5,
            "chat-raw delta must include 5-token Llama per-message; got raw={raw_n} chat={chat_n}"
        );
    }

    #[test]
    fn llama_envelope_and_bos_constants_match_trait() {
        let enc = LlamaEncoder::new().expect("boot");
        let env = enc.envelope_overhead();
        assert_eq!(env.per_message, 5);
        assert_eq!(env.per_turn_boundary, 0);
        assert_eq!(env.reply_priming, 0);
        assert_eq!(enc.bos_token_count(), 1);
    }

    #[test]
    fn llama_empty_request_is_0() {
        let enc = LlamaEncoder::new().expect("boot");
        let r = enc
            .count_tokens_request(&TokenizeRequest::default())
            .unwrap();
        assert_eq!(r.input_tokens, 0);
    }

    #[test]
    fn llama_drift_threshold_is_tightest_at_005() {
        // Spec §4.2 — SentencePiece is configuration-precise, so 0.005
        // (0.5%) is the tightest threshold of any SLICE_04 encoder.
        assert_eq!(EncoderKind::Llama.drift_threshold(), 0.005);
    }
}
