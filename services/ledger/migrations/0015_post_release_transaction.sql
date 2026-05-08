-- post_release_transaction stored procedure (Phase 2B Step 7.5).
--
-- Spec references:
--   - Contract DSL §6 stage 7 (commit_or_release)
--   - Contract DSL §7 (reservation TTL/release)
--   - Ledger §3 (per-unit balance), §10 (account_kinds)
--   - Stage 2 §0.2 D12 (audit.outcome strictly after audit.decision)
--
-- Authority model — fully server-derived (Codex Step 7.5 round 1 P1.1+P1.2):
--   * Caller passes only identity (decision_id, idempotency, fencing,
--     reason) + audit_event. NO source_ledger_transaction_id, NO pricing.
--   * SP looks up original reserve tx via (tenant_id, op='reserve',
--     decision_id), then reservations via source_ledger_transaction_id,
--     then pricing via ledger_entries' frozen tuple.
--
-- Single-reservation set only (POC limitation per Codex round 1 Q1):
--   * If COUNT(reservations WHERE source_tx) > 1, raise
--     MULTI_RESERVATION_SET_DEFERRED (mapped to existing
--     MultiReservationCommitDeferred domain error).
--
-- Audit pattern — same as commit_estimated:
--   * audit.outcome event_type with ORIGINAL decision_id.
--   * State check (LOCK FOR UPDATE + assert 'reserved') BEFORE any
--     ledger_transactions/audit_outbox INSERT (Codex round 1 P1.4).
--   * Mutually exclusive with commit_estimated — reservation cycle
--     either commits OR releases, never both. UNIQUE outcome_per_decision
--     enforced because the state check at step 6 rejects the second
--     path before audit insert.
--
-- Reason capture: stored in audit_outbox.cloudevent_payload data
-- (CloudEvent JSONB) and minimal_replay_response. NOT a new column.

