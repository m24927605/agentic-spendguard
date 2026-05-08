-- Tenant ledger pricing snapshot cache (per Stage 2 §9.4).
--
-- Authoritative pricing lives in the Platform Pricing Authority DB.
-- Tenant ledger holds an event-driven cached copy of versions referenced
-- by deployed contract bundles. Updated at contract bundle deployment time
-- (cold path); never queried in decision hot path.

CREATE TABLE pricing_snapshots (
    pricing_version          TEXT        PRIMARY KEY,
    price_snapshot_hash      BYTEA       NOT NULL,
    fx_rate_version          TEXT        NOT NULL,
    unit_conversion_version  TEXT        NOT NULL,
    schema_json              JSONB       NOT NULL,
    signature                BYTEA       NOT NULL,
    signing_key_id           TEXT        NOT NULL,
    deployed_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    deployed_by              TEXT        NOT NULL
);

CREATE INDEX idx_pricing_snapshots_hash
    ON pricing_snapshots (price_snapshot_hash);

COMMENT ON TABLE pricing_snapshots IS
    'Immutable cache from Platform Pricing Authority DB. Identity columns immutable (trigger).';
