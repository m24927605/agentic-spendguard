-- =====================================================================
-- 0039: reservations_with_ttl_status_v1 view
--       (Cost Advisor P0.6 — issue #49)
-- =====================================================================
--
-- The audit-report §8.1 + spec v4 §0.1 establish that `idle_reservation
-- _rate_v1` rule (spec §5.1) cannot fire on the bare reservations
-- projection because:
--
--   * `current_state` allows ('reserved', 'committed', 'released',
--     'overrun_debt'). It does NOT distinguish a TTL-driven release
--     from an explicit application-driven release. Both land as
--     `current_state = 'released'`.
--   * The release REASON ('TTL_EXPIRED' vs 'RUN_ABORTED' vs
--     'EXPLICIT' etc.) is encoded in the corresponding
--     `spendguard.audit.outcome` event's payload, not on reservations.
--   * `ttl_seconds` is derivable from existing columns
--     (ttl_expires_at - created_at) but no current consumer exposes
--     that derived form.
--
-- This view materializes the (reservation, derived_state, ttl_seconds,
-- release_reason) tuple cost_advisor rules need. Reads only — no
-- mutation, no immutability concerns.
--
-- JOIN path (codex-verifiable):
--   reservations.source_ledger_transaction_id
--     → ledger_transactions.decision_id  (reserve tx's decision_id)
--     → audit_outbox.decision_id  (same decision_id; carries release outcome)
--     → cloudevent_payload->>'data_b64' decoded → {kind, reason, ...}
--
-- audit_outbox is the LEDGER-DB outbox table (services/ledger/migrations
-- /0009_audit_outbox.sql); rows here flow async to spendguard_canonical
-- via outbox_forwarder. Reading from audit_outbox keeps the view inside
-- spendguard_ledger so no cross-DB join is needed.

-- =====================================================================
-- Safe decode helper (codex P0.6 r1 P1 fix)
-- =====================================================================
--
-- audit_outbox.cloudevent_payload is JSONB with no shape constraint at
-- the DB layer (SPs validate at write time, but defense-in-depth says
-- a malformed row can still land via direct SQL or via a future
-- producer that drifts from the expected envelope shape). The view
-- decodes `data_b64` → bytea → UTF8 text → jsonb → `reason` key. Any
-- of these steps can RAISE EXCEPTION on bad input:
--   - decode(bad_base64, 'base64') → 'invalid input syntax for type bytea'
--   - convert_from(invalid_utf8, 'UTF8') → 'invalid byte sequence for encoding'
--   - bad_text::jsonb → 'invalid input syntax for type json'
--
-- An exception in the view's LATERAL subquery would abort the entire
-- SELECT, breaking cost_advisor's tenant/day rule evaluation. The
-- helper wraps the decode chain in EXCEPTION WHEN OTHERS → RETURN
-- NULL so a single bad outcome row degrades to "unknown reason"
-- instead of taking down the rule.
--
-- IMMUTABLE so the planner can fold it. STRICT so NULL inputs short-
-- circuit without touching the body (cheap path for NULL data_b64).
CREATE OR REPLACE FUNCTION cost_advisor_safe_release_reason(p_data_b64 TEXT)
    RETURNS TEXT
    LANGUAGE plpgsql
    IMMUTABLE
    STRICT
AS $$
DECLARE
    v_jsonb JSONB;
BEGIN
    v_jsonb := convert_from(decode(p_data_b64, 'base64'), 'UTF8')::jsonb;
    -- Only return reason for release outcomes; nulls out other kinds
    -- (commit_estimated, etc.) so the view's CASE doesn't see them
    -- as a release.
    --
    -- Codex P0.6 r2 P2 fix: use IS DISTINCT FROM, not `<>`. If `kind`
    -- is absent OR JSON null, `v_jsonb->>'kind'` returns SQL NULL, and
    -- `NULL <> 'release'` evaluates to NULL — IF NULL THEN doesn't
    -- fire, so the function would fall through and return whatever
    -- `reason` says, promoting a malformed-but-decodable payload like
    -- `{"reason":"TTL_EXPIRED"}` (no kind) to a real release. IS
    -- DISTINCT FROM treats NULL as distinct from 'release', so
    -- missing/null kind correctly returns NULL.
    IF v_jsonb->>'kind' IS DISTINCT FROM 'release' THEN
        RETURN NULL;
    END IF;
    RETURN v_jsonb->>'reason';
EXCEPTION
    WHEN OTHERS THEN
        -- Degraded path: a malformed payload becomes "unknown reason"
        -- rather than a hard SELECT failure. Operator-grade alerting
        -- on audit_outbox payload validity is a separate concern
        -- (P5 monitoring).
        RETURN NULL;
END;
$$;

COMMENT ON FUNCTION cost_advisor_safe_release_reason(TEXT) IS
    'Cost Advisor P0.6 r1: safe decode of audit_outbox.cloudevent_payload.data_b64 to extract release reason. Returns NULL on any decode/JSON/encoding failure OR if kind <> release. Used by reservations_with_ttl_status_v1 view.';

CREATE OR REPLACE VIEW reservations_with_ttl_status_v1 AS
SELECT
    r.reservation_id,
    r.tenant_id,
    r.budget_id,
    r.window_instance_id,
    r.current_state,

    -- Derived state. Only flips when an audit.outcome release event
    -- carries reason='TTL_EXPIRED' AND the reservation is currently
    -- released. All other states pass through.
    CASE
        WHEN r.current_state = 'released'
             AND release_evt.reason = 'TTL_EXPIRED'
            THEN 'ttl_expired'
        ELSE r.current_state
    END AS derived_state,

    -- TTL window in seconds. Both ttl_expires_at and created_at are
    -- always populated (NOT NULL on reservations); EXTRACT EPOCH on a
    -- TIMESTAMPTZ subtraction is well-defined.
    EXTRACT(EPOCH FROM (r.ttl_expires_at - r.created_at))::INT
        AS ttl_seconds,

    r.created_at,
    r.ttl_expires_at,
    r.source_ledger_transaction_id,

    -- Release metadata. NULL for reservations still 'reserved' /
    -- 'committed' / 'overrun_debt' (no release event yet), AND NULL
    -- for any audit_outbox row whose payload fails to decode (degraded
    -- path; see cost_advisor_safe_release_reason). The (tenant_id,
    -- decision_id, event_type) uniqueness is enforced GLOBALLY by
    -- audit_outbox_global_keys (0009 lines 110-145); the partitioned
    -- partial UNIQUE indexes on audit_outbox itself are per-partition
    -- only. So in principle there's one outcome row per decision —
    -- the LIMIT 1 ORDER BY producer_sequence DESC is defense in depth.
    release_evt.reason     AS release_reason,
    release_evt.recorded_at AS released_at

  FROM reservations r
  -- Codex P0.6 r1 P2 fix: source_ledger_transaction_id FKs to
  -- ledger_transactions but the FK doesn't enforce operation_kind.
  -- Filter to reserve tx so a future direct-SQL drift can't make the
  -- view derive release metadata from a non-reserve decision.
  LEFT JOIN ledger_transactions reserve_tx
    ON reserve_tx.ledger_transaction_id = r.source_ledger_transaction_id
   AND reserve_tx.operation_kind = 'reserve'
  LEFT JOIN LATERAL (
      SELECT
          cost_advisor_safe_release_reason(o.cloudevent_payload->>'data_b64') AS reason,
          o.recorded_at
        FROM audit_outbox o
       WHERE o.tenant_id    = r.tenant_id
         AND o.decision_id  = reserve_tx.decision_id
         AND o.event_type   = 'spendguard.audit.outcome'
         -- The helper returns NULL for non-release kinds AND for
         -- malformed payloads; the IS NOT NULL filter combines both
         -- "wrong kind" and "decode failed" into a single skip.
         AND cost_advisor_safe_release_reason(o.cloudevent_payload->>'data_b64') IS NOT NULL
       ORDER BY o.producer_sequence DESC
       LIMIT 1
  ) release_evt ON TRUE;

COMMENT ON VIEW reservations_with_ttl_status_v1 IS
    'Cost Advisor P0.6 (issue #49 / spec v4 §0.5): derived view exposing ttl_expired state + ttl_seconds + release_reason for cost_advisor rules. Joins reservations + audit_outbox via decision_id, decoding the base64 CloudEvent data field. Read-only.';
