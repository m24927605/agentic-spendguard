-- =====================================================================
-- DEMO_MODE=cost_advisor — resolve the proposal + verify transition
-- =====================================================================

\set ON_ERROR_STOP 1

SELECT set_config('cost_advisor_demo.approval_id', :'approval_id', false);

DO $$
DECLARE
    v_approval_id  UUID := current_setting('cost_advisor_demo.approval_id')::uuid;
    v_final_state TEXT;
    v_transitioned BOOLEAN;
    v_event_id UUID;
BEGIN
    SELECT final_state, transitioned, event_id
      INTO v_final_state, v_transitioned, v_event_id
      FROM resolve_approval_request(
          v_approval_id,
          'approved'::text,
          'cost-advisor-demo-operator'::text,
          'cost-advisor-demo-issuer'::text,
          'demo: auto-approve the cost_advisor proposal'::text
      );

    IF NOT v_transitioned THEN
        RAISE EXCEPTION 'resolve_approval_request did not transition (final_state=%)',
            v_final_state;
    END IF;
    IF v_final_state <> 'approved' THEN
        RAISE EXCEPTION 'expected final_state=approved, got %', v_final_state;
    END IF;
    IF v_event_id IS NULL THEN
        RAISE EXCEPTION 'resolve returned NULL event_id';
    END IF;

    RAISE NOTICE '  approval transitioned pending → approved (event_id=%)', v_event_id;
END $$;

DO $$
DECLARE
    v_approval_id UUID := current_setting('cost_advisor_demo.approval_id')::uuid;
    v_state TEXT;
BEGIN
    SELECT state INTO v_state
      FROM approval_requests
     WHERE approval_id = v_approval_id;
    IF v_state <> 'approved' THEN
        RAISE EXCEPTION 'expected approval_requests.state=approved, got %', v_state;
    END IF;
    RAISE NOTICE '  approval_requests.state confirmed as approved';
END $$;

DO $$
DECLARE
    v_approval_id UUID := current_setting('cost_advisor_demo.approval_id')::uuid;
    v_count INT;
BEGIN
    SELECT COUNT(*) INTO v_count
      FROM approval_events
     WHERE approval_id = v_approval_id
       AND to_state = 'approved';
    IF v_count = 0 THEN
        RAISE EXCEPTION 'approval_events missing the pending→approved transition';
    END IF;
    RAISE NOTICE '  approval_events audit row written (count=%)', v_count;
END $$;
