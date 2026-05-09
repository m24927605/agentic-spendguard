-- Pricing reference table — Phase 4 onboarding O3.
--
-- Lives in canonical_ingest DB next to schema_bundles. Bundle builders
-- pick the latest pricing_version at build time and embed it in the
-- contract bundle metadata; sidecar reads it at hot path for USD claim
-- computation (O4).
--
-- Stage 2 §9.4 specifies pricing freeze tuple:
--   (pricing_version, price_snapshot_hash, fx_rate_version, unit_conversion_version)
-- This table owns the first two fields. fx and unit_conversion remain
-- separate dimensions — covered by their own (future) tables.

CREATE TABLE pricing_table (
    pricing_version TEXT NOT NULL,
    provider        TEXT NOT NULL,    -- 'openai', 'anthropic', 'azure_openai', 'bedrock', 'gemini', ...
    model           TEXT NOT NULL,    -- 'gpt-4o-mini', 'claude-haiku-4-5-20251001', ...
    token_kind      TEXT NOT NULL CHECK (token_kind IN
                        ('input', 'output', 'cached_input',
                         'vision_input', 'audio_input', 'reasoning')),
    -- Price in USD per 1,000,000 tokens. NUMERIC(20,8) covers up to
    -- $999,999,999,999.99999999 / 1M, well past any plausible price.
    price_usd_per_million NUMERIC(20, 8) NOT NULL CHECK (price_usd_per_million >= 0),
    fetched_at      TIMESTAMPTZ NOT NULL,
    source          TEXT NOT NULL CHECK (source IN
                        ('manual', 'openai_pricing_api',
                         'anthropic_console', 'azure_pricing_api',
                         'aws_pricing_api', 'gemini_pricing_api')),
    PRIMARY KEY (pricing_version, provider, model, token_kind)
);

CREATE INDEX pricing_table_lookup_idx
    ON pricing_table (provider, model, pricing_version);

-- Audit trail: every pricing_version cut produces a row here for
-- compliance review. price_snapshot_hash matches what bundles embed
-- so spec §9.4 reproducibility holds.
CREATE TABLE pricing_versions (
    pricing_version       TEXT PRIMARY KEY,
    price_snapshot_hash   BYTEA NOT NULL,
    row_count             INT NOT NULL CHECK (row_count > 0),
    cut_at                TIMESTAMPTZ NOT NULL,
    cut_by                TEXT NOT NULL,    -- 'pricing_sync' | 'manual:<operator>'
    sources_used          TEXT[] NOT NULL,  -- ['manual', 'openai_pricing_api']
    notes                 TEXT
);

-- Helper view: latest pricing_version per (provider, model, token_kind).
-- Bundle builders query this when freezing a contract.
CREATE VIEW pricing_table_latest AS
    SELECT DISTINCT ON (provider, model, token_kind)
           provider, model, token_kind,
           pricing_version, price_usd_per_million,
           fetched_at, source
      FROM pricing_table
      JOIN pricing_versions USING (pricing_version)
     ORDER BY provider, model, token_kind, cut_at DESC;
