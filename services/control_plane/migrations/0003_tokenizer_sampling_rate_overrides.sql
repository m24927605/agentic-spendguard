-- ============================================================================
-- 0003_tokenizer_sampling_rate_overrides.sql — Tokenizer Tier 1 shadow
-- sampling-rate override persistence.
--
-- Spec ancestors:
--   - docs/tokenizer-service-spec-v1alpha1.md §4.1 (per-tenant/model
--     operator override surface)
--   - docs/slices/HARDEN_03_production_blocker_gh_triage.md §2 (#137)
--
-- Control plane originally shipped POST/GET /v1/tokenizer/sampling-rate as
-- an echo-only skeleton. This table makes overrides durable across pod
-- restarts and gives the tokenizer polling path a stable source of truth.
-- ============================================================================

CREATE TABLE tokenizer_sampling_rate_overrides (
    tenant_id   UUID        NOT NULL,
    model       TEXT        NOT NULL CHECK (octet_length(model) BETWEEN 1 AND 256),
    rate        DOUBLE PRECISION NOT NULL CHECK (rate >= 0.0 AND rate <= 1.0),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_by  TEXT        NOT NULL CHECK (octet_length(updated_by) BETWEEN 1 AND 256),

    PRIMARY KEY (tenant_id, model)
);

ALTER TABLE tokenizer_sampling_rate_overrides ENABLE ROW LEVEL SECURITY;
ALTER TABLE tokenizer_sampling_rate_overrides FORCE ROW LEVEL SECURITY;

CREATE POLICY tokenizer_sampling_rate_overrides_tenant_isolation
    ON tokenizer_sampling_rate_overrides
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

REVOKE SELECT, INSERT, UPDATE, DELETE ON tokenizer_sampling_rate_overrides FROM PUBLIC;

GRANT SELECT, INSERT, UPDATE, DELETE
    ON tokenizer_sampling_rate_overrides
    TO control_plane_application_role;

GRANT SELECT ON tokenizer_sampling_rate_overrides TO control_plane_reader_role;

COMMENT ON TABLE tokenizer_sampling_rate_overrides IS
    'Durable per-(tenant_id, model) Tier 1 tokenizer shadow sampling-rate overrides. Written by control_plane POST /v1/tokenizer/sampling-rate and read by GET/tokenizer polling paths.';
COMMENT ON COLUMN tokenizer_sampling_rate_overrides.rate IS
    'Sampling probability in [0.0, 1.0]. 0 disables shadow sampling for the tenant/model; 1 forces 100% sampling.';

-- The local control-plane audit outbox originally admitted only plugin_* event
-- types. Tokenizer sampling-rate changes are operator mutations too, so allow
-- the new audit type without weakening the spendguard.audit.* prefix gate.
ALTER TABLE control_plane_audit_outbox
    DROP CONSTRAINT IF EXISTS control_plane_audit_outbox_event_type_check;

ALTER TABLE control_plane_audit_outbox
    ADD CONSTRAINT control_plane_audit_outbox_event_type_check
    CHECK (
        event_type ~ '^spendguard\.audit\.plugin_'
        OR event_type = 'spendguard.audit.tokenizer_sampling_rate_override.v1alpha1'
    );

DO $$
BEGIN
    SET LOCAL search_path = pg_catalog, pg_temp;
    PERFORM 1 FROM pg_class
        WHERE relname = 'tokenizer_sampling_rate_overrides'
          AND relrowsecurity = TRUE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'tokenizer_sampling_rate_overrides RLS not enabled after migration';
    END IF;
    PERFORM 1 FROM pg_policy
        WHERE polname = 'tokenizer_sampling_rate_overrides_tenant_isolation';
    IF NOT FOUND THEN
        RAISE EXCEPTION 'tokenizer_sampling_rate_overrides_tenant_isolation policy missing';
    END IF;
END $$;
