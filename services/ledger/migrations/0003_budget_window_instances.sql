-- Budget window instances (replay-critical immutable identity).
-- Per Ledger §5.1.

CREATE TABLE budget_window_instances (
    window_instance_id          UUID        PRIMARY KEY,
    tenant_id                   UUID        NOT NULL,
    budget_id                   UUID        NOT NULL,
    window_type                 TEXT        NOT NULL CHECK (window_type IN
                                    ('calendar_day', 'rolling',
                                     'calendar_month', 'billing_cycle')),
    timezone                    TEXT,
    tzdb_version                TEXT        NOT NULL,
    billing_anchor_rule_version TEXT,
    boundary_start              TIMESTAMPTZ,
    boundary_end                TIMESTAMPTZ,
    rolling_bucket_granularity  INTERVAL,
    computed_from_snapshot_at   TIMESTAMPTZ NOT NULL,
    created_at                  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_budget_window_instances_lookup
    ON budget_window_instances (tenant_id, budget_id, boundary_start);

COMMENT ON TABLE budget_window_instances IS
    'Immutable. tzdb_version + billing_anchor_rule_version + computed_from_snapshot_at frozen at creation.';
