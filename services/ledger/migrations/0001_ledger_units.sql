-- Ledger units (replay-critical immutable identity).
-- Per Ledger §5.1; immutability triggers in 0011_immutability_triggers.sql.

CREATE TABLE ledger_units (
    unit_id        UUID         PRIMARY KEY,
    tenant_id      UUID         NOT NULL,
    unit_kind      TEXT         NOT NULL CHECK (unit_kind IN
                       ('monetary', 'token', 'credit', 'non_monetary')),
    currency       CHAR(3),
    unit_name      TEXT,
    scale          INT          NOT NULL,
    rounding_mode  TEXT         NOT NULL CHECK (rounding_mode IN
                       ('half_even', 'half_up', 'truncate', 'banker')),
    display_format TEXT,
    effective_from TIMESTAMPTZ  NOT NULL DEFAULT now(),
    effective_until TIMESTAMPTZ,

    -- Token kind discriminator (per Contract §12.1; required when token).
    token_kind     TEXT,
    -- Token model family (per Contract §12.1; required when token).
    model_family   TEXT,
    -- Credit program (per Contract §12.1; required when credit).
    credit_program TEXT,

    -- NULLS NOT DISTINCT (Postgres 15+) ensures e.g. (tenant, monetary, USD,
    -- NULL, 6, NULL, NULL, NULL) is unique even when several columns are NULL.
    -- Without this, default UNIQUE treats NULL as distinct and admits dupes.
    UNIQUE NULLS NOT DISTINCT
        (tenant_id, unit_kind, currency, unit_name, scale,
         token_kind, model_family, credit_program)
);

COMMENT ON TABLE  ledger_units IS
    'Replay-critical unit identity. Identity columns are immutable (trigger).';
COMMENT ON COLUMN ledger_units.scale IS
    'Atomic-unit exponent: USD=6, JPY=0, tokens=0, etc.';
