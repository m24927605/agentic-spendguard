-- post_denied_decision_transaction — Phase 3 wedge.
--
-- Carries a contract-evaluator-denied decision (STOP / REQUIRE_APPROVAL
-- / DEGRADE / SKIP) into the audit chain without touching balances.
-- Preserves Contract §6.1 invariant 「無 audit 則無 effect」: every
-- decision (effect or not) produces exactly one
-- spendguard.audit.decision row in audit_outbox.
--
-- Why a carrier ledger_transactions row instead of a free-floating
-- audit_outbox insert:
--   * audit_outbox.ledger_transaction_id is NOT NULL with FK to
--     ledger_transactions (per migration 0009). Keeping that invariant
--     intact means denied decisions need a host tx row.
--   * The existing per-tenant balance assertion `assert_per_unit_balance_now`
--     short-circuits on transactions with zero ledger_entries (no entries
--     touch any (budget,window,unit,kind) tuple).
--   * Future analytics can join audit_outbox rows for denied decisions
--     to ledger_transactions(operation_kind='denied_decision') for free.
--
-- Idempotency:
--   * Replays via UNIQUE(tenant_id, operation_kind, idempotency_key).
--   * Cross-kind exclusivity (Codex R1 P0): before inserting a new
--     denied_decision row, this SP also checks for an existing
--     `reserve` row with the same idempotency_key and refuses to
--     write if found (raises IDEMPOTENCY_CONFLICT). This protects
--     against bundle hot-reload mid-retry producing both a
--     `reserve` and a `denied_decision` row for the same logical
--     request — at least in the CONTINUE→DENY direction. The
--     reverse direction (DENY→CONTINUE retry) requires an
--     analogous check in `post_ledger_transaction` (reserve SP);
--     deferred to GA because the reserve SP is foundational and
--     touching it risks regressions in already-shipped Phase 2B
--     demo modes. POC has no hot-reload anyway.

-- 1) Allow the new operation_kind on ledger_transactions.
ALTER TABLE ledger_transactions
    DROP CONSTRAINT IF EXISTS ledger_transactions_operation_kind_check;

ALTER TABLE ledger_transactions
    ADD CONSTRAINT ledger_transactions_operation_kind_check
        CHECK (operation_kind IN
            ('reserve', 'release',
             'commit_estimated', 'provider_report',
             'invoice_reconcile',
             'overrun_debt', 'adjustment',
             'refund_credit', 'dispute_adjustment',
             'compensating',
             'denied_decision'));

-- 2) Stored procedure.
CREATE OR REPLACE FUNCTION post_denied_decision_transaction(
    p_transaction      JSONB,
    p_audit_outbox_row JSONB
) RETURNS UUID AS $$
DECLARE
    v_tenant_id        UUID := (p_transaction->>'tenant_id')::UUID;
    v_idempotency_key  TEXT :=  p_transaction->>'idempotency_key';
    v_request_hash     BYTEA := decode(p_transaction->>'request_hash_hex', 'hex');
    v_decision_id      UUID := (p_transaction->>'decision_id')::UUID;
    v_audit_event_id   UUID := (p_transaction->>'audit_decision_event_id')::UUID;
    v_fencing_scope_id UUID := (p_transaction->>'fencing_scope_id')::UUID;
    v_caller_epoch     BIGINT := (p_transaction->>'fencing_epoch')::BIGINT;
    v_workload_id      TEXT := p_transaction->>'workload_instance_id';
    v_effective_at     TIMESTAMPTZ := (p_transaction->>'effective_at')::TIMESTAMPTZ;
    v_caller_tx_id     UUID := (p_transaction->>'ledger_transaction_id')::UUID;
    v_lock_order_token TEXT := p_transaction->>'lock_order_token';

    v_existing         RECORD;
    v_current          RECORD;
    v_tx_id            UUID;
