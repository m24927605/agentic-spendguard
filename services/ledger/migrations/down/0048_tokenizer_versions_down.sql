-- Down-migration: reverse 0048_tokenizer_versions.sql
-- (round-2 fix m2; round-3 fixes M4 + M10 + m4; round-4 fixes B1 + M4).
--
-- Apply BEFORE 0046_audit_outbox_prediction_columns_down.sql per
-- SLICE_01 §11 rollback order. Round-4 fix B1 corrects the round-3
-- header which incorrectly claimed "Apply AFTER 0046". The FK on
-- audit_outbox.tokenizer_version_id depends on the column existing in
-- audit_outbox; 0046_down drops the column. Reversing that order would
-- fail with "column does not exist" when trying to drop the FK in this
-- file. Slice §11 (the authoritative rollback runbook) is:
--   ledger:    0048_down → 0046_down (this is the correct order)
--   canonical: 0015_down → 0013_down
--
-- Round-3 fix M4 + round-4 fix M4 + round-5 fix N12-A: per-file destructive-down
-- guard (spendguard.allow_destructive_down_0048) — see slice §11 for the exact
-- SET form. SET (not SET LOCAL) because the migration runner autocommits
-- each statement; SET LOCAL would die at the next commit boundary before
-- the destructive DDL runs.
-- Round-3 fix M10: no explicit BEGIN/COMMIT (matches up-migration).
-- Round-3 fix m4: drop the explicit REVOKEs — DROP TABLE handles GRANTs
-- atomically (Postgres cleans up the privilege rows alongside the table).

DO $$
BEGIN
    IF current_setting('spendguard.allow_destructive_down_0048', true) IS DISTINCT FROM 'on' THEN
        RAISE EXCEPTION 'destructive down-migration 0048 requires `SET spendguard.allow_destructive_down_0048 = ''on''` first (session-scoped; runner autocommits so SET LOCAL would die at the commit boundary)';
    END IF;
    RAISE NOTICE 'DESTRUCTIVE down-migration 0048 proceeding (caller: %)', current_user;
END $$;

ALTER TABLE audit_outbox
    DROP CONSTRAINT IF EXISTS audit_outbox_tokenizer_version_id_fk;

DROP TRIGGER IF EXISTS tokenizer_versions_no_truncate ON tokenizer_versions;
DROP TRIGGER IF EXISTS tokenizer_versions_no_update_delete ON tokenizer_versions;

DROP INDEX IF EXISTS tokenizer_versions_active_idx;
DROP TABLE IF EXISTS tokenizer_versions;
