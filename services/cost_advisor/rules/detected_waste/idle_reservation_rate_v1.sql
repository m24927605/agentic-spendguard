-- =====================================================================
-- idle_reservation_rate_v1 — Cost Advisor v0.1 first fireable rule
-- =====================================================================
--
-- Detects:
--   Tenants where TTL'd reservations / total reservations > 20% in the
--   target time bucket AND the median TTL was short enough that the
--   contract's reserve-then-commit pattern is wasting concurrent budget
--   (per spec §5.1).
--
-- Provably wasted because:
--   A reservation that TTLs without committing means the workload
--   never actually spent the money but the budget was held out of
--   reach of other concurrent calls during the TTL window. High
--   idle-rates indicate the contract's reservation TTL is set higher
--   than the typical commit latency — operator should tighten it.
--
-- Recommended fix (emitted as the proposed_dsl_patch):
--   Tighten the contract's `reserve.ttl_seconds` to 1.5× the observed
--   median ttl_seconds AND/OR add a reservation `max_idle_ratio`
--   threshold so excess TTLs trigger STOP/DEGRADE.
--
-- Read shape:
--   * Tenant scope: passed as $1.
--   * Time bucket: 24h window starting at $2 (UTC date).
--   * Reservations source: reservations_with_ttl_status_v1 view
--     (services/ledger/migrations/0039_*.sql; lives in spendguard_ledger).
--
-- This SQL is invoked from services/cost_advisor/src/runtime.rs as a
-- prepared statement and the returned single row drives the
-- cost_findings_upsert() SP. NULL row = rule did not fire.
--
-- Fingerprint composition (spec §11.5 A1):
--   sha256(rule_id || '|' || scope_canonical || '|' || time_bucket_iso)
--   where scope_canonical = 'tenant_global|||||' (no agent/run/tool)
--   and time_bucket_iso = $2 formatted as ISO date.

SELECT
    -- Aggregates over the time bucket
    COUNT(*)::BIGINT                                            AS total_reservations,
    COUNT(*) FILTER (WHERE v.derived_state = 'ttl_expired')::BIGINT
                                                                 AS ttl_expired_count,
    PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY v.ttl_seconds)::INT
                                                                 AS median_ttl_seconds,
    PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY v.ttl_seconds)::INT
                                                                 AS p95_ttl_seconds,
    -- Sample CANONICAL decision_ids for FindingEvidence.decision_refs
    -- per spec §4.0 ("Sample decision_ids from canonical_events that
    -- this finding is derived from"). Codex r1 P1 caught that the
    -- earlier sample used reservation_id — the dashboard "view raw
    -- evidence" link expects decision_id which is the canonical
    -- audit-chain anchor. We JOIN to ledger_transactions to surface
    -- the decision_id for each reservation's source reserve tx.
    (
        SELECT array_agg(decision_id ORDER BY released_at DESC)
          FROM (
              SELECT lt.decision_id, v.released_at
                FROM reservations_with_ttl_status_v1 v
                JOIN ledger_transactions lt
                  ON lt.ledger_transaction_id = v.source_ledger_transaction_id
                 AND lt.operation_kind = 'reserve'
               WHERE v.tenant_id = $1
                 AND v.derived_state = 'ttl_expired'
                 AND v.created_at >= $2::date
                 AND v.created_at < ($2::date + INTERVAL '1 day')
                 AND lt.decision_id IS NOT NULL
               ORDER BY v.released_at DESC
               LIMIT 5
          ) sample
    )                                                            AS sample_decision_ids,
    -- WasteEstimate: returned as NULL until baseline_refresher (P2)
    -- can compute a real per-tenant USD figure. Codex r1 P2 caught
    -- that emitting `ttl_expired × $0.10` as medium-confidence USD
    -- waste leaks placeholder math into operator dashboards. The
    -- runtime maps NULL → method=heuristic + confidence=low + null
    -- micros_usd so consumers can render "USD estimate pending"
    -- instead of a misleading figure.
    NULL::BIGINT                                                 AS estimated_waste_micros_usd
  FROM reservations_with_ttl_status_v1 v
  -- Codex r1 P2: exclude drifted reservations whose source tx is
  -- NOT operation_kind='reserve'. The view's LEFT JOIN preserves them
  -- but they can never carry derived_state='ttl_expired' (release
  -- LATERAL has no reserve_tx.decision_id to match), so they'd
  -- silently dilute the denominator. Inner-join the reserve tx here
  -- to drop them from the count.
  JOIN ledger_transactions reserve_tx
    ON reserve_tx.ledger_transaction_id = v.source_ledger_transaction_id
   AND reserve_tx.operation_kind = 'reserve'
 WHERE v.tenant_id = $1
   AND v.created_at >= $2::date
   AND v.created_at < ($2::date + INTERVAL '1 day')
 GROUP BY v.tenant_id
HAVING
    -- Fire only if all 3 conditions hold (per spec §5.1 row + r1
    -- codex P2 min-sample):
    --   * total reservations >= 5 (codex r1 P2: degenerate medians
    --     for tiny samples are noise, not signal).
    --   * TTL'd / total > 20%
    --   * median TTL <= 60s (placeholder min_ttl_for_finding; comes
    --     from contract DSL rule config when that surface exists)
    COUNT(*) >= 5
    AND (COUNT(*) FILTER (WHERE v.derived_state = 'ttl_expired')::NUMERIC
         / NULLIF(COUNT(*), 0)) > 0.20
    AND PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY v.ttl_seconds) <= 60;
