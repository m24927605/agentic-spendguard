-- Audit outcome quarantine (Stage 2 §4.8).
--
-- audit.outcome events arriving without a preceding audit.decision are
-- quarantined here. Released when the matching decision arrives, or
-- promoted to ORPHAN_OUTCOME after 30s.
--
-- Release path: the AppendEvents handler, after committing an
-- audit.decision row, releases matching quarantined outcomes by
-- (a) inserting them into canonical_events + canonical_events_global_keys,
-- (b) UPDATEing audit_outcome_quarantine.state to 'released'.
-- Quarantine rows are NEVER deleted (immutability trigger forbids DELETE);
-- 'released' / 'orphaned' is terminal state.
--
-- Reaper process (separate binary; deferred to vertical slice expansion)
-- scans this table on a 1s tick to:
--   * mark quarantined > orphan_after as 'orphaned'; emit alert.
--   * (release-on-arrival happens inline in the handler, not in the reaper.)

CREATE TABLE audit_outcome_quarantine (
    quarantine_id       UUID PRIMARY KEY,
    event_id            UUID NOT NULL UNIQUE,           -- the outcome event_id
    tenant_id           UUID NOT NULL,
    decision_id         UUID NOT NULL,                  -- the decision_id this awaits

    -- Full event payload preserved for later release insert.
    storage_class       TEXT NOT NULL,
    producer_id         TEXT NOT NULL,
    producer_sequence   BIGINT NOT NULL,
    producer_signature  BYTEA NOT NULL,
    signing_key_id      TEXT NOT NULL,
    schema_bundle_id    UUID NOT NULL,
    schema_bundle_hash  BYTEA NOT NULL,
    event_type          TEXT NOT NULL,                  -- always "spendguard.audit.outcome"
    specversion         TEXT NOT NULL,
    source              TEXT NOT NULL,
    event_time          TIMESTAMPTZ NOT NULL,
    datacontenttype     TEXT NOT NULL,
    payload_json        JSONB,
    payload_blob_ref    TEXT,
    region_id           TEXT NOT NULL,
    ingest_shard_id     TEXT NOT NULL,
    ingest_log_offset   BIGINT NOT NULL,
    run_id              UUID,

    -- Quarantine state.
    state               TEXT NOT NULL DEFAULT 'awaiting_decision'
                            CHECK (state IN ('awaiting_decision', 'released',
                                             'orphaned')),
    quarantined_at      TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    state_changed_at    TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    released_to_event_id UUID,                          -- when state=released, points to the canonical_events row

    -- 30s deadline; reaper marks orphan past this.
    orphan_after        TIMESTAMPTZ NOT NULL
);

CREATE INDEX audit_outcome_quarantine_pending_idx
    ON audit_outcome_quarantine (state, orphan_after)
    WHERE state = 'awaiting_decision';

CREATE INDEX audit_outcome_quarantine_decision_idx
    ON audit_outcome_quarantine (tenant_id, decision_id)
    WHERE state = 'awaiting_decision';
