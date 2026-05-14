-- =====================================================================
-- failed_retry_burn_v1 — Cost Advisor v0.1 second fireable rule (P1.5)
-- =====================================================================
--
-- Detects: same (run_id, prompt_hash) retried after a billed failure
-- per spec §5.1 + §5.1.2.
--
-- Scope:
--   * `$1 tenant_id` (UUID)
--   * `$2 bucket_date` (DATE) — runtime invokes this once-per-day. The
--     SQL scans the FULL 24h interval and uses tumbling 1-hour windows
--     internally so retries clustered in any single hour fire even
--     when the runtime scheduler only runs daily (codex CA-P1.5 r2
--     P1 fix). A retry burst that crosses an hour boundary is split
--     across windows — accepted limitation; spec §5.1.1 dedup phase
--     would collapse cross-window duplicates in P2.
--
-- Rule fires when, for any (run_id, prompt_hash, hour_window):
--   * attempt_count >= 2 (retry actually happened)
--   * billed_failure_count >= 1 (at least one billed-waste attempt)
--
-- Wasted-attempt accounting (codex CA-P1.5 r2 P2 fix): the
-- `retry_then_success` failure_class is included in the billed-class
-- FILTER for counting failed attempts but NOT in the
-- `failed_atomic_sum` — the terminal success is not waste; only the
-- earlier failures are. The sample_decision_ids array excludes the
-- terminal success row too so dashboard "view evidence" points only
-- at the wasted rows.

WITH step1 AS (
    SELECT
        c.event_id,
        c.run_id,
        c.event_time,
        c.decision_id,
        c.failure_class,
        cost_advisor_safe_decode_payload(c.payload_json)        AS inner_data,
        date_trunc('hour', c.event_time)                        AS hour_window
      FROM canonical_events c
     WHERE c.tenant_id = $1
       AND c.event_type = 'spendguard.audit.outcome'
       AND c.event_time >= $2::date
       AND c.event_time <  $2::date + INTERVAL '1 day'
       AND c.run_id IS NOT NULL
       AND c.failure_class IS NOT NULL
),
step2 AS (
    SELECT
        run_id,
        inner_data->>'prompt_hash'                              AS prompt_hash,
        hour_window,
        COUNT(*)                                                AS attempt_count,
        -- Count "wasted" attempts: any class in the billed subset
        -- including retry_then_success (the terminal success row's
        -- failure_class marker, indicating earlier attempts were
        -- wasted).
        COUNT(*) FILTER (WHERE failure_class IN (
            'provider_5xx',
            'provider_4xx_billed',
            'malformed_json_response',
            'timeout_billed',
            'retry_then_success'
        ))                                                      AS billed_failure_count,
        -- Sum atomic over WASTED attempts only — exclude
        -- retry_then_success because that row IS the terminal
        -- success (codex r2 P2 fix).
        SUM(
            CASE WHEN failure_class IN (
                'provider_5xx',
                'provider_4xx_billed',
                'malformed_json_response',
                'timeout_billed'
            )
            THEN
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
            ELSE 0 END
        )                                                       AS failed_atomic_sum,
        -- Sample wasted decision_ids only (NOT retry_then_success).
        (array_agg(decision_id ORDER BY event_time DESC) FILTER (
            WHERE failure_class IN (
                'provider_5xx',
                'provider_4xx_billed',
                'malformed_json_response',
                'timeout_billed'
            )
        ))[1:5]                                                  AS sample_decision_ids,
        MIN(event_time)                                         AS first_event_time,
        MAX(event_time)                                         AS last_event_time
      FROM step1
     WHERE inner_data->>'prompt_hash' IS NOT NULL
     GROUP BY run_id, inner_data->>'prompt_hash', hour_window
),
step3 AS (
    SELECT *
      FROM step2
     WHERE attempt_count >= 2
       AND billed_failure_count >= 1
)
SELECT
    COUNT(*)::BIGINT                                            AS affected_run_prompt_groups,
    SUM(attempt_count)::BIGINT                                  AS total_attempts,
    SUM(billed_failure_count)::BIGINT                           AS total_billed_failures,
    SUM(failed_atomic_sum)::NUMERIC                             AS total_failed_atomic_sum,
    -- Surface a sample from the most-recent group across all hours.
    (SELECT sample_decision_ids
       FROM step3 ORDER BY last_event_time DESC LIMIT 1)        AS sample_decision_ids,
    MIN(first_event_time)                                       AS bucket_first_event_time,
    MAX(last_event_time)                                        AS bucket_last_event_time
  FROM step3
HAVING
    COUNT(*) >= 1;
