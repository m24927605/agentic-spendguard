-- D15 COV_74 — verify the Manus fixture-replay demo.
--
-- This SQL script runs against the demo's postgres after the
-- importer binary has emitted CloudEvent envelopes from the fixture
-- and the runner has INSERTed equivalent audit_outbox rows.
--
-- Invariants asserted:
--
--   1. EXACTLY 7 audit_outbox rows exist with:
--          import_source      = 'manus_team_api'
--          reservation_source = 'subscription_meter'
--      (positive gate — fixture has 8 sessions but 1 is in_progress
--      and the demo path filters it per review-standards E3.)
--
--   2. At least one team_plan row has amount_micro_usd > 0.
--      (Headline conversion correctness: 47 credits × 20_526 = 964_722.)
--
--   3. team_plan terminal-row total = 20_731_260 micro-USD.
--      (47 + 12 + 0 + 950 + 1 = 1010 credits × 20_526 = 20_731_260;
--       acceptance A5.4 headline gate.)
--
--   4. enterprise + enterprise_byok rows have amount_micro_usd = 0.
--      (Operator-override required for enterprise; BYOK load-bearing
--      $0 per review-standards P2 / P3.)
--
--   5. ZERO ledger_entries rows for the demo workspace IDs. (Strong
--      invariant — D13 §4.3 fork. Manus importer MUST NOT charge the
--      BYOK ledger.)
--
--   6. The mig 0060 CHECK widening accepts 'manus_team_api'. The
--      INSERTs in step 5 of the runner would have failed otherwise.
--
--   7. Every emitted row has input_tokens = output_tokens = 0 (honest
--      zero per review-standards E5; Manus does NOT expose per-LLM
--      call token detail).
--
--   8. Every emitted row carries the synthetic model slug
--      'manus.session/credit' (review-standards E4).
--
--   9. Every emitted row carries a dedupe_key starting with 'manus:'
--      (review-standards E6 / X3 — vendor-isolated dedupe space).

\set ON_ERROR_STOP on
\echo
\echo === D15 import_manus_fixture demo — invariants ===
\echo

DO $$
DECLARE
    v_total_rows         INT;
    v_team_positive      INT;
    v_team_total         BIGINT;
    v_enterprise_zero    INT;
    v_byok_zero          INT;
    v_ledger_rows        INT;
    v_nonzero_tokens     INT;
    v_wrong_model        INT;
    v_bad_dedupe         INT;
