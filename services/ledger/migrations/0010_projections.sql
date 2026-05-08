-- Projections: spending_window_projections / reservations / commits
-- (per Ledger §5.5).

CREATE TABLE spending_window_projections (
    tenant_id                 UUID NOT NULL,
    budget_id                 UUID NOT NULL,
    window_instance_id        UUID NOT NULL
        REFERENCES budget_window_instances(window_instance_id),
    unit_id                   UUID NOT NULL REFERENCES ledger_units(unit_id),

    available_atomic          NUMERIC(38,0) NOT NULL,
    reserved_hold_atomic      NUMERIC(38,0) NOT NULL DEFAULT 0,
    committed_spend_atomic    NUMERIC(38,0) NOT NULL DEFAULT 0,
    debt_atomic               NUMERIC(38,0) NOT NULL DEFAULT 0,
    adjustment_atomic         NUMERIC(38,0) NOT NULL DEFAULT 0,
    refund_credit_atomic      NUMERIC(38,0) NOT NULL DEFAULT 0,

    reservation_count         BIGINT NOT NULL DEFAULT 0,
    commit_count              BIGINT NOT NULL DEFAULT 0,

    projection_lag_shard_id   SMALLINT,
    projection_lag_sequence   BIGINT,

    derived_from_append_only_ledger BOOLEAN NOT NULL DEFAULT TRUE,
    rebuildable_from_entries  BOOLEAN NOT NULL DEFAULT TRUE,

    version                   BIGINT      NOT NULL DEFAULT 0,
    updated_at                TIMESTAMPTZ NOT NULL DEFAULT now(),

    PRIMARY KEY (tenant_id, budget_id, window_instance_id, unit_id)
);

CREATE INDEX idx_projection_cursor
    ON spending_window_projections (projection_lag_shard_id, projection_lag_sequence);

-- Reservations projection (latest state).
CREATE TABLE reservations (
    reservation_id             UUID PRIMARY KEY,
    tenant_id                  UUID NOT NULL,
    budget_id                  UUID NOT NULL,
    window_instance_id         UUID NOT NULL,
    current_state              TEXT NOT NULL CHECK (current_state IN
                                   ('reserved', 'committed', 'released',
                                    'overrun_debt')),
    trace_run_id               UUID,
    trace_step_id              UUID,
    trace_llm_call_id          UUID,
    source_ledger_transaction_id UUID NOT NULL
        REFERENCES ledger_transactions(ledger_transaction_id),
    ttl_expires_at             TIMESTAMPTZ NOT NULL,
    idempotency_key            TEXT NOT NULL,
    created_at                 TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- NOTE: previous spec drafts placed a UNIQUE on (tenant_id, budget_id,
-- idempotency_key). That's incorrect: a single ReserveSet may include
-- multiple claims for the same budget but different (unit_id,
-- window_instance_id), and they share the operation-level idempotency_key.
-- Idempotency at the operation level is enforced by the UNIQUE on
-- ledger_transactions (tenant_id, operation_kind, idempotency_key);
-- reservation_id PK is sufficient here.

CREATE INDEX idx_reservations_idempotency_lookup
    ON reservations (tenant_id, budget_id, idempotency_key);

CREATE INDEX idx_reservations_active
    ON reservations (tenant_id, budget_id, window_instance_id)
    WHERE current_state = 'reserved';

CREATE INDEX idx_reservations_ttl
    ON reservations (ttl_expires_at)
    WHERE current_state = 'reserved';

-- Commits projection (latest state per Contract §5 commit state machine).
CREATE TABLE commits (
    commit_id                       UUID PRIMARY KEY,
    reservation_id                  UUID NOT NULL,
    tenant_id                       UUID NOT NULL,
    budget_id                       UUID NOT NULL,
    unit_id                         UUID NOT NULL REFERENCES ledger_units(unit_id),
    latest_state                    TEXT NOT NULL CHECK (latest_state IN
                                        ('unknown', 'estimated',
                                         'provider_reported',
                                         'invoice_reconciled')),
    estimated_amount_atomic         NUMERIC(38,0),
    provider_reported_amount_atomic NUMERIC(38,0),
    invoice_reconciled_amount_atomic NUMERIC(38,0),
    delta_to_reserved_atomic        NUMERIC(38,0),
    pricing_version                 TEXT  NOT NULL,
    price_snapshot_hash             BYTEA NOT NULL,
    estimated_at                    TIMESTAMPTZ,
    provider_reported_at            TIMESTAMPTZ,
    invoice_reconciled_at           TIMESTAMPTZ,
    latest_projection_only          BOOLEAN NOT NULL DEFAULT TRUE,
    created_at                      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at                      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_commits_reservation ON commits (reservation_id);
CREATE INDEX idx_commits_state ON commits (latest_state, updated_at);
