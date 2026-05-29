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
// Convention: the timestamp half is 2026-05-30 00:00:00 UTC (the day
// SLICE_03 ships) and the random tail is a kind-specific hex
// signature so the IDs sort by ship-date and are visually
// distinguishable in audit dumps.
// ============================================================================

/// tiktoken-rs cl100k_base — used by gpt-4 / gpt-4-turbo / gpt-3.5-turbo.
pub const TIKTOKEN_CL100K_BASE_VERSION_ID: &str = "01918000-0000-7c10-0c10-000000000001";

/// tiktoken-rs o200k_base — used by gpt-4o / gpt-4o-mini.
pub const TIKTOKEN_O200K_BASE_VERSION_ID: &str = "01918000-0000-7c10-0c10-000000000002";

/// tiktoken-rs p50k_base — used by text-davinci-003 family.
pub const TIKTOKEN_P50K_BASE_VERSION_ID: &str = "01918000-0000-7c10-0c10-000000000003";

/// HEURISTIC marker row — the registry row that exists for
/// `audit_outbox` joins to identify the Tier-3 fallback path
/// explicitly (the FK column itself is NULL on Tier 3 rows; this row
/// exists for the calibration-report CLI and is referenced by the
/// `HEURISTIC` kind variant in `tokenizer_versions.kind`).
pub const HEURISTIC_MARKER_VERSION_ID: &str = "01918000-0000-7c10-0c10-00000000000f";

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
/// SLICE_04 will extend this with Anthropic / Gemini / Cohere /
/// SentencePiece rows. The HEURISTIC marker row has a per-spec
/// sha256 placeholder (zeros) because there is no asset bundle —
/// the row exists purely for FK + kind enumeration.
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
}
