-- =====================================================================
-- runaway_loop_v1 — Cost Advisor v0.1 third rule (P1.5)
-- =====================================================================
--
-- Detects: same (run_id, prompt_hash) called > 5 times in any single
-- 60-second tumbling window — i.e. the provider succeeded but the
-- agent never converged. Orthogonal to failed_retry_burn_v1.
--
-- Scope:
--   * `$1 tenant_id` (UUID)
--   * `$2 bucket_date` (DATE) — runtime invokes once-per-day. The
--     SQL scans the FULL 24h interval and tumbles 60-second windows
--     internally so loops in any single minute fire even though the
--     runtime scheduler only runs daily (codex CA-P1.5 r2 P1 fix).
--     Cross-minute loops are split; spec §5.1.1 dedup in P2 collapses.
--
-- Rule fires when, for any (run_id, prompt_hash, minute_window):
--   * call_count > 5

WITH step1 AS (
    SELECT
        c.event_id,
        c.run_id,
        c.event_time,
        c.decision_id,
        c.failure_class,
        cost_advisor_safe_decode_payload(c.payload_json)        AS inner_data,
        date_trunc('minute', c.event_time)                      AS minute_window
      FROM canonical_events c
     WHERE c.tenant_id = $1
       AND c.event_type = 'spendguard.audit.outcome'
       AND c.event_time >= $2::date
       AND c.event_time <  $2::date + INTERVAL '1 day'
       AND c.run_id IS NOT NULL
       -- Exclude billed-failure attempts: those belong to
       -- failed_retry_burn_v1. We want "successful loops" only.
       AND (c.failure_class IS NULL OR c.failure_class = 'unknown')
),
step2 AS (
    SELECT
        run_id,
        inner_data->>'prompt_hash'                              AS prompt_hash,
        minute_window,
        COUNT(*)                                                AS call_count,
        SUM(
            COALESCE(
                NULLIF(
                    regexp_replace(
                        inner_data->>'estimated_amount_atomic',
                        '[^0-9]', '', 'g'
                    ),
                    ''
                )::NUMERIC,
                0
            )
        )                                                       AS atomic_sum,
        (array_agg(decision_id ORDER BY event_time DESC))[1:5]  AS sample_decision_ids,
        MIN(event_time)                                         AS first_event_time,
        MAX(event_time)                                         AS last_event_time
      FROM step1
     WHERE inner_data->>'prompt_hash' IS NOT NULL
     GROUP BY run_id, inner_data->>'prompt_hash', minute_window
),
step3 AS (SELECT * FROM step2 WHERE call_count > 5)
SELECT
    COUNT(*)::BIGINT                                            AS affected_run_prompt_groups,
    SUM(call_count)::BIGINT                                     AS total_calls,
    MAX(call_count)::BIGINT                                     AS max_loop_depth,
    SUM(atomic_sum)::NUMERIC                                    AS total_atomic_sum,
    (SELECT sample_decision_ids
       FROM step3 ORDER BY call_count DESC LIMIT 1)             AS sample_decision_ids,
    MIN(first_event_time)                                       AS bucket_first_event_time,
    MAX(last_event_time)                                        AS bucket_last_event_time
  FROM step3
HAVING
    COUNT(*) >= 1;
