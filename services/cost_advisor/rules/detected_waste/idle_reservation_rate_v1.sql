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
    COUNT(*) FILTER (WHERE derived_state = 'ttl_expired')::BIGINT
                                                                 AS ttl_expired_count,
    PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY ttl_seconds)::INT
                                                                 AS median_ttl_seconds,
    PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY ttl_seconds)::INT
                                                                 AS p95_ttl_seconds,
    -- Sample decision_ids for FindingEvidence.decision_refs. Picks the
    -- 5 most recent TTL'd reservations to anchor the finding to real
    -- audit rows the operator can inspect.
    (
        SELECT array_agg(reservation_id ORDER BY released_at DESC)
          FROM (
              SELECT reservation_id, released_at
                FROM reservations_with_ttl_status_v1
               WHERE tenant_id = $1
                 AND derived_state = 'ttl_expired'
                 AND created_at >= $2::date
                 AND created_at < ($2::date + INTERVAL '1 day')
               ORDER BY released_at DESC
               LIMIT 5
          ) sample
    )                                                            AS sample_reservation_ids,
    -- WasteEstimate (spec §11.5 A3 baseline_excess method): use the
    -- "idle reservation hours" as a heuristic proxy. Real USD
    -- conversion requires per-tenant budget USD price; deferred to
    -- post-P1 baseline_refresher. Confidence: medium (heuristic, not
    -- counterfactual).
    (
        -- micros_usd estimate placeholder: 1 ttl_expired_count × $0.10
        -- per (1 microUSD = 1e-6 USD; $0.10 = 100_000 microUSD).
        -- Replaced by baseline_refresher in P2.
        COUNT(*) FILTER (WHERE derived_state = 'ttl_expired')::BIGINT * 100000
    )                                                            AS estimated_waste_micros_usd
  FROM reservations_with_ttl_status_v1
 WHERE tenant_id = $1
   AND created_at >= $2::date
   AND created_at < ($2::date + INTERVAL '1 day')
 GROUP BY tenant_id
HAVING
    -- Fire only if both conditions hold (per spec §5.1 row):
    --   * TTL'd / total > 20%
    --   * median TTL <= 60s (placeholder min_ttl_for_finding; comes
    --     from contract DSL rule config when that surface exists)
    COUNT(*) > 0
    AND (COUNT(*) FILTER (WHERE derived_state = 'ttl_expired')::NUMERIC
         / NULLIF(COUNT(*), 0)) > 0.20
    AND PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY ttl_seconds) <= 60;
