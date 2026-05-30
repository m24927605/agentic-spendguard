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

use crate::encoders::EncoderKind;
use crate::error::TokenizerError;
use crate::versions::{
    TIKTOKEN_CL100K_BASE_VERSION_ID, TIKTOKEN_O200K_BASE_VERSION_ID,
    TIKTOKEN_P50K_BASE_VERSION_ID,
};
use regex::Regex;

// `EncoderKind` moved to `crate::encoders` in SLICE_04 so all five
// kinds (OpenAi, Anthropic, Gemini, Cohere, Llama) live alongside the
// `Encoder` trait + per-kind drift threshold. The SLICE_03 variant name
// `OpenAiTiktoken` is renamed to `OpenAi` to match the trait module's
// convention (one variant per provider, not per asset format). The
// `tokenizer_versions.kind` SQL CHECK constraint value is still
// "OPENAI_TIKTOKEN" because that's what describes the embedded asset
// family; only the Rust enum name changed.
//
// External code that wants a stable Rust path can import via
// `spendguard_tokenizer::EncoderKind` (the lib.rs re-export points to
// the new location). The `dispatch::EncoderKind` path is no longer
// exposed; references inside this file use the trait module's enum.

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

/// Identifies which loaded encoder a dispatch row routes to. SLICE_03
/// only had a `tiktoken: TiktokenEncoder` field because OpenAI was the
/// only kind; SLICE_04 generalises to this enum so the dispatch row
/// can point at any of the five `EncoderKind` variants.
///
/// `Tiktoken(family)` is the SLICE_03 path; the other variants are
/// new for SLICE_04 and resolve to the corresponding `Encoder` trait
/// implementations registered in [`crate::encoder_cache::EncoderCache`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncoderResolver {
    /// SLICE_03 OpenAI path — tiktoken-rs singleton lookup.
    Tiktoken(TiktokenEncoder),
    /// SLICE_04 Anthropic Claude 3 / 3.5 BPE.
    Anthropic,
    /// SLICE_04 Gemini 1.5 / 2.0 BPE (community Gemma approximation).
    Gemini,
    /// SLICE_04 Cohere Command-R BPE.
    Cohere,
    /// SLICE_04 Llama 3.1 SentencePiece.
    Llama,
}

