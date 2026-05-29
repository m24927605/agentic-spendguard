//! Per-provider dispatch table.
//!
//! Spec ref `tokenizer-service-spec-v1alpha1.md` §3.1 + §3.3.
//!
//! The table maps a model-string pattern to an encoder kind +
//! canonical encoder name. Patterns are anchored regex (`^...$`)
//! per §3.3 ("no fuzzy match"). Unknown models route to Tier 3 with
//! a `tokenizer_unknown_model` metric emission (the metric itself is
//! emitted by the caller — sidecar / gRPC service — using the
//! [`TokenizerError::EncoderInternal`]-free code path).
//!
//! ## SLICE_03 scope
//!
//! Only the OpenAI entries are compiled. SLICE_04 will append the
//! Anthropic / Gemini / Cohere / Bedrock-routing entries; that
//! change is purely additive (no existing row mutates).
//!
//! ## Adding a new entry
//!
//! 1. Add a `DispatchEntry` literal here.
//! 2. Mint a new `tokenizer_versions` UUIDv7 in `versions.rs`.
//! 3. Add the seed migration row in
//!    `services/ledger/migrations/00XX_tokenizer_versions_*.sql`.
//! 4. If the encoder kind / asset is new, embed the bundle in
//!    `data/` and add a sha256 const in `lib.rs::asset_sha256`.

use crate::error::TokenizerError;
use crate::versions::{
    TIKTOKEN_CL100K_BASE_VERSION_ID, TIKTOKEN_O200K_BASE_VERSION_ID,
    TIKTOKEN_P50K_BASE_VERSION_ID,
};
use regex::Regex;

/// Encoder kind discriminant — mirrors `tokenizer_versions.kind`
/// CHECK constraint values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncoderKind {
    OpenAiTiktoken,
    // SLICE_04 will add: AnthropicBpe, GeminiBpe, CohereBpe,
    // SentencepieceLlama. Defining them now would force noop
    // match arms across the codebase without test coverage; we
    // keep the enum minimal so SLICE_03's compile graph cleanly
    // refuses to compile if anyone accidentally references a
    // SLICE_04 variant.
}

impl EncoderKind {
    /// Stable string discriminant used in
    /// [`crate::TokenizeResponse::kind`] and in the
    /// `tokenizer_versions.kind` SQL CHECK constraint.
    pub fn as_str(self) -> &'static str {
        match self {
            EncoderKind::OpenAiTiktoken => "OPENAI_TIKTOKEN",
        }
    }
}

/// Identifies which tiktoken-rs encoder a SLICE_03 entry resolves
/// to. Lets the [`crate::encoder_cache::EncoderCache`] pick the
/// pre-loaded singleton without re-running the dispatch regex.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TiktokenEncoder {
    Cl100kBase,
    O200kBase,
    P50kBase,
}

impl TiktokenEncoder {
    pub fn encoder_name(self) -> &'static str {
        match self {
            TiktokenEncoder::Cl100kBase => "cl100k_base",
            TiktokenEncoder::O200kBase => "o200k_base",
            TiktokenEncoder::P50kBase => "p50k_base",
        }
    }

    pub fn tokenizer_version_id(self) -> &'static str {
        match self {
            TiktokenEncoder::Cl100kBase => TIKTOKEN_CL100K_BASE_VERSION_ID,
            TiktokenEncoder::O200kBase => TIKTOKEN_O200K_BASE_VERSION_ID,
            TiktokenEncoder::P50kBase => TIKTOKEN_P50K_BASE_VERSION_ID,
        }
    }
}

/// One row in the dispatch table — a compiled regex + the encoder
/// it dispatches to.
#[derive(Debug)]
pub struct DispatchEntry {
    /// Anchored regex pattern; matches the LLM request body's
    /// `model` field byte-for-byte.
    pub pattern: Regex,

    /// Human-readable pattern for error / log messages.
    pub pattern_source: &'static str,

    pub kind: EncoderKind,
    pub tiktoken: TiktokenEncoder,
}

