-- D16 COV_88 — verify the Genspark fixture-replay demo.
--
-- This SQL script runs against the demo's postgres after the
-- importer binary has emitted CloudEvent envelopes from the fixture
-- and the runner has INSERTed equivalent audit_outbox rows.
--
-- Invariants asserted:
--
--   1. >= 3 audit_outbox rows exist with:
--          tenant_id          = 'demo'
--          import_source      = 'genspark_team_api'
--          reservation_source = 'subscription_meter'
--      (positive gate — fixture replay lands rows).
--
--   2. At least one plus-plan row has amount_micro_usd > 0.
--      (credit → $ conversion correctness, plus rate.)
--
--   3. At least one premium-plan row has amount_micro_usd > 0.
--      (credit → $ conversion correctness, premium rate.)
--
--   4. At least one unknown-plan row has amount_micro_usd = 0
--      AND reason_code = 'genspark_plan_unknown'.
--      (unknown-plan fallback path, T7 + F4.)
--
--   5. ZERO ledger_entries rows under the demo tenant for the demo
--      window. (Strong invariant — D13 §4.3 fork. Genspark importer
--      MUST NOT charge the BYOK ledger.)
--
--   6. The mig 0061 CHECK widening accepts 'genspark_team_api'. The
--      INSERTs in step 3 would have failed otherwise.
--
--   7. The plus-row hand-computed expected value matches:
--      3200 credits × $0.001999/credit × 1e6 = 6_396_800 micro-USD.
--
-- The runner script (`import_genspark_fixture_demo.sh`) inserts
-- the rows from the binary's JSON output; this SQL is purely the
-- assertion gate.

\set ON_ERROR_STOP on
\echo
\echo === D16 import_genspark_fixture demo — invariants ===
\echo

DO $$
DECLARE
    v_tenant            TEXT := 'demo';
    v_total_rows        INT;
    v_plus_positive     INT;
    v_premium_positive  INT;
    v_unknown_zero      INT;
    v_ledger_rows       INT;
    v_plus_3200_amount  BIGINT;
BEGIN
    -- ── Gate 1: positive — Genspark rows landed ────────────────────
    SELECT COUNT(*) INTO v_total_rows
      FROM audit_outbox
     WHERE tenant_id::text = v_tenant
       AND import_source = 'genspark_team_api'
       AND reservation_source = 'subscription_meter';
    RAISE NOTICE 'D16 audit_outbox genspark rows = %', v_total_rows;
    IF v_total_rows < 3 THEN
        RAISE EXCEPTION
            'D16 INVARIANT VIOLATION: expected >= 3 audit_outbox rows with import_source=genspark_team_api + reservation_source=subscription_meter for tenant %; got %',
            v_tenant, v_total_rows;
    END IF;

    -- ── Gate 2: plus-plan amount conversion correctness ────────────
    SELECT COUNT(*) INTO v_plus_positive
      FROM audit_outbox
     WHERE tenant_id::text = v_tenant
       AND import_source = 'genspark_team_api'
       AND model LIKE 'genspark/credit/plus%'
       AND amount_micro_usd > 0;
    RAISE NOTICE 'D16 plus-plan positive-amount rows = %', v_plus_positive;
    IF v_plus_positive < 1 THEN
        RAISE EXCEPTION
            'D16 INVARIANT VIOLATION: expected >= 1 plus-plan row with amount_micro_usd > 0; got %',
            v_plus_positive;
    END IF;

    -- ── Gate 3: premium-plan amount conversion correctness ─────────
    SELECT COUNT(*) INTO v_premium_positive
      FROM audit_outbox
     WHERE tenant_id::text = v_tenant
       AND import_source = 'genspark_team_api'
       AND model LIKE 'genspark/credit/premium%'
       AND amount_micro_usd > 0;
    RAISE NOTICE 'D16 premium-plan positive-amount rows = %', v_premium_positive;
    IF v_premium_positive < 1 THEN
        RAISE EXCEPTION
            'D16 INVARIANT VIOLATION: expected >= 1 premium-plan row with amount_micro_usd > 0; got %',
            v_premium_positive;
    END IF;

    -- ── Gate 4: unknown-plan fallback path ─────────────────────────
    SELECT COUNT(*) INTO v_unknown_zero
      FROM audit_outbox
     WHERE tenant_id::text = v_tenant
       AND import_source = 'genspark_team_api'
       AND amount_micro_usd = 0
       AND reason_code = 'genspark_plan_unknown';
    RAISE NOTICE 'D16 unknown-plan fallback rows = %', v_unknown_zero;
    IF v_unknown_zero < 1 THEN
        RAISE EXCEPTION
            'D16 INVARIANT VIOLATION (T7/F4): expected >= 1 row with amount_micro_usd = 0 + reason_code = genspark_plan_unknown; got %',
            v_unknown_zero;
    END IF;

    -- ── Gate 5: ZERO ledger_entries (D13 §4.3 fork) ────────────────
    SELECT COUNT(*) INTO v_ledger_rows
      FROM ledger_entries
     WHERE tenant_id::text = v_tenant;
    RAISE NOTICE 'D16 ledger_entries rows for tenant % = %', v_tenant, v_ledger_rows;
    IF v_ledger_rows <> 0 THEN
        RAISE EXCEPTION
            'D16 INVARIANT VIOLATION (D13 §4.3): subscription_meter MUST NOT write ledger_entries; got % row(s) for tenant %',
            v_ledger_rows, v_tenant;
    END IF;

    -- ── Gate 7: hand-computed plus-row conversion correctness ──────
    -- The fixture record with credits_consumed = 3200 should produce
    -- amount_micro_usd = 6_396_800 (3200 × 0.001999 × 1e6).
    SELECT amount_micro_usd INTO v_plus_3200_amount
      FROM audit_outbox
     WHERE tenant_id::text = v_tenant
       AND import_source = 'genspark_team_api'
       AND credits_consumed = 3200
     LIMIT 1;
    IF v_plus_3200_amount IS NULL THEN
        RAISE EXCEPTION
            'D16 INVARIANT VIOLATION: expected fixture row with credits_consumed = 3200; not found';
    END IF;
    IF v_plus_3200_amount <> 6396800 THEN
        RAISE EXCEPTION
            'D16 INVARIANT VIOLATION: hand-computed plus-row conversion drift — expected 6_396_800 micro-USD, got %',
            v_plus_3200_amount;
    END IF;
    RAISE NOTICE 'D16 plus 3200-credit row amount = % (hand-computed match)', v_plus_3200_amount;

    RAISE NOTICE 'D16 import_genspark_fixture invariants HOLD';
END $$;
