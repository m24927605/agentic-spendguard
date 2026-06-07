-- D13 COV_61 — Subscription-Tier Meter Mode.
--
-- Tracks subscription-meter consumption for tenants on flat-fee plans
-- (Claude Code Pro/Max, Codex on ChatGPT Plus/Pro, etc). Increments are
-- driven by services/sidecar/src/subscription_meter on the metered
-- decision path; ledger entries are NOT written for these tenants.
--
-- Spec: docs/specs/coverage/D13_subscription_meter/design.md §4.3
--
-- Wire-compat: this migration is purely additive. Existing ledger /
-- audit_outbox rows are unaffected. New subscription_meters rows are
-- written only when classifier returns a subscription kind AND the
-- sidecar has consumed_atomic increment logic enabled (default mode is
-- `meter`).

CREATE TABLE IF NOT EXISTS subscription_meters (
    tenant_id           UUID        NOT NULL,
    plan                TEXT        NOT NULL
        CHECK (plan IN ('claude_code_pro', 'codex_chatgpt', 'unknown')),
    monthly_cap_atomic  BIGINT      NOT NULL DEFAULT 0
        CHECK (monthly_cap_atomic >= 0),
    period_start        TIMESTAMPTZ NOT NULL,
    period_end          TIMESTAMPTZ NOT NULL,
    consumed_atomic     BIGINT      NOT NULL DEFAULT 0
        CHECK (consumed_atomic >= 0),
    -- Soft-cap alert trigger; 0 = no soft alert configured.
    alert_at_atomic     BIGINT      NOT NULL DEFAULT 0
        CHECK (alert_at_atomic >= 0),
    -- Optional hard cap; NULL = soft mode (no synthetic 429).
    hard_cap_at_atomic  BIGINT      NULL
        CHECK (hard_cap_at_atomic IS NULL OR hard_cap_at_atomic >= 0),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, period_start)
);

-- Hot-path lookup index: classifier-resolved tenant + current period.
CREATE INDEX IF NOT EXISTS idx_subscription_meters_tenant_window
    ON subscription_meters (tenant_id, period_end DESC);

-- Row-Level Security: each tenant only sees their own rows when the
-- sidecar runtime role is active. mirrors the pattern used by
-- `subscription_caps` later in 0045.
ALTER TABLE subscription_meters ENABLE ROW LEVEL SECURITY;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_policies
         WHERE schemaname = 'public'
           AND tablename  = 'subscription_meters'
           AND policyname = 'subscription_meters_tenant_isolation'
    ) THEN
        EXECUTE $POLICY$
            CREATE POLICY subscription_meters_tenant_isolation
              ON subscription_meters
              FOR ALL
              USING (
                  current_setting('spendguard.tenant_id', true) IS NULL
                  OR current_setting('spendguard.tenant_id', true) = ''
                  OR tenant_id::text = current_setting('spendguard.tenant_id', true)
              )
        $POLICY$;
    END IF;
END $$;

-- audit_outbox `reservation_source` column — distinguishes BYOK rows
-- (ledger-charged) from subscription-meter rows (advisory). Default
-- preserves wire-compat for legacy producers.
--
-- audit_outbox is RANGE-partitioned by `recorded_month` per migration
-- 0009, so partial indexes on the parent table are NOT supported.  We
-- write the partial index against the partition key (recorded_month)
-- + tenant_id + recorded_at instead; queries filtering on
-- `reservation_source = 'subscription_meter'` will inherit the
-- index via constraint exclusion.
ALTER TABLE audit_outbox
    ADD COLUMN IF NOT EXISTS reservation_source TEXT NOT NULL DEFAULT 'byok'
        CHECK (reservation_source IN ('byok', 'subscription_meter'));

CREATE INDEX IF NOT EXISTS idx_audit_outbox_subscription_meter
    ON audit_outbox (recorded_month, tenant_id, recorded_at)
    WHERE reservation_source = 'subscription_meter';

COMMENT ON TABLE subscription_meters IS
    'D13 — subscription-tier flat-fee meter; consumed_atomic is best-effort retail $ in micro-USD';
COMMENT ON COLUMN subscription_meters.hard_cap_at_atomic IS
    'D13 §4.5 — when consumed_atomic >= this, sidecar short-circuits CONTINUE to DENY with reason subscription_cap_exceeded';
COMMENT ON COLUMN audit_outbox.reservation_source IS
    'D13 §4.3 — byok (ledger-charged) or subscription_meter (advisory; no ledger write)';
