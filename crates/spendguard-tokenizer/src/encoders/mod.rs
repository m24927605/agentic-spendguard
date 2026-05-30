//! Multi-encoder abstraction layer.
//!
//! Spec refs:
//!   - `tokenizer-service-spec-v1alpha1.md` §3.1 (dispatch table → encoder
//!     kind), §3.2 (encoder cache eager-load), §3.4 (per-kind message
//!     envelope), §4.2 (per-kind drift thresholds).
//!
//! ## Why the trait
//!
//! SLICE_03 hard-coded the OpenAI tiktoken-rs path through
//! `encoder_cache::tokenize_with_entry`. SLICE_04 adds Anthropic /
//! Gemini / Cohere / Llama, each of which loads BPE assets through the
//! Hugging Face `tokenizers` crate (`tokenizer.json`) instead of
//! tiktoken-rs's `.tiktoken` format. Rather than scatter `match` arms
//! across the cache, dispatch, and per-encoder files, we lift the
//! contract into [`Encoder`] — one object per encoder family — and let
//! the cache dispatch via trait object.
//!
//! Per spec §3.2 the encoders are immutable + thread-safe (boot-time
//! eager-load; hot path is lock-free). The trait inherits `Send + Sync`
//! so trait objects can live behind `Arc<dyn Encoder>` for cheap
//! per-request dispatch.
//!
//! ## What each kind owns
//!
//! Each encoder implementation owns:
//!   1. Asset bytes (`include_bytes!`) + Layer A sha256 verification.
//!   2. Layer B runtime cross-check fixture (per spec §7.4.1).
//!   3. The actual `encode()` path (encode + count_tokens).
//!   4. Per-kind envelope rules (per spec §3.4) — see [`ChatEnvelope`]
//!      and [`Encoder::envelope_overhead`].
//!   5. BOS token accounting on raw_text — see
//!      [`Encoder::bos_token_count`]. Bedrock invokeModel prepends
//!      `<|begin_of_text|>` / `<|START_OF_TURN|>` markers for vendors
//!      that ship a BOS in their tokenizer config; the count must
//!      include them so Tier 2 = vendor-observed count.
//!   6. Its `tokenizer_version_id` stable UUIDv7.

use crate::error::TokenizerError;

pub mod anthropic;
#[cfg(feature = "cohere")]
pub mod cohere;
pub mod gemini;
pub mod llama;
pub mod openai;

/// Stable string discriminant matching `tokenizer_versions.kind`
/// CHECK constraint values. Used for `TokenizeResponse.kind` and for
/// per-kind drift threshold lookups (spec §4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EncoderKind {
    /// tiktoken-rs `cl100k_base` / `o200k_base` / `p50k_base`.
    OpenAi,
    /// Anthropic Claude 3 / 3.5 BPE via vendored HF tokenizer.json.
    Anthropic,
    /// Google Gemini 1.5 / 2.0 BPE — note Gemini official tokenizer is
    /// API-only, we ship the Xenova community approximation.
    /// Spec §4.2 documents 0.01 drift threshold to account for the
    /// approximation gap; SLICE_05 shadow worker quantifies it.
    Gemini,
    /// Cohere Command-R BPE via vendored HF tokenizer.json.
    Cohere,
    /// Meta Llama 3.1 SentencePiece via vendored HF tokenizer.json.
    Llama,
}

impl EncoderKind {
    /// Stable string discriminant used in
    /// [`crate::TokenizeResponse::kind`] and the
    /// `tokenizer_versions.kind` SQL CHECK constraint.
    ///
    /// Note the SQL CHECK constraint uses the `_BPE` / `_TIKTOKEN`
    /// suffixed forms; this mapping must stay in sync with
    /// `services/ledger/migrations/0048_tokenizer_versions.sql` and
    /// `0050_tokenizer_versions_slice04_seed.sql`.
    pub fn as_str(self) -> &'static str {
        match self {
            EncoderKind::OpenAi => "OPENAI_TIKTOKEN",
            EncoderKind::Anthropic => "ANTHROPIC_BPE",
            EncoderKind::Gemini => "GEMINI_BPE",
            EncoderKind::Cohere => "COHERE_BPE",
            EncoderKind::Llama => "SENTENCEPIECE_LLAMA",
        }
    }

    /// Per spec §4.2 drift alert threshold — the tolerance |T1 - T2| /
    /// T1 above which the shadow worker emits a `drift_alert`. SLICE_05
    /// consumes this; SLICE_04 ships it inline so the dispatch table
    /// owns the policy alongside the routing.
    ///
    /// | Kind      | Threshold | Rationale (per spec §4.2)               |
    /// | --------- | --------- | --------------------------------------- |
    /// | OpenAi    | 0.0       | tiktoken byte-exact; any drift = bug.   |
    /// | Anthropic | 0.01      | vendored BPE may lag vendor microtune.  |
    /// | Gemini    | 0.01      | community approximation (API-only true). |
    /// | Cohere    | 0.015     | Cohere tokenizer less stable historically. |
    /// | Llama     | 0.005     | SentencePiece is config-precise; tight. |
    pub fn drift_threshold(self) -> f32 {
        match self {
            EncoderKind::OpenAi => 0.0,
            EncoderKind::Anthropic => 0.01,
            EncoderKind::Gemini => 0.01,
            EncoderKind::Cohere => 0.015,
            EncoderKind::Llama => 0.005,
        }
    }
}

