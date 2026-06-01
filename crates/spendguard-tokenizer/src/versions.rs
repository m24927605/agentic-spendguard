//! `tokenizer_versions` registry helpers.
//!
//! Spec refs:
//!   - `tokenizer-service-spec-v1alpha1.md` §6 (canonical schema)
//!   - `audit-chain-prediction-extension-v1alpha1.md` §2.1
//!     (`tokenizer_version_id` audit column targets this row)
//!   - `services/ledger/migrations/0048_tokenizer_versions.sql`
//!     (SLICE_01 substrate — DDL + immutability triggers; this slice
//!     contributes the SEED rows via 0049).
//!
//! The library does NOT touch the database directly — the seed rows
//! are inserted by migration 0049. This module owns:
//!
//!   1. **Stable UUIDv7 constants** for each shipped encoder so the
//!      library, the gRPC service, and the seed migration all agree
//!      on the same `tokenizer_version_id`. The IDs are minted once
//!      (deterministic; recorded here as constants) so that
//!      replaying audit rows from a prior deployment reproduces the
//!      same FK target.
//!   2. **`initial_seed_rows()`** — Rust-side representation of the
//!      rows the 0049 migration inserts. Used by:
//!        a. The seed migration test fixture (rust-side audit of the
//!           SQL).
//!        b. The library's runtime mapping of `EncoderKind` →
//!           `tokenizer_version_id` when populating
//!           `TokenizeResponse.tokenizer_version_id`.

/// Convenience type alias — the column in `tokenizer_versions` and
/// the proto field are both UUIDv7 strings.
pub type TokenizerVersionId = String;

/// Sentinel used by [`crate::tier3::tier3_fallback`] to express
/// "Tier 3 hit — `tokenizer_version_id` is NULL on the audit row".
/// The mirror crate (`spendguard-prediction-mirror`) translates this
/// empty string to / from SQL NULL and proto3 default per
/// `audit-chain-prediction-extension-v1alpha1.md` §3.3.
pub const HEURISTIC_FALLBACK_VERSION_ID: &str = "";

// ============================================================================
// Stable UUIDv7 constants for SLICE_03 seed rows.
//
// These are minted with explicit UUIDv7 bytes (deterministic) so the
// 0049 SQL migration, the library mapping, and any verify-chain
// reproduction read the same values byte-for-byte.
//
// Convention: these IDs are stable application-minted UUIDv7-shaped
// constants. The timestamp half is decorative: the current prefix
// decodes to 2024-08-23T16:09:29.344Z, not the SLICE_03 ship date.
// Do not infer registration time from these UUIDs; use
// tokenizer_versions.registered_at for operator-visible chronology. The
// random tail is a kind-specific hex signature so the IDs are visually
// distinguishable in audit dumps.
//
// ## UUIDv7 layout (RFC 9562 §5.7)
//
//   xxxxxxxx-xxxx-7xxx-Vxxx-xxxxxxxxxxxx
//                     ^
//                     variant nibble — MUST be 8/9/a/b (10xx2 prefix)
//
// Round-2 fix B2 (panel finding): the original constants used `0` for
// the variant nibble (NCS-reserved range), which fails RFC 9562
// `Uuid::get_variant() == Variant::RFC4122` checks downstream. We
// re-mint the constants with variant `8` (the simplest deterministic
// choice in the 10xx2 range). Since SLICE_03 has not yet shipped any
// audit_outbox rows that FK to these IDs, no data migration is needed
// — the rotation is purely textual across `versions.rs` + the 0049
// seed SQL.
// ============================================================================

/// tiktoken-rs cl100k_base — used by gpt-4 / gpt-4-turbo / gpt-3.5-turbo.
pub const TIKTOKEN_CL100K_BASE_VERSION_ID: &str = "01918000-0000-7c10-8c10-000000000001";

/// tiktoken-rs o200k_base — used by gpt-4o / gpt-4o-mini.
pub const TIKTOKEN_O200K_BASE_VERSION_ID: &str = "01918000-0000-7c10-8c10-000000000002";

/// tiktoken-rs p50k_base — used by text-davinci-003 family.
pub const TIKTOKEN_P50K_BASE_VERSION_ID: &str = "01918000-0000-7c10-8c10-000000000003";

/// HEURISTIC marker row — the registry row that exists for
/// `audit_outbox` joins to identify the Tier-3 fallback path
/// explicitly (the FK column itself is NULL on Tier 3 rows; this row
/// exists for the calibration-report CLI and is referenced by the
/// `HEURISTIC` kind variant in `tokenizer_versions.kind`).
pub const HEURISTIC_MARKER_VERSION_ID: &str = "01918000-0000-7c10-8c10-00000000000f";

