-- ============================================================================
-- 0020_event_replay_dedup.sql — HARDEN_05 CloudEvent replay protection.
--
-- Production blocker #144: canonical_ingest must reject or idempotently dedupe
-- repeated `(producer_id, event.id)` submissions before immutable append. The
-- existing `canonical_events_global_keys(event_id)` mirror stays as the global
-- audit-chain uniqueness guard; this producer-scoped ledger lets the service
-- detect replay attempts and payload hash mismatches before any canonical_events
-- row or ingest offset is allocated.
-- ============================================================================

CREATE TABLE canonical_event_replay_dedup (
    producer_id   TEXT        NOT NULL CHECK (octet_length(producer_id) BETWEEN 1 AND 256),
    event_id      UUID        NOT NULL,
    tenant_id     UUID        NOT NULL,
    payload_hash  BYTEA       NOT NULL CHECK (octet_length(payload_hash) = 32),
    reservation_only BOOLEAN  NOT NULL DEFAULT FALSE,
    first_seen_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    expires_at    TIMESTAMPTZ NOT NULL,

    PRIMARY KEY (producer_id, event_id),
    CONSTRAINT canonical_event_replay_dedup_event_id_key UNIQUE (event_id),
    CHECK (expires_at > first_seen_at)
);

CREATE INDEX canonical_event_replay_dedup_expires_idx
    ON canonical_event_replay_dedup (expires_at);

CREATE INDEX canonical_event_replay_dedup_tenant_idx
    ON canonical_event_replay_dedup (tenant_id, first_seen_at DESC);

INSERT INTO canonical_event_replay_dedup (
    producer_id, event_id, tenant_id, payload_hash, reservation_only, expires_at
)
SELECT
    producer_id,
    event_id,
    tenant_id,
    decode(repeat('00', 32), 'hex'),
    TRUE,
    'infinity'::TIMESTAMPTZ
FROM audit_outcome_quarantine
WHERE state <> 'released'
ON CONFLICT (event_id) DO NOTHING;

REVOKE SELECT, INSERT, UPDATE, DELETE ON canonical_event_replay_dedup FROM PUBLIC;

GRANT SELECT, INSERT, UPDATE
    ON canonical_event_replay_dedup
    TO canonical_ingest_application_role;

COMMENT ON TABLE canonical_event_replay_dedup IS
    'Replay ledger for canonical_ingest AppendEvents. PRIMARY KEY(producer_id,event_id) detects idempotent producer retries; UNIQUE(event_id) reserves globally while audit outcomes are quarantined so cross-producer hijacks cannot preempt release.';
COMMENT ON COLUMN canonical_event_replay_dedup.payload_hash IS
    'SHA-256 over canonical CloudEvent bytes as verified/admitted by canonical_ingest. Same producer+event_id with different bytes is a replay/tamper signal.';
COMMENT ON COLUMN canonical_event_replay_dedup.reservation_only IS
    'TRUE for migration backfills that reserve legacy quarantine event_ids when the original canonical CloudEvent hash cannot be reconstructed. Any new runtime claim for that event_id fails closed as a collision.';
COMMENT ON COLUMN canonical_event_replay_dedup.expires_at IS
    'Replay horizon expiry used by cleanup jobs. Quarantine reservations use infinity so event_ids stay reserved until canonical release or terminal orphan handling.';

DO $$
BEGIN
    SET LOCAL search_path = pg_catalog, pg_temp;
    PERFORM 1 FROM pg_indexes
     WHERE schemaname = 'public'
       AND indexname = 'canonical_event_replay_dedup_expires_idx';
    IF NOT FOUND THEN
        RAISE EXCEPTION 'canonical_event_replay_dedup_expires_idx missing';
    END IF;
END $$;
