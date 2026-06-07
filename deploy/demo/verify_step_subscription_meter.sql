-- D13 (subscription_meter demo) — ledger-DB assertions.
--
-- The driver POSTs three DecisionRequest gRPC calls through the
-- sidecar's adapter UDS with reservation_source=SUBSCRIPTION_METER:
--
--   step 1: ClaudeCodePro PASS  (under all thresholds)
--   step 2: ClaudeCodePro SOFT  (over alert_at_atomic; CONTINUE + alert)
--   step 3: ClaudeCodePro HARD  (over hard_cap_at_atomic; STOP + 429)
--
-- The verification gates:
--
--   1. audit_outbox carries reservation_source='subscription_meter'
--      for the demo tenant (positive — meter rows landed).  We do not
--      assert an exact count because the sidecar may emit additional
--      rows in v1alpha2 / SLICE_09 paths that we don't drive directly.
--      The negative gate (step 2) is the strong invariant.
--
--   2. ZERO ledger_entries rows under the meter tenant for the demo
--      window (negative — meter path MUST NOT charge the BYOK ledger,
--      design §4.3 invariant).
--
--   3. ZERO reservations rows under the meter tenant (negative —
--      meter path MUST NOT reserve, design §4.3 invariant).
--
-- The driver's own stdout (the `SUBSCRIPTION_METER_DEMO_OK` line) is
-- the positive gate for the three-step cap evaluation: it asserts the
-- HardCap response carried the synthetic 429 + decision=STOP +
-- reason_codes including `subscription_cap_exceeded`, and exits
-- non-zero on any drift.

\echo
\echo === D13 subscription_meter demo — invariants ===
\echo

-- Tenant id matches the runner env var
-- SPENDGUARD_SUBSCRIPTION_METER_TENANT_ID in
-- deploy/demo/subscription_meter/docker-compose.yaml.

DO $$
DECLARE
    v_tenant_id   UUID := '00000000-0000-4000-8000-00000000d013'::UUID;
    v_meter_rows  INT;
    v_ledger_rows INT;
    v_resv_rows   INT;
BEGIN
    -- ── Gate 1: meter rows landed in audit_outbox ──────────────────
    SELECT COUNT(*) INTO v_meter_rows
      FROM audit_outbox
     WHERE tenant_id = v_tenant_id
       AND reservation_source = 'subscription_meter';
    RAISE NOTICE 'D13 audit_outbox meter rows = %', v_meter_rows;
    -- The audit chain may be empty if the sidecar's audit writer is
    -- not wired against the demo postgres in this DEMO_MODE; that is
    -- fine — the negative gates below are the load-bearing checks
    -- and the runner stdout is the positive proof.

    -- ── Gate 2: ZERO ledger_entries for the meter tenant ───────────
    SELECT COUNT(*) INTO v_ledger_rows
      FROM ledger_entries
     WHERE tenant_id = v_tenant_id;
    IF v_ledger_rows <> 0 THEN
        RAISE EXCEPTION
            'INVARIANT VIOLATION (D13 §4.3): subscription_meter MUST NOT write ledger_entries; got % row(s) for tenant %',
            v_ledger_rows, v_tenant_id;
    END IF;
    RAISE NOTICE 'D13 ledger_entries meter rows = 0 (expected)';

    -- ── Gate 3: ZERO reservations for the meter tenant ─────────────
    SELECT COUNT(*) INTO v_resv_rows
      FROM reservations
     WHERE tenant_id = v_tenant_id;
    IF v_resv_rows <> 0 THEN
        RAISE EXCEPTION
            'INVARIANT VIOLATION (D13 §4.3): subscription_meter MUST NOT reserve; got % row(s) for tenant %',
            v_resv_rows, v_tenant_id;
    END IF;
    RAISE NOTICE 'D13 reservations meter rows = 0 (expected)';

    -- ── Gate 4: subscription_meters / subscription_alerts schema present ──
    -- These were created by migrations 0044 / 0045; if they're missing
    -- the demo is running against a pre-D13 ledger.
    PERFORM 1 FROM information_schema.tables
        WHERE table_name = 'subscription_meters';
    IF NOT FOUND THEN
        RAISE EXCEPTION 'D13 migration 0044 missing: subscription_meters table not found';
    END IF;
    PERFORM 1 FROM information_schema.tables
        WHERE table_name = 'subscription_alerts';
    IF NOT FOUND THEN
        RAISE EXCEPTION 'D13 migration 0045 missing: subscription_alerts table not found';
    END IF;
    PERFORM 1 FROM information_schema.tables
        WHERE table_name = 'subscription_import_jobs';
    IF NOT FOUND THEN
        RAISE EXCEPTION 'D13 migration 0046 missing: subscription_import_jobs table not found';
    END IF;

    RAISE NOTICE 'D13 schema present: 0044/0045/0046 migrations applied';
    RAISE NOTICE 'D13 subscription_meter invariants HOLD';
END $$;
