//! SpendGuard tokenizer library crate.
//!
//! ## Why this crate exists
//!
//! Per `docs/tokenizer-service-spec-v1alpha1.md` §2.1, the SpendGuard
//! tokenizer ships in two co-existing forms:
//!
//!   * **(a) gRPC service** — `services/tokenizer/` crate; tonic over
//!     mTLS (centralised form used by output_predictor, calibration,
//!     SDK fallback, and the Tier 1 shadow worker).
//!   * **(b) Rust library** — this crate; sidecar / egress_proxy link
//!     it directly for in-process hot-path tokenize, achieving the
//!     spec §10.1 SLO of p99 < 1ms.
//!
//! Both forms share the same dispatch table, encoder cache, asset
//! integrity check, and `tokenizer_versions` registry mapping so a
//! refresh rolls out consistently. The gRPC service crate links this
//! library and wraps its [`Tokenizer`] handle in a tonic `Service`
//! implementation (see `services/tokenizer/src/server.rs`).
//!
//! ## Three-tier architecture (recap)
//!
//! | Tier | Source | Hot path | SLO |
//! |------|--------|----------|-----|
//! | T1   | Provider `count_tokens` API (Anthropic / Gemini) | ❌ never | n/a |
//! | T2   | Local exact BPE (tiktoken-rs for OpenAI; SLICE_04 adds others) | ✅ source of truth | < 1ms p99 |
//! | T3   | `chars / 4 × 1.05` heuristic fallback | ✅ rare; < 0.1% hit | < 1ms |
//!
//! This SLICE_03 lands T2 OpenAI + T3 only. T1 (`shadow_verify`) is
//! out of scope and lives in `services/tokenizer/src/server.rs` as a
//! gRPC `UNIMPLEMENTED` stub.
//!
//! ## Public surface
//!
//! ```ignore
//! use spendguard_tokenizer::{Tokenizer, TokenizeRequest, Message};
//!
//! let tokenizer = Tokenizer::new_with_embedded_assets()?;
//!
//! let req = TokenizeRequest {
//!     model: "gpt-4o-mini".to_string(),
//!     messages: vec![Message {
//!         role: "user".to_string(),
//!         content: "hello".to_string(),
//!         tool_calls: vec![],
//!     }],
//!     raw_text: String::new(),
//!     request_id: String::new(),
//! };
//!
//! let resp = tokenizer.tokenize(&req)?;
//! assert_eq!(resp.tier, "T2");
//! assert!(!resp.tokenizer_version_id.is_empty());
//! ```
//!
//! ## Audit-chain integration
//!
//! Every [`TokenizeResponse`] carries `tier` + `tokenizer_version_id`
//! which the sidecar copies to its `BudgetClaim` metadata. The
//! `audit_outbox` migration 0046 introduced the columns
//! `tokenizer_tier` + `tokenizer_version_id` and migration 0048
//! created the `tokenizer_versions` FK target table; SLICE_03 ships
//! the initial registry rows via
//! `services/ledger/migrations/0049_tokenizer_versions_initial_seed.sql`.
//!
//! See `crates/spendguard-prediction-mirror` for the column ↔ proto
//! sentinel translation that producers (sidecar, webhook_receiver,
//! ttl_sweeper) use when writing audit rows.

pub mod dispatch;
pub mod encoder_cache;
pub mod encoders;
pub mod error;
pub mod tier3;
pub mod versions;

use std::sync::Arc;

// `EncoderKind` was scoped under `dispatch` in SLICE_03 (OpenAI-only).
// SLICE_04 lifts it into `encoders` so all 5 kinds (OpenAi, Anthropic,
// Gemini, Cohere, Llama) live together with the `Encoder` trait. The
// re-export below preserves `spendguard_tokenizer::EncoderKind` as the
// stable public path (the trait module is the source of truth).
pub use dispatch::DispatchEntry;
pub use encoder_cache::{EncoderBootMetric, EncoderCache};
pub use encoders::{EncodeResult, Encoder, EncoderKind};
pub use error::TokenizerError;
pub use tier3::tier3_fallback;
pub use versions::{
    initial_seed_rows, slice04_seed_rows, TokenizerVersionId, TokenizerVersionRow,
    ANTHROPIC_CLAUDE3_VERSION_ID, COHERE_COMMAND_R_VERSION_ID, GEMINI_15_VERSION_ID,
    LLAMA_31_VERSION_ID, TIER3_NULL_SENTINEL_VERSION_ID, TIKTOKEN_CL100K_BASE_VERSION_ID,
    TIKTOKEN_O200K_BASE_VERSION_ID, TIKTOKEN_P50K_BASE_VERSION_ID,
};

