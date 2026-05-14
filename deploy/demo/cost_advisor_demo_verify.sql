-- =====================================================================
-- DEMO_MODE=cost_advisor — verify cost_findings + approval_requests
-- =====================================================================
--
-- Asserts post-run state after spendguard-advise --write-proposals:
--   * cost_findings: 1 row for the demo bucket, BUDGET-scoped,
--     evidence.scope.budget_id matches the demo budget.
--   * cost_findings_id_keys: 1 row mirroring the finding.
--   * approval_requests: 1 row state='pending', proposal_source=
--     'cost_advisor', proposing_finding_id FK points at the finding,
--     proposed_dsl_patch is a 2-op test+replace patch passing the
--     allowlist CHECK.
--
-- Parameters bound via psql -v (file-scope, OUTSIDE DO blocks) and
-- republished as GUC settings so DO blocks can read them via
-- current_setting() (psql variable interpolation does NOT penetrate
-- dollar-quoted blocks — wire-time bug caught by this demo).

\set ON_ERROR_STOP 1

SELECT set_config('cost_advisor_demo.tenant',      :'tenant',      false);
SELECT set_config('cost_advisor_demo.budget',      :'budget',      false);
SELECT set_config('cost_advisor_demo.approval_id', :'approval_id', false);
SELECT set_config('cost_advisor_demo.demo_date',   :'demo_date',   false);

DO $$
DECLARE
    v_tenant      UUID := current_setting('cost_advisor_demo.tenant')::uuid;
    v_budget      UUID := current_setting('cost_advisor_demo.budget')::uuid;
    v_approval_id UUID := current_setting('cost_advisor_demo.approval_id')::uuid;
    v_demo_date   DATE := current_setting('cost_advisor_demo.demo_date')::date;
    v_finding_count INT;
    v_finding_id UUID;
    v_evidence_scope JSONB;
    v_id_keys_count INT;
    v_approval_count INT;
    v_proposal_source TEXT;
    v_finding_ref UUID;
    v_state TEXT;
    v_patch JSONB;
    v_op0 TEXT; v_path0 TEXT; v_val0 TEXT;
    v_op1 TEXT; v_path1 TEXT; v_val1 BIGINT;
BEGIN
    -- cost_findings: exactly 1 row for this (tenant, date, budget)
    SELECT COUNT(*) INTO v_finding_count
      FROM cost_findings
     WHERE tenant_id = v_tenant
       AND evidence ->> 'time_bucket' = v_demo_date::text
       AND evidence -> 'scope' ->> 'budget_id' = v_budget::text;
    IF v_finding_count <> 1 THEN
        RAISE EXCEPTION
          'expected 1 cost_findings row for tenant=% budget=% date=%, got %',
          v_tenant, v_budget, v_demo_date, v_finding_count;
    END IF;

    SELECT finding_id, evidence -> 'scope' INTO v_finding_id, v_evidence_scope
      FROM cost_findings
     WHERE tenant_id = v_tenant
       AND evidence ->> 'time_bucket' = v_demo_date::text
       AND evidence -> 'scope' ->> 'budget_id' = v_budget::text;
    IF v_evidence_scope ->> 'scope_type' <> 'budget' THEN
        RAISE EXCEPTION 'finding scope_type expected ''budget'', got %',
            v_evidence_scope ->> 'scope_type';
    END IF;
    RAISE NOTICE '  cost_findings row OK (finding_id=%, scope.budget_id=%)',
        v_finding_id, v_evidence_scope ->> 'budget_id';

    SELECT COUNT(*) INTO v_id_keys_count
      FROM cost_findings_id_keys
     WHERE finding_id = v_finding_id;
    IF v_id_keys_count <> 1 THEN
        RAISE EXCEPTION 'id_keys mirror missing for finding %', v_finding_id;
    END IF;
    RAISE NOTICE '  cost_findings_id_keys mirror OK';

    SELECT COUNT(*) INTO v_approval_count
      FROM approval_requests
     WHERE approval_id = v_approval_id;
    IF v_approval_count <> 1 THEN
        RAISE EXCEPTION 'expected 1 approval_requests row, got %', v_approval_count;
    END IF;

    SELECT proposal_source, proposing_finding_id, state, proposed_dsl_patch
      INTO v_proposal_source, v_finding_ref, v_state, v_patch
      FROM approval_requests
     WHERE approval_id = v_approval_id;

    IF v_proposal_source <> 'cost_advisor' THEN
        RAISE EXCEPTION 'expected proposal_source=cost_advisor, got %', v_proposal_source;
    END IF;
    IF v_finding_ref <> v_finding_id THEN
        RAISE EXCEPTION 'proposing_finding_id mismatch (got % vs finding %)',
            v_finding_ref, v_finding_id;
    END IF;
    IF v_state <> 'pending' THEN
        RAISE EXCEPTION 'expected approval state=pending, got %', v_state;
    END IF;

    IF jsonb_array_length(v_patch) <> 2 THEN
        RAISE EXCEPTION 'expected 2-op patch, got % ops', jsonb_array_length(v_patch);
    END IF;
    v_op0 := v_patch -> 0 ->> 'op';
    v_path0 := v_patch -> 0 ->> 'path';
    v_val0 := v_patch -> 0 ->> 'value';
    v_op1 := v_patch -> 1 ->> 'op';
    v_path1 := v_patch -> 1 ->> 'path';
    v_val1 := (v_patch -> 1 ->> 'value')::BIGINT;

    IF v_op0 <> 'test' OR v_path0 <> '/spec/budgets/0/id' THEN
        RAISE EXCEPTION 'patch op[0] expected test on /spec/budgets/0/id, got %/% ', v_op0, v_path0;
    END IF;
    IF v_val0 <> v_budget::text THEN
        RAISE EXCEPTION 'patch test value (%) does not pin the demo budget (%)', v_val0, v_budget;
    END IF;
    IF v_op1 <> 'replace' OR v_path1 <> '/spec/budgets/0/reservation_ttl_seconds' THEN
        RAISE EXCEPTION 'patch op[1] expected replace on reservation_ttl_seconds, got %/% ', v_op1, v_path1;
    END IF;
    IF v_val1 <> 45 THEN
        RAISE EXCEPTION 'patch replace value expected 45 (1.5x median 30), got %', v_val1;
    END IF;
    RAISE NOTICE '  approval_requests row OK (state=pending, patch is 2-op test+replace pinning budget %)', v_budget;
END $$;

DO $$
DECLARE v_count INT;
BEGIN
    SELECT COUNT(*) INTO v_count
      FROM pg_trigger
     WHERE tgname = 'approval_requests_state_change_notify'
       AND NOT tgisinternal;
    IF v_count <> 1 THEN
        RAISE EXCEPTION 'approval_requests_state_change_notify trigger missing';
    END IF;
    RAISE NOTICE '  approval_requests_state_change_notify trigger present';
END $$;
