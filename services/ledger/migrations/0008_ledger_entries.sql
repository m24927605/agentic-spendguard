-- Ledger entries (per Ledger §5.3 + §22 v2.1 partition-safe DDL).

CREATE TABLE ledger_entries (
    ledger_entry_id        UUID NOT NULL,
    ledger_transaction_id  UUID NOT NULL
        REFERENCES ledger_transactions(ledger_transaction_id),
    ledger_account_id      UUID NOT NULL
        REFERENCES ledger_accounts(ledger_account_id),

    -- Per-unit balancing (denormalized server-side from ledger_account_id).
    tenant_id              UUID NOT NULL,
    budget_id              UUID NOT NULL,
    window_instance_id     UUID,
    unit_id                UUID NOT NULL REFERENCES ledger_units(unit_id),

    direction              TEXT NOT NULL CHECK (direction IN ('debit', 'credit')),
    amount_atomic          NUMERIC(38, 0) NOT NULL CHECK (amount_atomic >= 0),

    -- Pricing freeze (4-layer per Ledger §13).
    pricing_version          TEXT  NOT NULL,
    price_snapshot_hash      BYTEA NOT NULL,
    fx_rate_version          TEXT,
    unit_conversion_version  TEXT,

    -- Cross-references.
    reservation_id         UUID,
    commit_event_kind      TEXT,
    invoice_line_item_ref  TEXT,

    -- Sequence ordering (per Ledger §22 v2.1).
    ledger_shard_id        SMALLINT NOT NULL
        REFERENCES ledger_shards(ledger_shard_id),
    ledger_sequence        BIGINT   NOT NULL,

    -- Time semantics.
    effective_at           TIMESTAMPTZ NOT NULL,
    effective_month        DATE        NOT NULL,
    recorded_at            TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    recorded_month         DATE        NOT NULL,

    -- Trace anchor.
    ingest_position        JSONB,
    created_at             TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),

    -- Partition-safe primary key (Ledger §22 v2.1).
    PRIMARY KEY (recorded_month, ledger_entry_id)
)
PARTITION BY RANGE (recorded_month);

-- Partition-safe unique index (Ledger §22 v2.1).
CREATE UNIQUE INDEX ledger_entries_partition_sequence_uq
    ON ledger_entries (recorded_month, ledger_shard_id, ledger_sequence);

CREATE INDEX ledger_entries_effective_query_idx
    ON ledger_entries (tenant_id, budget_id, effective_month, effective_at);

CREATE INDEX idx_ledger_entries_account
    ON ledger_entries (ledger_account_id, effective_at);

CREATE INDEX idx_ledger_entries_transaction
    ON ledger_entries (ledger_transaction_id);

CREATE INDEX idx_ledger_entries_reservation
    ON ledger_entries (reservation_id)
    WHERE reservation_id IS NOT NULL;

-- Initial monthly partitions for POC. pg_partman / migration runner takes
-- over rotation post-bootstrap. DEFAULT partition is a backstop so writes
-- never fail with "no partition"; ops alerts on default usage.
CREATE TABLE ledger_entries_2026_05 PARTITION OF ledger_entries
    FOR VALUES FROM ('2026-05-01') TO ('2026-06-01');
CREATE TABLE ledger_entries_2026_06 PARTITION OF ledger_entries
    FOR VALUES FROM ('2026-06-01') TO ('2026-07-01');
CREATE TABLE ledger_entries_2026_07 PARTITION OF ledger_entries
    FOR VALUES FROM ('2026-07-01') TO ('2026-08-01');
CREATE TABLE ledger_entries_default PARTITION OF ledger_entries DEFAULT;
