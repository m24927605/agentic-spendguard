-- =====================================================================
-- runaway_loop_v1 — Cost Advisor v0.1 third rule (P1.5)
-- =====================================================================
--
-- Detects: same (run_id, prompt_hash) retried > 5 times in 60s WITHOUT
-- a failure_class match — i.e. each individual call SUCCEEDED at the
-- provider layer but the agent never converged (e.g. ReAct loop that
-- keeps re-querying the same prompt indefinitely). This is orthogonal
-- to failed_retry_burn_v1 (which fires on billed-failures); the same
-- agent could trigger BOTH rules and the §5.1.1 dedup phase decides
-- which one survives.
--
-- Provably wasted because:
--   N calls to the same prompt that produce N similar outputs with
--   no termination = N-1 are redundant (the first call's output
--   could have been used). The waste figure is N-1 × per-call cost.
--
-- Read shape:
--   * canonical_events with event_type='spendguard.audit.outcome'
--   * Decoded payload_json.data_b64 → prompt_hash
--   * run_id (envelope column, CA-P0.5)
--   * failure_class IS NULL OR failure_class IN ('unknown') — to
--     exclude billed-failure retries (those go to failed_retry_burn_v1)
--
-- Bound parameters:
--   $1 tenant_id
--   $2 bucket_start — 60-second window
--
-- Time granularity: 60-second windows. Anything tighter is just
-- agent step latency; anything looser misses the "tight ReAct loop"
-- pattern. Aligns with spec §5.1 "retried > 5 in 60s".

WITH step1 AS (
    SELECT
        c.event_id,
        c.run_id,
        c.event_time,
        c.decision_id,
        c.failure_class,
        cost_advisor_safe_decode_payload(c.payload_json)        AS inner_data
      FROM canonical_events c
     WHERE c.tenant_id = $1
       AND c.event_type = 'spendguard.audit.outcome'
       AND c.event_time >= $2
       AND c.event_time < $2 + INTERVAL '60 seconds'
       AND c.run_id IS NOT NULL
       -- Exclude billed-failure attempts: those belong to
       -- failed_retry_burn_v1. We want "successful loops" only.
       AND (c.failure_class IS NULL OR c.failure_class = 'unknown')
),
step2 AS (
    SELECT
        run_id,
        inner_data->>'prompt_hash'                              AS prompt_hash,
        COUNT(*)                                                AS call_count,
        SUM(COALESCE(
            (inner_data->>'estimated_amount_atomic')::NUMERIC, 0
        ))                                                      AS atomic_sum,
        (array_agg(decision_id ORDER BY event_time DESC))[1:5]  AS sample_decision_ids,
        MIN(event_time)                                         AS first_event_time,
        MAX(event_time)                                         AS last_event_time
      FROM step1
     WHERE inner_data->>'prompt_hash' IS NOT NULL
     GROUP BY run_id, inner_data->>'prompt_hash'
),
step3 AS (
    SELECT *
      FROM step2
     -- Spec §5.1: "Same (run_id, prompt_hash) retried > 5 in 60s with
     -- no terminal output". We use `> 5` (strict) so 6+ calls fire.
     WHERE call_count > 5
)
SELECT
    COUNT(*)::BIGINT                                            AS affected_run_prompt_groups,
    SUM(call_count)::BIGINT                                     AS total_calls,
    MAX(call_count)::BIGINT                                     AS max_loop_depth,
    SUM(atomic_sum)::NUMERIC                                    AS total_atomic_sum,
    -- Sample decision_refs from the most-aggressive loop group.
    (SELECT sample_decision_ids
       FROM step3 ORDER BY call_count DESC LIMIT 1)             AS sample_decision_ids,
    MIN(first_event_time)                                       AS bucket_first_event_time,
    MAX(last_event_time)                                        AS bucket_last_event_time
  FROM step3
HAVING
    COUNT(*) >= 1;