/// All raw (pattern, kind, encoder) tuples. Lifted to a const so
/// SLICE_04's diff is a clean append-only edit.
///
/// Per spec §3.1 — OpenAI subset:
///
///   * `gpt-4o` / `gpt-4o-mini` (+ optional dated suffix) → o200k_base
///   * `gpt-4` / `gpt-4-turbo` / `gpt-4-XXXX-XX-XX` → cl100k_base
///   * `gpt-3.5-turbo` (+ optional dated suffix) → cl100k_base
///   * `text-davinci-003` (+ older completion models) → p50k_base
///
/// Pattern ordering: more specific (e.g. `gpt-4o`) listed BEFORE
/// the broader `gpt-4` pattern so first-match wins. The dispatch
/// loop iterates top-to-bottom and stops at the first regex match.
const RAW_ENTRIES: &[(&str, EncoderKind, TiktokenEncoder)] = &[
    // ── o200k_base (latest, narrowest patterns first) ──────────
    (
        r"^gpt-4o-mini(-\d{4}-\d{2}-\d{2})?$",
        EncoderKind::OpenAiTiktoken,
        TiktokenEncoder::O200kBase,
    ),
    (
        r"^gpt-4o(-\d{4}-\d{2}-\d{2})?$",
        EncoderKind::OpenAiTiktoken,
        TiktokenEncoder::O200kBase,
    ),
    // ── cl100k_base ───────────────────────────────────────────
    (
        r"^gpt-4-turbo(-preview)?(-\d{4}-\d{2}-\d{2})?$",
        EncoderKind::OpenAiTiktoken,
        TiktokenEncoder::Cl100kBase,
    ),
    (
        r"^gpt-4(-\d{4})?(-\d{4}-\d{2}-\d{2})?$",
        EncoderKind::OpenAiTiktoken,
        TiktokenEncoder::Cl100kBase,
    ),
    (
        r"^gpt-3\.5-turbo(-\d{4})?(-\d{2}k)?$",
        EncoderKind::OpenAiTiktoken,
        TiktokenEncoder::Cl100kBase,
    ),
    // ── p50k_base (legacy completion models) ──────────────────
    (
        r"^text-davinci-(002|003)$",
        EncoderKind::OpenAiTiktoken,
        TiktokenEncoder::P50kBase,
    ),
    (
        r"^code-davinci-(001|002)$",
        EncoderKind::OpenAiTiktoken,
        TiktokenEncoder::P50kBase,
    ),
];

/// The compiled dispatch table — one `DispatchEntry` per row.
#[derive(Debug)]
pub struct DispatchTable {
    entries: Vec<DispatchEntry>,
}

impl DispatchTable {
    /// Compile [`RAW_ENTRIES`] into a runtime-usable table. Returns
    /// [`TokenizerError::DispatchPatternInvalid`] if any pattern is
    /// malformed (programmer error; never expected at runtime).
    pub fn compile() -> Result<Self, TokenizerError> {
        let mut entries = Vec::with_capacity(RAW_ENTRIES.len());
        for (pattern, kind, encoder) in RAW_ENTRIES {
            let regex = Regex::new(pattern).map_err(|e| TokenizerError::DispatchPatternInvalid {
                pattern: (*pattern).to_string(),
                source: e,
            })?;
            entries.push(DispatchEntry {
                pattern: regex,
                pattern_source: pattern,
                kind: *kind,
                tiktoken: *encoder,
            });
        }
        Ok(Self { entries })
    }

    /// First-match lookup. Returns `None` for unknown models so the
    /// caller can route to Tier 3.
    pub fn lookup(&self, model: &str) -> Option<&DispatchEntry> {
        self.entries
            .iter()
            .find(|entry| entry.pattern.is_match(model))
    }

