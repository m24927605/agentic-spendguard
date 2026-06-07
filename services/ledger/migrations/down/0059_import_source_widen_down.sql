-- D14 COV_69 — revert CHECK widening to the D13 mig 0058 enum.

ALTER TABLE audit_outbox
    DROP CONSTRAINT IF EXISTS audit_outbox_import_source_check;

ALTER TABLE audit_outbox
    ADD CONSTRAINT audit_outbox_import_source_check
        CHECK (import_source IS NULL OR import_source IN (
            'anthropic_console_usage',
            'openai_admin_usage',
            'devin_admin_usage',
            'manus_admin_usage',
            'genspark_admin_usage'
        ));
