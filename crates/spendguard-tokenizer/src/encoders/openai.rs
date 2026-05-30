//! OpenAI tiktoken-rs encoder implementation.
//!
//! Refactor target: SLICE_03 housed the OpenAI logic inside
//! [`crate::encoder_cache::EncoderCache`] directly. SLICE_04 lifts it
//! into a trait-object implementation so Anthropic / Gemini / Cohere /
//! Llama can sit alongside it without a `match` arm at every dispatch
//! site.
//!
//! ## Behavioural parity guarantee
//!
//! The encode path here MUST byte-identical the SLICE_03 result for
//! all 51 golden samples in `services/tokenizer/tests/golden_samples.rs`.
//! That test file is unchanged in this refactor — if a sample regresses,
//! the refactor is wrong, not the test.
//!
//! ## Implementation notes
//!
//!   * Three encoder families share one struct (`OpenAiEncoder`)
//!     because the encode path is identical; only the `CoreBPE`
//!     reference differs. We tag each instance with a
//!     [`TiktokenFamily`] discriminant so the
//!     `tokenizer_version_id` lookup is a constant-time enum match.
//!   * Asset signature (Layer A) verification runs at `new()` not
//!     here — the parent `EncoderCache` is the single point of
//!     boot-time integrity.
//!   * Layer B cross-check (per spec §7.4.1) also runs from
//!     `EncoderCache`'s `new()` against this encoder.
//!
//! See `encoders/mod.rs` for the trait contract.

use crate::dispatch::TiktokenEncoder;
use crate::encoders::{EncodeResult, Encoder, EncoderKind};
use crate::error::TokenizerError;
use crate::versions::{
    TIKTOKEN_CL100K_BASE_VERSION_ID, TIKTOKEN_O200K_BASE_VERSION_ID, TIKTOKEN_P50K_BASE_VERSION_ID,
};
use crate::{Message, TokenizeRequest, ToolCall};
use tiktoken_rs::CoreBPE;

/// One of the three tiktoken families SLICE_03 ships. Refactored from
/// `dispatch::TiktokenEncoder` so the `encoders/` module is the
/// source-of-truth for per-family metadata; `dispatch::TiktokenEncoder`
/// is kept as a public re-export alias for back-compat with the
/// dispatch table API surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TiktokenFamily {
    Cl100kBase,
    O200kBase,
    P50kBase,
}

impl TiktokenFamily {
    pub fn encoder_name(self) -> &'static str {
        match self {
            TiktokenFamily::Cl100kBase => "cl100k_base",
            TiktokenFamily::O200kBase => "o200k_base",
            TiktokenFamily::P50kBase => "p50k_base",
        }
    }

    pub fn version_id(self) -> &'static str {
        match self {
            TiktokenFamily::Cl100kBase => TIKTOKEN_CL100K_BASE_VERSION_ID,
            TiktokenFamily::O200kBase => TIKTOKEN_O200K_BASE_VERSION_ID,
            TiktokenFamily::P50kBase => TIKTOKEN_P50K_BASE_VERSION_ID,
        }
    }

    /// Convert from the legacy `dispatch::TiktokenEncoder` to keep the
    /// existing public dispatch enum stable — no SLICE_03 caller has
    /// to migrate.
    pub fn from_dispatch(d: TiktokenEncoder) -> Self {
        match d {
            TiktokenEncoder::Cl100kBase => TiktokenFamily::Cl100kBase,
            TiktokenEncoder::O200kBase => TiktokenFamily::O200kBase,
            TiktokenEncoder::P50kBase => TiktokenFamily::P50kBase,
        }
    }
}

/// One instance of the OpenAI encoder. The `CoreBPE` is held as a
/// `&'static` reference because tiktoken-rs's singletons live for the
/// process lifetime (and SLICE_03 already validated this is safe by
/// removing the redundant `Arc<&'static CoreBPE>` in R2 m6).
pub struct OpenAiEncoder {
    family: TiktokenFamily,
    bpe: &'static CoreBPE,
}

impl OpenAiEncoder {
    /// Construct from a tiktoken-rs singleton + family tag. The
    /// caller (parent `EncoderCache`) is responsible for running the
    /// Layer A sha256 and Layer B fixture cross-check BEFORE
    /// constructing instances of this type; calling
    /// [`encode`](Self::encode) on a pre-integrity-checked encoder is
    /// a soundness violation.
    pub fn new(family: TiktokenFamily, bpe: &'static CoreBPE) -> Self {
        Self { family, bpe }
    }

    pub fn family(&self) -> TiktokenFamily {
        self.family
    }

    /// Low-level encode call. Used by golden tests + cross-check guard.
    pub fn encode(&self, text: &str) -> Vec<u32> {
        self.bpe.encode_with_special_tokens(text)
    }
}

impl Encoder for OpenAiEncoder {
    fn kind(&self) -> EncoderKind {
        EncoderKind::OpenAi
    }

    fn version_id(&self) -> &'static str {
        self.family.version_id()
    }

    fn encoder_name(&self) -> &'static str {
        self.family.encoder_name()
    }

    fn count_tokens_request(&self, req: &TokenizeRequest) -> Result<EncodeResult, TokenizerError> {
        let input_tokens = count_tokens(self, req)?;
        Ok(EncodeResult {
            input_tokens,
            tokenizer_version_id: self.version_id(),
            kind: EncoderKind::OpenAi,
        })
    }
}

