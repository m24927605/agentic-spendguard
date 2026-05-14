-- =====================================================================
-- idle_reservation_rate_v1 — Cost Advisor v0.1 first fireable rule
-- =====================================================================
--
-- Detects:
--   PER-BUDGET (CA-P3.1): for each (tenant, budget) where >20% of
--   reservations in the 24h window TTL'd AND the median TTL was
--   short enough that the contract's reserve-then-commit pattern is
--   wasting concurrent budget (per spec §5.1).
--
-- Provably wasted because:
--   A reservation that TTLs without committing means the workload
--   never actually spent the money but the budget was held out of
--   reach of other concurrent calls during the TTL window. High
--   idle-rates indicate the contract's reservation TTL is set higher
--   than the typical commit latency — operator should tighten it.
--
-- Recommended fix (emitted as a 2-op proposed_dsl_patch):
--   [
--     {"op":"test",   "path":"/spec/budgets/<i>/id",                "value":"<uuid>"},
--     {"op":"replace","path":"/spec/budgets/<i>/reservation_ttl_seconds","value":N}
--   ]
--   where N = 1.5 × observed median ttl_seconds (clamped to [1,86400])
--   and the test op locks budget identity so apply fails if the
--   bundle's array position <i> has been reshuffled.
--
-- Read shape:
--   * Tenant scope: passed as $1.
--   * Time bucket: 24h window starting at $2 (UTC date).
--   * Reservations source: reservations_with_ttl_status_v1 view
--     (services/ledger/migrations/0039_*.sql; lives in spendguard_ledger).
--   * Returns ZERO OR MORE rows (one per offending budget) — the
--     runtime decoder iterates and emits a finding per row. This is
--     a fan-out change from v0.1's "one row per tenant" output.
--
-- Fingerprint composition (spec §11.5 A1):
--   sha256(rule_id || '|' || tenant_id || '|' || canonical_scope
--          || '|' || time_bucket_iso)
--   where canonical_scope = '5|||||<budget_id>' for ScopeType=BUDGET
--   (CA-P3.1) and time_bucket_iso = $2 formatted as ISO date.

SELECT
    v.budget_id::TEXT                                            AS budget_id,
    COUNT(*)::BIGINT                                            AS total_reservations,
    COUNT(*) FILTER (WHERE v.derived_state = 'ttl_expired')::BIGINT
                                                                 AS ttl_expired_count,
    PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY v.ttl_seconds)::INT
                                                                 AS median_ttl_seconds,
    PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY v.ttl_seconds)::INT
                                                                 AS p95_ttl_seconds,
    (
        SELECT array_agg(decision_id ORDER BY released_at DESC)
          FROM (
              SELECT lt.decision_id, v2.released_at
                FROM reservations_with_ttl_status_v1 v2
                JOIN ledger_transactions lt
                  ON lt.ledger_transaction_id = v2.source_ledger_transaction_id
                 AND lt.operation_kind = 'reserve'
               WHERE v2.tenant_id   = v.tenant_id
                 AND v2.budget_id   = v.budget_id
                 AND v2.derived_state = 'ttl_expired'
                 AND v2.created_at >= $2::date
                 AND v2.created_at <  ($2::date + INTERVAL '1 day')
                 AND lt.decision_id IS NOT NULL
               ORDER BY v2.released_at DESC
               LIMIT 5
          ) sample
    )                                                            AS sample_decision_ids,
    NULL::BIGINT                                                 AS estimated_waste_micros_usd
  FROM reservations_with_ttl_status_v1 v
  -- Codex CA-P1 r1 P2: exclude drifted reservations whose source tx
  -- is NOT operation_kind='reserve'.
  JOIN ledger_transactions reserve_tx
    ON reserve_tx.ledger_transaction_id = v.source_ledger_transaction_id
   AND reserve_tx.operation_kind = 'reserve'
 WHERE v.tenant_id = $1
   AND v.created_at >= $2::date
   AND v.created_at <  ($2::date + INTERVAL '1 day')
 GROUP BY v.tenant_id, v.budget_id
HAVING
    -- Fire only if all 3 conditions hold (per spec §5.1 + codex r1 P2):
    --   * total reservations >= 5 (degenerate medians for tiny samples
    --     are noise, not signal)
    --   * TTL'd / total > 20%
    --   * median TTL <= 60s (placeholder min_ttl_for_finding)
    COUNT(*) >= 5
    AND (COUNT(*) FILTER (WHERE v.derived_state = 'ttl_expired')::NUMERIC
         / NULLIF(COUNT(*), 0)) > 0.20
    AND PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY v.ttl_seconds) <= 60;