BEGIN
    -- ── Gate 1: 7 import rows landed ───────────────────────────────
    SELECT COUNT(*) INTO v_total_rows
      FROM audit_outbox
     WHERE import_source = 'manus_team_api'
       AND reservation_source = 'subscription_meter';
    RAISE NOTICE 'D15 audit_outbox manus rows = %', v_total_rows;
    IF v_total_rows <> 7 THEN
        RAISE EXCEPTION
            'D15 INVARIANT VIOLATION (A5.1): expected exactly 7 audit_outbox rows with import_source=manus_team_api + reservation_source=subscription_meter; got %',
            v_total_rows;
    END IF;

    -- ── Gate 2: at least one team_plan row positive ────────────────
    SELECT COUNT(*) INTO v_team_positive
      FROM audit_outbox
     WHERE import_source = 'manus_team_api'
       AND tier = 'team_plan'
       AND amount_micro_usd > 0;
    RAISE NOTICE 'D15 team_plan positive-amount rows = %', v_team_positive;
    IF v_team_positive < 1 THEN
        RAISE EXCEPTION
            'D15 INVARIANT VIOLATION (A5.4): expected >= 1 team_plan row with amount_micro_usd > 0; got %',
            v_team_positive;
    END IF;

    -- ── Gate 3: team_plan total matches headline math ──────────────
    -- 47 + 12 + 0 + 950 + 1 = 1010 credits × 20_526 = 20_731_260.
    SELECT COALESCE(SUM(amount_micro_usd), 0) INTO v_team_total
      FROM audit_outbox
     WHERE import_source = 'manus_team_api'
       AND tier = 'team_plan';
    RAISE NOTICE 'D15 team_plan total micro_usd = %', v_team_total;
    IF v_team_total <> 20731260 THEN
        RAISE EXCEPTION
            'D15 INVARIANT VIOLATION (A5.4 headline): team_plan total expected 20731260; got %',
            v_team_total;
    END IF;

    -- ── Gate 4a: enterprise rows zero amount (default) ─────────────
    SELECT COUNT(*) INTO v_enterprise_zero
      FROM audit_outbox
     WHERE import_source = 'manus_team_api'
       AND tier = 'enterprise'
       AND amount_micro_usd = 0;
    RAISE NOTICE 'D15 enterprise zero-amount rows = %', v_enterprise_zero;
    IF v_enterprise_zero < 1 THEN
        RAISE EXCEPTION
            'D15 INVARIANT VIOLATION (P2): expected >= 1 enterprise row with amount_micro_usd = 0; got %',
            v_enterprise_zero;
    END IF;

    -- ── Gate 4b: BYOK rows zero amount (LOAD-BEARING) ──────────────
    SELECT COUNT(*) INTO v_byok_zero
      FROM audit_outbox
     WHERE import_source = 'manus_team_api'
       AND tier = 'enterprise_byok'
       AND amount_micro_usd = 0;
    RAISE NOTICE 'D15 enterprise_byok zero-amount rows = %', v_byok_zero;
    IF v_byok_zero < 1 THEN
        RAISE EXCEPTION
            'D15 INVARIANT VIOLATION (P3 LOAD-BEARING): expected >= 1 enterprise_byok row with amount_micro_usd = 0; got % — BYOK customers MUST NOT be double-billed',
            v_byok_zero;
    END IF;

    -- ── Gate 5: ZERO ledger_entries for any demo workspace ─────────
    SELECT COUNT(*) INTO v_ledger_rows
      FROM ledger_entries
     WHERE tenant_id LIKE 'ws_FAKE_%';
    RAISE NOTICE 'D15 ledger_entries rows for ws_FAKE prefix = %', v_ledger_rows;
    IF v_ledger_rows <> 0 THEN
        RAISE EXCEPTION
            'D15 INVARIANT VIOLATION (D13 §4.3): subscription_meter import MUST NOT write ledger_entries; got % row(s)',
            v_ledger_rows;
    END IF;

    -- ── Gate 7: input/output tokens always zero ────────────────────
    SELECT COUNT(*) INTO v_nonzero_tokens
      FROM audit_outbox
     WHERE import_source = 'manus_team_api'
       AND (COALESCE(input_tokens, 0) <> 0 OR COALESCE(output_tokens, 0) <> 0);
    IF v_nonzero_tokens <> 0 THEN
        RAISE EXCEPTION
            'D15 INVARIANT VIOLATION (E5): expected input_tokens=0 + output_tokens=0 on every row; got % non-zero',
            v_nonzero_tokens;
    END IF;

    -- ── Gate 8: model slug always synthetic ─────────────────────────
    SELECT COUNT(*) INTO v_wrong_model
      FROM audit_outbox
     WHERE import_source = 'manus_team_api'
       AND model <> 'manus.session/credit';
    IF v_wrong_model <> 0 THEN
        RAISE EXCEPTION
            'D15 INVARIANT VIOLATION (E4): expected model=manus.session/credit on every row; got % drift',
            v_wrong_model;
    END IF;

    -- ── Gate 9: dedupe_key vendor-prefixed ──────────────────────────
    SELECT COUNT(*) INTO v_bad_dedupe
      FROM audit_outbox
     WHERE import_source = 'manus_team_api'
       AND (dedupe_key IS NULL OR dedupe_key NOT LIKE 'manus:%');
    IF v_bad_dedupe <> 0 THEN
        RAISE EXCEPTION
            'D15 INVARIANT VIOLATION (E6 / X3): expected dedupe_key starting with manus colon on every row; got % drift',
            v_bad_dedupe;
    END IF;

    RAISE NOTICE 'D15 import_manus_fixture invariants HOLD';
END $$;
