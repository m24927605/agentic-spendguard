-- SLICE_04 seed rows for the tokenizer_versions registry.
--
-- Spec ancestors:
--   - tokenizer-service-spec-v1alpha1.md §6.1 (schema), §6.2
--     (when to register), §7.1 (vendor sources + licenses).
--   - audit-chain-prediction-extension-v1alpha1.md §2.1 (FK target
--     column tokenizer_version_id).
--   - SLICE_04 §5 (this slice's seed owner).
--
-- This migration inserts 4 new tokenizer_versions rows for the SLICE_04
-- Tier 2 expansion (Anthropic + Gemini + Cohere + Llama):
--
--   * ANTHROPIC_BPE / anthropic-v3-bpe / xenova-claude-tokenizer-2026-05-30
--     - Source: https://huggingface.co/Xenova/claude-tokenizer
--     - Asset:  crates/spendguard-tokenizer/data/anthropic-claude3/tokenizer.json
--   * GEMINI_BPE / gemini-1.5-bpe / xenova-gemma-tokenizer-2026-05-30
--     - Source: https://huggingface.co/Xenova/gemma-tokenizer (community
--       approximation; Google's official Gemini tokenizer is API-only)
--     - Asset:  crates/spendguard-tokenizer/data/gemini-1.5/tokenizer.json
--   * COHERE_BPE / cohere-v2-bpe / xenova-c4ai-command-r-2026-05-30
--     - Source: https://huggingface.co/Xenova/c4ai-command-r-v01-tokenizer
--     - Asset:  crates/spendguard-tokenizer/data/cohere-command-r/tokenizer.json
--   * SENTENCEPIECE_LLAMA / llama-sentencepiece / xenova-llama-3.1-tokenizer-2026-05-30
--     - Source: https://huggingface.co/Xenova/Meta-Llama-3.1-Tokenizer
--     - Asset:  crates/spendguard-tokenizer/data/llama-3.1/tokenizer.json
--
-- The SLICE_03 rows (0049 seed) are NOT modified — this migration is
-- purely additive. Existing audit_outbox FKs to SLICE_03
-- tokenizer_version_id values continue to resolve unchanged.
--
-- Row identity:
--   * tokenizer_version_id is a stable application-minted UUIDv7
--     hard-coded here AND in crates/spendguard-tokenizer/src/versions.rs
--     so the library mapping and the FK target byte-match. Replaying
--     an audit_outbox row from a prior deployment reproduces the same
--     encoder lookup.
--   * UUIDv7 variant nibble is `8` (the simplest deterministic 10xx2
--     choice; matches SLICE_03 R2 B2 standardisation). The
--     `slice04_rows_have_valid_uuidv7_per_rfc_9562` test enforces
--     this across all 4 rows.
--   * (kind, encoder_name, version_string) is the UNIQUE business
--     key enforced by the 0048 schema; reseeding this migration on
--     top of an existing deployment is a no-op via
--     ON CONFLICT DO NOTHING.
--
-- Privilege boundary:
--   0048 already revokes PUBLIC INSERT/UPDATE/DELETE and grants INSERT
--   to ledger_application_role. The migration runner role MUST be a
--   member of ledger_application_role or have BYPASSRLS — the same
--   posture all prior 0040+ migrations require.
--
-- Forward-only:
--   tokenizer_versions has an immutability trigger blocking UPDATE /
--   DELETE on existing rows. There is no `0050_down.sql` — the
--   SLICE_03 R2 M3 convention is that 0048_down cascades drop the
--   table entirely if rollback is needed. SLICE_04 rows follow the
--   same lifecycle.
--
-- Round-2 fix M2 convention (SLICE_03 carryover): `SET LOCAL
-- search_path = pg_catalog, pg_temp;` inside the DO body forces
-- plpgsql to resolve COUNT / RAISE / etc against pg_catalog only —
-- CVE-2018-1058 hardening.

INSERT INTO tokenizer_versions (
    tokenizer_version_id,
    kind,
    encoder_name,
    version_string,
    asset_sha256
) VALUES
    (
        '01918000-0000-7c10-8c10-000000000004'::uuid,
        'ANTHROPIC_BPE',
        'anthropic-v3-bpe',
        'xenova-claude-tokenizer-2026-05-30',
        'c241737df24b4e7f7c9af4fdcee29a0ca903dcb288a8b753bc346a3092911767'
    ),
    (
        '01918000-0000-7c10-8c10-000000000005'::uuid,
        'GEMINI_BPE',
        'gemini-1.5-bpe',
        'xenova-gemma-tokenizer-2026-05-30',
        '05e97791a5e007260de1db7e1692e53150e08cea481e2bf25435553380c147ee'
    ),
    (
        '01918000-0000-7c10-8c10-000000000006'::uuid,
        'COHERE_BPE',
        'cohere-v2-bpe',
        'xenova-c4ai-command-r-2026-05-30',
        '0af6e6fe50ce1bb5611b103482de6bac000c82e06898138d57f35af121aec772'
    ),
    (
        '01918000-0000-7c10-8c10-000000000007'::uuid,
        'SENTENCEPIECE_LLAMA',
        'llama-sentencepiece',
        'xenova-llama-3.1-tokenizer-2026-05-30',
        '79e3e522635f3171300913bb421464a87de6222182a0570b9b2ccba2a964b2b4'
    )
ON CONFLICT (kind, encoder_name, version_string) DO NOTHING;

-- Sanity assertion: confirm all 4 SLICE_04 rows are present after the
-- INSERT. Defensive — the ON CONFLICT clause silently no-ops on
-- re-run, so a partial pre-existing seed (3 of 4 rows) would leave
-- the deployment in a state where the library expects to map to a
-- missing FK target. RAISE EXCEPTION fails the migration so the
-- runner rolls back rather than ship a broken registry.
DO $$
DECLARE
    expected_count INTEGER := 4;
    actual_count   INTEGER;
BEGIN
    SET LOCAL search_path = pg_catalog, pg_temp;
    SELECT COUNT(*) INTO actual_count
    FROM tokenizer_versions
    WHERE tokenizer_version_id IN (
        '01918000-0000-7c10-8c10-000000000004'::uuid,
        '01918000-0000-7c10-8c10-000000000005'::uuid,
        '01918000-0000-7c10-8c10-000000000006'::uuid,
        '01918000-0000-7c10-8c10-000000000007'::uuid
    );
    IF actual_count <> expected_count THEN
        RAISE EXCEPTION
            'tokenizer_versions SLICE_04 seed sanity check failed: expected % SLICE_04 rows, got %',
            expected_count, actual_count;
    END IF;
END $$;

COMMENT ON TABLE tokenizer_versions IS
    'Registry of tokenizer encoder versions per tokenizer-service-spec-v1alpha1.md §6.1. Initial SLICE_03 seed: 3 OpenAI tiktoken-rs encoders + 1 HEURISTIC marker (0049). SLICE_04 expansion: Anthropic + Gemini + Cohere + Llama BPE/SentencePiece (0050).';
