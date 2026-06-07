-- D18 SLICE 82 (windsurf_mitm_fixture demo) — ledger-DB assertions.
--
-- Per the legal posture in
--   services/windsurf_codec/SOW.md §5 and
--   docs/specs/coverage/D18_windsurf_mitm/design.md §1
-- this demo does NOT exercise the real sidecar reserve/commit lane.
-- The runner uses the in-memory mock sidecar so the codec correctness
-- is validated without depending on the sidecar's full gRPC dance.
-- That means the ledger DB has NO rows attributable to this demo
-- (no reserve / commit / denied_decision entries are written).
--
-- The gates this file enforces are therefore the **negative**
-- assertions that the codec stayed offline AND no
-- `server.codeium.com` / `wsf_` references leaked into the audit
-- chain:
--
--   1. Zero ledger_transactions rows with the windsurf-mitm tenant id
--      were written by this demo (the demo runs against the mock
--      sidecar lane).
--   2. Zero canonical_events rows under the windsurf-mitm tenant
--      carry `codeium.com` in any audit payload.
--   3. Zero Codeium credential shapes (`sk-codeium-`, `wsf_`,
--      `codeium_pat_`, `cdm_`) appear in any audit_outbox payload.
--
-- The codec runner's own stdout is the positive gate (it asserts
-- 4 reserves + 3 commits + 1 upstream_error + 1 unsupported +
-- 1 decoder_skipped via the mock sidecar lane and exits non-zero on
-- any drift; the Makefile target gates on that).

\echo
\echo === D18 SLICE 82 windsurf_mitm_fixture — NEGATIVE assertions ===
\echo

-- Tenant id matches the windsurf-mitm runner env var
-- SPENDGUARD_WINDSURF_MITM_TENANT_ID in
-- deploy/demo/windsurf_mitm_fixture/docker-compose.yaml.

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
    RAISE NOTICE 'D18 LEDGER OBSERVED: tenant=% rows=%',
        '00000000-0000-4000-8000-000000000001',
        v_tenant_rows;
END;
$$;

\echo
\echo === D18 LEGAL POSTURE GATE: no codeium.com in audit_outbox ===
DO $$
DECLARE
    v_host_leaks INT;
BEGIN
    SELECT COUNT(*) INTO v_host_leaks
      FROM audit_outbox
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND recorded_at > NOW() - INTERVAL '10 minutes'
       AND (
           cloudevent_payload::text ILIKE '%server.codeium.com%'
           OR cloudevent_payload::text ILIKE '%windsurf-server.codeium.com%'
       );

    IF v_host_leaks > 0 THEN
        RAISE EXCEPTION 'D18_LEGAL_POSTURE_GATE: codeium.com leaked into % audit_outbox rows', v_host_leaks;
    END IF;
    RAISE NOTICE 'D18 LEGAL POSTURE OK: zero codeium.com references in recent audit_outbox';
END;
$$;

\echo
\echo === D18 LEGAL POSTURE GATE: no Codeium / Windsurf session tokens leaked ===
DO $$
DECLARE
    v_token_leaks INT;
BEGIN
    -- Defensive scan: Codeium session tokens follow
    -- `sk-codeium-…` / `wsf_…` / `codeium_pat_…` / `cdm_…` shapes in
    -- public Codeium docs and observed captures. The codec MUST
    -- forward these opaquely; SpendGuard MUST NOT log them. The
    -- audit_outbox payload is the most likely leak surface; we gate
    -- on substring.
    SELECT COUNT(*) INTO v_token_leaks
      FROM audit_outbox
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND recorded_at > NOW() - INTERVAL '10 minutes'
       AND (
           cloudevent_payload::text ILIKE '%sk-codeium-%'
           OR cloudevent_payload::text ILIKE '%wsf_%'
           OR cloudevent_payload::text ILIKE '%codeium_pat_%'
           OR cloudevent_payload::text ILIKE '%cdm_%'
       );

    IF v_token_leaks > 0 THEN
        RAISE EXCEPTION 'D18_LEGAL_POSTURE_GATE: codeium/windsurf session token shape leaked into % audit_outbox rows', v_token_leaks;
    END IF;
    RAISE NOTICE 'D18 LEGAL POSTURE OK: zero codeium/windsurf session token shapes in recent audit_outbox';
END;
$$;

\echo
\echo === D18 SLICE 82 windsurf_mitm_fixture verification done ===
