-- D13 COV_63 — Subscription alerts table + cooldown bookkeeping.
--
-- Companion to 0044_subscription_meter.sql.  Stores soft-cap and
-- hard-cap alert events with a cooldown column so the alert emitter
-- (services/sidecar/src/subscription_meter/alerts.rs) can avoid
-- storming the canonical_events stream when a meter hovers over the
-- threshold.
--
-- Cooldown semantics:
--   * `last_fired_at` is updated on every alert emission.
--   * Caller compares `NOW() - last_fired_at >= cooldown_seconds`; if
--     below, the alert is suppressed.  Default = 1h (3600s) matches
--     stats_aggregator drift-alert backoff.
--   * `fire_count` is monotonic — useful for dashboards to see how
--     many times a tenant has hit the threshold within a billing
--     window.
--
-- Spec: docs/specs/coverage/D13_subscription_meter/design.md §4.4 +
-- review-standards §6 (alert storm budget).

CREATE TABLE IF NOT EXISTS subscription_alerts (
    tenant_id          UUID        NOT NULL,
    period_start       TIMESTAMPTZ NOT NULL,
    severity           TEXT        NOT NULL
        CHECK (severity IN ('soft_cap', 'hard_cap')),
    threshold_atomic   BIGINT      NOT NULL CHECK (threshold_atomic >= 0),
    last_fired_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    cooldown_seconds   INT         NOT NULL DEFAULT 3600
        CHECK (cooldown_seconds >= 0),
    fire_count         BIGINT      NOT NULL DEFAULT 1
        CHECK (fire_count >= 0),
    PRIMARY KEY (tenant_id, period_start, severity)
);

CREATE INDEX IF NOT EXISTS idx_subscription_alerts_tenant_recent
    ON subscription_alerts (tenant_id, last_fired_at DESC);

ALTER TABLE subscription_alerts ENABLE ROW LEVEL SECURITY;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_policies
         WHERE schemaname = 'public'
           AND tablename  = 'subscription_alerts'
           AND policyname = 'subscription_alerts_tenant_isolation'
    ) THEN
        EXECUTE $POLICY$
            CREATE POLICY subscription_alerts_tenant_isolation
              ON subscription_alerts
              FOR ALL
              USING (
                  current_setting('spendguard.tenant_id', true) IS NULL
                  OR current_setting('spendguard.tenant_id', true) = ''
                  OR tenant_id::text = current_setting('spendguard.tenant_id', true)
              )
        $POLICY$;
    END IF;
END $$;

COMMENT ON TABLE  subscription_alerts IS
    'D13 — soft/hard cap alert cooldown bookkeeping; mirrors stats_aggregator drift_alert pattern';
COMMENT ON COLUMN subscription_alerts.cooldown_seconds IS
    'D13 review-standards §6 — minimum interval between consecutive alerts; default 1h';
