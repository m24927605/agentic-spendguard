-- tokenizer_versions registry table — final DDL per
-- tokenizer-service-spec-v1alpha1.md §6.1.
--
-- Spec ancestors:
--   - tokenizer-service-spec-v1alpha1.md §6 (this is the canonical home)
--   - audit-chain-prediction-extension-v1alpha1.md §2.1 (audit_outbox.tokenizer_version_id FK target)
--
-- Encoder rows are NOT populated in this migration — SLICE 03 (tokenizer
-- service skeleton) owns initial row inserts (cl100k_base / o200k_base /
-- anthropic-v3-bpe etc.) along with asset bundling. This migration only
-- creates the table substrate so audit_outbox.tokenizer_version_id can
-- reference it once SLICE 06+ producers start writing values.
--
-- The 0046 audit_outbox migration did NOT declare the FK constraint
-- because (a) tokenizer_versions did not yet exist at that point in the
-- ordering, and (b) the FK target table is in the same DB so we can add
-- the constraint here in 0048 as a follow-up ALTER TABLE. ON DELETE
-- behavior is RESTRICT per SLICE_01 §9 question 9 — losing audit
-- lineage by cascading a tokenizer_versions row delete would silently
-- break verify-chain replay.

BEGIN;

CREATE TABLE tokenizer_versions (
    tokenizer_version_id UUID        PRIMARY KEY,             -- UUIDv7

    kind                 TEXT        NOT NULL
                         CHECK (kind IN (
                             'OPENAI_TIKTOKEN',
                             'ANTHROPIC_BPE',
                             'GEMINI_BPE',
                             'COHERE_BPE',
                             'SENTENCEPIECE_LLAMA',
                             'HEURISTIC'
                         )),

    encoder_name         TEXT        NOT NULL,
        -- e.g. "cl100k_base", "o200k_base", "anthropic-v3-bpe",
        --      "gemini-1.5-bpe", "cohere-v2-bpe", "llama-sentencepiece".

    version_string       TEXT        NOT NULL,
        -- e.g. "tiktoken-0.7.0", "anthropic-bpe-2026-03",
        --      "gemini-bpe-2025-12-17". Combined with kind +
        --      encoder_name forms the unique identity of the asset.

    asset_sha256         TEXT        NOT NULL,
        -- 64-char hex; integrity check used by Tokenizer::new at boot.

    registered_at        TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),

    retired_at           TIMESTAMPTZ,
        -- NULL = active; non-NULL = retired (still verify-able for old
        -- audit rows). Per §6.3 retire only after no audit_outbox row
        -- has referenced for >= 7 years (SOX retention).

    UNIQUE (kind, encoder_name, version_string)
);

CREATE INDEX tokenizer_versions_active_idx
    ON tokenizer_versions (kind, encoder_name)
    WHERE retired_at IS NULL;

COMMENT ON TABLE tokenizer_versions IS
    'Registry of tokenizer encoder versions per tokenizer-service-spec-v1alpha1.md §6.1. audit_outbox.tokenizer_version_id FK targets this. SLICE 03 populates initial rows.';

-- ============================================================================
-- FK from audit_outbox.tokenizer_version_id -> tokenizer_versions.
--
-- Declared here (not in 0046) because the target table is created above.
-- ON DELETE RESTRICT prevents losing audit lineage if a tokenizer_versions
-- row is mistakenly DELETEd while any audit_outbox row still references it
-- (per SLICE_01 §9 adversarial question 9).
--
-- The FK is NOT NULL-able from the audit side (audit_outbox.tokenizer_version_id
-- is NULLable per §2.1 to represent Tier 3 fallback). A NULL on audit_outbox
-- side simply does not exercise the FK; no referential integrity loss.
-- ============================================================================

ALTER TABLE audit_outbox
    ADD CONSTRAINT audit_outbox_tokenizer_version_id_fk
        FOREIGN KEY (tokenizer_version_id)
        REFERENCES tokenizer_versions(tokenizer_version_id)
        ON DELETE RESTRICT;

COMMIT;
