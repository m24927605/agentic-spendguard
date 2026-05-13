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

    -- Release metadata. NULL for reservations still 'reserved' / 'committed'
    -- / 'overrun_debt' (no release event yet). For 'released' rows, the
    -- LATERAL subquery picks the most-recent release outcome by
    -- producer_sequence (same decision_id can have at most one
    -- audit.outcome per the unique index in 0009, so this LIMIT 1 is
    -- a defense-in-depth ORDER BY).
    release_evt.reason     AS release_reason,
    release_evt.recorded_at AS released_at

  FROM reservations r
  LEFT JOIN ledger_transactions reserve_tx
    ON reserve_tx.ledger_transaction_id = r.source_ledger_transaction_id
  LEFT JOIN LATERAL (
      SELECT
          -- audit_outbox.cloudevent_payload has `data_b64` (base64-
          -- encoded JSON) per the ledger handlers' extract_cloudevent
          -- _payload helper. Decode -> jsonb -> reason.
          convert_from(
              decode(o.cloudevent_payload->>'data_b64', 'base64'),
              'UTF8'
          )::jsonb->>'reason' AS reason,
          o.recorded_at,
          o.cloudevent_payload
        FROM audit_outbox o
       WHERE o.tenant_id    = r.tenant_id
         AND o.decision_id  = reserve_tx.decision_id
         AND o.event_type   = 'spendguard.audit.outcome'
         AND (convert_from(
                decode(o.cloudevent_payload->>'data_b64', 'base64'),
                'UTF8'
              )::jsonb->>'kind') = 'release'
       ORDER BY o.producer_sequence DESC
       LIMIT 1
  ) release_evt ON TRUE;

COMMENT ON VIEW reservations_with_ttl_status_v1 IS
    'Cost Advisor P0.6 (issue #49 / spec v4 §0.5): derived view exposing ttl_expired state + ttl_seconds + release_reason for cost_advisor rules. Joins reservations + audit_outbox via decision_id, decoding the base64 CloudEvent data field. Read-only.';
