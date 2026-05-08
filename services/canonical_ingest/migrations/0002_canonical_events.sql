-- Canonical event store (append-only).
-- Per Trace §10.2 storage classes:
--   immutable_audit_log     — 7yr SOX retention; spendguard.audit.*
--   canonical_raw_log       — 7yr; non-audit events; hashes only
--   profile_payload_blob    — tenant policy; full payloads (RTBF deletable)
--
-- POC stores all classes in this single table with `storage_class` column.
-- Phase 1 後段: split classes into separate backends for retention + RTBF.

CREATE TABLE canonical_events (
    -- event_id is the canonical dedup key (UUID v7 from producer).
    event_id            UUID NOT NULL,
    tenant_id           UUID NOT NULL,
    decision_id         UUID,                          -- NULL for non-decision events
    run_id              UUID,
    event_type          TEXT NOT NULL,                  -- "spendguard.audit.decision" etc.

    storage_class       TEXT NOT NULL CHECK (storage_class IN
                            ('immutable_audit_log', 'canonical_raw_log',
                             'profile_payload_blob')),

    -- Producer trust (per Trace §13).
    producer_id         TEXT NOT NULL,
    producer_sequence   BIGINT NOT NULL,
    producer_signature  BYTEA NOT NULL,
    signing_key_id      TEXT NOT NULL,

    -- Schema bundle (per Trace §12).
    schema_bundle_id    UUID NOT NULL REFERENCES schema_bundles(schema_bundle_id),
    schema_bundle_hash  BYTEA NOT NULL,

    -- CloudEvents 1.0 envelope (per Trace §7.5).
    specversion         TEXT NOT NULL,                  -- "1.0"
    source              TEXT NOT NULL,
    event_time          TIMESTAMPTZ NOT NULL,
    datacontenttype     TEXT NOT NULL,
    payload_json        JSONB,                          -- canonical_raw_log + immutable_audit_log
    payload_blob_ref    TEXT,                           -- profile_payload_blob backend ref

    -- Ingest metadata (per Trace §10.5 cross-region ordering).
    region_id           TEXT NOT NULL,
    ingest_shard_id     TEXT NOT NULL,
    ingest_log_offset   BIGINT NOT NULL,
    ingest_at           TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),

    -- Time partition.
    recorded_month      DATE NOT NULL,

    PRIMARY KEY (recorded_month, event_id)
)
PARTITION BY RANGE (recorded_month);

-- ============================================================================
-- Global event_id uniqueness mirror (canonical_events is partitioned and
-- partition-local UNIQUE alone allows duplicate event_ids across months).
-- ============================================================================
CREATE TABLE canonical_events_global_keys (
    event_id        UUID PRIMARY KEY,                   -- global dedup
    tenant_id       UUID NOT NULL,
    decision_id     UUID,
    event_type      TEXT NOT NULL,
    recorded_month  DATE NOT NULL,
    ingest_at       TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

CREATE INDEX canonical_events_global_keys_pointer_idx
    ON canonical_events_global_keys (recorded_month, event_id);
CREATE INDEX canonical_events_global_keys_decision_idx
    ON canonical_events_global_keys (tenant_id, decision_id, event_type)
    WHERE decision_id IS NOT NULL;

-- Per-decision uniqueness for audit chain (Stage 2 §4.8).
-- Globally exactly one audit.decision per (tenant, decision_id); same for outcome.
CREATE UNIQUE INDEX canonical_global_one_decision_uq
    ON canonical_events_global_keys (tenant_id, decision_id)
    WHERE event_type = 'spendguard.audit.decision';

CREATE UNIQUE INDEX canonical_global_one_outcome_uq
    ON canonical_events_global_keys (tenant_id, decision_id)
    WHERE event_type = 'spendguard.audit.outcome';

-- Per-decision indexes on partitioned table.
CREATE INDEX canonical_events_decision_idx
    ON canonical_events (tenant_id, decision_id, event_type)
    WHERE decision_id IS NOT NULL;

-- Per-event-type / tenant analysis index.
CREATE INDEX canonical_events_event_type_idx
    ON canonical_events (tenant_id, event_type, ingest_at);

-- Per-(region, shard) ordering index. Partition-safe; not unique across the
-- partitioned table (Postgres limitation). Global uniqueness is enforced
-- by canonical_ingest_positions below.
CREATE INDEX canonical_events_ingest_position_idx
    ON canonical_events (region_id, ingest_shard_id, ingest_log_offset);

-- Non-partitioned position mirror for global uniqueness (Trace §10.5).
CREATE TABLE canonical_ingest_positions (
    region_id         TEXT   NOT NULL,
    ingest_shard_id   TEXT   NOT NULL,
    ingest_log_offset BIGINT NOT NULL,
    event_id          UUID   NOT NULL,
    recorded_month    DATE   NOT NULL,
    PRIMARY KEY (region_id, ingest_shard_id, ingest_log_offset)
);
CREATE INDEX canonical_ingest_positions_event_idx
    ON canonical_ingest_positions (event_id);

-- Partitions.
CREATE TABLE canonical_events_2026_05 PARTITION OF canonical_events
    FOR VALUES FROM ('2026-05-01') TO ('2026-06-01');
CREATE TABLE canonical_events_2026_06 PARTITION OF canonical_events
    FOR VALUES FROM ('2026-06-01') TO ('2026-07-01');
CREATE TABLE canonical_events_2026_07 PARTITION OF canonical_events
    FOR VALUES FROM ('2026-07-01') TO ('2026-08-01');
CREATE TABLE canonical_events_default PARTITION OF canonical_events DEFAULT;