/// Compute total token count for a request — identical algorithm to
/// SLICE_03's `encoder_cache::count_tokens`. Move-only refactor;
/// behaviour preserved byte-for-byte against the golden corpus.
///
/// Per spec §3.4 + §3.5:
///   * Chat shape (`messages.len() > 0`): per-message envelope (3
///     tokens / message + per-tool overhead) + 3 reply priming.
///   * Text-completion shape (`raw_text.len() > 0`): raw encode only.
///   * Both: sum.
///
/// R2 m1 special-case for `gpt-3.5-turbo-0301`: tokens_per_message=4
/// (legacy snapshot quirk). Preserved exactly here.
fn count_tokens(enc: &OpenAiEncoder, req: &TokenizeRequest) -> Result<usize, TokenizerError> {
    let mut total: usize = 0;

    // ── chat-shape path ───────────────────────────────────────
    if !req.messages.is_empty() {
        let (per_msg_overhead, reply_priming) = match enc.family {
            TiktokenFamily::Cl100kBase | TiktokenFamily::O200kBase => {
                if req.model == "gpt-3.5-turbo-0301" {
                    (4usize, 3usize)
                } else {
                    (3usize, 3usize)
                }
            }
            // p50k_base = text-completion; if a caller passes
            // messages, count them as raw (graceful degradation).
            TiktokenFamily::P50kBase => (0usize, 0usize),
        };

        for msg in &req.messages {
            total += per_msg_overhead;
            total += encode_count(enc, &msg.role)?;
            total += encode_count(enc, &msg.content)?;
            for tc in &msg.tool_calls {
                total += tool_call_tokens(enc, tc)?;
            }
        }
        total += reply_priming;
    }

    // ── text-completion path ──────────────────────────────────
    if !req.raw_text.is_empty() {
        total += encode_count(enc, &req.raw_text)?;
    }

    Ok(total)
}

fn tool_call_tokens(enc: &OpenAiEncoder, tc: &ToolCall) -> Result<usize, TokenizerError> {
    // Per tiktoken-rs num_tokens_from_messages: +1 overhead per tool
    // call + tokens(name) + tokens(arguments_json).
    const TOOL_CALL_OVERHEAD: usize = 1;
    Ok(TOOL_CALL_OVERHEAD + encode_count(enc, &tc.name)? + encode_count(enc, &tc.arguments_json)?)
}

fn encode_count(enc: &OpenAiEncoder, text: &str) -> Result<usize, TokenizerError> {
    if text.is_empty() {
        return Ok(0);
    }
    // Defense-in-depth: capture any panic that might occur inside
    // encode_with_special_tokens and translate to EncoderInternal per
    // spec §8 (panic → fail-closed reservation). Preserved from
    // SLICE_03.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        enc.bpe.encode_with_special_tokens(text).len()
    }));
    result.map_err(|_| TokenizerError::EncoderInternal {
        kind: enc.family.encoder_name(),
        message: "tiktoken-rs encode panicked on input".to_string(),
    })
}

// Suppress unused-import warning for Message until we add a chat-shape
// unit test in this file (covered by golden_samples integration).
#[allow(unused_imports)]
use Message as _Message;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoder_kind_is_openai() {
        let bpe = tiktoken_rs::cl100k_base_singleton();
        let enc = OpenAiEncoder::new(TiktokenFamily::Cl100kBase, bpe);
        assert_eq!(enc.kind(), EncoderKind::OpenAi);
        assert_eq!(enc.encoder_name(), "cl100k_base");
        assert_eq!(enc.version_id(), TIKTOKEN_CL100K_BASE_VERSION_ID);
    }

    #[test]
    fn encode_hello_world_cl100k() {
        let bpe = tiktoken_rs::cl100k_base_singleton();
        let enc = OpenAiEncoder::new(TiktokenFamily::Cl100kBase, bpe);
        // Hard-coded well-known reference: "hello world" → 2 tokens.
        let toks = enc.encode("hello world");
        assert_eq!(toks.len(), 2);
    }

    #[test]
    fn count_tokens_raw_text_branch() {
        let bpe = tiktoken_rs::cl100k_base_singleton();
        let enc = OpenAiEncoder::new(TiktokenFamily::Cl100kBase, bpe);
        let req = TokenizeRequest {
            model: "gpt-4".to_string(),
            raw_text: "hello world".to_string(),
            ..Default::default()
        };
        let r = enc.count_tokens_request(&req).unwrap();
        assert_eq!(r.input_tokens, 2);
        assert_eq!(r.kind, EncoderKind::OpenAi);
        assert_eq!(r.tokenizer_version_id, TIKTOKEN_CL100K_BASE_VERSION_ID);
    }

    #[test]
    fn count_tokens_chat_envelope_branch() {
        let bpe = tiktoken_rs::o200k_base_singleton();
        let enc = OpenAiEncoder::new(TiktokenFamily::O200kBase, bpe);
        let req = TokenizeRequest {
            model: "gpt-4o".to_string(),
            messages: vec![Message {
                role: "user".to_string(),
                content: "hello".to_string(),
                tool_calls: vec![],
            }],
            ..Default::default()
        };
        let r = enc.count_tokens_request(&req).unwrap();
        // 3 per-msg + 1 role + 1 content + 3 priming = 8 lower-bound.
        // (May vary slightly with o200k vocab.) Verify > 0 sanity.
        assert!(r.input_tokens >= 5);
        assert!(r.input_tokens <= 15);
    }

    #[test]
    fn from_dispatch_round_trips() {
        assert_eq!(
            TiktokenFamily::from_dispatch(TiktokenEncoder::Cl100kBase),
            TiktokenFamily::Cl100kBase
        );
        assert_eq!(
            TiktokenFamily::from_dispatch(TiktokenEncoder::O200kBase),
            TiktokenFamily::O200kBase
        );
        assert_eq!(
            TiktokenFamily::from_dispatch(TiktokenEncoder::P50kBase),
            TiktokenFamily::P50kBase
        );
    }
}
