-- ============================================================================
-- 0002_audit_outbox.sql — control_plane local audit_outbox for plugin lifecycle.
--
-- Spec ancestors:
--   - docs/output-predictor-plugin-contract-v1alpha1.md §8 (control plane
--     emits signed CloudEvents for plugin lifecycle mutations)
--   - SLICE_07 R2 M1 (Security F3) — register/update/delete/force_reset
--     handlers must emit `spendguard.audit.plugin_*` CloudEvents
--   - SLICE_05 R2 B2 — audit-routed event_type prefix `spendguard.audit.*`
--
-- ## Why a local outbox in control_plane (vs. cross-DB write to ledger)
--
-- The control_plane DB owns the plugin registry rows. A
-- transactional outbox MUST live in the same DB as the row it audits
-- so the INSERT into the outbox happens atomically with the row
-- mutation (single tx, single COMMIT). Writing to the ledger DB's
-- audit_outbox from the control_plane handler would split the commit
-- across two DBs — the row mutation could land while the audit row
-- is lost on a network blip.
--
-- A separate forwarder (out of scope for SLICE_07 R2) picks up rows
-- from this control_plane audit_outbox + appends them to the
-- canonical_ingest AppendEvents RPC. The forwarder pattern mirrors the
-- ledger-side outbox forwarder so the audit chain semantics
-- (per-producer monotonic producer_sequence, signed cloudevent_payload)
-- are uniform.
--
-- v1alpha1 ships the row capture + tx-bound INSERT. The forwarder
-- emission to canonical_ingest is staged for a follow-up SLICE-extra
-- (tracked as GH issue per the R2 outputs section). For now the rows
-- accumulate; operators can drain them via direct SQL inspection
-- (`SELECT * FROM control_plane_audit_outbox ORDER BY producer_sequence`).
--
-- ## Schema stylistic alignment
--
-- - UUIDv7 minted application-side (no DEFAULT gen_random_uuid())
-- - TIMESTAMPTZ with TZ-explicit default (clock_timestamp())
-- - psql autocommit (no BEGIN/COMMIT wrapping the migration)
-- - SET LOCAL search_path = pg_catalog, pg_temp in DO blocks
-- - No down migration file per SLICE_03 R2 M3 convention
-- ============================================================================

CREATE TABLE control_plane_audit_outbox (
    -- UUIDv7 minted by the handler (so producer_sequence ordering can
    -- be enforced + the row is addressable for the forwarder).
    audit_outbox_id          UUID         PRIMARY KEY,

    -- Tenant scope — RLS-bound. The handler sets
    -- `app.current_tenant_id` before INSERT.
    tenant_id                UUID         NOT NULL,

    -- CloudEvent type per SLICE_05 R2 B2 audit-routed prefix:
    --   spendguard.audit.plugin_registered.v1alpha1
    --   spendguard.audit.plugin_updated.v1alpha1
    --   spendguard.audit.plugin_deleted.v1alpha1
    --   spendguard.audit.plugin_force_reset.v1alpha1
    event_type               TEXT         NOT NULL
                             CHECK (event_type ~ '^spendguard\.audit\.plugin_'),

    -- Full CloudEvent v1.0 payload (specversion, type, id, source,
    -- tenantid, data, producer_sequence). JSONB so the forwarder can
    -- replay verbatim.
    cloudevent_payload       JSONB        NOT NULL,

    -- Signature hex — v1alpha1 ships empty signature (DisabledSigner);
    -- production wires Ed25519 PKCS8 signing via a SLICE-extra. The
    -- column shape is correct so the forwarder doesn't need a schema
    -- migration to add signing.
    cloudevent_payload_signature_hex TEXT NOT NULL DEFAULT '',

    -- Producer sequence per-tenant. Monotonic; the forwarder uses this
    -- to detect gaps. Application-supplied because the handler is the
    -- producer.
    producer_sequence        BIGINT       NOT NULL,

    -- Forwarder state. NULL = pending; non-NULL = forwarded at this
    -- wallclock. Forwarder updates atomically before COMMIT.
    forwarded_at             TIMESTAMPTZ,

    -- When the handler wrote the row.
    created_at               TIMESTAMPTZ  NOT NULL DEFAULT clock_timestamp(),

    UNIQUE (tenant_id, producer_sequence)
);

-- Forwarder sweep index: pick the next pending row per producer
-- sequence. Partial index keeps it small.
CREATE INDEX control_plane_audit_outbox_pending_idx
    ON control_plane_audit_outbox (tenant_id, producer_sequence)
    WHERE forwarded_at IS NULL;

-- RLS — handlers MUST set app.current_tenant_id before INSERT.
ALTER TABLE control_plane_audit_outbox ENABLE ROW LEVEL SECURITY;
ALTER TABLE control_plane_audit_outbox FORCE ROW LEVEL SECURITY;

CREATE POLICY control_plane_audit_outbox_tenant_isolation
    ON control_plane_audit_outbox
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

-- Audit immutability: block DELETE so a forwarder bug can't drop rows.
CREATE OR REPLACE FUNCTION control_plane_audit_outbox_block_delete()
    RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    SET LOCAL search_path = pg_catalog, pg_temp;
    RAISE EXCEPTION
        'audit-immutable table %: DELETE forbidden (SLICE_07 R2 M1 invariant)',
        TG_TABLE_NAME
        USING ERRCODE = '42P01';
END;
$$;

CREATE TRIGGER control_plane_audit_outbox_block_delete_trg
    BEFORE DELETE ON control_plane_audit_outbox
    FOR EACH ROW
    EXECUTE FUNCTION control_plane_audit_outbox_block_delete();

-- Privilege boundary.
REVOKE SELECT, INSERT, UPDATE, DELETE ON control_plane_audit_outbox FROM PUBLIC;
GRANT SELECT, INSERT, UPDATE ON control_plane_audit_outbox TO control_plane_application_role;
-- UPDATE for the forwarder to flip forwarded_at; no DELETE grant.
GRANT SELECT ON control_plane_audit_outbox TO control_plane_reader_role;

COMMENT ON TABLE control_plane_audit_outbox IS
    'SLICE_07 R2 M1: per-tenant audit_outbox for plugin lifecycle CloudEvents. Forwarder (staged for SLICE-extra) drains rows into canonical_ingest AppendEvents. Per-tenant producer_sequence enforced UNIQUE.';

COMMENT ON COLUMN control_plane_audit_outbox.event_type IS
    'CloudEvent type per SLICE_05 R2 B2 audit-routed prefix (`spendguard.audit.plugin_*.v1alpha1`).';
COMMENT ON COLUMN control_plane_audit_outbox.producer_sequence IS
    'Per-tenant monotonic sequence; UNIQUE(tenant_id, producer_sequence) lets the forwarder detect gaps.';
COMMENT ON COLUMN control_plane_audit_outbox.forwarded_at IS
    'NULL until the forwarder relays the event to canonical_ingest. Indexed (partial) for fast sweep.';

-- DO-block smoke check.
DO $$
BEGIN
    SET LOCAL search_path = pg_catalog, pg_temp;
    PERFORM 1 FROM pg_class
        WHERE relname = 'control_plane_audit_outbox' AND relrowsecurity = TRUE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'control_plane_audit_outbox RLS not enabled after migration';
    END IF;
    PERFORM 1 FROM pg_policy
        WHERE polname = 'control_plane_audit_outbox_tenant_isolation';
    IF NOT FOUND THEN
        RAISE EXCEPTION 'control_plane_audit_outbox_tenant_isolation policy missing';
    END IF;
END $$;
