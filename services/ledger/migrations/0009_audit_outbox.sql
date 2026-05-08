-- Audit outbox (Stage 2 §4.3 + v2.1 patch).
--
-- Sidecar audit_decision events are inserted into this table in the SAME
-- Postgres transaction as the corresponding ledger_transactions / ledger_entries
-- rows. With synchronous_commit=on + sync replica quorum, ReserveSet's
-- response is returned only after the audit row is durable + replicated.
-- Audit invariant ("no audit, no effect") is preserved.
--
-- Async outbox forwarder reads pending_forward=TRUE rows and pushes to
-- Canonical Ingest with idempotent dedupe by event_id (UUID v7).

CREATE TABLE audit_outbox (
    audit_outbox_id          UUID        NOT NULL,                -- UUID v7
    audit_decision_event_id  UUID        NOT NULL,                -- per Trace §11.1
    decision_id              UUID        NOT NULL,                -- Contract §6
    tenant_id                UUID        NOT NULL,

    ledger_transaction_id    UUID        NOT NULL
        REFERENCES ledger_transactions(ledger_transaction_id),

    event_type               TEXT        NOT NULL CHECK (event_type IN
                                 ('spendguard.audit.decision',
                                  'spendguard.audit.outcome')),
    cloudevent_payload       JSONB       NOT NULL,
    cloudevent_payload_signature BYTEA   NOT NULL,

    -- Fencing
    ledger_fencing_epoch     BIGINT      NOT NULL,
    workload_instance_id     TEXT        NOT NULL,

    -- Forwarding state — ONLY columns allowed to UPDATE post-insert.
    pending_forward          BOOLEAN     NOT NULL DEFAULT TRUE,
    forwarded_at             TIMESTAMPTZ,
    forward_attempts         INT         NOT NULL DEFAULT 0,
    last_forward_error       TEXT,

    -- Time
    recorded_at              TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    recorded_month           DATE        NOT NULL,

    -- Replay support
    producer_sequence        BIGINT      NOT NULL,
    idempotency_key          TEXT        NOT NULL,

    -- Partition-safe primary key (single PK; Stage 2 v2.1 fix).
    PRIMARY KEY (recorded_month, audit_outbox_id)
)
PARTITION BY RANGE (recorded_month);

-- Partition-safe UNIQUE constraints.
CREATE UNIQUE INDEX audit_outbox_decision_event_uq
    ON audit_outbox (recorded_month, audit_decision_event_id);

-- Per-partition idempotency uniqueness intentionally omitted here:
-- global uniqueness on (tenant_id, operation_kind, idempotency_key) is
-- enforced by audit_outbox_global_keys below. Including operation_kind in
-- the partitioned UNIQUE would require denormalizing operation_kind onto
-- audit_outbox; we keep audit_outbox lean and let the non-partitioned
-- global mirror own that invariant.

CREATE UNIQUE INDEX audit_outbox_producer_seq_uq
    ON audit_outbox (recorded_month, tenant_id, workload_instance_id,
                     producer_sequence);

-- v2.1 patch: per-decision uniqueness (partial unique).
CREATE UNIQUE INDEX audit_outbox_decision_per_decision_uq
    ON audit_outbox (recorded_month, tenant_id, decision_id)
    WHERE event_type = 'spendguard.audit.decision';

CREATE UNIQUE INDEX audit_outbox_outcome_per_decision_uq
    ON audit_outbox (recorded_month, tenant_id, decision_id)
    WHERE event_type = 'spendguard.audit.outcome';

CREATE INDEX audit_outbox_pending_forwarder_idx
    ON audit_outbox (recorded_month, pending_forward, recorded_at)
    WHERE pending_forward = TRUE;

CREATE INDEX audit_outbox_replay_cursor_idx
    ON audit_outbox (tenant_id, workload_instance_id, producer_sequence);

CREATE INDEX audit_outbox_decision_id_idx
    ON audit_outbox (tenant_id, decision_id);

-- Initial partitions + DEFAULT partition for unknown months.
-- pg_partman / migration runner is expected to pre-create future partitions
-- before they are needed; the default exists as a backstop so writes never
-- fail with "no partition" but ops alerts on default usage.
CREATE TABLE audit_outbox_2026_05 PARTITION OF audit_outbox
    FOR VALUES FROM ('2026-05-01') TO ('2026-06-01');
CREATE TABLE audit_outbox_2026_06 PARTITION OF audit_outbox
    FOR VALUES FROM ('2026-06-01') TO ('2026-07-01');
CREATE TABLE audit_outbox_2026_07 PARTITION OF audit_outbox
    FOR VALUES FROM ('2026-07-01') TO ('2026-08-01');
CREATE TABLE audit_outbox_default PARTITION OF audit_outbox DEFAULT;

-- ============================================================================
-- Global key uniqueness (non-partitioned).
--
-- Postgres partitioned tables can only enforce UNIQUE constraints that
-- include the partition key. The PARTIAL/PARTITION-safe indexes above are
-- scoped to a single (recorded_month, ...) partition, so duplicate
-- audit_decision_event_id / decision_id / producer_sequence / idempotency_key
-- could otherwise re-occur across months — breaking dedup and replay.
--
-- We therefore mirror the global keys into a small non-partitioned table.
-- The post_ledger_transaction stored proc inserts into both audit_outbox AND
-- audit_outbox_global_keys atomically; UNIQUE violations on either table
-- abort the transaction.
-- ============================================================================
CREATE TABLE audit_outbox_global_keys (
    audit_decision_event_id UUID NOT NULL PRIMARY KEY,
    tenant_id               UUID NOT NULL,
    decision_id             UUID NOT NULL,
    event_type              TEXT NOT NULL,
    -- operation_kind denormalized from ledger_transactions to scope idempotency
    -- correctly: ledger idempotency is (tenant_id, operation_kind, idempotency_key);
    -- without operation_kind, key K used for `reserve` would collide with key K
    -- used for `release`, etc.
    operation_kind          TEXT NOT NULL,
    workload_instance_id    TEXT NOT NULL,
    producer_sequence       BIGINT NOT NULL,
    idempotency_key         TEXT   NOT NULL,
    -- Pointer to the partitioned row.
    recorded_month          DATE   NOT NULL,
    audit_outbox_id         UUID   NOT NULL,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

-- Per-decision uniqueness (one decision event + one outcome event globally).
CREATE UNIQUE INDEX audit_outbox_global_per_decision_uq
    ON audit_outbox_global_keys (tenant_id, decision_id, event_type);

-- Global producer sequence uniqueness per (tenant, workload).
CREATE UNIQUE INDEX audit_outbox_global_producer_seq_uq
    ON audit_outbox_global_keys (tenant_id, workload_instance_id, producer_sequence);

-- Global idempotency uniqueness — must include operation_kind to mirror
-- the ledger_transactions idempotency scope.
CREATE UNIQUE INDEX audit_outbox_global_idempotency_uq
    ON audit_outbox_global_keys (tenant_id, operation_kind, idempotency_key);

-- Lookup back into partitioned table.
CREATE INDEX audit_outbox_global_keys_pointer_idx
    ON audit_outbox_global_keys (recorded_month, audit_outbox_id);

COMMENT ON TABLE audit_outbox_global_keys IS
    'Global uniqueness mirror for audit_outbox. Inserted in same tx as audit_outbox. Provides cross-partition uniqueness Postgres cannot natively enforce on partitioned tables.';
