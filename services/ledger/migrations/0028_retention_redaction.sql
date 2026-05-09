-- Phase 5 GA hardening S19: retention + redaction + tenant data policy.
--
-- Three deliverables in one migration:
--   1. `tenant_data_policy` — per-tenant retention + redaction config.
--   2. `retention_sweeper_log` — audit log for the background sweeper.
--   3. Defense-in-depth triggers asserting the spec invariants:
--      "Retention code cannot delete ledger/audit invariants" — DELETE
--      on canonical_events / audit_outbox / ledger_transactions /
--      ledger_entries is REJECTED at the trigger layer regardless of
--      the application-level role.
--
-- Out of scope here (S19-followup):
--   * Retention sweeper service (background job that scans
--     provider_usage_records.raw_payload + redacts per policy).
--   * Redaction helper module in dashboard / canonical_ingest
--     (used at export time before bytes leave the service boundary).
--   * Data classification doc page (catalog of every event field +
--     class: prompt | metadata | pricing | identity | provider_secret).

-- =====================================================================
-- tenant_data_policy
-- =====================================================================

CREATE TABLE tenant_data_policy (
    tenant_id                  UUID NOT NULL PRIMARY KEY,

    -- Audit chain retention. Compliance window for the IMMUTABLE
    -- canonical_events + audit_outbox rows. Sweeper does NOT delete
    -- these even after the window — the trigger below blocks DELETE
    -- regardless of policy. The window is operator-attested for
    -- compliance reporting.
    audit_retention_days       INT NOT NULL DEFAULT 365 CHECK (audit_retention_days >= 1),

    -- Prompt retention. 0 = store only hashes/metadata, never the
    -- raw prompt text (canonical_events.cloudevent_payload's `data`
    -- field). Higher values = retain raw payload for the configured
    -- number of days, then redact in place (clear `data` field, set
    -- `redacted_at`).
    prompt_retention_days      INT NOT NULL DEFAULT 30 CHECK (prompt_retention_days >= 0),

    -- Provider raw payload retention. Separate axis — operators
    -- routinely keep prompts shorter than provider invoices for
    -- billing reconciliation. provider_usage_records.raw_payload
    -- is the target.
    provider_raw_retention_days INT NOT NULL DEFAULT 90 CHECK (provider_raw_retention_days >= 0),

    -- Export redaction policy. Operator-controlled list of field
    -- paths within cloudevent_payload that the export endpoint
    -- redacts before bytes leave the service boundary. Default
    -- redacts the most common prompt-bearing paths.
    export_redaction_field_paths JSONB NOT NULL DEFAULT
        '["data", "data.prompt", "data.messages", "data.input"]'::JSONB,

    -- Tombstone state. When TRUE, all writes for this tenant are
    -- rejected at the application layer; existing audit rows stay
    -- queryable per the spec invariant "Tombstoned tenant remains
    -- auditable."
    tombstoned                 BOOLEAN NOT NULL DEFAULT FALSE,
    tombstoned_at              TIMESTAMPTZ,
    tombstoned_by              TEXT,
    tombstoned_reason          TEXT,

    -- Metadata.
    created_at                 TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at                 TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_by                 TEXT,

    CONSTRAINT tenant_data_policy_tombstone_consistent
        CHECK (
            NOT tombstoned
            OR (tombstoned_at IS NOT NULL AND tombstoned_by IS NOT NULL)
        )
);

CREATE INDEX tenant_data_policy_active_idx
    ON tenant_data_policy (tenant_id)
    WHERE NOT tombstoned;

COMMENT ON TABLE tenant_data_policy IS
    'S19: per-tenant retention + redaction + tombstone state. Defaults conservative (1y audit, 30d prompts, 90d provider raw, redact common prompt paths).';

-- =====================================================================
-- retention_sweeper_log
-- =====================================================================
--
-- Append-only log of every sweeper pass. Captures: what was redacted
-- vs what was kept, error counts, the policy version that drove the
-- decision. Operators audit this for compliance reviews.

