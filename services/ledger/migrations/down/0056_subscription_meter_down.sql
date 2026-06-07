-- D13 COV_61 — Subscription-Tier Meter Mode (down).
DROP INDEX IF EXISTS idx_audit_outbox_subscription_meter;
ALTER TABLE audit_outbox DROP COLUMN IF EXISTS reservation_source;

DROP POLICY IF EXISTS subscription_meters_tenant_isolation ON subscription_meters;
DROP INDEX IF EXISTS idx_subscription_meters_tenant_window;
DROP TABLE IF EXISTS subscription_meters;
