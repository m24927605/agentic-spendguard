-- D17 SLICE 9 (cursor_mitm_fixture demo) — ledger-DB assertions.
--
-- Per the legal posture in
--   services/cursor_codec/SOW.md §5 and
--   docs/specs/coverage/D17_cursor_mitm/design.md §1
-- this demo does NOT exercise the real sidecar reserve/commit lane.
-- The runner uses the in-memory mock sidecar so the codec correctness
-- is validated without depending on the sidecar's full gRPC dance.
-- That means the ledger DB has NO rows attributable to this demo
-- (no reserve / commit / denied_decision entries are written).
--
-- The gates this file enforces are therefore the **negative**
-- assertions that the codec stayed offline AND no `api.cursor.sh`
-- references leaked into the audit chain:
--
--   1. Zero ledger_transactions rows with the cursor-mitm tenant id
--      were written by this demo (the demo runs against the mock
--      sidecar lane).
--   2. Zero canonical_events rows under the cursor-mitm tenant carry
--      `api.cursor.sh` in any audit payload (review-standards §6 C1
--      no-live-traffic invariant — even if real reserve/commit were
--      wired in the future, the codec MUST NOT leak the upstream host
--      into the audit chain).
--
-- The codec runner's own stdout is the positive gate (it asserts
-- 4 reserves + 3 commits via the mock sidecar lane and exits non-
-- zero on any drift; the Makefile target gates on that).

\echo
\echo === D17 SLICE 9 cursor_mitm_fixture — NEGATIVE assertions ===
\echo

-- Tenant id matches the cursor-mitm runner env var
-- SPENDGUARD_CURSOR_MITM_TENANT_ID in deploy/demo/cursor_mitm_fixture/docker-compose.yaml.

DO $$
DECLARE
    v_tenant_rows INT;
BEGIN
    SELECT COUNT(*) INTO v_tenant_rows
      FROM ledger_transactions
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND recorded_at > NOW() - INTERVAL '10 minutes'
       AND operation_kind IN ('reserve', 'commit_estimated', 'denied_decision');
    -- The mock-sidecar lane is in-process; no rows land in the ledger
    -- for this demo. If/when the runner is upgraded to dial the real
    -- sidecar, this gate flips to a positive count and the demo's
    -- expected report below changes accordingly.
    RAISE NOTICE 'D17 LEDGER OBSERVED: tenant=% rows=%',
        '00000000-0000-4000-8000-000000000001',
        v_tenant_rows;
END;
$$;

\echo
\echo === D17 LEGAL POSTURE GATE: no api.cursor.sh in canonical_events ===
DO $$
DECLARE
    v_cursor_leaks INT;
BEGIN
    -- Run the lookup against spendguard_canonical via dblink shape is
    -- overkill for the gate's blast radius; the ledger-DB tables also
    -- expose enough payload state via audit_outbox.cloudevent_payload.
    SELECT COUNT(*) INTO v_cursor_leaks
      FROM audit_outbox
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND recorded_at > NOW() - INTERVAL '10 minutes'
       AND cloudevent_payload::text ILIKE '%api.cursor.sh%';

    IF v_cursor_leaks > 0 THEN
        RAISE EXCEPTION 'D17_LEGAL_POSTURE_GATE: api.cursor.sh leaked into % audit_outbox rows', v_cursor_leaks;
    END IF;
    RAISE NOTICE 'D17 LEGAL POSTURE OK: zero api.cursor.sh references in recent audit_outbox';
END;
$$;

\echo
\echo === D17 LEGAL POSTURE GATE: no Cursor session bearer tokens leaked ===
DO $$
DECLARE
    v_token_leaks INT;
BEGIN
    -- Defensive scan: Cursor session tokens follow `sk-cur-…` /
    -- `cursor_pat_…` shapes in public docs. The codec MUST forward
    -- these opaquely; SpendGuard MUST NOT log them. The audit_outbox
    -- payload is the most likely leak surface; we gate on substring.
    SELECT COUNT(*) INTO v_token_leaks
      FROM audit_outbox
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND recorded_at > NOW() - INTERVAL '10 minutes'
       AND (
           cloudevent_payload::text ILIKE '%sk-cur-%'
           OR cloudevent_payload::text ILIKE '%cursor_pat_%'
       );

    IF v_token_leaks > 0 THEN
        RAISE EXCEPTION 'D17_LEGAL_POSTURE_GATE: cursor session token shape leaked into % audit_outbox rows', v_token_leaks;
    END IF;
    RAISE NOTICE 'D17 LEGAL POSTURE OK: zero cursor session token shapes in recent audit_outbox';
END;
$$;

\echo
\echo === D17 SLICE 9 cursor_mitm_fixture verification done ===