CREATE OR REPLACE FUNCTION post_release_transaction(
    p_transaction        JSONB,    -- ledger_transaction shape (see step 11)
    p_reservation_set_id UUID,     -- wire identity; opaque to SP (decision_id is authoritative)
    p_reason             TEXT,     -- 'TTL_EXPIRED' | 'RUNTIME_ERROR' | 'RUN_ABORTED' | 'EXPLICIT'
    p_audit_outbox_row   JSONB     -- cloudevent_payload + signature + ids
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

    v_existing         RECORD;
    v_current          RECORD;
    v_reserve_tx_id    UUID;
    v_reservation_count INT;
    v_reservation      RECORD;
    v_reserve_entry    RECORD;
    v_account_reserved UUID;
    v_account_available UUID;
    v_lock_order_token TEXT;
    v_canonical_keys   TEXT;
    v_tx_id            UUID;
    v_seq_a            BIGINT;
    v_seq_b            BIGINT;
    v_shard_id         SMALLINT := 1;
    v_rowcount         INT;
BEGIN
    -- =========================================================
    -- 1) Idempotency authoritative replay.
    -- =========================================================
    SELECT ledger_transaction_id, request_hash
      INTO v_existing
      FROM ledger_transactions
     WHERE tenant_id      = v_tenant_id
       AND operation_kind = 'release'
       AND idempotency_key = v_idempotency_key;

    IF FOUND THEN
        IF v_existing.request_hash <> v_request_hash THEN
            RAISE EXCEPTION
                'idempotency_key reused with different request_hash'
                USING ERRCODE = '40P03';
        END IF;
        RETURN v_existing.ledger_transaction_id;
    END IF;

    -- =========================================================
    -- 2) Fencing CAS. Sidecar-originated → reservation/budget_window
    --    scope_type allowed (mirrors 0013 commit_estimated SP).
    -- =========================================================
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
    IF v_current.scope_type NOT IN ('reservation', 'budget_window') THEN
        RAISE EXCEPTION
            'fencing_scope type % not allowed for operation release',
            v_current.scope_type
            USING ERRCODE = '40P02';
    END IF;

    -- =========================================================
    -- 3) Idempotency re-check after fencing CAS (same lesson as
    --    Step 8 0014 — fencing serializes us with prior winner).
    -- =========================================================
    SELECT ledger_transaction_id, request_hash
      INTO v_existing
      FROM ledger_transactions
     WHERE tenant_id      = v_tenant_id
       AND operation_kind = 'release'
       AND idempotency_key = v_idempotency_key;

    IF FOUND THEN
        IF v_existing.request_hash <> v_request_hash THEN
            RAISE EXCEPTION
                'idempotency_key reused with different request_hash'
                USING ERRCODE = '40P03';
        END IF;
        RETURN v_existing.ledger_transaction_id;
    END IF;

    -- =========================================================
    -- 4) Find original reserve tx by (tenant, op='reserve', decision_id).
    --    idx_ledger_transactions_decision exists (0007).
    --    decision_id is authoritative; p_reservation_set_id is wire
    --    identity only (not verified by SP per Codex round 2 M1.1
    --    byte-stability concern).
    -- =========================================================
    SELECT ledger_transaction_id INTO v_reserve_tx_id
      FROM ledger_transactions
     WHERE tenant_id      = v_tenant_id
       AND operation_kind = 'reserve'
       AND decision_id    = v_decision_id;

    IF NOT FOUND THEN
        RAISE EXCEPTION
            'RESERVE_NOT_FOUND: no reserve tx for decision_id %',
            v_decision_id
            USING ERRCODE = 'P0001';
    END IF;

    -- =========================================================
    -- 5) Lookup reservations in set; assert single-claim (POC).
    --    Multi-reservation set release deferred to future slice.
    -- =========================================================
    SELECT COUNT(*) INTO v_reservation_count
      FROM reservations
     WHERE tenant_id = v_tenant_id
       AND source_ledger_transaction_id = v_reserve_tx_id;

    IF v_reservation_count = 0 THEN
        RAISE EXCEPTION
            'RESERVATION_SET_EMPTY: no reservations for source_tx %',
            v_reserve_tx_id
            USING ERRCODE = 'P0001';
    END IF;
    IF v_reservation_count > 1 THEN
        RAISE EXCEPTION
            'MULTI_RESERVATION_SET_DEFERRED: source_tx % has % reservations; CommitEstimatedSet/ReleaseSet RPC is a future slice',
            v_reserve_tx_id, v_reservation_count
            USING ERRCODE = 'P0001';
    END IF;

    -- =========================================================
    -- 6) LOCK reservation row + assert 'reserved' BEFORE any
    --    ledger_transactions/audit_outbox INSERT.
    --    Codex round 1 P1.4: state check ordering ensures
    --    commit-then-release race produces RESERVATION_STATE_CONFLICT
    --    (not DuplicateDecisionEvent).
    -- =========================================================
    SELECT reservation_id, current_state, budget_id, window_instance_id,
           ttl_expires_at
      INTO v_reservation
      FROM reservations
     WHERE tenant_id = v_tenant_id
       AND source_ledger_transaction_id = v_reserve_tx_id
       FOR UPDATE;

    IF v_reservation.current_state <> 'reserved' THEN
        RAISE EXCEPTION
            'RESERVATION_STATE_CONFLICT: reservations.current_state=%, expected reserved',
            v_reservation.current_state
            USING ERRCODE = 'P0001';
    END IF;

    -- =========================================================
    -- 7) Recover frozen pricing tuple + amount from original
    --    reserve credit on reserved_hold (server-derived).
    -- =========================================================
    SELECT le.amount_atomic AS amt,
           la.unit_id        AS unit_id,
           la.budget_id      AS budget_id,
           le.window_instance_id AS window_instance_id,
           le.pricing_version,
           le.price_snapshot_hash,
           le.fx_rate_version,
           le.unit_conversion_version
      INTO v_reserve_entry
      FROM ledger_entries le
      JOIN ledger_accounts la ON le.ledger_account_id = la.ledger_account_id
     WHERE le.tenant_id     = v_tenant_id
       AND le.reservation_id = v_reservation.reservation_id
       AND la.account_kind  = 'reserved_hold'
       AND le.direction     = 'credit'
     LIMIT 1;

    IF NOT FOUND THEN
        RAISE EXCEPTION
            'reserve credit entry not found for reservation %',
            v_reservation.reservation_id
            USING ERRCODE = '22023';  -- P0001 fell to Internal; 22023 maps to InvalidRequest
    END IF;

    -- =========================================================
    -- 8) Resolve account_ids for reserved_hold + available_budget.
    -- =========================================================
    SELECT ledger_account_id INTO v_account_reserved
      FROM ledger_accounts
     WHERE tenant_id = v_tenant_id
       AND budget_id = v_reserve_entry.budget_id
       AND window_instance_id = v_reserve_entry.window_instance_id
       AND unit_id = v_reserve_entry.unit_id
       AND account_kind = 'reserved_hold';
    IF NOT FOUND THEN
        RAISE EXCEPTION
            'ledger_account not found for kind=reserved_hold tenant=% budget=%',
            v_tenant_id, v_reserve_entry.budget_id
            USING ERRCODE = '22023';
    END IF;

    SELECT ledger_account_id INTO v_account_available
      FROM ledger_accounts
     WHERE tenant_id = v_tenant_id
       AND budget_id = v_reserve_entry.budget_id
       AND window_instance_id = v_reserve_entry.window_instance_id
       AND unit_id = v_reserve_entry.unit_id
       AND account_kind = 'available_budget';
    IF NOT FOUND THEN
        RAISE EXCEPTION
            'ledger_account not found for kind=available_budget tenant=% budget=%',
            v_tenant_id, v_reserve_entry.budget_id
            USING ERRCODE = '22023';
    END IF;

    -- =========================================================
    -- 9) Lock account rows in canonical order; derive lock token
    --    over (budget_id, window_instance_id, unit_id, account_kind)
    --    per Codex P2.5 fix.
    -- =========================================================
    PERFORM 1
      FROM ledger_accounts la
     WHERE la.ledger_account_id IN (v_account_reserved, v_account_available)
     ORDER BY la.budget_id, la.window_instance_id, la.unit_id, la.account_kind
       FOR UPDATE OF la;

    v_canonical_keys := v_reserve_entry.budget_id::TEXT
                        || ':' || v_reserve_entry.window_instance_id::TEXT
                        || ':' || v_reserve_entry.unit_id::TEXT
                        || ':available_budget,'
                        || v_reserve_entry.budget_id::TEXT
                        || ':' || v_reserve_entry.window_instance_id::TEXT
                        || ':' || v_reserve_entry.unit_id::TEXT
                        || ':reserved_hold';
    v_lock_order_token := 'v1:' || encode(digest(v_canonical_keys, 'sha256'), 'hex');

    -- =========================================================
    -- 10) Allocate sequences (2 entries: 1 debit reserved_hold + 1 credit available_budget).
    -- =========================================================
    v_seq_a := nextval_per_shard(v_shard_id);
    v_seq_b := nextval_per_shard(v_shard_id);

    -- =========================================================
    -- 11) INSERT ledger_transactions (operation_kind='release').
    -- =========================================================
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
            v_tx_id, v_tenant_id, 'release',
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
         WHERE tenant_id = v_tenant_id
           AND operation_kind = 'release'
           AND idempotency_key = v_idempotency_key;
        IF v_existing.request_hash <> v_request_hash THEN
            RAISE EXCEPTION
                'idempotency_key reused with different request_hash'
                USING ERRCODE = '40P03';
        END IF;
        RETURN v_existing.ledger_transaction_id;
    END IF;

    v_tx_id := v_existing.ledger_transaction_id;

    -- =========================================================
    -- 12) INSERT 2 ledger_entries: debit reserved_hold + credit available_budget,
    --     amount = full original_reserved (full refund).
    -- =========================================================
    INSERT INTO ledger_entries (
        ledger_entry_id, ledger_transaction_id, ledger_account_id,
        tenant_id, budget_id, window_instance_id, unit_id,
        direction, amount_atomic,
        pricing_version, price_snapshot_hash, fx_rate_version, unit_conversion_version,
        reservation_id, commit_event_kind,
        ledger_shard_id, ledger_sequence,
        effective_at, effective_month, recorded_at, recorded_month
    ) VALUES
    (
        gen_random_uuid(), v_tx_id, v_account_reserved,
        v_tenant_id, v_reserve_entry.budget_id, v_reserve_entry.window_instance_id, v_reserve_entry.unit_id,
        'debit', v_reserve_entry.amt,
        v_reserve_entry.pricing_version, v_reserve_entry.price_snapshot_hash,
        v_reserve_entry.fx_rate_version, v_reserve_entry.unit_conversion_version,
        v_reservation.reservation_id, NULL,  -- commit_event_kind not applicable for release
        v_shard_id, v_seq_a,
        v_effective_at, date_trunc('month', v_effective_at)::DATE,
        clock_timestamp(), date_trunc('month', clock_timestamp())::DATE
    ),
    (
        gen_random_uuid(), v_tx_id, v_account_available,
        v_tenant_id, v_reserve_entry.budget_id, v_reserve_entry.window_instance_id, v_reserve_entry.unit_id,
        'credit', v_reserve_entry.amt,
        v_reserve_entry.pricing_version, v_reserve_entry.price_snapshot_hash,
        v_reserve_entry.fx_rate_version, v_reserve_entry.unit_conversion_version,
        v_reservation.reservation_id, NULL,
        v_shard_id, v_seq_b,
        v_effective_at, date_trunc('month', v_effective_at)::DATE,
        clock_timestamp(), date_trunc('month', clock_timestamp())::DATE
    );

    -- =========================================================
    -- 13) Per-unit balance check (debit==credit per unit_id).
    -- =========================================================
    PERFORM assert_per_unit_balance_now(v_tx_id);

    -- =========================================================
    -- 14) audit_outbox + audit_outbox_global_keys.
    --     event_type='spendguard.audit.outcome' with ORIGINAL decision_id.
    --     Pairs with reserve's audit.decision; UNIQUE outcome_per_decision
    --     satisfied (1 outcome for the lifecycle; no commit happened).
    -- =========================================================
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
        'release',
        v_workload_id,
        (p_audit_outbox_row->>'producer_sequence')::BIGINT,
        v_idempotency_key,
        date_trunc('month', clock_timestamp())::DATE,
        (p_audit_outbox_row->>'audit_outbox_id')::UUID
    );

    -- =========================================================
    -- 15) Transition reservations.current_state='released'.
    --     Row already locked at step 6.
    -- =========================================================
    UPDATE reservations
       SET current_state = 'released'
     WHERE reservation_id = v_reservation.reservation_id
       AND tenant_id = v_tenant_id
       AND current_state = 'reserved';

    GET DIAGNOSTICS v_rowcount = ROW_COUNT;
    IF v_rowcount <> 1 THEN
        RAISE EXCEPTION
            'RESERVATION_STATE_CONFLICT: UPDATE reservations affected % rows (expected 1)',
            v_rowcount
            USING ERRCODE = 'P0001';
    END IF;

    RETURN v_tx_id;
END;
$$ LANGUAGE plpgsql SECURITY DEFINER;

GRANT EXECUTE ON FUNCTION post_release_transaction(JSONB, UUID, TEXT, JSONB)
    TO ledger_application_role;

-- Index for SP step 5 set lookup.
CREATE INDEX IF NOT EXISTS idx_reservations_source_tx
    ON reservations (tenant_id, source_ledger_transaction_id);
