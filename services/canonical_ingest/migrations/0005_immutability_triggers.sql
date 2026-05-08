-- Immutability triggers (per Trace §10.2 immutable_audit_log + canonical_raw_log).
-- canonical_events: append-only — no UPDATE / DELETE.
-- audit_outcome_quarantine: state machine UPDATEs allowed for state +
--   state_changed_at + released_to_event_id only; everything else immutable.
-- schema_bundles: append-only.

CREATE OR REPLACE FUNCTION reject_canonical_event_mutation()
RETURNS TRIGGER AS $$
BEGIN
    RAISE EXCEPTION 'canonical_events are immutable; corrections via tombstone events'
        USING ERRCODE = '42P10';
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER canonical_events_no_update_delete
    BEFORE UPDATE OR DELETE ON canonical_events
    FOR EACH ROW EXECUTE FUNCTION reject_canonical_event_mutation();

-- canonical_events_global_keys: append-only.
CREATE TRIGGER canonical_events_global_keys_no_update_delete
    BEFORE UPDATE OR DELETE ON canonical_events_global_keys
    FOR EACH ROW EXECUTE FUNCTION reject_canonical_event_mutation();

-- schema_bundles: append-only.
CREATE TRIGGER schema_bundles_no_update_delete
    BEFORE UPDATE OR DELETE ON schema_bundles
    FOR EACH ROW EXECUTE FUNCTION reject_canonical_event_mutation();

-- audit_outcome_quarantine: only state-machine columns may UPDATE.
CREATE OR REPLACE FUNCTION reject_quarantine_immutable_columns()
RETURNS TRIGGER AS $$
BEGIN
    IF (OLD.quarantine_id, OLD.event_id, OLD.tenant_id, OLD.decision_id,
        OLD.storage_class, OLD.producer_id, OLD.producer_sequence,
        OLD.producer_signature, OLD.signing_key_id, OLD.schema_bundle_id,
        OLD.schema_bundle_hash, OLD.event_type, OLD.specversion, OLD.source,
        OLD.event_time, OLD.datacontenttype, OLD.payload_json,
        OLD.payload_blob_ref, OLD.region_id, OLD.ingest_shard_id,
        OLD.ingest_log_offset, OLD.run_id, OLD.quarantined_at,
        OLD.orphan_after)
       IS DISTINCT FROM
       (NEW.quarantine_id, NEW.event_id, NEW.tenant_id, NEW.decision_id,
        NEW.storage_class, NEW.producer_id, NEW.producer_sequence,
        NEW.producer_signature, NEW.signing_key_id, NEW.schema_bundle_id,
        NEW.schema_bundle_hash, NEW.event_type, NEW.specversion, NEW.source,
        NEW.event_time, NEW.datacontenttype, NEW.payload_json,
        NEW.payload_blob_ref, NEW.region_id, NEW.ingest_shard_id,
        NEW.ingest_log_offset, NEW.run_id, NEW.quarantined_at,
        NEW.orphan_after) THEN
        RAISE EXCEPTION 'audit_outcome_quarantine immutable columns cannot be changed'
            USING ERRCODE = '42P10';
    END IF;
    -- Allowed transitions: awaiting_decision -> released | orphaned.
    IF OLD.state = 'released' OR OLD.state = 'orphaned' THEN
        RAISE EXCEPTION 'audit_outcome_quarantine state is terminal'
            USING ERRCODE = '42P10';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER audit_outcome_quarantine_immutability
    BEFORE UPDATE ON audit_outcome_quarantine
    FOR EACH ROW EXECUTE FUNCTION reject_quarantine_immutable_columns();

CREATE TRIGGER audit_outcome_quarantine_no_delete
    BEFORE DELETE ON audit_outcome_quarantine
    FOR EACH ROW EXECUTE FUNCTION reject_canonical_event_mutation();

-- ============================================================================
-- Per-decision sequence enforcement (Stage 2 §4.8).
-- For each (tenant_id, decision_id):
--   * audit.outcome must come strictly after audit.decision.
--   * Per-decision uniqueness for each event_type (handled via partial UNIQUE
--     indexes on canonical_events_global_keys).
-- The trigger is enforced at INSERT TIME on BOTH canonical_events_global_keys
-- and canonical_events. App code SHOULD redirect outcomes lacking a preceding
-- decision to the quarantine table; the triggers are defense-in-depth against
-- direct INSERTs.
-- ============================================================================
CREATE OR REPLACE FUNCTION assert_audit_outcome_has_preceding_decision()
RETURNS TRIGGER AS $$
BEGIN
    IF NEW.event_type = 'spendguard.audit.outcome' THEN
        IF NEW.decision_id IS NULL THEN
            RAISE EXCEPTION 'audit.outcome event missing decision_id'
                USING ERRCODE = '23514';
        END IF;
        IF NOT EXISTS (
            SELECT 1 FROM canonical_events_global_keys
             WHERE tenant_id = NEW.tenant_id
               AND decision_id = NEW.decision_id
               AND event_type = 'spendguard.audit.decision'
        ) THEN
            RAISE EXCEPTION
                'AWAITING_PRECEDING_DECISION: audit.outcome for decision % has no audit.decision yet',
                NEW.decision_id
                USING ERRCODE = 'P0002';
        END IF;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER canonical_events_global_keys_audit_sequence
    BEFORE INSERT ON canonical_events_global_keys
    FOR EACH ROW EXECUTE FUNCTION assert_audit_outcome_has_preceding_decision();

-- Mirror trigger on canonical_events: prevents direct INSERTs that bypass
-- the global_keys table. This complements the role REVOKEs and the
-- "INSERT into canonical_events only via handler tx" convention.
CREATE TRIGGER canonical_events_audit_sequence_mirror
    BEFORE INSERT ON canonical_events
    FOR EACH ROW EXECUTE FUNCTION assert_audit_outcome_has_preceding_decision();

-- ============================================================================
-- Restricted writer role.
-- ============================================================================
CREATE ROLE canonical_ingest_application_role NOINHERIT;

REVOKE INSERT, UPDATE, DELETE ON canonical_events FROM PUBLIC;
REVOKE INSERT, UPDATE, DELETE ON canonical_events_global_keys FROM PUBLIC;
REVOKE INSERT, UPDATE, DELETE ON canonical_ingest_positions FROM PUBLIC;
REVOKE INSERT, UPDATE, DELETE ON schema_bundles FROM PUBLIC;
REVOKE INSERT, UPDATE, DELETE ON audit_outcome_quarantine FROM PUBLIC;

GRANT INSERT ON canonical_events TO canonical_ingest_application_role;
GRANT INSERT ON canonical_events_global_keys TO canonical_ingest_application_role;
GRANT INSERT ON canonical_ingest_positions TO canonical_ingest_application_role;
GRANT INSERT ON schema_bundles TO canonical_ingest_application_role;
GRANT INSERT, UPDATE ON audit_outcome_quarantine TO canonical_ingest_application_role;
GRANT EXECUTE ON FUNCTION next_ingest_offset(TEXT, TEXT)
    TO canonical_ingest_application_role;

CREATE ROLE canonical_ingest_reader_role;
GRANT SELECT ON
    canonical_events, canonical_events_global_keys, canonical_ingest_positions,
    schema_bundles, audit_outcome_quarantine, ingest_shards
TO canonical_ingest_reader_role;
