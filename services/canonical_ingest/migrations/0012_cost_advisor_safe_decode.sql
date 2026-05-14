-- Cost Advisor P1.5 (issue #51): safe decode helper for
-- canonical_events.payload_json's base64 data_b64 field.
--
-- Rule SQL (failed_retry_burn_v1, runaway_loop_v1, future P1.5+
-- rules) needs to read `prompt_hash`, `agent_id`, etc. that live
-- INSIDE the base64-encoded CloudEvent `data` field (per CA-P0
-- audit-report §0.2). Naive `convert_from(decode(...))::jsonb`
-- RAISES on malformed input — one bad row would abort the whole
-- rule SELECT and silently miss the bucket.
--
-- This helper mirrors the spendguard_ledger.cost_advisor_safe_
-- release_reason from migration 0039 (P0.6 r1 fix): EXCEPTION
-- WHEN OTHERS → RETURN NULL. IMMUTABLE STRICT so the planner can
-- fold + cheap-path NULL inputs.

CREATE OR REPLACE FUNCTION cost_advisor_safe_decode_payload(p_payload_json JSONB)
    RETURNS JSONB
    LANGUAGE plpgsql
    IMMUTABLE
    STRICT
AS $$
DECLARE
    v_b64 TEXT;
BEGIN
    v_b64 := p_payload_json->>'data_b64';
    IF v_b64 IS NULL THEN
        RETURN NULL;
    END IF;
    RETURN convert_from(decode(v_b64, 'base64'), 'UTF8')::jsonb;
EXCEPTION
    WHEN OTHERS THEN
        RETURN NULL;
END;
$$;

COMMENT ON FUNCTION cost_advisor_safe_decode_payload(JSONB) IS
    'Cost Advisor P1.5: safe decode of canonical_events.payload_json.data_b64 → inner CloudEvent data JSONB. Returns NULL on invalid base64 / invalid UTF8 / invalid JSON. Rule SQL invokes per-row; malformed payloads degrade to NULL (rule filters those out) instead of aborting the SELECT.';
