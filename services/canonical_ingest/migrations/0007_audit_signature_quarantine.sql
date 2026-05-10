-- Phase 5 GA hardening S8: signature-failure quarantine.
--
-- Distinct from the existing `audit_outcome_quarantine` table (0003)
-- which holds outcomes awaiting their preceding decision. This table
-- holds events that FAILED signature verification (or matched a
-- known-degraded signing mode like pre-S6 / disabled in strict mode).
--
-- Append-only, immutable for the application. Only an out-of-band
-- forensics tool (or operator with direct DB access) may delete rows.
-- Defense in depth: the canonical_ingest service role's grants do NOT
-- include DELETE on this table — that's enforced at deploy time via
-- the role bootstrap (S8 doesn't add the GRANT change because Phase
-- 5 chart values don't yet pin per-role grants; tracked as follow-up).

CREATE TABLE audit_signature_quarantine (
    -- Pure forensics row id.
    quarantine_id     UUID        NOT NULL DEFAULT gen_random_uuid()
                      PRIMARY KEY,

    -- Producer-claimed event identity (what the producer SAID this row
    -- is). Trust nothing here for replay — the quarantine is exactly
    -- because we don't trust this row.
    claimed_event_id  TEXT        NOT NULL,
    claimed_tenant_id TEXT        NOT NULL,
    claimed_event_type TEXT       NOT NULL,
    claimed_decision_id TEXT,
    claimed_run_id    TEXT,
    claimed_producer_id TEXT      NOT NULL,
    claimed_producer_sequence BIGINT NOT NULL,

    -- The bytes the producer claimed are the canonical encoding (we
    -- store this so a future re-verifier can re-derive the truth).
    -- For sidecar/webhook/ttl_sweeper this is the proto-encoded
    -- CloudEvent with producer_signature cleared. For ledger-minted
    -- rows this is the JSON serialization of decision_payload.
    -- Capped at 1MiB; oversized rows hit the CHECK and are dropped
    -- with a metric (cannot quarantine a row we can't store).
    claimed_canonical_bytes BYTEA NOT NULL,

    -- Whatever the producer wrote for the signature — empty when the
    -- producer used disabled mode.
    claimed_signature BYTEA       NOT NULL,
    claimed_signing_key_id TEXT   NOT NULL,
    claimed_signing_algorithm TEXT NOT NULL,

    -- Why it landed here. Mirrors VerifyFailure enum in
    -- spendguard-signing/src/lib.rs.
    reason            TEXT        NOT NULL CHECK (reason IN
                          ('unknown_key',
                           'invalid_signature',
                           'pre_s6',
                           'disabled',
                           'oversized_canonical',
                           'schema_failure')),

    -- Free-form JSONB for operator-readable context: which schema
    -- bundle was claimed, the route, batch producer_id, etc.
    debug_info        JSONB       NOT NULL DEFAULT '{}'::JSONB,

    received_at       TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),

    CONSTRAINT canonical_bytes_max_size
        CHECK (octet_length(claimed_canonical_bytes) <= 1048576)
);

CREATE INDEX audit_signature_quarantine_received_idx
    ON audit_signature_quarantine (received_at DESC);

CREATE INDEX audit_signature_quarantine_reason_idx
    ON audit_signature_quarantine (reason, received_at DESC);

CREATE INDEX audit_signature_quarantine_key_id_idx
    ON audit_signature_quarantine (claimed_signing_key_id, received_at DESC);

CREATE INDEX audit_signature_quarantine_tenant_idx
    ON audit_signature_quarantine (claimed_tenant_id, received_at DESC);

COMMENT ON TABLE audit_signature_quarantine IS
    'S8: events that failed signature verification. Append-only. Operators inspect to triage compromised producers, rotated-but-not-yet-trusted keys, or attacker-style probes.';
