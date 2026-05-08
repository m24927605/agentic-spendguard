-- Ledger shards & sequence allocators (per Ledger §5.1, §22 v2.1 patch).

CREATE TABLE ledger_shards (
    ledger_shard_id  SMALLINT    PRIMARY KEY,
    shard_generation BIGINT      NOT NULL,
    status           TEXT        NOT NULL CHECK (status IN
                         ('active', 'draining', 'retired')),
    parent_shard_id  SMALLINT    REFERENCES ledger_shards(ledger_shard_id),
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    retired_at       TIMESTAMPTZ
);

-- Per-shard monotonic counter (Ledger §22 v2.1 patch).
CREATE TABLE ledger_sequence_allocators (
    ledger_shard_id SMALLINT PRIMARY KEY
        REFERENCES ledger_shards(ledger_shard_id),
    last_sequence   BIGINT   NOT NULL DEFAULT 0
);

COMMENT ON TABLE ledger_shards IS
    'Shard identity + generation. shard_id MUST NOT be reused; new generation = new shard.';
COMMENT ON TABLE ledger_sequence_allocators IS
    'Per-shard monotonic counter. Replaces nextval sequences for cross-partition stability.';
