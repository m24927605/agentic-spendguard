-- Phase 5 GA hardening S13: pricing authority audit + staleness.
--
-- Builds on 0006 (pricing_table + pricing_versions). S13 adds:
--
--   * `pricing_sync_attempts` — every periodic sync attempt + its
--     outcome. Operators monitor `last_success_at` to enforce the
--     max-staleness policy.
--   * `pricing_overrides_audit` — append-only audit of every manual
--     pricing edit. Captures reviewer identity, justification, and
--     the resulting pricing_version that the override created.
--
-- Spec invariant: "manual override requires audit event and reviewer
-- identity." Schema-level CHECK enforces this. The pricing-sync
-- worker (S13-followup) writes these rows; S13's surface ships the
-- schema + the documented staleness policy.

CREATE TABLE pricing_sync_attempts (
    attempt_id          UUID NOT NULL DEFAULT gen_random_uuid()
                        PRIMARY KEY,
    provider            TEXT NOT NULL,
    started_at          TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    finished_at         TIMESTAMPTZ,
    outcome             TEXT NOT NULL CHECK (outcome IN
                            ('in_progress', 'success', 'no_change',
                             'transient_failure', 'permanent_failure')),
    -- Populated on success: the pricing_version this sync produced.
    -- Sources may have not changed → 'no_change' outcome with
    -- new_pricing_version = NULL. Operators distinguish "sync
    -- worked, nothing changed" from "sync failed".
    new_pricing_version TEXT REFERENCES pricing_versions(pricing_version),
    rows_changed        INT NOT NULL DEFAULT 0,
    error_message       TEXT,
    -- For S13 staleness alerting:
    --   alertable iff (latest sync.outcome != success
    --                  AND now() - latest_success_at > max_staleness).
    duration_ms         INT
);

CREATE INDEX pricing_sync_attempts_provider_started_idx
    ON pricing_sync_attempts (provider, started_at DESC);

CREATE INDEX pricing_sync_attempts_last_success_idx
    ON pricing_sync_attempts (provider, finished_at DESC)
    WHERE outcome IN ('success', 'no_change');

COMMENT ON TABLE pricing_sync_attempts IS
    'S13: every pricing-sync run logged here. Operators query for last_success_at per provider; alert on gap > max_staleness_seconds.';

-- Manual override audit. Operators who hand-edit pricing_table
-- (e.g. when a provider adds a new model before the sync adapter
-- handles it) MUST write a row here at the same time. Application
-- writers enforce this; defense in depth via app code, not just SQL,
-- because an operator with direct DB access can always bypass.
CREATE TABLE pricing_overrides_audit (
    override_id           UUID NOT NULL DEFAULT gen_random_uuid()
                          PRIMARY KEY,
    pricing_version       TEXT NOT NULL REFERENCES pricing_versions(pricing_version),
    -- Reviewer identity: principal.subject from S17 auth (typically
    -- the human operator's email + IdP issuer).
    reviewer_subject      TEXT NOT NULL,
    reviewer_issuer       TEXT NOT NULL,
    -- Reason: free-form. Must be non-empty (CHECK).
    reason                TEXT NOT NULL CHECK (length(reason) > 0),
    -- Snapshot of the rows that changed.
    affected_rows         JSONB NOT NULL,
    overridden_at         TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    -- Distinguishes "added new model" vs "corrected typo" vs
    -- "rolled back to prior version" without parsing the reason.
    override_kind         TEXT NOT NULL CHECK (override_kind IN
                              ('add_model',
                               'correct_price',
                               'rollback_to_prior',
                               'emergency_freeze',
                               'other'))
);

CREATE INDEX pricing_overrides_audit_pricing_version_idx
    ON pricing_overrides_audit (pricing_version, overridden_at DESC);

CREATE INDEX pricing_overrides_audit_reviewer_idx
    ON pricing_overrides_audit (reviewer_subject, overridden_at DESC);

COMMENT ON TABLE pricing_overrides_audit IS
    'S13: every manual pricing edit logged with reviewer identity + reason. Append-only; operators query for change-management reviews.';

-- Helper view: latest sync status per provider (operators dashboard
-- and staleness alerter consume this).
CREATE OR REPLACE VIEW pricing_sync_status AS
    SELECT DISTINCT ON (provider)
           provider,
           started_at AS last_attempt_at,
           outcome AS last_outcome,
           (
             SELECT max(finished_at)
               FROM pricing_sync_attempts a2
              WHERE a2.provider = a1.provider
                AND a2.outcome IN ('success', 'no_change')
           ) AS last_success_at,
           new_pricing_version AS last_pricing_version,
           error_message AS last_error
      FROM pricing_sync_attempts a1
     ORDER BY provider, started_at DESC;

COMMENT ON VIEW pricing_sync_status IS
    'S13: latest sync status per provider. Used by dashboard + staleness alerter.';
