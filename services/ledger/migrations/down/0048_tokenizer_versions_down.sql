-- Down-migration: reverse 0048_tokenizer_versions.sql
-- (round-2 fix m2; round-3 fixes M4 + M10 + m4).
--
-- Apply AFTER 0046_audit_outbox_prediction_columns_down.sql per
-- SLICE_01 §11 rollback order — the FK on audit_outbox.tokenizer_version_id
-- depends on tokenizer_versions existing.
--
-- Round-3 fix M4: destructive-down guard.
-- Round-3 fix M10: no explicit BEGIN/COMMIT (matches up-migration).
-- Round-3 fix m4: drop the explicit REVOKEs — DROP TABLE handles GRANTs
-- atomically (Postgres cleans up the privilege rows alongside the table).

DO $$
BEGIN
    IF current_setting('spendguard.allow_destructive_down', true) IS DISTINCT FROM 'on' THEN
        RAISE EXCEPTION 'destructive down-migration 0048 requires `SET spendguard.allow_destructive_down = on` first';
    END IF;
END $$;

ALTER TABLE audit_outbox
    DROP CONSTRAINT IF EXISTS audit_outbox_tokenizer_version_id_fk;

DROP TRIGGER IF EXISTS tokenizer_versions_no_truncate ON tokenizer_versions;
DROP TRIGGER IF EXISTS tokenizer_versions_no_update_delete ON tokenizer_versions;

DROP INDEX IF EXISTS tokenizer_versions_active_idx;
DROP TABLE IF EXISTS tokenizer_versions;