/// Embedded sha256 manifest for the vendored encoder assets.
///
/// SLICE_03 v1alpha1 mechanism: at `Tokenizer::new` boot time, every
/// embedded asset's sha256 is recomputed and compared against the
/// const value here. Mismatch → refuse to start (per spec §7.4 fail-
/// fast). The ed25519 signature half of the bundle integrity check
/// is wired by the production release pipeline + verified in SLICE-
/// extra; the sha256 const is the v1alpha1 floor that prevents an
/// attacker from swapping a tampered .tiktoken file into the binary
/// out-of-tree.
///
/// **Source of these constants**: bytes embedded by `include_bytes!`
/// at the dispatch sites below. The pre-commit hook in
/// `data/.sha256-manifest` lists the recomputed values; running
/// `crates/spendguard-tokenizer/data/recompute_sha256.sh` regenerates
/// them after an encoder asset swap (e.g., bumping tiktoken-rs).
pub mod asset_sha256 {
    /// sha256 of the embedded cl100k_base.tiktoken bytes (used by
    /// gpt-4 / gpt-4-turbo / gpt-3.5-turbo families).
    pub const CL100K_BASE: &str =
        "223921b76ee99bde995b7ff738513eef100fb51d18c93597a113bcffe865b2a7";

    /// sha256 of the embedded o200k_base.tiktoken bytes (used by
    /// gpt-4o / gpt-4o-mini families).
    pub const O200K_BASE: &str = "446a9538cb6c348e3516120d7c08b09f57c36495e2acfffe59a5bf8b0cfb1a2d";

    /// sha256 of the embedded p50k_base.tiktoken bytes (used by
    /// text-davinci-003 / older models).
    pub const P50K_BASE: &str = "94b5ca7dff4d00767bc256fdd1b27e5b17361d7b8a5f968547f9f23eb70d2069";

    // ──────────────────────────────────────────────────────────────
    // SLICE_04 — Tier 2 expansion (Anthropic + Cohere via tokenizers
    // crate; Gemini approximation + Llama SentencePiece via same).
    // Each constant is the sha256 of the file at
    // `data/<vendor>/tokenizer.json` byte-for-byte; rotated together
    // with the asset whenever a vendor's tokenizer.json is refreshed
    // (per spec §6.2 + §7.3 quarterly cadence).
    // ──────────────────────────────────────────────────────────────

    /// sha256 of the vendored Anthropic Claude 3 tokenizer.json
    /// (from Xenova/claude-tokenizer on Hugging Face;
    /// see `LICENSE_NOTICES.md` for the pinned revision hash).
    pub const ANTHROPIC_CLAUDE3: &str =
        "c241737df24b4e7f7c9af4fdcee29a0ca903dcb288a8b753bc346a3092911767";

    /// sha256 of the vendored Cohere Command-R tokenizer.json
    /// (from Xenova/c4ai-command-r-v01-tokenizer on Hugging Face).
    pub const COHERE_COMMAND_R: &str =
        "0af6e6fe50ce1bb5611b103482de6bac000c82e06898138d57f35af121aec772";

    /// sha256 of the vendored Gemini-approximation tokenizer.json
    /// (from Xenova/gemma-tokenizer on Hugging Face — community
    /// approximation since Google's official Gemini tokenizer is
    /// API-only). Spec §4.2 0.01 drift threshold accommodates the
    /// approximation gap; SLICE_05 shadow worker measures.
    pub const GEMINI_15: &str = "05e97791a5e007260de1db7e1692e53150e08cea481e2bf25435553380c147ee";

    /// sha256 of the vendored Llama 3.1 tokenizer.json
    /// (from Xenova/Meta-Llama-3.1-Tokenizer on Hugging Face).
    pub const LLAMA_31: &str = "79e3e522635f3171300913bb421464a87de6222182a0570b9b2ccba2a964b2b4";
}

/// Public-surface request shape.
///
/// Mirrors the proto `TokenizeRequest` (see
/// `proto/spendguard/tokenizer/v1/tokenizer.proto`). Kept as a plain
/// Rust struct in the library so callers (sidecar / egress_proxy)
/// don't have to depend on tonic / prost for in-process tokenize.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenizeRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub raw_text: String,
    pub request_id: String,
}

/// Sub-message of [`TokenizeRequest`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Message {
    pub role: String,
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolCall {
    pub name: String,
    pub arguments_json: String,
}

