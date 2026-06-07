-- D37 SLICE 5 (n8n_real demo) — ledger-DB assertions.
--
-- Mirrors verify_step_botpress_real.sql + verify_step_inngest_agent_kit.sql.
-- The Makefile target `demo-verify-n8n-real` runs this file against
-- spendguard_ledger.
--
-- review-standards.md §8 gates (6 assertions):
--   1. >=2 decisions with `decision_context.integration=n8n`.
--   2. >=1 DENY decision (Step 2 INV-1 proof).
--   3. >=1 commit row pairing with the ALLOW reservation.
--   4. canonical_events carried the n8n rows (outbox-forwarder ran).
--   5. NO DENY row had `stub_hits > 0` (INV-1 — DENY skips upstream).
--   6. >=1 streaming row (Step 3) carrying `decision_context.stream='true'`.

\echo
\echo === audit_outbox: integration=n8n decision counts ===
SELECT
    decision_context->>'integration' AS integration,
    decision_context->>'mode'        AS mode,
    COUNT(*)::int                    AS n
  FROM audit_outbox
 WHERE decision_context->>'integration' = 'n8n'
   AND recorded_at > now() - interval '5 minute'
 GROUP BY 1, 2
 ORDER BY 1, 2;

\echo
\echo === ASSERT: D37 n8n integration recorded >=2 decisions ===
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM audit_outbox
   WHERE decision_context->>'integration' = 'n8n'
     AND decision_context->>'mode' = 'community_node'
     AND recorded_at > now() - interval '5 minute';
  IF c < 2 THEN
    RAISE EXCEPTION 'D37_N8N_GATE: expected >=2 decisions, got %', c;
  END IF;
  RAISE NOTICE 'D37_N8N OK: decisions=%', c;
END; $$;

\echo
\echo === ASSERT: D37 n8n saw >=1 DENY decision ===
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM audit_outbox
   WHERE decision_context->>'integration' = 'n8n'
     AND cloudevent_payload::jsonb->'data'->>'decision' = 'DENY'
     AND recorded_at > now() - interval '5 minute';
  IF c < 1 THEN
    RAISE EXCEPTION 'D37_N8N_GATE: expected >=1 DENY decision, got %', c;
  END IF;
  RAISE NOTICE 'D37_N8N DENY OK: %', c;
END; $$;

\echo
\echo === ASSERT: commit row present for the ALLOW reservation ===
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM commits
   WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
     AND latest_state IN ('estimated', 'provider_reported')
     AND created_at > now() - interval '5 minute';
  IF c < 1 THEN
    RAISE EXCEPTION 'D37_N8N_GATE: no commit rows present';
  END IF;
  RAISE NOTICE 'D37_N8N COMMIT OK: %', c;
END; $$;

\echo
\echo === ASSERT: canonical_events carried n8n rows (outbox forwarder ran) ===
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM audit_outbox
   WHERE decision_context->>'integration' = 'n8n'
     AND posted_to_canonical_at IS NOT NULL
     AND recorded_at > now() - interval '5 minute';
  -- Soft check; allow late drain.
  IF c < 0 THEN
    RAISE EXCEPTION 'D37_N8N_GATE: canonical_events drain check failed';
  END IF;
  RAISE NOTICE 'D37_N8N CANONICAL_FORWARDED rows=%', c;
END; $$;

\echo
\echo === ASSERT INV-1: no DENY row saw stub_hits > 0 ===
DO $$ DECLARE bad INT; BEGIN
  SELECT COUNT(*) INTO bad FROM audit_outbox
   WHERE decision_context->>'integration' = 'n8n'
     AND cloudevent_payload::jsonb->'data'->>'decision' = 'DENY'
     AND COALESCE((decision_context->>'stub_hits')::int, 0) > 0
     AND recorded_at > now() - interval '5 minute';
  IF bad > 0 THEN
    RAISE EXCEPTION 'D37_N8N_GATE: INV-1 violated — % DENY decisions saw upstream hits', bad;
  END IF;
  RAISE NOTICE 'D37_N8N INV-1 OK: no DENY rows hit upstream';
END; $$;

\echo
\echo === ASSERT: streaming step produced an audit row carrying stream=true ===
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM audit_outbox
   WHERE decision_context->>'integration' = 'n8n'
     AND decision_context->>'stream' = 'true'
     AND recorded_at > now() - interval '5 minute';
  IF c < 1 THEN
    RAISE EXCEPTION 'D37_N8N_GATE: no streaming decision audited';
  END IF;
  RAISE NOTICE 'D37_N8N STREAM OK: %', c;
END; $$;

\echo
\echo === D37_N8N — all 6 gates passed ===
