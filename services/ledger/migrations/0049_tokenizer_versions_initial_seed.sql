-- Initial seed rows for tokenizer_versions registry.
--
-- Spec ancestors:
--   - tokenizer-service-spec-v1alpha1.md §6.1 (schema), §6.2 (when to register)
--   - audit-chain-prediction-extension-v1alpha1.md §2.1 (FK target column)
--   - SLICE_03 §5 (this slice's seed owner)
--
-- This migration inserts the four initial tokenizer_versions rows that
-- SLICE_03 ships:
--
--   * 3 OpenAI tiktoken-rs encoder rows (cl100k_base, o200k_base,
--     p50k_base) sourced from tiktoken-rs 0.11.0 vendored .tiktoken
--     bytes. The asset_sha256 column holds the canonical sha256 of
--     the vendored asset; the library's Tokenizer::new_with_embedded_assets()
--     constructor refuses to start if these don't match (spec §7.4).
--   * 1 HEURISTIC marker row. The audit_outbox.tokenizer_version_id
--     column itself is NULL for Tier 3 fallback rows (per
--     audit-chain extension §2.1 nullable rules); this marker row
--     exists so calibration-report SQL can JOIN on a stable kind
--     enum without a NULL coalescence trick.
--
-- INSERT order is irrelevant per the UNIQUE constraint; we order by
-- encoder_name alphabetically so a manual psql inspection reads
-- predictably.
--
-- Row identity:
--   * tokenizer_version_id is a stable application-minted UUIDv7
--     hard-coded here AND in crates/spendguard-tokenizer/src/versions.rs
--     so the library mapping and the FK target byte-match. Replaying
--     an audit_outbox row from a prior deployment reproduces the
--     same encoder lookup.
--   * (kind, encoder_name, version_string) is the UNIQUE business key
--     enforced by 0048; reseeding this migration on top of an existing
--     deployment is a no-op via ON CONFLICT DO NOTHING.
--
-- Privilege boundary:
--   0048 already revokes PUBLIC INSERT/UPDATE/DELETE and grants INSERT
--   to ledger_application_role. The migration runner role MUST be a
--   member of ledger_application_role or have BYPASSRLS — the same
--   posture all prior 0040+ migrations require.
--
-- This migration is forward-only: the immutability trigger on
-- tokenizer_versions blocks UPDATE / DELETE, so the rows here are
-- effectively permanent once inserted. To "retire" a version a future
-- slice (SLICE-extra) will introduce a retirement_log table; the
-- tokenizer_versions.retired_at column is reserved for that future
-- use but SLICE_03 never writes it.
--
-- Round-1 self-review note: the SQL is wrapped by the migration
-- runner's per-file transaction; no explicit BEGIN/COMMIT here
-- (matches 0048 convention).
--
-- Round-2 fix B2 (panel finding): the UUIDs below were re-minted with
-- variant nibble `8` (was `0` — NCS-reserved which fails RFC 9562
-- §5.7). The Rust-side constants in
-- crates/spendguard-tokenizer/src/versions.rs match byte-for-byte;
-- seed_parity.rs tests assert the rust↔SQL agreement so any future
-- drift fails CI loudly. No data fix needed since SLICE_03 ships
-- pre-FK-write (no audit_outbox rows reference these IDs yet).

INSERT INTO public.tokenizer_versions (
    tokenizer_version_id,
    kind,
    encoder_name,
    version_string,
    asset_sha256
) VALUES
    (
        '01918000-0000-7c10-8c10-000000000001'::uuid,
        'OPENAI_TIKTOKEN',
        'cl100k_base',
        'tiktoken-rs-0.11.0',
        '223921b76ee99bde995b7ff738513eef100fb51d18c93597a113bcffe865b2a7'
    ),
    (
        '01918000-0000-7c10-8c10-000000000002'::uuid,
        'OPENAI_TIKTOKEN',
        'o200k_base',
        'tiktoken-rs-0.11.0',
        '446a9538cb6c348e3516120d7c08b09f57c36495e2acfffe59a5bf8b0cfb1a2d'
    ),
    (
        '01918000-0000-7c10-8c10-000000000003'::uuid,
        'OPENAI_TIKTOKEN',
        'p50k_base',
        'tiktoken-rs-0.11.0',
        '94b5ca7dff4d00767bc256fdd1b27e5b17361d7b8a5f968547f9f23eb70d2069'
    ),
    (
        '01918000-0000-7c10-8c10-00000000000f'::uuid,
        'HEURISTIC',
        'chars_div_4_with_5pct_margin',
        'spec-v1alpha1',
        'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855'
        -- sha256 of empty bytes — HEURISTIC has no embedded asset;
        -- the row exists for FK + kind enumeration, not for
        -- integrity verification.
    )
ON CONFLICT (kind, encoder_name, version_string) DO NOTHING;

-- Sanity assertion: confirm all four rows are present after the INSERT.
-- This is defensive — the ON CONFLICT clause silently no-ops on
-- re-run, so a partial pre-existing seed (3 of 4 rows) would leave
-- the deployment in a state where the library expects to map to a
-- missing FK target. RAISE EXCEPTION fails the migration so the
-- runner rolls back rather than ship a broken registry.
--
-- Round-2 fix M2 (panel finding): `SET LOCAL search_path` inside the
-- DO body forces plpgsql to resolve COUNT / RAISE / etc against
-- pg_catalog only — CVE-2018-1058 hardening that matches the SLICE_01
-- R5 convention. SET LOCAL works here because the DO body runs inside
-- a single tx frame; this differs from autocommit-scoped destructive-
-- down GUC opt-ins (those need SET — see 0048_down).
DO $$
DECLARE
    expected_count INTEGER := 4;
    actual_count   INTEGER;
BEGIN
    SET LOCAL search_path = pg_catalog, pg_temp;
    SELECT COUNT(*) INTO actual_count
    FROM public.tokenizer_versions
    WHERE tokenizer_version_id IN (
        '01918000-0000-7c10-8c10-000000000001'::uuid,
        '01918000-0000-7c10-8c10-000000000002'::uuid,
        '01918000-0000-7c10-8c10-000000000003'::uuid,
        '01918000-0000-7c10-8c10-00000000000f'::uuid
    );
    IF actual_count <> expected_count THEN
        RAISE EXCEPTION
            'tokenizer_versions seed sanity check failed: expected % SLICE_03 rows, got %',
            expected_count, actual_count;
    END IF;
END $$;

COMMENT ON TABLE public.tokenizer_versions IS
    'Registry of tokenizer encoder versions per tokenizer-service-spec-v1alpha1.md §6.1. Initial SLICE_03 seed: 3 OpenAI tiktoken-rs encoders + 1 HEURISTIC marker. SLICE_04 appends Anthropic / Gemini / Cohere / SentencePiece rows.';
