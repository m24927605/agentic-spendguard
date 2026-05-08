-- post_commit_estimated_transaction stored procedure (Phase 2B Step 7).
--
-- Spec references:
--   - Contract DSL §5  (commitStateMachine: reserved -> estimated)
--   - Contract DSL §6  (decision transaction stage 7 commit_or_release)
--   - Ledger §3        (per-unit balance per (transaction, unit_id))
--   - Ledger §6.3      (post_ledger_transaction authority model)
--   - Ledger §10       (account_kinds: reserved_hold + committed_spend + available_budget)
--   - Ledger §13       (4-layer pricing freeze: pricing_version + price_snapshot_hash
--                       + fx_rate_version + unit_conversion_version)
--   - Stage 2 §4.3/§4.8 (audit_outbox per-decision uniqueness; outcome paired
--                       to its preceding decision via shared decision_id)
--   - Stage 2 §8.2.1   (CommitEstimated wire)
--
-- Authority model:
--   * The SP is the SOLE authority on the commit transaction's correctness.
--   * Caller (handler) supplies only identifiers and the requested
--     estimated_amount; the SP looks up reservation truth, derives the
--     entries shape, and writes ledger + audit + projections atomically.
--   * Caller must NOT pre-validate or pre-derive entries.
--
-- Stripe-style partial-capture release:
--   The reservation's residual (original_reserved - estimated) is
--   simultaneously released back to available_budget within the same
--   ledger_transaction. Without this, residual blocks budget until TTL
--   expiry — defeats the Optimize pillar (Phase 1 spec §22.4).
--   Future ProviderReport will adjust by debiting/crediting between
--   committed_spend and available_budget; reserved_hold is fully
--   discharged at CommitEstimated.
--
-- Cardinality limit:
--   This SP commits exactly one reservation. Multi-reservation decisions
--   (one ReserveSet creating N>1 reservations) are rejected at the
--   sidecar layer via QueryReservationContext returning
--   MULTI_RESERVATION_COMMIT_DEFERRED. CommitEstimatedSet for batches is
--   a future slice.
--
-- Idempotency:
--   UNIQUE (tenant_id, operation_kind='commit_estimated', idempotency_key)
--   collapses retries via post-SP step 1 (replay returns existing tx_id).

