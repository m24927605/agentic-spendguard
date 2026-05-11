-- post_approval_required_decision — Round-2 #9 part 2 producer SP.
--
-- Closes the gap left by PR #15 (mark_approval_bundled SP) + PR #37
-- (GetApprovalForResume / MarkApprovalBundled RPCs) + PR #38 (sidecar
-- resume wiring) + PR #39 (Python SDK).
--
-- BEFORE this SP: when the contract evaluator returned REQUIRE_APPROVAL,
-- sidecar called post_denied_decision_transaction which wrote a
-- ledger_transactions(operation_kind='denied_decision') row + a single
-- audit_outbox row. No approval_requests row was created, so the
-- resume() round-trip had no row to look up — it returned
-- [PRODUCER_SP_NOT_WIRED] from sidecar's resume_after_approval handler.
--
-- AFTER this SP: REQUIRE_APPROVAL routes here instead. The SP performs
-- both inserts atomically (single Postgres tx):
--
--   1. carrier ledger_transactions(operation_kind='denied_decision')
--      + audit_outbox row + audit_outbox_global_keys row (delegated
--      to post_denied_decision_transaction body — kept here as a
--      wrapper so the existing SP is unchanged and approval-specific
--      logic stays isolated)
--   2. approval_requests row in state='pending' with the full
--      decision_context + requested_effect JSON the sidecar resume
--      path needs to rebuild a fresh ReserveSetRequest
--
-- JSON shape contract — MUST match what
-- `services/sidecar/src/server/adapter_uds.rs::approval_resume_payload`
-- deserializes (sister change in this PR):
--
--   decision_context = {
--     "tenant_id":                       UUID string,
--     "budget_id":                       UUID string,
--     "window_instance_id":              UUID string,
--     "fencing_scope_id":                UUID string,
--     "fencing_epoch":                   uint64,
--     "decision_id":                     UUID string,
--     "matched_rule_ids":                array of strings,
--     "reason_codes":                    array of strings,
--     "contract_bundle_id":              UUID string,
--     "contract_bundle_hash_hex":        hex string (64 chars),
--     "schema_bundle_id":                UUID string,
--     "schema_bundle_canonical_version": semver string
--   }
--
--   requested_effect = {
--     "unit_id":         UUID string,
--     "unit_kind":       "MONETARY" | "TOKEN" | "CREDIT" | "NON_MONETARY",
--     "unit_token_kind": string,
--     "amount_atomic":   NUMERIC(38,0) decimal string,
--     "direction":       "DEBIT" | "CREDIT"
--   }
--
-- Idempotency: replays via underlying SP's UNIQUE(tenant_id,
-- operation_kind, idempotency_key); approval_requests insert is also
-- idempotent on (tenant_id, decision_id) — second call returns the
-- existing approval_id.

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
    v_existing_appr   RECORD;
BEGIN
    -- 1) Carrier ledger_transactions + audit_outbox via the existing
    --    denied_decision SP. This keeps every approval decision
    --    visible in the same audit lineage as other denial kinds
    --    (Contract §6.1 「無 audit 則無 effect」 invariant intact).
    v_tx_id := post_denied_decision_transaction(p_transaction, p_audit_outbox_row);

    -- 2) Idempotent approval_requests insert.
    SELECT approval_id INTO v_existing_appr
      FROM approval_requests
     WHERE tenant_id   = v_tenant_id
       AND decision_id = v_decision_id;

    IF FOUND THEN
        -- Replay: surface the existing approval_id without inserting.
        v_approval_id := v_existing_appr.approval_id;
        RETURN QUERY SELECT v_tx_id, v_approval_id, FALSE;
        RETURN;
    END IF;

    -- 3) Fresh insert. approval_id is generated server-side so the
    --    caller doesn't need to mint one.
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

COMMENT ON FUNCTION post_approval_required_decision IS
    'Round-2 #9 producer SP. Wraps post_denied_decision_transaction + '
    'inserts approval_requests row atomically. Caller supplies '
    'decision_context + requested_effect JSON in the shape sidecar '
    'resume path deserializes — see services/sidecar/src/server/'
    'adapter_uds.rs::approval_resume_payload. Idempotent on '
    '(tenant_id, decision_id).';
