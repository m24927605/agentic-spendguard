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
--
-- Round-2 security hardening (Codex B1):
--   * REVOKE INSERT/UPDATE/DELETE FROM PUBLIC + GRANT to
--     ledger_application_role (mirror of the 0012 grant convention at
--     lines 515-518; without this, any role with default PUBLIC access
--     could insert / update / delete tokenizer rows and a Tier-3
--     fallback would silently widen).
--   * BEFORE UPDATE OR DELETE trigger reusing
--     reject_immutable_reference_mutation() (precedent: 0011 lines
--     50-52 for budget_window_instances) — tokenizer versions are
--     append-only by design; rotation is "register a new row + retire
--     the old via retired_at" not "UPDATE in place".
--   * BEFORE TRUNCATE statement-level trigger reusing
--     reject_immutable_ledger_entry_mutation() (mirror of the new
--     audit_outbox_no_truncate trigger in 0046).
--
-- Round-2 deployment-safety hardening (Codex M18):
--   * The FK declaration is split into NOT VALID + VALIDATE so a
--     production re-run on a large audit_outbox does not take a long
--     ACCESS EXCLUSIVE lock during the existing-row scan (the column
--     is all-NULL in legacy rows so the scan is a no-op, but the
--     two-step form is the deployment-safe pattern and keeps SLICE_01
--     consistent with §4.3 cross-DB ordering documentation).
--
-- Round-2 stylistic alignment (Codex m3): no explicit BEGIN/COMMIT —
-- migration runner wraps each file in its own transaction (matches the
-- 57 pre-existing migrations 0000-0045 / 0001-0012).

CREATE TABLE tokenizer_versions (
    -- Round-2 fix m6: PK semantics annotated. UUIDv7 minted application-side;
    -- no DEFAULT to force caller awareness (PG 18+ ships native uuidv7()
    -- but we cannot rely on PG 18 across all customer deployments yet).
    tokenizer_version_id UUID        PRIMARY KEY,

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

-- Round-2 fix m7: registered_at DESC tie-breaker so "active by kind+encoder
-- with newest-first ordering" is a tight index scan during dispatch.
CREATE INDEX tokenizer_versions_active_idx
    ON tokenizer_versions (kind, encoder_name, registered_at DESC)
    WHERE retired_at IS NULL;

COMMENT ON TABLE tokenizer_versions IS
    'Registry of tokenizer encoder versions per tokenizer-service-spec-v1alpha1.md §6.1. audit_outbox.tokenizer_version_id FK targets this. SLICE 03 populates initial rows. Round-2 (Codex B1) hardened with REVOKE PUBLIC + immutability trigger + TRUNCATE guard.';

-- ============================================================================
-- Round-2 fix B1: lock down DML to ledger_application_role.
-- Mirror of 0012 lines 515-518 convention.
-- ============================================================================

REVOKE INSERT, UPDATE, DELETE ON tokenizer_versions FROM PUBLIC;
GRANT INSERT ON tokenizer_versions TO ledger_application_role;
-- Reader role mirrors the 0012 pattern: lookup-only.
GRANT SELECT ON tokenizer_versions TO ledger_reader_role;

-- ============================================================================
-- Round-2 fix B1: BEFORE UPDATE OR DELETE trigger. Reuses
-- reject_immutable_reference_mutation() from 0011 (per the 0011:50-52
-- budget_window_instances precedent). Tokenizer versions are
-- append-only — rotation = register a new row + flip retired_at on the
-- old via INSERT-then-UPDATE-retire path which is itself blocked here.
-- Retirement therefore happens via a separate retirement_log table in
-- a future slice (SLICE 03 will introduce it). For SLICE_01 we keep
-- the registry strictly append-only — no retired_at writes after INSERT.
-- ============================================================================

CREATE TRIGGER tokenizer_versions_no_update_delete
    BEFORE UPDATE OR DELETE ON tokenizer_versions
    FOR EACH ROW EXECUTE FUNCTION reject_immutable_reference_mutation();

-- ============================================================================
-- Round-2 fix B1 + M13 mirror: TRUNCATE statement-level guard.
-- Round-3 fix M6: reuse the generic reject_truncate_on_immutable_table()
-- function defined in 0046 (replaces the ledger-entry-named helper
-- whose error message lied about the table). TG_TABLE_NAME inside the
-- function reports 'tokenizer_versions' correctly.
-- ============================================================================

CREATE TRIGGER tokenizer_versions_no_truncate
    BEFORE TRUNCATE ON tokenizer_versions
    FOR EACH STATEMENT
    EXECUTE FUNCTION reject_truncate_on_immutable_table();

-- ============================================================================
-- FK from audit_outbox.tokenizer_version_id -> tokenizer_versions.
--
-- Declared here (not in 0046) because the target table is created above.
-- ON DELETE RESTRICT prevents losing audit lineage if a tokenizer_versions
-- row is mistakenly DELETEd while any audit_outbox row still references it
-- (per SLICE_01 §9 adversarial question 9). The DELETE-attempt is also
-- blocked by the new immutability trigger above; the RESTRICT is
-- defense-in-depth.
--
-- The FK is NOT NULL-able from the audit side (audit_outbox.tokenizer_version_id
-- is NULLable per §2.1 to represent Tier 3 fallback). A NULL on audit_outbox
-- side simply does not exercise the FK; no referential integrity loss.
--
-- Round-2 fix M18: declared NOT VALID first, then VALIDATEd. The legacy
-- audit_outbox rows have NULL tokenizer_version_id so the scan is empty,
-- but the two-step form matches the M6 / 0046 deployment-safe pattern
-- and keeps the lock window minimal on future production re-runs.
-- ============================================================================

ALTER TABLE audit_outbox
    ADD CONSTRAINT audit_outbox_tokenizer_version_id_fk
        FOREIGN KEY (tokenizer_version_id)
        REFERENCES tokenizer_versions(tokenizer_version_id)
        ON DELETE RESTRICT
        NOT VALID;

ALTER TABLE audit_outbox VALIDATE CONSTRAINT audit_outbox_tokenizer_version_id_fk;