BEGIN
    -- 1a) Idempotency replay (denied_decision rows for this key).
    SELECT ledger_transaction_id, request_hash
      INTO v_existing
      FROM ledger_transactions
     WHERE tenant_id      = v_tenant_id
       AND operation_kind = 'denied_decision'
       AND idempotency_key = v_idempotency_key;
    IF FOUND THEN
        IF v_existing.request_hash <> v_request_hash THEN
            RAISE EXCEPTION 'idempotency_key reused with different request_hash'
                USING ERRCODE = '40P03';
        END IF;
        RETURN v_existing.ledger_transaction_id;
    END IF;

    -- 1b) Cross-kind exclusivity (Codex R1 P0): if a `reserve` row already
    --     exists for the same idempotency_key, the contract evaluator
    --     reaching DENY must be the result of a divergent bundle (hot-
    --     reload mid-retry). Refuse to write a competing audit row.
    --     Surface as 40P03 (idempotency conflict) so adapter sees a
    --     hard error instead of silently double-auditing.
    IF EXISTS (
        SELECT 1 FROM ledger_transactions
         WHERE tenant_id      = v_tenant_id
           AND operation_kind = 'reserve'
           AND idempotency_key = v_idempotency_key
    ) THEN
        RAISE EXCEPTION
            'idempotency_key % already used by reserve; cannot record DENY for the same logical request',
            v_idempotency_key
            USING ERRCODE = '40P03';
    END IF;

    -- 2) Fencing CAS.
    SELECT current_epoch, tenant_id AS fence_tenant, active_owner_instance_id,
           ttl_expires_at, scope_type
      INTO v_current
      FROM fencing_scopes
     WHERE fencing_scope_id = v_fencing_scope_id
       FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'fencing_scope_id not found' USING ERRCODE = '40P02';
    END IF;
    IF v_current.fence_tenant <> v_tenant_id THEN
        RAISE EXCEPTION 'fencing_scope tenant mismatch' USING ERRCODE = '40P02';
    END IF;
    IF v_current.active_owner_instance_id IS NULL
       OR v_current.active_owner_instance_id <> v_workload_id THEN
        RAISE EXCEPTION
            'fencing_scope owner mismatch: scope owner=%, caller=%',
            v_current.active_owner_instance_id, v_workload_id
            USING ERRCODE = '40P02';
    END IF;
    IF v_current.ttl_expires_at IS NOT NULL
       AND v_current.ttl_expires_at <= clock_timestamp() THEN
        RAISE EXCEPTION 'fencing_scope lease expired' USING ERRCODE = '40P02';
    END IF;
    IF v_current.current_epoch <> v_caller_epoch THEN
        RAISE EXCEPTION
            'FENCING_EPOCH_STALE: caller=%, current=%',
            v_caller_epoch, v_current.current_epoch
            USING ERRCODE = '40P02';
    END IF;
    IF v_caller_epoch = 0 THEN
        RAISE EXCEPTION 'FENCING_EPOCH_STALE: epoch 0 is not a valid lease'
            USING ERRCODE = '40P02';
    END IF;

    -- 3) DENY uses sidecar's normal scope (reservation/budget_window).
    --    No reason-branched scope_type check — DENY only writes
    --    audit_outbox + a no-entry carrier tx, so it's strictly less
    --    invasive than the existing 'reserve' write. The sidecar's
    --    existing fencing scope is the right authority.
    IF v_current.scope_type NOT IN ('reservation', 'budget_window') THEN
        RAISE EXCEPTION
            'fencing_scope type % not allowed for denied_decision (need reservation or budget_window)',
            v_current.scope_type
            USING ERRCODE = '40P02';
    END IF;

    -- 4) Idempotency re-check after fencing CAS (race-safety).
    SELECT ledger_transaction_id, request_hash
      INTO v_existing
      FROM ledger_transactions
     WHERE tenant_id      = v_tenant_id
       AND operation_kind = 'denied_decision'
       AND idempotency_key = v_idempotency_key;
    IF FOUND THEN
        IF v_existing.request_hash <> v_request_hash THEN
            RAISE EXCEPTION 'idempotency_key reused with different request_hash'
                USING ERRCODE = '40P03';
        END IF;
        RETURN v_existing.ledger_transaction_id;
    END IF;

    -- 4b) Cross-kind re-check after fencing CAS (Codex R1 P0 race
    --     coverage): a concurrent reserve SP may have committed
    --     between step 1b and now. Same fail-closed disposition.
    IF EXISTS (
        SELECT 1 FROM ledger_transactions
         WHERE tenant_id      = v_tenant_id
           AND operation_kind = 'reserve'
           AND idempotency_key = v_idempotency_key
    ) THEN
        RAISE EXCEPTION
            'idempotency_key % already used by reserve (concurrent winner); cannot record DENY for the same logical request',
            v_idempotency_key
            USING ERRCODE = '40P03';
    END IF;

    -- 5) INSERT carrier ledger_transactions row (no ledger_entries).
    v_tx_id := COALESCE(v_caller_tx_id, gen_random_uuid());
    WITH ins AS (
        INSERT INTO ledger_transactions (
            ledger_transaction_id, tenant_id, operation_kind,
            posting_state, posted_at,
            idempotency_key, request_hash, minimal_replay_response,
            trace_event_id, audit_decision_event_id, decision_id,
            effective_at, recorded_at,
            lock_order_token, fencing_scope_id, fencing_epoch_at_post
        ) VALUES (
            v_tx_id, v_tenant_id, 'denied_decision',
            'posted', clock_timestamp(),
            v_idempotency_key, v_request_hash,
            COALESCE(p_transaction->'minimal_replay_response', '{}'::JSONB),
            (p_transaction->>'trace_event_id')::UUID,
            v_audit_event_id, v_decision_id,
            v_effective_at, clock_timestamp(),
            v_lock_order_token, v_fencing_scope_id, v_caller_epoch
        )
        ON CONFLICT (tenant_id, operation_kind, idempotency_key) DO NOTHING
        RETURNING ledger_transaction_id, request_hash
    )
    SELECT ledger_transaction_id, request_hash
      INTO v_existing
      FROM ins;
    IF NOT FOUND THEN
        SELECT ledger_transaction_id, request_hash
          INTO v_existing
          FROM ledger_transactions
         WHERE tenant_id      = v_tenant_id
           AND operation_kind = 'denied_decision'
           AND idempotency_key = v_idempotency_key;
        IF v_existing.request_hash <> v_request_hash THEN
            RAISE EXCEPTION 'idempotency_key reused with different request_hash'
                USING ERRCODE = '40P03';
        END IF;
        RETURN v_existing.ledger_transaction_id;
    END IF;
    v_tx_id := v_existing.ledger_transaction_id;

    -- 6) audit_outbox + audit_outbox_global_keys (audit.decision).
    INSERT INTO audit_outbox (
        audit_outbox_id, audit_decision_event_id, decision_id,
        tenant_id, ledger_transaction_id,
        event_type, cloudevent_payload, cloudevent_payload_signature,
        ledger_fencing_epoch, workload_instance_id,
        pending_forward, forward_attempts,
        recorded_at, recorded_month,
        producer_sequence, idempotency_key
    ) VALUES (
        (p_audit_outbox_row->>'audit_outbox_id')::UUID,
        v_audit_event_id, v_decision_id,
        v_tenant_id, v_tx_id,
        p_audit_outbox_row->>'event_type',
        p_audit_outbox_row->'cloudevent_payload',
        decode(p_audit_outbox_row->>'cloudevent_payload_signature_hex', 'hex'),
        v_caller_epoch, v_workload_id,
        TRUE, 0,
        clock_timestamp(),
        date_trunc('month', clock_timestamp())::DATE,
        (p_audit_outbox_row->>'producer_sequence')::BIGINT,
        v_idempotency_key
    );

    INSERT INTO audit_outbox_global_keys (
        audit_decision_event_id, tenant_id, decision_id,
        event_type, operation_kind,
        workload_instance_id, producer_sequence,
        idempotency_key, recorded_month, audit_outbox_id
    ) VALUES (
        v_audit_event_id, v_tenant_id, v_decision_id,
        p_audit_outbox_row->>'event_type',
        'denied_decision',
        v_workload_id,
        (p_audit_outbox_row->>'producer_sequence')::BIGINT,
        v_idempotency_key,
        date_trunc('month', clock_timestamp())::DATE,
        (p_audit_outbox_row->>'audit_outbox_id')::UUID
    );

    RETURN v_tx_id;
END;
$$ LANGUAGE plpgsql SECURITY DEFINER;

GRANT EXECUTE ON FUNCTION post_denied_decision_transaction(JSONB, JSONB)
    TO PUBLIC;