/// Public-surface response shape.
///
/// Mirrors the proto `TokenizeResponse`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TokenizeResponse {
    pub input_tokens: i64,
    /// `"T2"` or `"T3"`.
    pub tier: String,
    /// UUIDv7 of the `tokenizer_versions` row; empty for Tier 3.
    pub tokenizer_version_id: String,
    /// `EncoderKind` discriminant string, e.g. `"OPENAI_TIKTOKEN"`.
    pub kind: String,
    /// Tier 3 only: char count input to the heuristic.
    pub fallback_char_count: i64,
    /// Tier 3 only: 1.05 per spec §5.1 (5% conservative margin).
    pub fallback_margin_ratio: f32,
    /// Time spent inside [`Tokenizer::tokenize`]; for SLO tracking.
    pub latency_ns: i64,
}

/// The library entry point.
///
/// Lifecycle:
///   1. `Tokenizer::new_with_embedded_assets()` once at sidecar boot;
///      panics-or-errors-out on asset signature mismatch (spec §7.4
///      fail-fast).
///   2. Share a single `Arc<Tokenizer>` across worker threads. The
///      [`EncoderCache`] is immutable after construction; encoders are
///      eagerly loaded in step 1 and there is no runtime mutation in
///      the shipped hot path (hot-reload is a future slice).
///   3. Each request hits `tokenize(&req)`; the dispatcher returns a
///      Tier 2 result for known models or a Tier 3 fallback for
///      unknown ones.
pub struct Tokenizer {
    cache: EncoderCache,
    /// Compiled dispatch table — patterns are pre-built once.
    dispatch: Arc<dispatch::DispatchTable>,
}

impl Tokenizer {
    /// Construct a `Tokenizer` with the embedded encoder assets.
    ///
    /// Errors if any embedded asset's sha256 disagrees with the
    /// `asset_sha256` constants (spec §7.4 fail-fast). Production
    /// deployments always use this constructor.
    pub fn new_with_embedded_assets() -> Result<Self, TokenizerError> {
        let dispatch = Arc::new(dispatch::DispatchTable::compile()?);
        let cache = EncoderCache::with_embedded_assets()?;
        Ok(Self { cache, dispatch })
    }

    /// Tokenize one request.
    ///
    /// Per spec §3.1 — model string matched against the dispatch
    /// table; unknown models route to Tier 3 fallback. Per spec
    /// §3.4 — envelope tokens added per-kind (SLICE_03 OpenAI uses
    /// the published "3 tokens per message + 1 token for role" rule).
    pub fn tokenize(&self, req: &TokenizeRequest) -> Result<TokenizeResponse, TokenizerError> {
        let start = std::time::Instant::now();

        let resp = match self.dispatch.lookup(&req.model) {
            Some(entry) => self.cache.tokenize_with_entry(entry, req)?,
            None => {
                tracing::info!(
                    model = %req.model,
                    "tokenizer_unknown_model — Tier 3 fallback"
                );
                tier3::tier3_fallback(req)
            }
        };

        let latency_ns = start.elapsed().as_nanos() as i64;
        Ok(TokenizeResponse { latency_ns, ..resp })
    }

    /// Read-only access to the compiled dispatch table — useful for
    /// audit-chain `verify-chain` reproductions where the verifier
    /// must reconstruct the encoder lookup that the producer used.
    pub fn dispatch(&self) -> &dispatch::DispatchTable {
        &self.dispatch
    }

    /// Boot-time duration samples captured while eager-loading
    /// encoder assets. Empty only for dispatch-only test tokenizers.
    pub fn encoder_boot_durations(&self) -> &[EncoderBootMetric] {
        self.cache.boot_durations()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_unknown_model_falls_back_to_tier3() {
        // Use a deliberately fake encoder cache constructor so we
        // can run this test without loading real BPE assets — the
        // dispatch lookup is what we're testing here.
        let dispatch = Arc::new(dispatch::DispatchTable::compile().unwrap());
        let cache = EncoderCache::test_empty();
        let tokenizer = Tokenizer { cache, dispatch };

        let req = TokenizeRequest {
            model: "some-experimental-internal-model".to_string(),
            raw_text: "hello world this is fine".to_string(),
            ..Default::default()
        };
        let resp = tokenizer.tokenize(&req).unwrap();
        assert_eq!(resp.tier, "T3");
        assert!(resp.tokenizer_version_id.is_empty());
        assert_eq!(resp.kind, "HEURISTIC");
        assert!(resp.fallback_char_count > 0);
        assert!((resp.fallback_margin_ratio - 1.05).abs() < 1e-6);
    }
}
