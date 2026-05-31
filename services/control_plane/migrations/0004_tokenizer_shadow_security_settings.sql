-- ============================================================================
-- 0004_tokenizer_shadow_security_settings.sql — HARDEN_05 tenant opt-in for
-- tokenizer Tier 1 raw-text shadow calls and per-tenant count_tokens quotas.
-- ============================================================================

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'tokenizer_shadow_runtime_role') THEN
        CREATE ROLE tokenizer_shadow_runtime_role NOLOGIN;
    END IF;
END $$;

CREATE TABLE tokenizer_shadow_security_settings (
    tenant_id                      UUID        PRIMARY KEY,
    pii_shadow_enabled             BOOLEAN     NOT NULL DEFAULT FALSE,
    count_tokens_quota_per_minute  INTEGER     NOT NULL DEFAULT 0
        CHECK (count_tokens_quota_per_minute >= 0 AND count_tokens_quota_per_minute <= 100000),
    updated_at                     TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_by                     TEXT        NOT NULL CHECK (octet_length(updated_by) BETWEEN 1 AND 256)
);

ALTER TABLE tokenizer_shadow_security_settings ENABLE ROW LEVEL SECURITY;
ALTER TABLE tokenizer_shadow_security_settings FORCE ROW LEVEL SECURITY;

CREATE POLICY tokenizer_shadow_security_settings_tenant_isolation
    ON tokenizer_shadow_security_settings
    FOR ALL
    USING (
        tenant_id = COALESCE(
            NULLIF(current_setting('app.current_tenant_id', TRUE), ''),
            '00000000-0000-0000-0000-000000000000'
        )::uuid
    )
    WITH CHECK (
        tenant_id = COALESCE(
            NULLIF(current_setting('app.current_tenant_id', TRUE), ''),
            '00000000-0000-0000-0000-000000000000'
        )::uuid
    );

REVOKE SELECT, INSERT, UPDATE, DELETE ON tokenizer_shadow_security_settings FROM PUBLIC;

GRANT SELECT, INSERT, UPDATE, DELETE
    ON tokenizer_shadow_security_settings
    TO control_plane_application_role;

GRANT SELECT ON tokenizer_shadow_security_settings TO control_plane_reader_role;
GRANT SELECT ON tokenizer_shadow_security_settings TO tokenizer_shadow_runtime_role;

GRANT SELECT ON tokenizer_sampling_rate_overrides TO tokenizer_shadow_runtime_role;

COMMENT ON TABLE tokenizer_shadow_security_settings IS
    'Durable per-tenant controls for tokenizer Tier 1 shadow provider calls. Absence of a row means raw-text PII shadow disabled and count_tokens quota 0.';
COMMENT ON COLUMN tokenizer_shadow_security_settings.pii_shadow_enabled IS
    'Tenant-level explicit opt-in for sending raw prompt text to provider count_tokens APIs.';
COMMENT ON COLUMN tokenizer_shadow_security_settings.count_tokens_quota_per_minute IS
    'Per-provider count_tokens calls allowed per tenant per minute. Zero blocks all provider calls.';

CREATE TABLE tokenizer_count_tokens_quota_usage (
    tenant_id     UUID        NOT NULL,
    provider      TEXT        NOT NULL CHECK (octet_length(provider) BETWEEN 1 AND 64),
    window_start  TIMESTAMPTZ NOT NULL,
    used_count    INTEGER     NOT NULL DEFAULT 0 CHECK (used_count >= 0),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),

    PRIMARY KEY (tenant_id, provider, window_start)
);

ALTER TABLE tokenizer_count_tokens_quota_usage ENABLE ROW LEVEL SECURITY;
ALTER TABLE tokenizer_count_tokens_quota_usage FORCE ROW LEVEL SECURITY;

CREATE POLICY tokenizer_count_tokens_quota_usage_tenant_isolation
    ON tokenizer_count_tokens_quota_usage
    FOR ALL
    USING (
        tenant_id = COALESCE(
            NULLIF(current_setting('app.current_tenant_id', TRUE), ''),
            '00000000-0000-0000-0000-000000000000'
        )::uuid
    )
    WITH CHECK (
        tenant_id = COALESCE(
            NULLIF(current_setting('app.current_tenant_id', TRUE), ''),
            '00000000-0000-0000-0000-000000000000'
        )::uuid
    );

REVOKE SELECT, INSERT, UPDATE, DELETE ON tokenizer_count_tokens_quota_usage FROM PUBLIC;

GRANT SELECT, INSERT, UPDATE, DELETE
    ON tokenizer_count_tokens_quota_usage
    TO control_plane_application_role, tokenizer_shadow_runtime_role;

CREATE INDEX tokenizer_count_tokens_quota_usage_cleanup_idx
    ON tokenizer_count_tokens_quota_usage (tenant_id, window_start);

COMMENT ON TABLE tokenizer_count_tokens_quota_usage IS
    'Shared per-(tenant, provider, minute) usage ledger for tokenizer provider count_tokens quota. Tokenizer replicas claim quota here atomically so horizontal scaling cannot multiply the configured cap.';
COMMENT ON COLUMN tokenizer_count_tokens_quota_usage.window_start IS
    'UTC minute bucket from date_trunc(''minute'', clock_timestamp()).';

ALTER TABLE control_plane_audit_outbox
    DROP CONSTRAINT IF EXISTS control_plane_audit_outbox_event_type_check;

ALTER TABLE control_plane_audit_outbox
    ADD CONSTRAINT control_plane_audit_outbox_event_type_check
    CHECK (
        event_type ~ '^spendguard\.audit\.plugin_'
        OR event_type = 'spendguard.audit.tokenizer_sampling_rate_override.v1alpha1'
        OR event_type = 'spendguard.audit.tokenizer_shadow_security_settings.v1alpha1'
    );

DO $$
BEGIN
    SET LOCAL search_path = pg_catalog, pg_temp;
    PERFORM 1 FROM pg_class
     WHERE relname = 'tokenizer_shadow_security_settings'
       AND relrowsecurity = TRUE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'tokenizer_shadow_security_settings RLS not enabled after migration';
    END IF;
    PERFORM 1 FROM pg_policy
     WHERE polname = 'tokenizer_shadow_security_settings_tenant_isolation';
    IF NOT FOUND THEN
        RAISE EXCEPTION 'tokenizer_shadow_security_settings_tenant_isolation policy missing';
    END IF;
    PERFORM 1 FROM pg_policy
     WHERE polname = 'tokenizer_count_tokens_quota_usage_tenant_isolation';
    IF NOT FOUND THEN
        RAISE EXCEPTION 'tokenizer_count_tokens_quota_usage_tenant_isolation policy missing';
    END IF;
END $$;
