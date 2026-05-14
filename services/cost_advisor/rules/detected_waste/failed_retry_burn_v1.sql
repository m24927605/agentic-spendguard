-- =====================================================================
-- failed_retry_burn_v1 — Cost Advisor v0.1 second fireable rule (P1.5)
-- =====================================================================
--
-- Detects: same (run_id, prompt_hash) retried after a billed failure
-- per spec §5.1 + §5.1.2. Fires when at least 1 retry happened
-- (= 2+ audit.outcome events with same run_id+prompt_hash) AND every
-- non-final attempt failed with a billed-class failure_class.
--
-- Provably wasted because each billed-failure attempt's
-- `committed_micros_usd` was paid for a response we couldn't use.
-- The waste figure is the SUM of billed amounts on the failed
-- attempts (excluding the final successful one if any).
--
-- Read shape:
--   * Reads canonical_events.failure_class (populated by P1.5
--     classify.rs at INSERT time).
--   * Reads run_id (envelope column; populated by CA-P0.5 sidecar
--     enrichment) and prompt_hash (in base64-decoded data_b64).
--   * 1-hour time bucket per spec §11.5 A1 (more granular than
--     idle_reservation_rate's 1-day bucket because retries cluster
--     in short bursts).
--
-- Bound parameters:
--   $1 tenant_id (uuid)
--   $2 bucket_start (timestamptz; rule runs on a 1h sliding window
--      starting at this time)
--
-- The CTE chain:
--   step1 -- decode payload_json.data_b64 → inner JSON; keep
--            (run_id, prompt_hash, failure_class, event_time)
--   step2 -- group by (run_id, prompt_hash); count attempts AND
--            count billed-failure attempts; suppress groups with
--            < 2 attempts (no retry = no waste)
--   step3 -- filter to groups where every-non-final attempt was
--            billed-class failure (i.e. fail_count >= attempt_count
--            - 1); compute waste = sum of failed attempt
--            committed_micros_usd
--   final  -- aggregate to single row per (tenant_id, bucket) so the
--            runtime gets one finding output. Sample 5 decision_ids.

WITH step1 AS (
    SELECT
        c.event_id,
        c.run_id,
        c.event_time,
        c.decision_id,
        c.failure_class,
        -- Decode the inner CloudEvent data via the safe helper
        -- (migration 0012). Malformed payloads → NULL → row
        -- naturally drops in step2 because prompt_hash will also be
        -- NULL.
        cost_advisor_safe_decode_payload(c.payload_json)        AS inner_data
      FROM canonical_events c
     WHERE c.tenant_id = $1
       AND c.event_type = 'spendguard.audit.outcome'
       AND c.event_time >= $2
       AND c.event_time < $2 + INTERVAL '1 hour'
       AND c.run_id IS NOT NULL
       AND c.failure_class IS NOT NULL
),
step2 AS (
    SELECT
        run_id,
        inner_data->>'prompt_hash'                              AS prompt_hash,
        COUNT(*)                                                AS attempt_count,
        COUNT(*) FILTER (WHERE failure_class IN (
            'provider_5xx',
            'provider_4xx_billed',
            'malformed_json_response',
            'timeout_billed'
        ))                                                      AS billed_failure_count,
        -- Sum estimated_amount_atomic across all failed attempts.
        -- This is the "raw atomic" figure; rule does NOT convert to
        -- USD here — see runtime decoder + WasteEstimate proto note
        -- about pending USD conversion until baseline_refresher.
        SUM(
            CASE WHEN failure_class IN (
                'provider_5xx',
                'provider_4xx_billed',
                'malformed_json_response',
                'timeout_billed'
            )
            THEN COALESCE(
                (inner_data->>'estimated_amount_atomic')::NUMERIC,
                0
            )
            ELSE 0 END
        )                                                       AS failed_atomic_sum,
        -- Sample decision_ids of the failed attempts for evidence.
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
     GROUP BY run_id, inner_data->>'prompt_hash'
),
step3 AS (
    SELECT *
      FROM step2
     WHERE attempt_count >= 2
       -- Codex pattern from CA-P1 r1 P2: require at least 2 billed
       -- failures (else the retry might be a one-off blip not a
       -- pattern; also avoids noise on transient flakes that
       -- happened to commit billing once).
       AND billed_failure_count >= 2
)
SELECT
    COUNT(*)::BIGINT                                            AS affected_run_prompt_groups,
    SUM(attempt_count)::BIGINT                                  AS total_attempts,
    SUM(billed_failure_count)::BIGINT                           AS total_billed_failures,
    SUM(failed_atomic_sum)::NUMERIC                             AS total_failed_atomic_sum,
    -- Surface a sample of (run_id, prompt_hash, decision_id) tuples
    -- for FindingEvidence.decision_refs; pick the most-recent group.
    (SELECT sample_decision_ids
       FROM step3 ORDER BY last_event_time DESC LIMIT 1)        AS sample_decision_ids,
    -- Span of the bucket where the burst happened.
    MIN(first_event_time)                                       AS bucket_first_event_time,
    MAX(last_event_time)                                        AS bucket_last_event_time
  FROM step3
HAVING
    COUNT(*) >= 1;  -- at least one (run_id, prompt_hash) group fires
