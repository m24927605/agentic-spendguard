-- D13 COV_65 — Subscription importer (down).
DROP INDEX IF EXISTS idx_audit_outbox_import_source;
ALTER TABLE audit_outbox DROP COLUMN IF EXISTS import_source;

DROP POLICY IF EXISTS subscription_import_jobs_tenant_isolation ON subscription_import_jobs;
DROP INDEX IF EXISTS idx_subscription_import_jobs_pending;
DROP INDEX IF EXISTS idx_subscription_import_jobs_tenant_status;
DROP TABLE IF EXISTS subscription_import_jobs;
