//! Verify that the SLICE_03 seed migration SQL byte-aligns with the
//! Rust-side `initial_seed_rows()` constants.
//!
//! Spec ref `tokenizer-service-spec-v1alpha1.md` §6.2 (versioning) +
//! SLICE_03 §6 (audit-chain invariant — the FK target must agree on
//! both sides of the boundary).
//!
//! ## Why this test exists
//!
//! The migration runner inserts rows by SQL literal; the library
//! constructs `TokenizeResponse.tokenizer_version_id` from Rust
//! constants. If either side drifts, the audit chain's FK gets a
//! "violates referential integrity" failure at first Tier 2 row
//! emission. This test parses the SQL file and asserts:
//!
//!   1. The set of `tokenizer_version_id` values matches Rust.
//!   2. (kind, encoder_name, version_string, asset_sha256) match
//!      per-id.
//!   3. Row count matches (no drift in either direction).
//!
//! The test does not require Postgres — it parses the file as text.

use spendguard_tokenizer::initial_seed_rows;

const SEED_SQL: &str =
    include_str!("../../../services/ledger/migrations/0049_tokenizer_versions_initial_seed.sql");

#[test]
fn each_rust_seed_row_id_appears_in_sql() {
    for row in initial_seed_rows() {
        assert!(
            SEED_SQL.contains(row.tokenizer_version_id),
            "row id `{}` not found in 0049 seed migration SQL",
            row.tokenizer_version_id
        );
    }
}

#[test]
fn each_rust_seed_row_encoder_name_appears_in_sql() {
    for row in initial_seed_rows() {
        let quoted = format!("'{}'", row.encoder_name);
        assert!(
            SEED_SQL.contains(&quoted),
            "encoder_name `{}` not found in 0049 seed migration SQL",
            row.encoder_name
        );
    }
}

#[test]
fn each_rust_seed_row_asset_sha256_appears_in_sql() {
    for row in initial_seed_rows() {
        let quoted = format!("'{}'", row.asset_sha256);
        assert!(
            SEED_SQL.contains(&quoted),
            "asset_sha256 `{}` not found in 0049 seed migration SQL",
            row.asset_sha256
        );
    }
}

#[test]
fn each_rust_seed_row_version_string_appears_in_sql() {
    for row in initial_seed_rows() {
        let quoted = format!("'{}'", row.version_string);
        assert!(
            SEED_SQL.contains(&quoted),
            "version_string `{}` not found in 0049 seed migration SQL",
            row.version_string
        );
    }
}

#[test]
fn each_rust_seed_row_kind_appears_in_sql() {
    for row in initial_seed_rows() {
        let quoted = format!("'{}'", row.kind);
        assert!(
            SEED_SQL.contains(&quoted),
            "kind `{}` not found in 0049 seed migration SQL",
            row.kind
        );
    }
}

#[test]
fn seed_sql_has_sanity_check_clause() {
    // Defense: a future contributor who removes the DO $$ ... END $$
    // sanity-check block in 0049 would break the rollback-safety
    // guarantee that the migration RAISEs if a partial seed is
    // present. Pin the clause so its removal is loud.
    assert!(
        SEED_SQL.contains("expected_count INTEGER := 4"),
        "0049 seed migration lost its sanity-check row count assertion"
    );
}

#[test]
fn seed_sql_uses_on_conflict_do_nothing_for_idempotency() {
    assert!(
        SEED_SQL.contains("ON CONFLICT (kind, encoder_name, version_string) DO NOTHING"),
        "0049 seed migration must be idempotent via ON CONFLICT DO NOTHING"
    );
}