// ============================================================================
// SLICE_04 — Tier 2 expansion (Anthropic + Cohere + Gemini + Llama).
//
// Same UUIDv7-shaped minting convention as SLICE_03:
//   * Timestamp half: decorative stable prefix; currently decodes to
//     2024-08-23T16:09:29.344Z and is not a registration timestamp.
//   * Version nibble: 7 (RFC 9562 §5.7).
//   * Variant nibble: 8 (the simplest deterministic 10xx2 choice;
//     also what SLICE_03 R2 B2 standardised on after the original
//     mint used `0`).
//   * Random tail: hex signature distinct per encoder kind so audit
//     dumps visually group the SLICE_04 rows.
//
// Each constant MUST byte-match the corresponding INSERT in
// `services/ledger/migrations/0050_tokenizer_versions_slice04_seed.sql`.
// The seed_parity tests assert byte-for-byte agreement so a future
// edit that drifts either side fails CI loudly.
// ============================================================================

/// Anthropic Claude 3 / 3.5 BPE — covers `claude-3-*`, `claude-3-5-*`,
/// and the Bedrock-routed `anthropic.claude-*` family.
pub const ANTHROPIC_CLAUDE3_VERSION_ID: &str = "01918000-0000-7c10-8c10-000000000004";

/// Google Gemini 1.5 / 2.0 BPE (community Gemma approximation; see
/// `encoders/gemini.rs` for the §4.2 drift threshold rationale).
pub const GEMINI_15_VERSION_ID: &str = "01918000-0000-7c10-8c10-000000000005";

/// Cohere Command-R BPE — covers `command-r*`, `cohere.*` Bedrock.
pub const COHERE_COMMAND_R_VERSION_ID: &str = "01918000-0000-7c10-8c10-000000000006";

/// Meta Llama 3.1 SentencePiece — covers `meta.llama*` Bedrock.
pub const LLAMA_31_VERSION_ID: &str = "01918000-0000-7c10-8c10-000000000007";

/// Rust-side mirror of a `tokenizer_versions` SQL row. Used by the
/// seed migration audit fixture (`tests/seed_parity.rs`) so a future
/// edit to the SQL is caught against the library's expectations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenizerVersionRow {
    pub tokenizer_version_id: &'static str,
    /// One of the tokenizer_versions.kind CHECK values.
    pub kind: &'static str,
    pub encoder_name: &'static str,
    pub version_string: &'static str,
    pub asset_sha256: &'static str,
}

/// The exact rows inserted by `0049_tokenizer_versions_initial_seed.sql`.
///
/// SLICE_04 ships a SEPARATE seed migration (0050) with
/// [`slice04_seed_rows`]; this function is **append-frozen** so existing
/// audit_outbox FKs referencing SLICE_03 rows always resolve. The
/// HEURISTIC marker row has a per-spec sha256 placeholder (zeros)
/// because there is no asset bundle — the row exists purely for FK +
/// kind enumeration.
pub fn initial_seed_rows() -> [TokenizerVersionRow; 4] {
    [
        TokenizerVersionRow {
            tokenizer_version_id: TIKTOKEN_CL100K_BASE_VERSION_ID,
            kind: "OPENAI_TIKTOKEN",
            encoder_name: "cl100k_base",
            version_string: "tiktoken-rs-0.11.0",
            asset_sha256: crate::asset_sha256::CL100K_BASE,
        },
        TokenizerVersionRow {
            tokenizer_version_id: TIKTOKEN_O200K_BASE_VERSION_ID,
            kind: "OPENAI_TIKTOKEN",
            encoder_name: "o200k_base",
            version_string: "tiktoken-rs-0.11.0",
            asset_sha256: crate::asset_sha256::O200K_BASE,
        },
        TokenizerVersionRow {
            tokenizer_version_id: TIKTOKEN_P50K_BASE_VERSION_ID,
            kind: "OPENAI_TIKTOKEN",
            encoder_name: "p50k_base",
            version_string: "tiktoken-rs-0.11.0",
            asset_sha256: crate::asset_sha256::P50K_BASE,
        },
        TokenizerVersionRow {
            tokenizer_version_id: HEURISTIC_MARKER_VERSION_ID,
            kind: "HEURISTIC",
            encoder_name: "chars_div_4_with_5pct_margin",
            version_string: "spec-v1alpha1",
            // No asset → sha256 of empty string. (Encoded as a
            // string literal so the SQL seed lines up byte-for-byte
            // with the migration constant.)
            asset_sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        },
    ]
}

