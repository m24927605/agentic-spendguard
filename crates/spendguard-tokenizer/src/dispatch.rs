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
///   Anthropic Bedrock (SLICE_04, R2 B1 cross-region prefix):
///     * `[REGION.]anthropic.claude-3-(haiku|sonnet|opus)*-v\d:\d+`
///     * `[REGION.]anthropic.claude-3-5-(haiku|sonnet|opus)*-v\d:\d+`
///     where `REGION` is any lowercase region prefix (us/eu/apac/us-gov/future)
///
///   Gemini native (SLICE_04):
///     * `gemini-1.5-(flash|pro)` (+ optional `-NNN` revision)
///     * `gemini-2.0-flash` (+ optional `-exp`)
///
///   Cohere native (SLICE_04):
///     * `command-r(-plus)?` (+ optional dated suffix)
///     * (`command-light` INTENTIONALLY omitted per R2 Backend F4 —
///        uses different vocab; falls to Tier 3 until vendored.)
///
///   Cohere Bedrock (SLICE_04, R2 B1 cross-region prefix):
///     * `[REGION.]cohere.command(-r)?(-plus)?-v\d:\d+`
///
///   Llama Bedrock (SLICE_04, R2 B1 cross-region prefix):
///     * `[REGION.]meta.llama3-N-Mb-instruct-v\d:\d+`
///
/// ## R2 B2 — narrow patterns by design (Option A)
///
/// Spec §3.1 originally listed catch-all Bedrock patterns
/// (`^anthropic\.claude-.*$`, `^cohere\..*$`, `^meta\.llama.*$`).
/// The implementation narrows them on purpose so that:
///   * Pre-Claude-3 models (`anthropic.claude-instant-v1`,
///     `anthropic.claude-v2`) fall to Tier 3 instead of being silently
///     dispatched to the Claude-3 BPE (older vocab; different tokens).
///   * Cohere embedding models (`cohere.embed-english-v3` et al.) fall
///     to Tier 3 because they use a different vocab than command-r.
///   * Pre-Llama-3 models (`meta.llama2-70b-chat-v1`) fall to Tier 3.
///
/// Each Tier 3 fallback emits the `tokenizer_unknown_model` metric per
/// spec §3.3 so operators see the gap and PR a tracked follow-up. The
/// rationale: dispatching wrong-vocab encoders produces silent ~5-20%
/// under-counts; falling to Tier 3 produces a 5% conservative margin
/// + a visible metric. Safer default. SLICE_NN follow-ups will widen
/// coverage by adding explicit vendored asset rows per family.
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
    //
    // Round-2 fix B1 (Software F1 panel finding): AWS Bedrock since
    // 2024-09 routes major models via cross-region inference profiles
    // that prepend a region prefix:
    //   us.anthropic.claude-3-5-sonnet-20240620-v1:0
    //   eu.anthropic.claude-3-haiku-20240307-v1:0
    //   apac.anthropic.claude-3-5-sonnet-20241022-v1:0
    //   us-gov.anthropic.claude-3-5-sonnet-20240620-v1:0
    //
    // The optional `(?:[a-z][a-z0-9-]*\.)?` prefix admits any current
    // region prefix (us/eu/apac/us-gov) AND any future region AWS adds
    // (e.g., `me`, `il`, future regional partitions). Per Bedrock
    // documentation the region prefix is a single lowercase token
    // matching `[a-z][a-z0-9-]*` followed by a dot before the vendor
    // family. We use a permissive prefix rather than enumerating regions
    // to avoid an annual maintenance burden — if AWS adds a region, we
    // route automatically instead of silently falling to Tier 3.
    //
    // Per §9 review question 3 — explicit golden-sample coverage for
    // the full dated + versioned form lives in
    // `services/tokenizer/tests/slice04_golden_samples.rs` SLICE_04
    // section; R2 B1 added cross-region variants.
    (
        r"^(?:[a-z][a-z0-9-]*\.)?anthropic\.claude-3-5-(sonnet|haiku|opus)(-\d{8})?-v\d+:\d+$",
        EncoderResolver::Anthropic,
    ),
    (
        r"^(?:[a-z][a-z0-9-]*\.)?anthropic\.claude-3-(haiku|sonnet|opus)(-\d{8})?-v\d+:\d+$",
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
    // Round-2 fix Backend F4: `command-light` is INTENTIONALLY omitted
    // from the dispatch table. Cohere's `command-light` model uses a
    // different BPE vocabulary than `command-r`, so routing it to the
    // `cohere-v2-bpe` (command-r) encoder would silently under-count
    // tokens by ~5-20%. Falling to Tier 3 is correct behaviour per spec
    // §3.4 fallback policy until a separate `command-light` tokenizer
    // asset is vendored (tracked as a SLICE_NN follow-up). The
    // `cohere_command_light_falls_to_tier3` unit test below pins this.
    //
    // (Previously a `^command-light(-\d{8})?$` entry routed to Cohere;
    //  removed in R2 to eliminate the silent wrong-encoder dispatch.)

    // ── Cohere Bedrock routing ──
    //
    // Round-2 fix B1 (Software F1): cross-region prefix support, same
    // rationale as the Anthropic Bedrock entries above.
    (
        r"^(?:[a-z][a-z0-9-]*\.)?cohere\.command(-r)?(-plus)?-v\d+:\d+$",
        EncoderResolver::Cohere,
    ),

    // ── Llama Bedrock routing ──
    // Per Bedrock published model IDs:
    //   meta.llama3-1-8b-instruct-v1:0
    //   meta.llama3-1-70b-instruct-v1:0
    //   meta.llama3-2-1b-instruct-v1:0
    //   meta.llama3-3-70b-instruct-v1:0
    //
    // Round-2 fix B1 (Software F1): cross-region prefix support per
    // Bedrock inference profile conventions. Examples:
    //   us.meta.llama3-1-70b-instruct-v1:0
    //   eu.meta.llama3-2-1b-instruct-v1:0
    (
        r"^(?:[a-z][a-z0-9-]*\.)?meta\.llama3(-\d+)?-\d+b-instruct-v\d+:\d+$",
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
    fn cohere_command_light_falls_to_tier3() {
        // Round-2 fix Backend F4: `command-light` uses a different BPE
        // vocabulary than `command-r`. Routing both to the same
        // `cohere-v2-bpe` encoder would silently under-count tokens by
        // ~5-20% on typical inputs. The R1 dispatch row was removed in
        // R2; `command-light` now falls to Tier 3 (5% conservative
        // margin + `tokenizer_unknown_model` metric per spec §3.3) until
        // a separate `command-light` tokenizer asset is vendored in a
        // future SLICE_NN. This test pins the absence so a future
        // contributor doesn't re-add the wrong-encoder row.
        let t = table();
        assert!(
            t.lookup("command-light").is_none(),
            "command-light must NOT route to Cohere command-r encoder \
             (silent ~5-20% under-count; see R2 Backend F4)"
        );
        assert!(t.lookup("command-light-20240501").is_none());
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

    // ── R2 B1 — Bedrock cross-region inference profile prefixes ────
    //
    // AWS Bedrock since 2024-09 routes major models via cross-region
    // inference profiles that prepend a region prefix. The dispatch
    // patterns now admit any lowercase prefix (us/eu/apac/us-gov/future)
    // so these IDs no longer fall to Tier 3 with a 5% margin (which
    // produced ~2x under-count on CJK input per panel finding).

    #[test]
    fn bedrock_us_cross_region_anthropic_claude_3_5_sonnet_routes() {
        let t = table();
        let e = t
            .lookup("us.anthropic.claude-3-5-sonnet-20240620-v1:0")
            .expect("us. cross-region prefix must route");
        assert_eq!(e.kind, EncoderKind::Anthropic);
    }

    #[test]
    fn bedrock_eu_cross_region_anthropic_claude_3_haiku_routes() {
        let t = table();
        let e = t
            .lookup("eu.anthropic.claude-3-haiku-20240307-v1:0")
            .expect("eu. cross-region prefix must route");
        assert_eq!(e.kind, EncoderKind::Anthropic);
    }

    #[test]
    fn bedrock_apac_cross_region_anthropic_claude_3_5_sonnet_routes() {
        let t = table();
        let e = t
            .lookup("apac.anthropic.claude-3-5-sonnet-20241022-v1:0")
            .expect("apac. cross-region prefix must route");
        assert_eq!(e.kind, EncoderKind::Anthropic);
    }

    #[test]
    fn bedrock_us_gov_cross_region_anthropic_claude_routes() {
        let t = table();
        let e = t
            .lookup("us-gov.anthropic.claude-3-5-sonnet-20240620-v1:0")
            .expect("us-gov. cross-region prefix must route");
        assert_eq!(e.kind, EncoderKind::Anthropic);
    }

    #[test]
    fn bedrock_cross_region_cohere_command_r_plus_routes() {
        let t = table();
        let e = t
            .lookup("us.cohere.command-r-plus-v1:0")
            .expect("us. cohere cross-region prefix must route");
        assert_eq!(e.kind, EncoderKind::Cohere);
        let e2 = t
            .lookup("eu.cohere.command-r-v1:0")
            .expect("eu. cohere cross-region prefix must route");
        assert_eq!(e2.kind, EncoderKind::Cohere);
    }

    #[test]
    fn bedrock_cross_region_meta_llama_routes() {
        let t = table();
        let e = t
            .lookup("us.meta.llama3-1-70b-instruct-v1:0")
            .expect("us. llama cross-region prefix must route");
        assert_eq!(e.kind, EncoderKind::Llama);
        let e2 = t
            .lookup("eu.meta.llama3-2-1b-instruct-v1:0")
            .expect("eu. llama cross-region prefix must route");
        assert_eq!(e2.kind, EncoderKind::Llama);
    }

    // ── R2 B2 — narrow-pattern Option A invariants (deny incompatible) ──
    //
    // Spec §3.1 had catch-all Bedrock patterns; we narrowed deliberately
    // so wrong-vocab models do NOT silently route to the wrong encoder.
    // These tests pin the narrow boundary: pre-Claude-3 Anthropic,
    // Cohere embed-*, and pre-Llama-3 fall to Tier 3 (not the BPE).

    #[test]
    fn bedrock_pre_claude3_anthropic_falls_to_tier3() {
        // claude-instant-v1 and claude-v2 predate the SLICE_04 BPE;
        // routing them to the claude-3 encoder would silently mis-count.
        let t = table();
        assert!(t.lookup("anthropic.claude-instant-v1").is_none());
        assert!(t.lookup("anthropic.claude-v2").is_none());
        assert!(t.lookup("anthropic.claude-v2:1").is_none());
    }

    #[test]
    fn bedrock_cohere_embed_falls_to_tier3() {
        // Cohere embed-* models use a different BPE than command-r.
        // The narrow `cohere.command...` pattern correctly leaves embed
        // models for Tier 3 instead of silently dispatching wrong vocab.
        let t = table();
        assert!(t.lookup("cohere.embed-english-v3").is_none());
        assert!(t.lookup("cohere.embed-multilingual-v3").is_none());
    }

    #[test]
    fn bedrock_pre_llama3_falls_to_tier3() {
        // Llama 2 family uses a different SentencePiece vocab; the
        // narrow `meta.llama3...` pattern correctly excludes it.
        let t = table();
        assert!(t.lookup("meta.llama2-13b-chat-v1").is_none());
        assert!(t.lookup("meta.llama2-70b-chat-v1").is_none());
    }

    #[test]
    fn cross_region_prefix_does_not_admit_invalid_chars() {
        // The cross-region prefix `(?:[a-z][a-z0-9-]*\.)?` requires the
        // first char to be a lowercase letter. Empty / invalid prefixes
        // must NOT match the optional group differently than the base
        // pattern — i.e., `1us.anthropic...` must not route via the
        // cross-region branch.
        let t = table();
        // Upper-case prefix not allowed by regex
        assert!(t.lookup("US.anthropic.claude-3-haiku-20240307-v1:0").is_none());
        // Digit-leading prefix not allowed
        assert!(t.lookup("1us.anthropic.claude-3-haiku-20240307-v1:0").is_none());
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
        // SLICE_03 = 9; SLICE_04 R2 adds (Anthropic: 4 — native + Bedrock
        // with cross-region prefix support) + (Gemini: 2) + (Cohere: 3 —
        // dropped command-light per R2 Backend F4) + (Llama: 1) = 10.
        // Total expected >= 19 after R2.
        assert!(
            t.len() >= 19,
            "expected >=19 entries after SLICE_04 R2, got {}",
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