    /// Number of compiled entries. Used by acceptance test §8.1.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate all compiled entries — used by the gRPC service to
    /// eager-load encoders + emit a startup log.
    pub fn entries(&self) -> &[DispatchEntry] {
        &self.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> DispatchTable {
        DispatchTable::compile().expect("dispatch compiles")
    }

    #[test]
    fn gpt_4o_routes_to_o200k_base() {
        let t = table();
        let e = t.lookup("gpt-4o").expect("hit");
        assert_eq!(e.tiktoken, TiktokenEncoder::O200kBase);
        assert_eq!(e.kind, EncoderKind::OpenAiTiktoken);
    }

    #[test]
    fn gpt_4o_mini_routes_to_o200k_base() {
        let t = table();
        let e = t.lookup("gpt-4o-mini").expect("hit");
        assert_eq!(e.tiktoken, TiktokenEncoder::O200kBase);
    }

    #[test]
    fn gpt_4o_2024_08_06_routes_to_o200k_base() {
        // Per §9 question 1 — explicit fixture for the dated suffix.
        let t = table();
        let e = t.lookup("gpt-4o-2024-08-06").expect("hit");
        assert_eq!(e.tiktoken, TiktokenEncoder::O200kBase);
    }

    #[test]
    fn gpt_4o_mini_2024_07_18_routes_to_o200k_base() {
        // Per §9 question 1.
        let t = table();
        let e = t.lookup("gpt-4o-mini-2024-07-18").expect("hit");
        assert_eq!(e.tiktoken, TiktokenEncoder::O200kBase);
    }

    #[test]
    fn gpt_4_turbo_routes_to_cl100k() {
        let t = table();
        let e = t.lookup("gpt-4-turbo").expect("hit");
        assert_eq!(e.tiktoken, TiktokenEncoder::Cl100kBase);
    }

    #[test]
    fn gpt_4_routes_to_cl100k() {
        let t = table();
        let e = t.lookup("gpt-4").expect("hit");
        assert_eq!(e.tiktoken, TiktokenEncoder::Cl100kBase);
    }

    #[test]
    fn gpt_35_turbo_routes_to_cl100k() {
        let t = table();
        let e = t.lookup("gpt-3.5-turbo").expect("hit");
        assert_eq!(e.tiktoken, TiktokenEncoder::Cl100kBase);
    }

    #[test]
    fn gpt_35_turbo_16k_routes_to_cl100k() {
        let t = table();
        let e = t.lookup("gpt-3.5-turbo-16k").expect("hit");
        assert_eq!(e.tiktoken, TiktokenEncoder::Cl100kBase);
    }

    #[test]
    fn text_davinci_003_routes_to_p50k() {
        let t = table();
        let e = t.lookup("text-davinci-003").expect("hit");
        assert_eq!(e.tiktoken, TiktokenEncoder::P50kBase);
    }

    #[test]
    fn unknown_model_returns_none() {
        let t = table();
        assert!(t.lookup("some-experimental-internal-model").is_none());
        assert!(t.lookup("gpt-5-doesnt-exist-yet").is_none());
        assert!(t.lookup("").is_none());
    }

    #[test]
    fn no_fuzzy_match_per_spec_3_3() {
        // Spec §3.3 — `gpt-4o-mini-foo-bar` must be unknown, NOT
        // silently dispatched to o200k_base. Anchored regex
        // guarantees this.
        let t = table();
        assert!(t.lookup("gpt-4o-mini-foo-bar").is_none());
        assert!(t.lookup("gpt-4-bogus").is_none());
    }

    #[test]
    fn dispatch_pattern_ordering_specific_before_general() {
        // gpt-4o must hit the o200k_base entry NOT the cl100k_base
        // gpt-4 entry, even though the latter's `\d{4}` could
        // mis-capture the `o-` suffix tail. Verify that the o200k
        // entry comes earlier in the table.
        let t = table();
        let o4o_idx = t
            .entries()
            .iter()
            .position(|e| e.tiktoken == TiktokenEncoder::O200kBase)
            .expect("has o200k entry");
        let cl100k_4_idx = t
            .entries()
            .iter()
            .position(|e| e.pattern_source.contains("gpt-4(-"))
            .expect("has cl100k gpt-4 entry");
        assert!(
            o4o_idx < cl100k_4_idx,
            "o200k pattern must be checked before cl100k gpt-4 pattern"
        );
    }

    #[test]
    fn dispatch_table_has_expected_minimum_entries() {
        // SLICE_03 ships 7 OpenAI entries. SLICE_04 will add more.
        let t = table();
        assert!(
            t.len() >= 7,
            "expected >=7 SLICE_03 OpenAI entries, got {}",
            t.len()
        );
    }

    #[test]
    fn version_id_consistency_per_encoder() {
        let t = table();
        for entry in t.entries() {
            let id = entry.tiktoken.tokenizer_version_id();
            assert!(
                uuid::Uuid::parse_str(id).is_ok(),
                "tokenizer_version_id `{}` for encoder `{:?}` must be valid UUID",
                id,
                entry.tiktoken,
            );
        }
    }
}
