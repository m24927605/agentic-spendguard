-- Down-migration: reverse 0048_tokenizer_versions.sql (round-2 fix m2).
--
-- Apply AFTER 0046_audit_outbox_prediction_columns_down.sql per
-- SLICE_01 §11 rollback order — the FK on audit_outbox.tokenizer_version_id
-- depends on tokenizer_versions existing.
--
-- Idempotent + safe to re-run.

BEGIN;

ALTER TABLE audit_outbox
    DROP CONSTRAINT IF EXISTS audit_outbox_tokenizer_version_id_fk;

DROP TRIGGER IF EXISTS tokenizer_versions_no_truncate ON tokenizer_versions;
DROP TRIGGER IF EXISTS tokenizer_versions_no_update_delete ON tokenizer_versions;

REVOKE INSERT ON tokenizer_versions FROM ledger_application_role;
REVOKE SELECT ON tokenizer_versions FROM ledger_reader_role;

DROP INDEX IF EXISTS tokenizer_versions_active_idx;
DROP TABLE IF EXISTS tokenizer_versions;

COMMIT;
