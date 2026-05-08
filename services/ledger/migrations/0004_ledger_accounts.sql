-- Ledger accounts (per Ledger §5.1; account kinds per §2.2 + §10).

CREATE TABLE ledger_accounts (
    ledger_account_id   UUID         PRIMARY KEY,
    tenant_id           UUID         NOT NULL,
    budget_id           UUID         NOT NULL,
    window_instance_id  UUID         NOT NULL
        REFERENCES budget_window_instances(window_instance_id),
    account_kind        TEXT         NOT NULL CHECK (account_kind IN
                            ('available_budget', 'reserved_hold',
                             'committed_spend', 'debt', 'adjustment',
                             'refund_credit', 'dispute_adjustment')),
    unit_id             UUID         NOT NULL REFERENCES ledger_units(unit_id),
    created_at          TIMESTAMPTZ  NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, budget_id, window_instance_id, account_kind, unit_id)
);

CREATE INDEX idx_ledger_accounts_lookup
    ON ledger_accounts (tenant_id, budget_id, window_instance_id);

COMMENT ON TABLE ledger_accounts IS
    'Per-(tenant, budget, window, kind, unit) account. Per-unit balance constraint applies across debits/credits.';
