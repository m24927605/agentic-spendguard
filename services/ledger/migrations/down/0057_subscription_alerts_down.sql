-- D13 COV_63 — Subscription alerts (down).
DROP POLICY IF EXISTS subscription_alerts_tenant_isolation ON subscription_alerts;
DROP INDEX IF EXISTS idx_subscription_alerts_tenant_recent;
DROP TABLE IF EXISTS subscription_alerts;
