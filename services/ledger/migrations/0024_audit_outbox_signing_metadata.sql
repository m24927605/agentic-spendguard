-- Phase 5 GA hardening S6: signing metadata columns on audit_outbox.
--
-- Until S6, the audit_outbox row builder only stored the raw signature
-- bytes (`cloudevent_payload_signature`). The signing identity was
-- buried inside the BYTEA-serialized CloudEvent payload, which made
-- forensics queries like "which key signed this row" require
-- deserializing the payload — slow on large queries and impossible from
-- pure SQL.
--
-- S6 surfaces signing metadata as first-class GENERATED columns. Using
-- GENERATED ALWAYS AS ... STORED means:
--
--   * No SP changes required — all six existing post_*_transaction SPs
--     keep working unchanged. They write the JSONB cloudevent_payload
--     including the producer's `signing_key_id` extension attribute
--     (which already exists on the CloudEvent proto, field 203);
--     Postgres derives the new columns from JSONB at INSERT time.
--   * Backwards compatible — existing rows have an empty
--     `signing_key_id` in cloudevent_payload, so the CASE branch falls
--     through to the `pre-S6` sentinel and remains queryable.
--   * Forensic queries become trivial:
--         SELECT count(*) FROM audit_outbox WHERE signing_key_id = '<key>';
--         SELECT count(*) FROM audit_outbox WHERE signing_algorithm = 'pre-S6';
--
-- Three columns:
--
--   * `signing_key_id`     — extracted from cloudevent_payload->>'signing_key_id'.
--                            Empty/missing falls through to 'pre-S6:legacy'.
--   * `signing_algorithm`  — derived from signing_key_id prefix (the
--                            signing crate uses `ed25519:`, `arn:aws:kms:`
--                            or `kms-`, `disabled:` namespaces). Pre-S6
--                            rows resolve to 'pre-S6' so the audit row
--                            is always queryable by algorithm.
--   * `signed_at`          — server-side wallclock at row insertion.
--                            Independent of cloudevent_payload->>'time'
--                            (which is producer-attested); auditors use
--                            the gap between the two to detect a
--                            producer fabricating timestamps.
--
-- All three columns are STORED so they participate in indexes and don't
-- recompute on every read.

ALTER TABLE audit_outbox
    ADD COLUMN signing_key_id TEXT GENERATED ALWAYS AS (
        CASE
            WHEN COALESCE(cloudevent_payload->>'signing_key_id', '') = ''
                THEN 'pre-S6:legacy'
            ELSE cloudevent_payload->>'signing_key_id'
        END
    ) STORED,
    ADD COLUMN signing_algorithm TEXT GENERATED ALWAYS AS (
        CASE
            WHEN cloudevent_payload->>'signing_key_id' LIKE 'ed25519:%'
                THEN 'ed25519'
            WHEN cloudevent_payload->>'signing_key_id' LIKE 'arn:aws:kms:%'
              OR cloudevent_payload->>'signing_key_id' LIKE 'kms-%'
                THEN 'kms-ed25519'
            WHEN cloudevent_payload->>'signing_key_id' LIKE 'disabled:%'
                THEN 'disabled'
            ELSE 'pre-S6'
        END
    ) STORED,
    ADD COLUMN signed_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp();

-- Index on signing_key_id for forensics: "show me all audit rows
-- signed by this compromised key". Partial — most queries care about
-- the active-mode rows (skip pre-S6 backfill rows + disabled-mode
-- demo rows that have no real key material).
CREATE INDEX audit_outbox_signing_key_idx
    ON audit_outbox (recorded_month, signing_key_id)
    WHERE signing_algorithm IN ('ed25519', 'kms-ed25519');

-- Helpful index for "find all rows signed by a given algorithm".
-- Partial because pre-S6 backfill is a bounded one-time concern and
-- disabled-mode rows belong to demo profiles operators don't audit.
CREATE INDEX audit_outbox_signing_algo_idx
    ON audit_outbox (recorded_month, signing_algorithm)
    WHERE signing_algorithm <> 'pre-S6';

-- COMMENT for clarity in DB migration audits.
COMMENT ON COLUMN audit_outbox.signing_key_id IS
    'S6: stable id of signing key. ed25519:<hex16> for local mode, KMS arn for kms mode, disabled:<producer> for demo, pre-S6:legacy for backfilled rows.';
COMMENT ON COLUMN audit_outbox.signing_algorithm IS
    'S6: algorithm derived from signing_key_id prefix (ed25519 | kms-ed25519 | disabled | pre-S6).';
COMMENT ON COLUMN audit_outbox.signed_at IS
    'S6: server-side wallclock at row insertion, independent of cloudevent_payload->time (producer-attested).';
