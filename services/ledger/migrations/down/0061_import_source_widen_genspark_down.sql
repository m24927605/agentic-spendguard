-- D16 COV_88 — revert CHECK widening to the pre-D16 enum (D13 mig 0058
-- + D14 mig 0059 + D15 mig 0060).
--
-- This down-migration will FAIL when any `genspark_team_api` rows
-- exist — operator must purge or migrate first. This is intentional;
-- a silent narrowing would leave orphan rows that fail subsequent
-- INSERTs. Review-standards G8.

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
            'devin_team_api',
            'manus_team_api'
        ));
