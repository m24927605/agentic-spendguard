-- Phase 5 GA hardening S7: extend audit_signature_quarantine.reason
-- CHECK constraint to include the validity-window failure modes
-- introduced by S7's key registry / rotation:
--
--   * key_expired       — event_time > key.valid_until
--   * key_not_yet_valid — event_time < key.valid_from
--   * key_revoked       — operator-driven revocation
--
-- 0007 had the original CHECK list. We DROP + re-ADD with the
-- expanded set; existing rows keep their reason strings unchanged.

ALTER TABLE audit_signature_quarantine
    DROP CONSTRAINT IF EXISTS audit_signature_quarantine_reason_check;

ALTER TABLE audit_signature_quarantine
    ADD CONSTRAINT audit_signature_quarantine_reason_check
        CHECK (reason IN (
            'unknown_key',
            'invalid_signature',
            'pre_s6',
            'disabled',
            'oversized_canonical',
            'schema_failure',
            'key_expired',
            'key_not_yet_valid',
            'key_revoked'
        ));

COMMENT ON CONSTRAINT audit_signature_quarantine_reason_check
    ON audit_signature_quarantine IS
    'S7-extended: includes key_expired / key_not_yet_valid / key_revoked from validity window enforcement.';
