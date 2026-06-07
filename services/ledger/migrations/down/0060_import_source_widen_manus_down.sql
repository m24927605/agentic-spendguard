-- D15 COV_74 — revert mig 0060 CHECK widening back to the D13 + D14
-- enum (no `manus_team_api`).
--
-- The down migration drops `manus_team_api` from the CHECK; it will
-- fail (intentionally) when any 'manus_team_api' rows exist — the
-- operator must purge or re-migrate before downgrading.

ALTER TABLE audit_outbox
    DROP CONSTRAINT IF EXISTS audit_outbox_import_source_check;

ALTER TABLE audit_outbox
    ADD CONSTRAINT audit_outbox_import_source_check
        CHECK (import_source IS NULL OR import_source IN (
            'anthropic_console_usage',
            'openai_admin_usage',
            'devin_admin_usage',
            'manus_admin_usage',
            'genspark_admin_usage',
            'devin_team_api'
        ));