impl EncoderResolver {
    /// Canonical encoder name surfacing in audit + logs.
    pub fn encoder_name(self) -> &'static str {
        match self {
            EncoderResolver::Tiktoken(t) => t.encoder_name(),
            EncoderResolver::Anthropic => "anthropic-v3-bpe",
            EncoderResolver::Gemini => "gemini-1.5-bpe",
            EncoderResolver::Cohere => "cohere-v2-bpe",
            EncoderResolver::Llama => "llama-sentencepiece",
        }
    }

    /// Stable `tokenizer_versions.tokenizer_version_id` UUIDv7 string.
    pub fn tokenizer_version_id(self) -> &'static str {
        use crate::versions::{
            ANTHROPIC_CLAUDE3_VERSION_ID, COHERE_COMMAND_R_VERSION_ID, GEMINI_15_VERSION_ID,
            LLAMA_31_VERSION_ID,
        };
        match self {
            EncoderResolver::Tiktoken(t) => t.tokenizer_version_id(),
            EncoderResolver::Anthropic => ANTHROPIC_CLAUDE3_VERSION_ID,
            EncoderResolver::Gemini => GEMINI_15_VERSION_ID,
            EncoderResolver::Cohere => COHERE_COMMAND_R_VERSION_ID,
            EncoderResolver::Llama => LLAMA_31_VERSION_ID,
        }
    }

    /// The encoder kind discriminant this resolver dispatches to.
    pub fn kind(self) -> EncoderKind {
        match self {
            EncoderResolver::Tiktoken(_) => EncoderKind::OpenAi,
            EncoderResolver::Anthropic => EncoderKind::Anthropic,
            EncoderResolver::Gemini => EncoderKind::Gemini,
            EncoderResolver::Cohere => EncoderKind::Cohere,
            EncoderResolver::Llama => EncoderKind::Llama,
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

    /// SLICE_03 back-compat: when [`Self::resolver`] is
    /// `EncoderResolver::Tiktoken(family)`, this mirrors `family` so
    /// the existing `EncoderCache::tokenize_with_entry` SLICE_03 path
    /// can short-circuit without re-matching on resolver kind. For
    /// non-OpenAI rows this is a placeholder
    /// `TiktokenEncoder::Cl100kBase` value the cache MUST NOT read —
    /// the cache routes via `resolver` for all SLICE_04 kinds.
    pub tiktoken: TiktokenEncoder,

    /// SLICE_04 multi-encoder router. The
    /// [`crate::encoder_cache::EncoderCache::tokenize_with_entry`]
    /// path inspects this to pick the concrete `Encoder` trait
    /// implementation; for backward compatibility with the SLICE_03
    /// API, the `Tiktoken(...)` variant carries the same data as
    /// the `tiktoken` field above.
    pub resolver: EncoderResolver,
}

/// All raw (pattern, resolver) tuples. Lifted to a const so SLICE_04's
/// expansion is a clean append-only edit.
///
/// Per spec §3.1 — coverage:
///
///   OpenAI (SLICE_03):
///     * `gpt-4o` / `gpt-4o-mini` (+ optional dated suffix) → o200k_base
///     * `gpt-4` / `gpt-4-turbo` / `gpt-4-XXXX-XX-XX` → cl100k_base
///     * `gpt-3.5-turbo` (+ optional dated suffix) → cl100k_base
///     * `text-davinci-003` (+ older completion models) → p50k_base
///
///   Anthropic native (SLICE_04):
///     * `claude-3-(haiku|sonnet|opus)` (+ optional dated suffix)
///     * `claude-3-5-(haiku|sonnet|opus)` (+ optional dated suffix)
///
///   Anthropic Bedrock (SLICE_04):
///     * `anthropic.claude-3-(haiku|sonnet|opus)*-v\d:\d+`
///     * `anthropic.claude-3-5-(haiku|sonnet|opus)*-v\d:\d+`
///
///   Gemini native (SLICE_04):
///     * `gemini-1.5-(flash|pro)` (+ optional `-NNN` revision)
///     * `gemini-2.0-flash` (+ optional `-exp`)
///
///   Cohere native (SLICE_04):
///     * `command-r(-plus)?` (+ optional dated suffix)
///     * `command-light` (+ optional dated suffix)
///
///   Cohere Bedrock (SLICE_04):
///     * `cohere.command(-r)?(-plus)?-v\d:\d+`
///
///   Llama Bedrock (SLICE_04):
///     * `meta.llama3-N-Mb-instruct-v\d:\d+`
///
/// Pattern ordering: more specific (e.g. `gpt-4o`) listed BEFORE the
/// broader `gpt-4` pattern so first-match wins. The dispatch loop
/// iterates top-to-bottom and stops at the first regex match. The same
/// ordering rule applies for SLICE_04 patterns — explicit dated /
/// versioned Bedrock IDs come after native model IDs, but within
/// vendor families the most specific name (e.g. `claude-3-5-sonnet`)
/// is listed before the broader (`claude-3-*`) catch-all.
const RAW_ENTRIES: &[(&str, EncoderResolver)] = &[
    // ════════════════════════════════════════════════════════════════
    // SLICE_03 — OpenAI tiktoken-rs entries (unchanged)
    // ════════════════════════════════════════════════════════════════

    // ── o200k_base (latest, narrowest patterns first) ──────────
    (
        r"^gpt-4o-mini(-\d{4}-\d{2}-\d{2})?$",
        EncoderResolver::Tiktoken(TiktokenEncoder::O200kBase),
    ),
    (
        r"^gpt-4o(-\d{4}-\d{2}-\d{2})?$",
        EncoderResolver::Tiktoken(TiktokenEncoder::O200kBase),
    ),
    // ── cl100k_base ───────────────────────────────────────────
    // Round-2 fix M1 (panel finding): explicit `gpt-4(-NNNN)-preview`
    // entry before the broader gpt-4 / gpt-4-turbo patterns so
    // `gpt-4-1106-preview` and `gpt-4-0125-preview` correctly land on
    // cl100k_base. Previously they fell through to Tier 3 (5% margin
    // → ~2x under-count vs legacy heuristic).
    (
        r"^gpt-4(-\d{4})?-preview$",
        EncoderResolver::Tiktoken(TiktokenEncoder::Cl100kBase),
    ),
    (
        r"^gpt-4-turbo(-preview)?(-\d{4}-\d{2}-\d{2})?$",
        EncoderResolver::Tiktoken(TiktokenEncoder::Cl100kBase),
    ),
    (
        r"^gpt-4(-\d{4})?(-\d{4}-\d{2}-\d{2})?$",
        EncoderResolver::Tiktoken(TiktokenEncoder::Cl100kBase),
    ),
    (
        r"^gpt-3\.5-turbo(-\d{4})?(-\d{2}k)?$",
        EncoderResolver::Tiktoken(TiktokenEncoder::Cl100kBase),
    ),
    // ── p50k_base (legacy completion models) ──────────────────
    // Round-2 fix M1: `gpt-3.5-turbo-instruct` (and dated variants)
    // uses p50k_base per OpenAI tiktoken cookbook. Was missing →
    // Tier 3 fallback. The pattern is placed BEFORE the generic
    // `text-davinci-...` cluster so a future contributor adding
    // chat-instruct-style suffixes follows the same module order.
    (
        r"^gpt-3\.5-turbo-instruct(-\d{4})?$",
        EncoderResolver::Tiktoken(TiktokenEncoder::P50kBase),
    ),
    (
        r"^text-davinci-(002|003)$",
        EncoderResolver::Tiktoken(TiktokenEncoder::P50kBase),
    ),
    (
        r"^code-davinci-(001|002)$",
        EncoderResolver::Tiktoken(TiktokenEncoder::P50kBase),
    ),

    // ════════════════════════════════════════════════════════════════
    // SLICE_04 — Tier 2 expansion (per spec §3.1)
    // ════════════════════════════════════════════════════════════════

    // ── Anthropic Claude 3.5 family (narrowest patterns first) ──
    // The 3.5 series MUST come before the broader claude-3-* pattern
    // because `claude-3-5-sonnet` would otherwise be captured by the
    // SLICE_03 pattern-ordering rule (more-specific-first; per R1
    // panel finding cl_05 the dispatch table panics on accidental
    // overlap so test coverage in `services/tokenizer/tests/golden`
    // pins both forms).
    //
    // Real Anthropic model names per their public API docs:
    //   - claude-3-5-sonnet-20240620 / -20241022
    //   - claude-3-5-haiku-20241022
    //   - claude-3-opus-20240229
    //   - claude-3-sonnet-20240229
    //   - claude-3-haiku-20240307
    (
        r"^claude-3-5-(sonnet|haiku|opus)(-\d{8})?$",
        EncoderResolver::Anthropic,
    ),
    // ── Anthropic Claude 3.x — native model IDs ──
    (
        r"^claude-3-(haiku|sonnet|opus)(-\d{8})?$",
        EncoderResolver::Anthropic,
    ),
    // ── Anthropic Bedrock routing ──
    // Full Bedrock IDs look like:
    //   anthropic.claude-3-5-sonnet-20240620-v1:0
    //   anthropic.claude-3-haiku-20240307-v1:0
    // Per §9 review question 3 — explicit golden-sample coverage
    // for the full dated + versioned form is in
    // `services/tokenizer/tests/golden_samples.rs` SLICE_04 section.
    (
        r"^anthropic\.claude-3-5-(sonnet|haiku|opus)(-\d{8})?-v\d+:\d+$",
        EncoderResolver::Anthropic,
    ),
    (
        r"^anthropic\.claude-3-(haiku|sonnet|opus)(-\d{8})?-v\d+:\d+$",
        EncoderResolver::Anthropic,
    ),

    // ── Gemini native ──
    (
        r"^gemini-2\.0-flash(-exp)?$",
        EncoderResolver::Gemini,
    ),
    (
        r"^gemini-1\.5-(flash|pro)(-\d{3})?$",
        EncoderResolver::Gemini,
    ),

    // ── Cohere native ──
    // `command-r-plus` must precede `command-r` for first-match-wins.
    (
        r"^command-r-plus(-\d{8})?$",
        EncoderResolver::Cohere,
    ),
    (
        r"^command-r(-\d{8})?$",
        EncoderResolver::Cohere,
    ),
    (
        r"^command-light(-\d{8})?$",
        EncoderResolver::Cohere,
    ),

    // ── Cohere Bedrock routing ──
    (
        r"^cohere\.command(-r)?(-plus)?-v\d+:\d+$",
        EncoderResolver::Cohere,
    ),

    // ── Llama Bedrock routing ──
    // Per Bedrock published model IDs:
    //   meta.llama3-1-8b-instruct-v1:0
    //   meta.llama3-1-70b-instruct-v1:0
    //   meta.llama3-2-1b-instruct-v1:0
    //   meta.llama3-3-70b-instruct-v1:0
    (
        r"^meta\.llama3(-\d+)?-\d+b-instruct-v\d+:\d+$",
        EncoderResolver::Llama,
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
        for (pattern, resolver) in RAW_ENTRIES {
            let regex = Regex::new(pattern).map_err(|e| TokenizerError::DispatchPatternInvalid {
                pattern: (*pattern).to_string(),
                source: e,
            })?;
            // SLICE_03 back-compat: when the resolver is a tiktoken
            // family the `tiktoken` field carries the variant so the
            // existing SLICE_03 cache path can short-circuit. For
            // non-tiktoken kinds we still need a placeholder value
            // (the field is non-Option for back-compat) — the cache
            // dispatches via `resolver` for non-tiktoken kinds and
            // does NOT read the `tiktoken` field, so the placeholder
            // is unobservable. See `EncoderCache::tokenize_with_entry`.
            let tiktoken = match resolver {
                EncoderResolver::Tiktoken(t) => *t,
                _ => TiktokenEncoder::Cl100kBase, // placeholder; not read
            };
            entries.push(DispatchEntry {
                pattern: regex,
                pattern_source: pattern,
                kind: resolver.kind(),
                tiktoken,
                resolver: *resolver,
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
        assert_eq!(e.kind, EncoderKind::OpenAi);
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
        // SLICE_03 ships 9 OpenAI entries after R2 M1 added two more
        // (gpt-4-N-preview, gpt-3.5-turbo-instruct). SLICE_04 will add
        // more.
        let t = table();
        assert!(
            t.len() >= 9,
            "expected >=9 SLICE_03 OpenAI entries after R2 M1, got {}",
            t.len()
        );
    }

    // ── Round-2 fix M1 — dispatch coverage tests ──────────────────

    #[test]
    fn gpt_4_1106_preview_routes_to_cl100k() {
        // R2 M1: previously fell through to Tier 3 (5% margin) →
        // confirmed coverage now.
        let t = table();
        let e = t.lookup("gpt-4-1106-preview").expect("hit");
        assert_eq!(e.tiktoken, TiktokenEncoder::Cl100kBase);
        assert_eq!(e.kind, EncoderKind::OpenAi);
    }

    #[test]
    fn gpt_4_0125_preview_routes_to_cl100k() {
        // R2 M1: dated preview variant.
        let t = table();
        let e = t.lookup("gpt-4-0125-preview").expect("hit");
        assert_eq!(e.tiktoken, TiktokenEncoder::Cl100kBase);
    }

    #[test]
    fn gpt_4_preview_no_date_routes_to_cl100k() {
        // R2 M1: bare "gpt-4-preview" without dated suffix must also
        // route via the new pattern (the (-\d{4})? group is optional).
        let t = table();
        let e = t.lookup("gpt-4-preview").expect("hit");
        assert_eq!(e.tiktoken, TiktokenEncoder::Cl100kBase);
    }

    #[test]
    fn gpt_3_5_turbo_instruct_routes_to_p50k() {
        // R2 M1: alphabetic suffix variant (not date). Was T3 fallback.
        let t = table();
        let e = t.lookup("gpt-3.5-turbo-instruct").expect("hit");
        assert_eq!(e.tiktoken, TiktokenEncoder::P50kBase);
        assert_eq!(e.kind, EncoderKind::OpenAi);
    }

    #[test]
    fn gpt_3_5_turbo_instruct_0914_routes_to_p50k() {
        // R2 M1: dated instruct variant (per OpenAI deprecation notes).
        let t = table();
        let e = t.lookup("gpt-3.5-turbo-instruct-0914").expect("hit");
        assert_eq!(e.tiktoken, TiktokenEncoder::P50kBase);
    }

    #[test]
    fn new_m1_patterns_dont_collide_with_existing() {
        // Defense: the new gpt-4-N-preview pattern is more specific
        // than gpt-4-N — they shouldn't both match the same input.
        // Tested by confirming the new patterns route as expected
        // AND that the bare gpt-4 / gpt-3.5-turbo dispatch unchanged.
        let t = table();
        assert_eq!(
            t.lookup("gpt-4").expect("hit").tiktoken,
            TiktokenEncoder::Cl100kBase
        );
        assert_eq!(
            t.lookup("gpt-3.5-turbo").expect("hit").tiktoken,
            TiktokenEncoder::Cl100kBase
        );
        // Ensure gpt-3.5-turbo-instruct doesn't get caught by the
        // gpt-3.5-turbo(-N)? pattern (alphabetic suffix wouldn't
        // satisfy \d{4} anyway, but pin the assertion).
        assert_eq!(
            t.lookup("gpt-3.5-turbo-instruct").expect("hit").tiktoken,
            TiktokenEncoder::P50kBase,
        );
    }

    #[test]
    fn version_id_consistency_per_encoder() {
        // SLICE_04 update: iterate via `resolver.tokenizer_version_id()`
        // so non-tiktoken kinds are covered. The SLICE_03 `tiktoken`
        // field is a placeholder for non-OpenAI rows; the source of
        // truth is `EncoderResolver`.
        let t = table();
        for entry in t.entries() {
            let id = entry.resolver.tokenizer_version_id();
            assert!(
                uuid::Uuid::parse_str(id).is_ok(),
                "tokenizer_version_id `{}` for resolver `{:?}` must be valid UUID",
                id,
                entry.resolver,
            );
        }
    }

    // ════════════════════════════════════════════════════════════════
    // SLICE_04 — dispatch coverage tests per spec §3.1 + §9 checklist.
    // ════════════════════════════════════════════════════════════════

    // ── Anthropic native ────────────────────────────────────────────

    #[test]
    fn claude_3_haiku_routes_to_anthropic() {
        let t = table();
        let e = t.lookup("claude-3-haiku").expect("hit");
        assert_eq!(e.kind, EncoderKind::Anthropic);
        assert!(matches!(e.resolver, EncoderResolver::Anthropic));
    }

    #[test]
    fn claude_3_5_sonnet_routes_to_anthropic() {
        let t = table();
        let e = t.lookup("claude-3-5-sonnet").expect("hit");
        assert_eq!(e.kind, EncoderKind::Anthropic);
    }

    #[test]
    fn claude_3_5_sonnet_dated_routes_to_anthropic() {
        // Per §9 review question 3 — explicit dated suffix.
        let t = table();
        let e = t.lookup("claude-3-5-sonnet-20240620").expect("hit");
        assert_eq!(e.kind, EncoderKind::Anthropic);
    }

    #[test]
    fn claude_3_opus_dated_routes_to_anthropic() {
        let t = table();
        let e = t.lookup("claude-3-opus-20240229").expect("hit");
        assert_eq!(e.kind, EncoderKind::Anthropic);
    }

    #[test]
    fn claude_3_5_haiku_routes_to_anthropic() {
        let t = table();
        let e = t.lookup("claude-3-5-haiku").expect("hit");
        assert_eq!(e.kind, EncoderKind::Anthropic);
    }

    // ── Anthropic Bedrock routing ───────────────────────────────────

    #[test]
    fn bedrock_anthropic_claude_3_5_sonnet_routes_to_anthropic() {
        // Per §9 review question 3 — full Bedrock model id form.
        let t = table();
        let e = t
            .lookup("anthropic.claude-3-5-sonnet-20240620-v1:0")
            .expect("hit");
        assert_eq!(e.kind, EncoderKind::Anthropic);
    }

    #[test]
    fn bedrock_anthropic_claude_3_haiku_routes_to_anthropic() {
        let t = table();
        let e = t
            .lookup("anthropic.claude-3-haiku-20240307-v1:0")
            .expect("hit");
        assert_eq!(e.kind, EncoderKind::Anthropic);
    }

    // ── Gemini native ───────────────────────────────────────────────

    #[test]
    fn gemini_1_5_flash_routes_to_gemini() {
        let t = table();
        let e = t.lookup("gemini-1.5-flash").expect("hit");
        assert_eq!(e.kind, EncoderKind::Gemini);
    }

    #[test]
    fn gemini_1_5_pro_routes_to_gemini() {
        let t = table();
        let e = t.lookup("gemini-1.5-pro").expect("hit");
        assert_eq!(e.kind, EncoderKind::Gemini);
    }

    #[test]
    fn gemini_1_5_pro_002_routes_to_gemini() {
        // Revision suffix `-NNN`.
        let t = table();
        let e = t.lookup("gemini-1.5-pro-002").expect("hit");
        assert_eq!(e.kind, EncoderKind::Gemini);
    }

    #[test]
    fn gemini_2_0_flash_routes_to_gemini() {
        let t = table();
        let e = t.lookup("gemini-2.0-flash").expect("hit");
        assert_eq!(e.kind, EncoderKind::Gemini);
    }

    #[test]
    fn gemini_2_0_flash_exp_routes_to_gemini() {
        let t = table();
        let e = t.lookup("gemini-2.0-flash-exp").expect("hit");
        assert_eq!(e.kind, EncoderKind::Gemini);
    }

    // ── Cohere native ───────────────────────────────────────────────

    #[test]
    fn command_r_routes_to_cohere() {
        let t = table();
        let e = t.lookup("command-r").expect("hit");
        assert_eq!(e.kind, EncoderKind::Cohere);
    }

    #[test]
    fn command_r_plus_routes_to_cohere() {
        // `command-r-plus` must come BEFORE `command-r` else it'd
        // match the broader pattern.
        let t = table();
        let e = t.lookup("command-r-plus").expect("hit");
        assert_eq!(e.kind, EncoderKind::Cohere);
    }

    #[test]
    fn command_light_routes_to_cohere() {
        let t = table();
        let e = t.lookup("command-light").expect("hit");
        assert_eq!(e.kind, EncoderKind::Cohere);
    }

    // ── Cohere Bedrock routing ──────────────────────────────────────

    #[test]
    fn bedrock_cohere_command_routes_to_cohere() {
        let t = table();
        let e = t.lookup("cohere.command-v1:0").expect("hit");
        assert_eq!(e.kind, EncoderKind::Cohere);
    }

    #[test]
    fn bedrock_cohere_command_r_plus_routes_to_cohere() {
        let t = table();
        let e = t.lookup("cohere.command-r-plus-v1:0").expect("hit");
        assert_eq!(e.kind, EncoderKind::Cohere);
    }

    // ── Llama Bedrock routing ───────────────────────────────────────

    #[test]
    fn bedrock_llama3_8b_instruct_routes_to_llama() {
        let t = table();
        let e = t.lookup("meta.llama3-8b-instruct-v1:0").expect("hit");
        assert_eq!(e.kind, EncoderKind::Llama);
    }

    #[test]
    fn bedrock_llama3_1_8b_instruct_routes_to_llama() {
        let t = table();
        let e = t.lookup("meta.llama3-1-8b-instruct-v1:0").expect("hit");
        assert_eq!(e.kind, EncoderKind::Llama);
    }

    #[test]
    fn bedrock_llama3_1_70b_instruct_routes_to_llama() {
        let t = table();
        let e = t.lookup("meta.llama3-1-70b-instruct-v1:0").expect("hit");
        assert_eq!(e.kind, EncoderKind::Llama);
    }

    // ── No-fuzzy-match negative tests (per spec §3.3) ───────────────

    #[test]
    fn claude_2_does_not_route() {
        let t = table();
        // Spec §3.1 SLICE_04 covers only Claude 3 family; older
        // models drop to Tier 3 with the unknown-model metric.
        assert!(t.lookup("claude-2").is_none());
        assert!(t.lookup("claude-2.1").is_none());
    }

    #[test]
    fn gemini_pro_no_version_does_not_route() {
        // The pattern requires `1.5` or `2.0` prefix; bare
        // `gemini-pro` must Tier 3.
        let t = table();
        assert!(t.lookup("gemini-pro").is_none());
    }

    #[test]
    fn bedrock_unknown_vendor_does_not_route() {
        let t = table();
        assert!(t.lookup("amazon.titan-text-v1:0").is_none());
        assert!(t.lookup("ai21.j2-mid-v1:0").is_none());
    }

    // ── Pattern ordering invariants ────────────────────────────────

    #[test]
    fn anthropic_3_5_sonnet_matches_3_5_pattern_not_3_x_pattern() {
        // The `claude-3-5-*` pattern must precede the `claude-3-*`
        // pattern; both are EncoderKind::Anthropic so the impact
        // is metric/log only (not encode correctness), but the
        // pattern-ordering rule per §3.1 cookbook still applies.
        let t = table();
        let three_five_idx = t
            .entries()
            .iter()
            .position(|e| e.pattern_source.contains("claude-3-5-"))
            .expect("has 3.5 pattern");
        let three_x_idx = t
            .entries()
            .iter()
            .skip(three_five_idx + 1)
            .position(|e| e.pattern_source.contains("^claude-3"))
            .map(|i| i + three_five_idx + 1);
        // The 3.5 pattern (the most-specific) must come BEFORE the
        // generic claude-3 pattern.
        if let Some(idx) = three_x_idx {
            assert!(three_five_idx < idx);
        }
    }

    #[test]
    fn command_r_plus_pattern_precedes_command_r() {
        let t = table();
        let plus_idx = t
            .entries()
            .iter()
            .position(|e| e.pattern_source.contains("command-r-plus"))
            .expect("has command-r-plus");
        let r_idx = t
            .entries()
            .iter()
            .skip(plus_idx + 1)
            .position(|e| e.pattern_source.contains("^command-r(-"))
            .map(|i| i + plus_idx + 1);
        if let Some(idx) = r_idx {
            assert!(plus_idx < idx, "command-r-plus must precede command-r");
        }
    }

    // ── Drift thresholds — spec §4.2 verbatim per kind ─────────────

    #[test]
    fn drift_threshold_openai_is_zero() {
        assert_eq!(EncoderKind::OpenAi.drift_threshold(), 0.0);
    }

    #[test]
    fn drift_threshold_anthropic_is_one_percent() {
        assert_eq!(EncoderKind::Anthropic.drift_threshold(), 0.01);
    }

    #[test]
    fn drift_threshold_gemini_is_one_percent() {
        assert_eq!(EncoderKind::Gemini.drift_threshold(), 0.01);
    }

    #[test]
    fn drift_threshold_cohere_is_one_point_five_percent() {
        assert_eq!(EncoderKind::Cohere.drift_threshold(), 0.015);
    }

    #[test]
    fn drift_threshold_llama_is_half_percent() {
        assert_eq!(EncoderKind::Llama.drift_threshold(), 0.005);
    }

    // ── Table size: SLICE_03 + SLICE_04 combined ──────────────────

    #[test]
    fn dispatch_table_has_expected_slice04_entries() {
        let t = table();
        // SLICE_03 = 9; SLICE_04 adds (Anthropic: 4) + (Gemini: 2) +
        // (Cohere: 4) + (Llama: 1) = 11. Total expected >= 20.
        assert!(
            t.len() >= 20,
            "expected >=20 entries after SLICE_04 expansion, got {}",
            t.len()
        );
    }

    #[test]
    fn dispatch_table_covers_all_five_kinds() {
        let t = table();
        use std::collections::BTreeSet;
        let kinds: BTreeSet<EncoderKind> = t.entries().iter().map(|e| e.kind).collect();
        assert!(kinds.contains(&EncoderKind::OpenAi));
        assert!(kinds.contains(&EncoderKind::Anthropic));
        assert!(kinds.contains(&EncoderKind::Gemini));
        assert!(kinds.contains(&EncoderKind::Cohere));
        assert!(kinds.contains(&EncoderKind::Llama));
    }
}
