-- =====================================================================
-- 0045: post_approval_required_decision — fix `approval_id` ambiguity
-- =====================================================================
--
-- Followup #9 part 2 wire bug caught by `DEMO_MODE=approval make demo-up`:
-- the SP in 0037 used unqualified `SELECT approval_id INTO ...` inside
-- a function whose `RETURNS TABLE (... approval_id UUID ...)` declares
-- `approval_id` as an OUT-table-column. Plpgsql then sees `approval_id`
-- as ambiguous between the OUT variable and the column on
-- approval_requests, raising:
--
--   ERROR:  column reference "approval_id" is ambiguous
--   DETAIL: It could refer to either a PL/pgSQL variable or a table column.
--
-- The RETURNING clause at the bottom of the SP already qualified
-- `approval_requests.approval_id` — this migration applies the same
-- qualification to the existence check and the assignment.
-- =====================================================================

CREATE OR REPLACE FUNCTION post_approval_required_decision(
    p_transaction          JSONB,
    p_audit_outbox_row     JSONB,
    p_decision_context     JSONB,
    p_requested_effect     JSONB,
    p_approval_ttl_seconds INT DEFAULT 3600
) RETURNS TABLE (
    ledger_transaction_id UUID,
    approval_id           UUID,
    was_first_insert      BOOLEAN
) AS $$
DECLARE
    v_tenant_id       UUID := (p_transaction->>'tenant_id')::UUID;
    v_decision_id     UUID := (p_transaction->>'decision_id')::UUID;
    v_audit_event_id  UUID := (p_transaction->>'audit_decision_event_id')::UUID;
    v_tx_id           UUID;
    v_approval_id     UUID;
    v_existing_appr_id UUID;
BEGIN
    -- 1) Carrier ledger_transactions + audit_outbox via the existing
    --    denied_decision SP.
    v_tx_id := post_denied_decision_transaction(p_transaction, p_audit_outbox_row);

    -- 2) Idempotent approval_requests insert. Qualify the column to
    --    disambiguate from the OUT-table-column of the same name.
    SELECT approval_requests.approval_id INTO v_existing_appr_id
      FROM approval_requests
     WHERE approval_requests.tenant_id   = v_tenant_id
       AND approval_requests.decision_id = v_decision_id;

    IF FOUND THEN
        v_approval_id := v_existing_appr_id;
        RETURN QUERY SELECT v_tx_id, v_approval_id, FALSE;
        RETURN;
    END IF;

    -- 3) Fresh insert.
    INSERT INTO approval_requests (
        tenant_id,
        decision_id,
        audit_decision_event_id,
        state,
        requested_effect,
        decision_context,
        ttl_expires_at,
        created_at
    ) VALUES (
        v_tenant_id,
        v_decision_id,
        v_audit_event_id,
        'pending',
        p_requested_effect,
        p_decision_context,
        clock_timestamp() + (p_approval_ttl_seconds || ' seconds')::INTERVAL,
        clock_timestamp()
    )
    RETURNING approval_requests.approval_id INTO v_approval_id;

    RETURN QUERY SELECT v_tx_id, v_approval_id, TRUE;
END;
$$ LANGUAGE plpgsql SECURITY DEFINER;

GRANT EXECUTE ON FUNCTION
    post_approval_required_decision(JSONB, JSONB, JSONB, JSONB, INT)
    TO PUBLIC;
