-- Phase 5 GA hardening S10: provider usage ingestion foundation.
--
-- Provider usage records (LLM token counts, model id, request id,
-- timestamps from a billing API or webhook) are NOT trusted to
-- mutate the ledger directly. They land here first, get matched
-- against existing reservations, and only then drive the existing
-- provider_report / invoice_reconcile flows.
--
-- Tables:
--
--   * `provider_usage_records` — every raw usage observation. The
--     "raw source evidence" the spec review standard demands. Rows
--     are immutable.
--   * `provider_usage_quarantine` — records that didn't cleanly match
--     exactly one reservation (zero matches, ambiguous N>1 matches,
--     time-window mismatch, missing required fields). Operators
--     inspect to investigate; future reaper resolves stale rows.
--
-- Matching algorithm (S10-followup wires the SP that consumes these):
--   1. Strict match by (tenant_id, provider, llm_call_id) when
--      llm_call_id is present.
--   2. Otherwise (provider, provider_request_id, run_id) plus a
--      time-window predicate against `reservations.created_at`.
--   3. On exact-1-match → emit ProviderReport via existing handler.
--   4. On 0 matches → quarantine with reason='unmatched'.
--   5. On N>1 matches → quarantine with reason='ambiguous_match'.
--      FAIL_CLOSED for ledger mutation per spec.
--
-- Idempotency (per-provider):
--   `idempotency_key = sha256(provider || ':' || provider_request_id ||
--    ':' || provider_account || ':' || event_kind)`.
--   Computed by webhook_receiver / poller and stored on the row.

CREATE TABLE provider_usage_records (
    -- Internal id; rows never mutate post-insert.
    record_id          UUID NOT NULL DEFAULT gen_random_uuid()
                       PRIMARY KEY,

    -- Provider identity.
    provider           TEXT NOT NULL,
    provider_account   TEXT NOT NULL,
    provider_request_id TEXT,

    -- The provider's own event id when the record came from a
    -- webhook. NULL for poller-discovered records that don't carry
    -- one.
    provider_event_id  TEXT,

    -- Match candidates (what the matcher uses).
    tenant_id          UUID NOT NULL,
    llm_call_id        TEXT,
    run_id             UUID,
    model_id           TEXT,

    -- Time fields.
    observed_at        TIMESTAMPTZ NOT NULL,
    received_at        TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),

    -- Idempotency (matches webhook_receiver canonical_hash).
    idempotency_key    TEXT NOT NULL,

    -- Raw provider payload preserved for forensics.
    raw_payload        JSONB NOT NULL,

    -- Normalized usage fields. Tokens are atomic units.
    prompt_tokens      BIGINT,
    completion_tokens  BIGINT,
    total_tokens       BIGINT,
    cost_micros_usd    BIGINT,

    -- Match outcome — populated by the reconciliation SP. NULL
    -- means "not yet processed".
    match_state        TEXT CHECK (match_state IN
                          ('pending', 'matched', 'quarantined')),
    matched_reservation_id UUID,
    quarantine_id      UUID,

    -- Constraints.
    CONSTRAINT provider_usage_records_idempotency_uq
        UNIQUE (idempotency_key)
);

CREATE INDEX provider_usage_records_tenant_observed_idx
    ON provider_usage_records (tenant_id, observed_at DESC);

CREATE INDEX provider_usage_records_match_state_idx
    ON provider_usage_records (match_state, received_at DESC)
    WHERE match_state IN ('pending', 'quarantined');

CREATE INDEX provider_usage_records_llm_call_idx
    ON provider_usage_records (tenant_id, provider, llm_call_id)
    WHERE llm_call_id IS NOT NULL;

CREATE INDEX provider_usage_records_request_idx
    ON provider_usage_records (tenant_id, provider, provider_request_id, run_id);

COMMENT ON TABLE provider_usage_records IS
    'S10: normalized provider usage observations. Immutable. Matched against reservations by reconciliation SP; never directly mutates ledger.';

CREATE TABLE provider_usage_quarantine (
    quarantine_id      UUID NOT NULL DEFAULT gen_random_uuid()
                       PRIMARY KEY,
    record_id          UUID NOT NULL REFERENCES provider_usage_records(record_id),
    reason             TEXT NOT NULL CHECK (reason IN
                          ('unmatched',
                           'ambiguous_match',
                           'time_window_mismatch',
                           'missing_required_fields',
                           'pricing_unknown')),
    candidate_reservation_ids UUID[] NOT NULL DEFAULT ARRAY[]::UUID[],
    notes              TEXT,
    quarantined_at     TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    -- Resolution tracking. Operators or a reaper update these fields.
    -- The original row in provider_usage_records is never mutated.
    resolved_at        TIMESTAMPTZ,
    resolved_by        TEXT,
    resolved_reservation_id UUID,
    resolution_notes   TEXT
);

CREATE INDEX provider_usage_quarantine_pending_idx
    ON provider_usage_quarantine (quarantined_at DESC)
    WHERE resolved_at IS NULL;

CREATE INDEX provider_usage_quarantine_reason_idx
    ON provider_usage_quarantine (reason, quarantined_at DESC);

COMMENT ON TABLE provider_usage_quarantine IS
    'S10: provider usage records that did NOT cleanly match exactly one reservation. Append-only audit; operator resolves by writing resolution fields.';