/// One token count returned by [`Encoder::count_tokens_request`].
///
/// We surface the version id + kind alongside the count so the cache
/// can fill the audit fields without re-asking the encoder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodeResult {
    pub input_tokens: usize,
    pub tokenizer_version_id: &'static str,
    pub kind: EncoderKind,
}

/// Per-vendor chat envelope rule per spec §3.4.
///
/// Different vendors frame chat turns with different special-token
/// sequences. The OpenAI cookbook formula
/// (`3 + role + content + 1 tool_call_overhead + 3 reply_priming`)
/// is wrong for Anthropic / Gemini / Cohere / Llama. SLICE_04 R2 M3
/// adds per-vendor envelopes so each encoder counts the real overhead.
///
/// ## Field meanings
///
/// * `per_message` — fixed tokens added once per message regardless of
///   role / content (e.g., Cohere `<|START_OF_TURN|>` markers ≈ 3
///   tokens per turn; Llama header template ≈ 5 tokens).
/// * `per_turn_boundary` — tokens at the boundary between turns
///   (e.g., Anthropic `\n\nHuman:` / `\n\nAssistant:` boundary
///   ≈ 4 tokens per turn).
/// * `reply_priming` — tokens added once at the end of the chat to
///   prime the assistant's reply (e.g., OpenAI's 3-token priming).
///
/// The total chat tokens = sum over messages of
/// `(per_message + per_turn_boundary + role_tokens + content_tokens +
/// tool_call_tokens)` + `reply_priming` + (BOS for the whole prompt
/// if applicable).
///
/// Per-vendor values (per spec §3.4 amendment):
///
/// | Vendor    | per_message | per_turn_boundary | reply_priming |
/// | --------- | ----------- | ----------------- | ------------- |
/// | OpenAI    | 3 (4 for gpt-3.5-turbo-0301) | 0          | 3             |
/// | Anthropic | 0           | 4                 | 0             |
/// | Gemini    | 0           | 0                 | 0             |
/// | Cohere    | 3           | 0                 | 0             |
/// | Llama     | 5           | 0                 | 0             |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChatEnvelope {
    pub per_message: usize,
    pub per_turn_boundary: usize,
    pub reply_priming: usize,
}

impl ChatEnvelope {
    /// Total fixed-envelope overhead for `n` messages, excluding role
    /// + content + tool_call payload tokens (those are encoded
    /// separately).
    pub fn fixed_overhead(self, n_messages: usize) -> usize {
        n_messages * (self.per_message + self.per_turn_boundary) + self.reply_priming
    }
}

/// Per-encoder dispatch contract.
///
/// Implementations live under `encoders/<vendor>.rs`. Each
/// implementation is **immutable after construction** and **`Send +
/// Sync`** — spec §3.2 requires the hot path be lock-free.
///
/// Implementations are constructed exactly once at
/// [`crate::Tokenizer::new_with_embedded_assets`] boot via their
/// crate-local `new()` constructor (which runs the Layer A sha256 +
/// Layer B fixture cross-check; failure → fail-fast per §7.4).
pub trait Encoder: Send + Sync {
    /// Stable encoder identity — discriminant for routing + audit.
    fn kind(&self) -> EncoderKind;

    /// `tokenizer_versions` UUIDv7 string. Stable across processes.
    fn version_id(&self) -> &'static str;

    /// Canonical encoder name — e.g. `"cl100k_base"`, `"anthropic-v3-bpe"`.
    /// Surfaces in `tokenizer_versions.encoder_name`.
    fn encoder_name(&self) -> &'static str;

    /// Per-vendor chat envelope policy per spec §3.4 (R2 M3 amendment).
    /// Returns the fixed token overhead applied per message / per turn /
    /// at reply priming. Each vendor returns its real envelope rather
    /// than the SLICE_04 R1 placeholder that used the OpenAI 3+3 formula
    /// for all 4 non-OpenAI vendors.
    fn envelope_overhead(&self) -> ChatEnvelope;

    /// Per-vendor BOS token count added to raw_text (R2 M4 amendment).
    /// Bedrock's invokeModel API and Anthropic / Cohere / Llama SDK
    /// completions prepend a BOS marker (`<|begin_of_text|>`,
    /// `<|START_OF_TURN|>`) to the user's text. Tier 2 must account
    /// for it so the count matches vendor-observed token count.
    ///
    /// Returned value is added once per non-empty raw_text encode.
    /// For chat-shape requests the BOS is conceptually part of the
    /// first per_turn_boundary so it is NOT double-counted here; chat
    /// path uses `envelope_overhead()` instead.
    ///
    /// Per-vendor values (per spec §3.4 amendment):
    ///   * OpenAI: 0 (cl100k / o200k have no BOS in encode output)
    ///   * Anthropic: 1 (`<|begin_of_text|>` prepended by Bedrock)
    ///   * Gemini: 0 (Gemma vocab has no BOS in countTokens semantics)
    ///   * Cohere: 1 (`<|START_OF_TURN|>` prepended by Bedrock)
    ///   * Llama: 1 (`<|begin_of_text|>` prepended by Bedrock invokeModel)
    fn bos_token_count(&self) -> usize;

    /// Tokenize a single `TokenizeRequest` into an
    /// [`EncodeResult`]. Per spec §3.4 the per-kind envelope rules
    /// live inside the implementation (chat-shape role markers,
    /// per-message overhead, tool-call accounting).
    fn count_tokens_request(
        &self,
        req: &crate::TokenizeRequest,
    ) -> Result<EncodeResult, TokenizerError>;
}