CREATE OR REPLACE FUNCTION post_commit_estimated_transaction(
    p_transaction       JSONB,    -- ledger_transaction shape (see step 12)
    p_reservation_id    UUID,
    p_estimated_amount  NUMERIC(38,0),
    p_pricing           JSONB,    -- 4 freeze fields supplied by caller for sanity check
    p_audit_outbox_row  JSONB     -- cloudevent_payload + signature + ids
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
    v_reservation      RECORD;
    v_reserve_entry    RECORD;
    v_residual_amount  NUMERIC(38,0);
    v_account_reserved UUID;
    v_account_committed UUID;
    v_account_available UUID;
    v_lock_order_token TEXT;
    v_canonical_keys   TEXT;
    v_tx_id            UUID;
    v_commit_id        UUID;
    v_existing_commit  RECORD;
    v_entry_count      INT;
    v_seq_residual_a   BIGINT;
    v_seq_residual_b   BIGINT;
    v_seq_estimated_a  BIGINT;
    v_seq_estimated_b  BIGINT;
    v_shard_id         SMALLINT := 1;  -- POC default; ledger_shards has shard_id=1
BEGIN
    -- =========================================================
    -- 1) Idempotency authoritative replay.
    -- =========================================================
    SELECT ledger_transaction_id, request_hash
      INTO v_existing
      FROM ledger_transactions
     WHERE tenant_id      = v_tenant_id
       AND operation_kind = 'commit_estimated'
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
    -- 2) Strict fencing CAS (mirror of 0012 step 2).
    -- =========================================================
    SELECT current_epoch, tenant_id AS fence_tenant, active_owner_instance_id,
           ttl_expires_at, scope_type
      INTO v_current
      FROM fencing_scopes
     WHERE fencing_scope_id = v_fencing_scope_id
       FOR UPDATE;

    IF NOT FOUND THEN
        RAISE EXCEPTION 'fencing_scope_id not found'
            USING ERRCODE = '40P02';
    END IF;
    IF v_current.fence_tenant <> v_tenant_id THEN
        RAISE EXCEPTION 'fencing_scope tenant mismatch'
            USING ERRCODE = '40P02';
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
        RAISE EXCEPTION 'fencing_scope lease expired'
            USING ERRCODE = '40P02';
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
            'fencing_scope type % not allowed for operation commit_estimated',
            v_current.scope_type
            USING ERRCODE = '40P02';
    END IF;

    -- =========================================================
    -- 3) LOCK the reservations row FOR UPDATE; assert state + TTL.
    --    Tenant predicate explicit per Codex round 1.5 N2.5.
    -- =========================================================
    SELECT reservation_id, tenant_id, budget_id, window_instance_id,
           current_state, ttl_expires_at, source_ledger_transaction_id
      INTO v_reservation
      FROM reservations
     WHERE tenant_id     = v_tenant_id
       AND reservation_id = p_reservation_id
       FOR UPDATE;

    IF NOT FOUND THEN
        RAISE EXCEPTION
            'RESERVATION_STATE_CONFLICT: reservation_id % not found for tenant %',
            p_reservation_id, v_tenant_id
            USING ERRCODE = 'P0001';
    END IF;

    IF v_reservation.current_state <> 'reserved' THEN
        RAISE EXCEPTION
            'RESERVATION_STATE_CONFLICT: reservation_id % current_state=%, expected reserved',
            p_reservation_id, v_reservation.current_state
            USING ERRCODE = 'P0001';
    END IF;

    IF v_reservation.ttl_expires_at <= clock_timestamp() THEN
        RAISE EXCEPTION
            'RESERVATION_TTL_EXPIRED: reservation_id % ttl_expires_at=% <= now()',
            p_reservation_id, v_reservation.ttl_expires_at
            USING ERRCODE = 'P0001';
    END IF;

    -- =========================================================
    -- 4) Lookup the original reserve entry (the credit on reserved_hold)
    --    via JOIN to ledger_accounts because account_kind lives there
    --    (Codex round 2 M1.1).
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
      JOIN ledger_accounts la
        ON le.ledger_account_id = la.ledger_account_id
     WHERE le.tenant_id        = v_tenant_id
       AND le.reservation_id   = p_reservation_id
       AND la.account_kind     = 'reserved_hold'
       AND le.direction        = 'credit'
     LIMIT 1;  -- POC: single-reservation, single-claim per reservation

    IF NOT FOUND THEN
        RAISE EXCEPTION
            'reserve credit entry not found for reservation %; multi-reservation decisions are rejected upstream',
            p_reservation_id
            USING ERRCODE = 'P0001';
    END IF;

    -- =========================================================
    -- 5) Validate caller-supplied pricing tuple == original (4 fields).
    --    Defense in depth: sidecar already validated against its cache.
    --    IS DISTINCT FROM is NULL-safe: any side NULL on either side
    --    counts as a mismatch (Codex round 2 challenge P2.2).
    -- =========================================================
    IF  (p_pricing->>'pricing_version')      IS DISTINCT FROM v_reserve_entry.pricing_version
       OR decode(p_pricing->>'price_snapshot_hash_hex','hex')
                                              IS DISTINCT FROM v_reserve_entry.price_snapshot_hash
       OR (p_pricing->>'fx_rate_version')    IS DISTINCT FROM v_reserve_entry.fx_rate_version
       OR (p_pricing->>'unit_conversion_version')
                                              IS DISTINCT FROM v_reserve_entry.unit_conversion_version
    THEN
        RAISE EXCEPTION
            'PRICING_FREEZE_MISMATCH: caller pricing differs from original reserve'
            USING ERRCODE = 'P0001';
    END IF;

    -- =========================================================
    -- 5b) Validate caller-supplied unit_id matches original reserve.
    --     Defense in depth: handler also validates against its cache.
    --     Codex round 2 challenge P2.3.
    -- =========================================================
    IF (p_transaction->>'unit_id') IS NOT NULL
       AND (p_transaction->>'unit_id')::UUID IS DISTINCT FROM v_reserve_entry.unit_id
    THEN
        RAISE EXCEPTION
            'UNIT_MISMATCH: caller unit_id % does not match original reserve %',
            p_transaction->>'unit_id', v_reserve_entry.unit_id
            USING ERRCODE = 'P0001';
    END IF;

    -- =========================================================
    -- 6) Validate 0 < estimated <= original_reserved.
    -- =========================================================
    IF p_estimated_amount IS NULL OR p_estimated_amount <= 0 THEN
        RAISE EXCEPTION
            'INVALID_AMOUNT: estimated_amount must be > 0; got %',
            p_estimated_amount
            USING ERRCODE = '22023';
    END IF;
    IF p_estimated_amount > v_reserve_entry.amt THEN
        RAISE EXCEPTION
            'OVERRUN_RESERVATION: estimated_amount % exceeds original_reserved %',
            p_estimated_amount, v_reserve_entry.amt
            USING ERRCODE = 'P0001';
    END IF;

    v_residual_amount := v_reserve_entry.amt - p_estimated_amount;

    -- =========================================================
    -- 7) Resolve ledger_account_id for the 3 account kinds we touch.
    --    All keyed by (tenant, budget, window, unit, account_kind).
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

    SELECT ledger_account_id INTO v_account_committed
      FROM ledger_accounts
     WHERE tenant_id = v_tenant_id
       AND budget_id = v_reserve_entry.budget_id
       AND window_instance_id = v_reserve_entry.window_instance_id
       AND unit_id = v_reserve_entry.unit_id
       AND account_kind = 'committed_spend';
    IF NOT FOUND THEN
        RAISE EXCEPTION
            'ledger_account not found for kind=committed_spend tenant=% budget=%',
            v_tenant_id, v_reserve_entry.budget_id
            USING ERRCODE = '22023';
    END IF;

    -- available_budget account is only needed when residual > 0.
    IF v_residual_amount > 0 THEN
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
    END IF;

    -- =========================================================
    -- 8) Derive commit-specific lock_order_token over the touched accounts.
    -- =========================================================
    SELECT string_agg(
               (v_reserve_entry.budget_id::TEXT || ':' || v_reserve_entry.unit_id::TEXT || ':' || k),
               ',' ORDER BY k)
      INTO v_canonical_keys
      FROM (
          SELECT 'reserved_hold' AS k
          UNION ALL SELECT 'committed_spend'
          UNION ALL SELECT 'available_budget'
              WHERE v_residual_amount > 0
      ) keys;

    v_lock_order_token := 'v1:' || encode(digest(v_canonical_keys, 'sha256'), 'hex');

    -- =========================================================
    -- 9) Acquire row locks on the touched account rows in canonical order.
    -- =========================================================
    PERFORM 1
      FROM ledger_accounts la
     WHERE la.ledger_account_id IN (
        v_account_reserved,
        v_account_committed,
        COALESCE(v_account_available, v_account_reserved)  -- placeholder dedupe
     )
     ORDER BY la.budget_id, la.window_instance_id, la.unit_id, la.account_kind
       FOR UPDATE OF la;

    -- =========================================================
    -- 10) Allocate sequences (1 per row, 2 or 4 entries).
    -- =========================================================
    v_seq_estimated_a := nextval_per_shard(v_shard_id);  -- reserved_hold debit
    v_seq_estimated_b := nextval_per_shard(v_shard_id);  -- committed_spend credit
    IF v_residual_amount > 0 THEN
        v_seq_residual_a := nextval_per_shard(v_shard_id); -- reserved_hold debit (residual)
        v_seq_residual_b := nextval_per_shard(v_shard_id); -- available_budget credit
    END IF;

    -- =========================================================
    -- 11) INSERT ledger_transactions (commit_estimated) with ON CONFLICT
    --     to handle concurrent same-key races.
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
            v_tx_id, v_tenant_id, 'commit_estimated',
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
           AND operation_kind = 'commit_estimated'
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
    -- 12) INSERT ledger_entries (server-built, never trust caller).
    --     2 entries when residual=0; 4 when residual > 0.
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
        'debit', p_estimated_amount,
        v_reserve_entry.pricing_version, v_reserve_entry.price_snapshot_hash,
        v_reserve_entry.fx_rate_version, v_reserve_entry.unit_conversion_version,
        p_reservation_id, 'estimated',
        v_shard_id, v_seq_estimated_a,
        v_effective_at, date_trunc('month', v_effective_at)::DATE,
        clock_timestamp(), date_trunc('month', clock_timestamp())::DATE
    ),
    (
        gen_random_uuid(), v_tx_id, v_account_committed,
        v_tenant_id, v_reserve_entry.budget_id, v_reserve_entry.window_instance_id, v_reserve_entry.unit_id,
        'credit', p_estimated_amount,
        v_reserve_entry.pricing_version, v_reserve_entry.price_snapshot_hash,
        v_reserve_entry.fx_rate_version, v_reserve_entry.unit_conversion_version,
        p_reservation_id, 'estimated',
        v_shard_id, v_seq_estimated_b,
        v_effective_at, date_trunc('month', v_effective_at)::DATE,
        clock_timestamp(), date_trunc('month', clock_timestamp())::DATE
    );

    IF v_residual_amount > 0 THEN
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
            'debit', v_residual_amount,
            v_reserve_entry.pricing_version, v_reserve_entry.price_snapshot_hash,
            v_reserve_entry.fx_rate_version, v_reserve_entry.unit_conversion_version,
            p_reservation_id, 'estimated',
            v_shard_id, v_seq_residual_a,
            v_effective_at, date_trunc('month', v_effective_at)::DATE,
            clock_timestamp(), date_trunc('month', clock_timestamp())::DATE
        ),
        (
            gen_random_uuid(), v_tx_id, v_account_available,
            v_tenant_id, v_reserve_entry.budget_id, v_reserve_entry.window_instance_id, v_reserve_entry.unit_id,
            'credit', v_residual_amount,
            v_reserve_entry.pricing_version, v_reserve_entry.price_snapshot_hash,
            v_reserve_entry.fx_rate_version, v_reserve_entry.unit_conversion_version,
            p_reservation_id, 'estimated',
            v_shard_id, v_seq_residual_b,
            v_effective_at, date_trunc('month', v_effective_at)::DATE,
            clock_timestamp(), date_trunc('month', clock_timestamp())::DATE
        );
    END IF;

    -- =========================================================
    -- 13) Per-unit balance check (existing helper from 0012).
    -- =========================================================
    PERFORM assert_per_unit_balance_now(v_tx_id);

    -- =========================================================
    -- 14) audit_outbox + audit_outbox_global_keys atomic insert.
    --     event_type='spendguard.audit.outcome'; decision_id is the
    --     ORIGINAL ReserveSet decision_id, satisfying the per-decision
    --     outcome uniqueness while pairing to the preceding decision row.
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
        'commit_estimated',
        v_workload_id,
        (p_audit_outbox_row->>'producer_sequence')::BIGINT,
        v_idempotency_key,
        date_trunc('month', clock_timestamp())::DATE,
        (p_audit_outbox_row->>'audit_outbox_id')::UUID
    );

    -- =========================================================
    -- 15) Transition reservations.current_state = 'committed'.
    --     Row already locked at step 3; this is the final write.
    -- =========================================================
    UPDATE reservations
       SET current_state = 'committed'
     WHERE reservation_id = p_reservation_id
       AND tenant_id = v_tenant_id
       AND current_state = 'reserved';

    GET DIAGNOSTICS v_entry_count = ROW_COUNT;
    IF v_entry_count <> 1 THEN
        RAISE EXCEPTION
            'RESERVATION_STATE_CONFLICT: UPDATE reservations affected % rows (expected 1)',
            v_entry_count
            USING ERRCODE = 'P0001';
    END IF;

    -- =========================================================
    -- 16) INSERT commits projection (idempotent on commit_id).
    --     commit_id is deterministic via sha256(reservation_id || ':commit_estimated')
    --     so retries collapse cleanly.
    -- =========================================================
    -- Build a deterministic UUID from sha256(reservation_id || ':commit_estimated').
    v_commit_id := encode(
        substring(digest(p_reservation_id::text || ':commit_estimated', 'sha256') from 1 for 16),
        'hex')::UUID;

    INSERT INTO commits (
        commit_id, reservation_id, tenant_id, budget_id, unit_id,
        latest_state, estimated_amount_atomic, delta_to_reserved_atomic,
        pricing_version, price_snapshot_hash,
        estimated_at,
        latest_projection_only,
        created_at, updated_at
    ) VALUES (
        v_commit_id, p_reservation_id, v_tenant_id,
        v_reserve_entry.budget_id, v_reserve_entry.unit_id,
        'estimated', p_estimated_amount,
        p_estimated_amount - v_reserve_entry.amt,  -- signed: estimated - reserved (typically negative)
        v_reserve_entry.pricing_version, v_reserve_entry.price_snapshot_hash,
        clock_timestamp(),
        TRUE,
        clock_timestamp(), clock_timestamp()
    )
    ON CONFLICT (commit_id) DO NOTHING;

    -- Verify divergence: if the existing row disagrees with our intent,
    -- raise. Same commit_id derives from same reservation_id, so the
    -- only divergence cause would be a different estimated_amount on
    -- replay-after-different-amount. Idempotency UNIQUE on
    -- ledger_transactions catches that earlier (step 1 / step 11), but
    -- defense-in-depth here.
    GET DIAGNOSTICS v_entry_count = ROW_COUNT;
    IF v_entry_count = 0 THEN
        SELECT estimated_amount_atomic, latest_state
          INTO v_existing_commit
          FROM commits
         WHERE commit_id = v_commit_id;
        IF v_existing_commit.estimated_amount_atomic <> p_estimated_amount
           OR v_existing_commit.latest_state <> 'estimated' THEN
            RAISE EXCEPTION
                'COMMIT_ROW_DIVERGENT: existing commit_id % differs from intent (state=%, amount=%)',
                v_commit_id, v_existing_commit.latest_state, v_existing_commit.estimated_amount_atomic
                USING ERRCODE = 'P0001';
        END IF;
    END IF;

    RETURN v_tx_id;
END;
$$ LANGUAGE plpgsql SECURITY DEFINER;

GRANT EXECUTE ON FUNCTION post_commit_estimated_transaction(JSONB, UUID, NUMERIC, JSONB, JSONB)
    TO ledger_application_role;