/// The exact rows inserted by
/// `0050_tokenizer_versions_slice04_seed.sql`.
///
/// SLICE_04 ships 4 new encoder rows (Anthropic + Gemini + Cohere +
/// Llama) and is purely additive; existing SLICE_03 FK targets are
/// unmodified. The version_string values use vendor-revision-style
/// tags pinned to the 2026-05-30 Hugging Face snapshots vendored
/// at `crates/spendguard-tokenizer/data/<vendor>/tokenizer.json`.
///
/// Per spec §6.2 a future refresh of the vendored `tokenizer.json`
/// requires:
///   1. New constant in this array (new UUIDv7 mint).
///   2. New `INSERT` in a follow-up migration (not 0050).
///   3. The SLICE_03 OPENAI rows stay unchanged.
pub fn slice04_seed_rows() -> [TokenizerVersionRow; 4] {
    [
        TokenizerVersionRow {
            tokenizer_version_id: ANTHROPIC_CLAUDE3_VERSION_ID,
            kind: "ANTHROPIC_BPE",
            encoder_name: "anthropic-v3-bpe",
            version_string: "xenova-claude-tokenizer-2026-05-30",
            asset_sha256: crate::asset_sha256::ANTHROPIC_CLAUDE3,
        },
        TokenizerVersionRow {
            tokenizer_version_id: GEMINI_15_VERSION_ID,
            kind: "GEMINI_BPE",
            encoder_name: "gemini-1.5-bpe",
            version_string: "xenova-gemma-tokenizer-2026-05-30",
            asset_sha256: crate::asset_sha256::GEMINI_15,
        },
        TokenizerVersionRow {
            tokenizer_version_id: COHERE_COMMAND_R_VERSION_ID,
            kind: "COHERE_BPE",
            encoder_name: "cohere-v2-bpe",
            version_string: "xenova-c4ai-command-r-2026-05-30",
            asset_sha256: crate::asset_sha256::COHERE_COMMAND_R,
        },
        TokenizerVersionRow {
            tokenizer_version_id: LLAMA_31_VERSION_ID,
            kind: "SENTENCEPIECE_LLAMA",
            encoder_name: "llama-sentencepiece",
            version_string: "xenova-llama-3.1-tokenizer-2026-05-30",
            asset_sha256: crate::asset_sha256::LLAMA_31,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_rows_have_unique_ids() {
        let rows = initial_seed_rows();
        let mut ids: Vec<&str> = rows.iter().map(|r| r.tokenizer_version_id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), rows.len(), "tokenizer_version_ids must be unique");
    }

    #[test]
    fn seed_rows_have_valid_uuids() {
        for row in initial_seed_rows() {
            uuid::Uuid::parse_str(row.tokenizer_version_id)
                .unwrap_or_else(|e| panic!("invalid UUID `{}`: {e}", row.tokenizer_version_id));
        }
    }

    #[test]
    fn seed_rows_have_valid_uuidv7_per_rfc_9562() {
        // Round-2 fix B2 (panel finding): the four constants must be
        // syntactically valid UUIDv7 per RFC 9562 §5.7 — version nibble
        // == 7 AND variant nibble in {8, 9, a, b} (the 10xx2 range).
        // The original SLICE_03 mint used variant `0` (NCS-reserved)
        // which would fail downstream `Uuid::get_variant() ==
        // Variant::RFC4122` checks. This test pins the fix so a future
        // edit that drifts back to an invalid variant fails loudly.
        use uuid::{Variant, Version};
        for row in initial_seed_rows() {
            let uuid = uuid::Uuid::parse_str(row.tokenizer_version_id)
                .unwrap_or_else(|e| panic!("invalid UUID `{}`: {e}", row.tokenizer_version_id));
            assert_eq!(
                uuid.get_version(),
                Some(Version::SortRand),
                "row id `{}` must be UUIDv7 (version nibble == 7) per spec §6.1",
                row.tokenizer_version_id
            );
            assert_eq!(
                uuid.get_variant(),
                Variant::RFC4122,
                "row id `{}` must have RFC4122 variant (10xx2 / 8|9|a|b) per RFC 9562 §5.7",
                row.tokenizer_version_id
            );
        }
    }

    #[test]
    fn seed_rows_satisfy_kind_check_constraint() {
        const VALID_KINDS: &[&str] = &[
            "OPENAI_TIKTOKEN",
            "ANTHROPIC_BPE",
            "GEMINI_BPE",
            "COHERE_BPE",
            "SENTENCEPIECE_LLAMA",
            "HEURISTIC",
        ];
        for row in initial_seed_rows() {
            assert!(
                VALID_KINDS.contains(&row.kind),
                "kind `{}` not in tokenizer_versions.kind CHECK list",
                row.kind
            );
        }
    }

    #[test]
    fn seed_rows_have_unique_kind_encoder_version_tuple() {
        // Matches the SQL UNIQUE (kind, encoder_name, version_string).
        let rows = initial_seed_rows();
        let mut tuples: Vec<(&str, &str, &str)> = rows
            .iter()
            .map(|r| (r.kind, r.encoder_name, r.version_string))
            .collect();
        tuples.sort();
        tuples.dedup();
        assert_eq!(tuples.len(), rows.len(), "UNIQUE constraint pre-check failed");
    }

    // ── SLICE_04 — Tier 2 expansion seed-row parity tests ─────────

    #[test]
    fn slice04_rows_have_unique_ids() {
        let rows = slice04_seed_rows();
        let mut ids: Vec<&str> = rows.iter().map(|r| r.tokenizer_version_id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), rows.len(), "SLICE_04 ids must be unique");
    }

    #[test]
    fn slice04_rows_have_valid_uuidv7_per_rfc_9562() {
        use uuid::{Variant, Version};
        for row in slice04_seed_rows() {
            let uuid = uuid::Uuid::parse_str(row.tokenizer_version_id)
                .unwrap_or_else(|e| panic!("invalid UUID `{}`: {e}", row.tokenizer_version_id));
            assert_eq!(
                uuid.get_version(),
                Some(Version::SortRand),
                "row id `{}` must be UUIDv7 (version nibble == 7) per spec §6.1",
                row.tokenizer_version_id
            );
            assert_eq!(
                uuid.get_variant(),
                Variant::RFC4122,
                "row id `{}` must have RFC4122 variant per RFC 9562 §5.7",
                row.tokenizer_version_id
            );
        }
    }

    #[test]
    fn slice04_rows_satisfy_kind_check_constraint() {
        const VALID_KINDS: &[&str] = &[
            "OPENAI_TIKTOKEN",
            "ANTHROPIC_BPE",
            "GEMINI_BPE",
            "COHERE_BPE",
            "SENTENCEPIECE_LLAMA",
            "HEURISTIC",
        ];
        for row in slice04_seed_rows() {
            assert!(
                VALID_KINDS.contains(&row.kind),
                "kind `{}` not in tokenizer_versions.kind CHECK list",
                row.kind
            );
        }
    }

    #[test]
    fn slice04_rows_have_unique_kind_encoder_version_tuple() {
        let rows = slice04_seed_rows();
        let mut tuples: Vec<(&str, &str, &str)> = rows
            .iter()
            .map(|r| (r.kind, r.encoder_name, r.version_string))
            .collect();
        tuples.sort();
        tuples.dedup();
        assert_eq!(tuples.len(), rows.len(), "UNIQUE constraint pre-check failed");
    }

    #[test]
    fn slice03_and_slice04_ids_dont_collide() {
        // Audit-chain invariant: SLICE_03 + SLICE_04 row IDs must
        // be globally unique across the entire `tokenizer_versions`
        // table. A duplicate ID would cause INSERT OR an FK lookup
        // ambiguity.
        let mut all_ids: Vec<&str> = initial_seed_rows()
            .iter()
            .map(|r| r.tokenizer_version_id)
            .chain(slice04_seed_rows().iter().map(|r| r.tokenizer_version_id))
            .collect();
        all_ids.sort();
        all_ids.dedup();
        assert_eq!(
            all_ids.len(),
            initial_seed_rows().len() + slice04_seed_rows().len(),
            "SLICE_03 + SLICE_04 row IDs must not collide"
        );
    }

    #[test]
    fn slice04_covers_all_new_encoder_kinds() {
        // Sanity: SLICE_04 ships exactly one row per new encoder
        // kind (Anthropic / Gemini / Cohere / Llama).
        let rows = slice04_seed_rows();
        let kinds: std::collections::BTreeSet<&str> =
            rows.iter().map(|r| r.kind).collect();
        let expected: std::collections::BTreeSet<&str> = [
            "ANTHROPIC_BPE",
            "GEMINI_BPE",
            "COHERE_BPE",
            "SENTENCEPIECE_LLAMA",
        ]
        .iter()
        .copied()
        .collect();
        assert_eq!(kinds, expected, "SLICE_04 must cover the 4 new kinds");
    }
}