CREATE TABLE retention_sweeper_log (
    sweep_id              UUID NOT NULL DEFAULT gen_random_uuid()
                          PRIMARY KEY,
    started_at            TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    finished_at           TIMESTAMPTZ,
    outcome               TEXT NOT NULL CHECK (outcome IN
                              ('in_progress', 'success',
                               'partial_failure', 'permanent_failure')),
    -- What kind of sweep happened.
    sweep_kind            TEXT NOT NULL CHECK (sweep_kind IN
                              ('prompt_redaction',
                               'provider_raw_redaction',
                               'tombstone_check')),
    -- Counts.
    rows_examined         BIGINT NOT NULL DEFAULT 0,
    rows_redacted         BIGINT NOT NULL DEFAULT 0,
    rows_failed           BIGINT NOT NULL DEFAULT 0,
    -- Free-form details.
    error_summary         TEXT
);

CREATE INDEX retention_sweeper_log_kind_started_idx
    ON retention_sweeper_log (sweep_kind, started_at DESC);

COMMENT ON TABLE retention_sweeper_log IS
    'S19: append-only audit of every retention-sweeper pass. Compliance reviews query for "show me all redactions in the last 90 days".';

-- =====================================================================
-- Defense-in-depth triggers — block DELETE on audit-immutable tables.
-- =====================================================================
--
-- Spec invariant: "Retention code cannot delete ledger/audit
-- invariants." The retention sweeper REDACTS data (UPDATE in place
-- to clear the `data` BYTEA in cloudevent_payload) but never
-- DELETEs rows. These triggers reject any DELETE — including from
-- application-level roles AND from the sweeper itself — to enforce
-- the invariant at the database layer.

CREATE OR REPLACE FUNCTION block_audit_immutable_delete()
    RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION
        'audit-immutable table %: DELETE forbidden (S19 invariant)',
        TG_TABLE_NAME
        USING ERRCODE = '42P01';
END;
$$;

CREATE TRIGGER audit_outbox_block_delete
    BEFORE DELETE ON audit_outbox
    FOR EACH ROW
    EXECUTE FUNCTION block_audit_immutable_delete();

CREATE TRIGGER audit_outbox_global_keys_block_delete
    BEFORE DELETE ON audit_outbox_global_keys
    FOR EACH ROW
    EXECUTE FUNCTION block_audit_immutable_delete();

CREATE TRIGGER ledger_transactions_block_delete
    BEFORE DELETE ON ledger_transactions
    FOR EACH ROW
    EXECUTE FUNCTION block_audit_immutable_delete();

CREATE TRIGGER ledger_entries_block_delete
    BEFORE DELETE ON ledger_entries
    FOR EACH ROW
    EXECUTE FUNCTION block_audit_immutable_delete();

COMMENT ON FUNCTION block_audit_immutable_delete IS
    'S19: rejects DELETE on audit-immutable tables. Defense-in-depth complement to application-level GRANT.';

-- =====================================================================
-- updated_at trigger on tenant_data_policy.
-- =====================================================================

CREATE OR REPLACE FUNCTION tenant_data_policy_touch()
    RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    NEW.updated_at = clock_timestamp();
    -- Tombstone is one-way (cannot be un-tombstoned via UPDATE).
    IF OLD.tombstoned AND NOT NEW.tombstoned THEN
        RAISE EXCEPTION
            'tenant_data_policy %: cannot un-tombstone (S19 invariant)',
            OLD.tenant_id
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER tenant_data_policy_touch_updated
    BEFORE UPDATE ON tenant_data_policy
    FOR EACH ROW
    EXECUTE FUNCTION tenant_data_policy_touch();

COMMENT ON FUNCTION tenant_data_policy_touch IS
    'S19: maintains updated_at + enforces tombstone is one-way (cannot revert).';
