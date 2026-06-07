-- D13 COV_65 — Subscription usage importer job tracking.
--
-- Companion to 0044/0045.  Tracks Day-2 reconciliation imports
-- (Anthropic Console Usage / OpenAI Admin Usage / Devin / Manus /
-- Genspark) that bring vendor billing truth into audit_outbox so
-- dashboards can reconcile our meter-only estimate against the
-- real flat-fee usage report.  The actual importers ship as stubs
-- in services/ledger/src/subscription_importer/ — see D14/D15/D16
-- for live implementations.
--
-- Spec: docs/specs/coverage/D13_subscription_meter/design.md §5

CREATE TABLE IF NOT EXISTS subscription_import_jobs (
    job_id             TEXT        NOT NULL,         -- UUIDv7 string
    tenant_id          UUID        NOT NULL,
    importer_kind      TEXT        NOT NULL
        CHECK (importer_kind IN (
            'anthropic_console_usage',
            'openai_admin_usage',
            'devin_admin_usage',
            'manus_admin_usage',
            'genspark_admin_usage'
        )),
    -- Source artefact: CSV path / API window / etc. opaque.
    source_artifact    TEXT        NOT NULL,
    window_start       TIMESTAMPTZ NOT NULL,
    window_end         TIMESTAMPTZ NOT NULL,
    status             TEXT        NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'running', 'completed', 'failed')),
    rows_imported      BIGINT      NOT NULL DEFAULT 0
        CHECK (rows_imported >= 0),
    error_message      TEXT        NULL,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at       TIMESTAMPTZ NULL,
    PRIMARY KEY (job_id)
);

CREATE INDEX IF NOT EXISTS idx_subscription_import_jobs_tenant_status
    ON subscription_import_jobs (tenant_id, status, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_subscription_import_jobs_pending
    ON subscription_import_jobs (importer_kind, created_at)
    WHERE status IN ('pending', 'running');

ALTER TABLE subscription_import_jobs ENABLE ROW LEVEL SECURITY;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_policies
         WHERE schemaname = 'public'
           AND tablename  = 'subscription_import_jobs'
           AND policyname = 'subscription_import_jobs_tenant_isolation'
    ) THEN
        EXECUTE $POLICY$
            CREATE POLICY subscription_import_jobs_tenant_isolation
              ON subscription_import_jobs
              FOR ALL
              USING (
                  current_setting('spendguard.tenant_id', true) IS NULL
                  OR current_setting('spendguard.tenant_id', true) = ''
                  OR tenant_id::text = current_setting('spendguard.tenant_id', true)
              )
        $POLICY$;
    END IF;
END $$;

-- Per design §5: importer-written audit_outbox rows carry import_source.
ALTER TABLE audit_outbox
    ADD COLUMN IF NOT EXISTS import_source TEXT NULL
        CHECK (import_source IS NULL OR import_source IN (
            'anthropic_console_usage',
            'openai_admin_usage',
            'devin_admin_usage',
            'manus_admin_usage',
            'genspark_admin_usage'
        ));

CREATE INDEX IF NOT EXISTS idx_audit_outbox_import_source
    ON audit_outbox (recorded_month, tenant_id, import_source, recorded_at)
    WHERE import_source IS NOT NULL;

COMMENT ON TABLE  subscription_import_jobs IS
    'D13 §5 — usage importer job tracking; stubs for D14/D15/D16 live importers';
COMMENT ON COLUMN audit_outbox.import_source IS
    'D13 §5 — set by importer crates only; live proxy/sidecar rows leave NULL';
