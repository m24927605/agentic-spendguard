-- D14 COV_72 — verify the Devin fixture-replay demo.
--
-- This SQL script runs against the demo's postgres after the
-- importer binary has emitted CloudEvent envelopes from the fixture
-- and the runner has INSERTed equivalent audit_outbox rows.
--
-- Invariants asserted:
--
--   1. ≥ 1 audit_outbox row exists with:
--          tenant_id          = 'demo'
--          import_source      = 'devin_team_api'
--          reservation_source = 'subscription_meter'
--      (positive gate — fixture replay lands rows).
--
--   2. At least one team-plan row has amount_micro_usd > 0.
--      (acceptance A10.3 — ACU → $ conversion correctness.)
--
--   3. At least one enterprise-plan row has amount_micro_usd IS NULL
--      AND reason_code = 'devin_enterprise_negotiated_rate'.
--      (acceptance A10.1 — enterprise null-amount path exercised.)
--
--   4. ZERO ledger_entries rows under the demo tenant for the demo
--      window. (Strong invariant — D13 §4.3 fork. Devin importer
--      MUST NOT charge the BYOK ledger.)
--
--   5. The mig 0059 CHECK widening accepts 'devin_team_api'. The
--      INSERTs in step 3 would have failed otherwise.
--
-- The runner script (`runtime/import_devin_fixture_demo.sh`) inserts
-- the rows from the binary's JSON output; this SQL is purely the
-- assertion gate.

\set ON_ERROR_STOP on
\echo
\echo === D14 import_devin_fixture demo — invariants ===
\echo

DO $$
DECLARE
    v_tenant            TEXT := 'demo';
    v_team_rows         INT;
    v_team_positive     INT;
    v_enterprise_rows   INT;
    v_ledger_rows       INT;
BEGIN
    -- ── Gate 1: positive — Devin rows landed ───────────────────────
    SELECT COUNT(*) INTO v_team_rows
      FROM audit_outbox
     WHERE tenant_id::text = v_tenant
       AND import_source = 'devin_team_api'
       AND reservation_source = 'subscription_meter';
    RAISE NOTICE 'D14 audit_outbox devin rows = %', v_team_rows;
    IF v_team_rows < 1 THEN
        RAISE EXCEPTION
            'D14 INVARIANT VIOLATION: expected >= 1 audit_outbox row with import_source=devin_team_api + reservation_source=subscription_meter for tenant %; got %',
            v_tenant, v_team_rows;
    END IF;

    -- ── Gate 2: team-plan amount conversion correctness ────────────
    SELECT COUNT(*) INTO v_team_positive
      FROM audit_outbox
     WHERE tenant_id::text = v_tenant
       AND import_source = 'devin_team_api'
       AND amount_micro_usd > 0;
    RAISE NOTICE 'D14 team-plan positive-amount rows = %', v_team_positive;
    IF v_team_positive < 1 THEN
        RAISE EXCEPTION
            'D14 INVARIANT VIOLATION (A10.3): expected >= 1 row with amount_micro_usd > 0; got %',
            v_team_positive;
    END IF;

    -- ── Gate 3: enterprise NULL-amount path ────────────────────────
    SELECT COUNT(*) INTO v_enterprise_rows
      FROM audit_outbox
     WHERE tenant_id::text = v_tenant
       AND import_source = 'devin_team_api'
       AND amount_micro_usd IS NULL
       AND reason_code = 'devin_enterprise_negotiated_rate';
    RAISE NOTICE 'D14 enterprise NULL-amount rows = %', v_enterprise_rows;
    IF v_enterprise_rows < 1 THEN
        RAISE EXCEPTION
            'D14 INVARIANT VIOLATION (A10.1 enterprise): expected >= 1 row with amount_micro_usd IS NULL + reason_code=devin_enterprise_negotiated_rate; got %',
            v_enterprise_rows;
    END IF;

    -- ── Gate 4: ZERO ledger_entries (D13 §4.3 fork) ────────────────
    SELECT COUNT(*) INTO v_ledger_rows
      FROM ledger_entries
     WHERE tenant_id::text = v_tenant;
    RAISE NOTICE 'D14 ledger_entries rows for tenant % = %', v_tenant, v_ledger_rows;
    IF v_ledger_rows <> 0 THEN
        RAISE EXCEPTION
            'D14 INVARIANT VIOLATION (D13 §4.3): subscription_meter MUST NOT write ledger_entries; got % row(s) for tenant %',
            v_ledger_rows, v_tenant;
    END IF;

    RAISE NOTICE 'D14 import_devin_fixture invariants HOLD';
END $$;
