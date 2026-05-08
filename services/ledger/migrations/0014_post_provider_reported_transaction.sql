-- post_provider_reported_transaction stored procedure (Phase 2B Step 8).
--
-- Spec references:
--   - Contract DSL §5  (commitStateMachine: estimated -> provider_reported;
--                       condition: out_of_band_provider_response_received;
--                       action: adjust_commit_amount + delta_to_audit:true)
--   - Contract DSL §5.1a / §6 (post-commit overrun -> overrun_debt path)
--   - Ledger §3        (per-unit balance per (transaction, unit_id))
--   - Ledger §10       (account_kinds; provider_report adjusts committed_spend
--                       vs available_budget for delta = provider - estimated)
--   - Stage 2 §0.2 D9  (Provider Webhook Receiver = only provider entry;
--                       audit goes via ledger.audit_outbox)
--   - Stage 2 §4       (audit_outbox + per-decision uniqueness)
--   - Stage 2 §8.2.3   (webhook flow: dedup by provider event id)
--   - Stage 2 §10      (audit.outcome captures FINAL state; provider_report
--                       is a TRANSITION -> writes audit.decision instead)
--
-- Authority model (mirrors 0013 SP):
--   * SP is the SOLE authority on the provider_report transaction.
--   * Caller (webhook receiver / demo simulator) supplies only identifiers
--     plus the new provider_amount. SP looks up reservation + commits
--     truth, verifies pricing tuple equality, and writes ledger + audit
--     + projection update atomically.
--
-- Webhook identity namespacing (Codex round 2 M2.1):
--   Caller derives `decision_id` and `idempotency_key` from
--   "provider_report:{provider}:{provider_account}:{provider_event_id}".
--   Replays of same provider event → same decision_id → SP step 1
--   collapses via UNIQUE(tenant_id, operation_kind, idempotency_key).
--
-- Audit pattern (Codex round 1 DD-A1):
--   ProviderReport is a *transition* event (estimated -> provider_reported);
--   final lifecycle close (audit.outcome) is reserved for invoice_reconcile
--   or release. SP writes ONE audit_outbox row with
--   event_type='spendguard.audit.decision'. Fresh decision_id avoids
--   audit_outbox_*_per_decision_uq collisions.
--
-- Delta=0 special case (Codex round 3 L1.1):
--   When provider_amount == estimated_amount, the transition still fires
--   (per Contract §5 condition is "response received", not "delta != 0"),
--   but no ledger_entries are emitted. ledger_transactions row is still
--   inserted; audit_outbox row + commits projection update still happen.
--   per-unit balance check is vacuously satisfied (0 rows aggregate to
--   nothing).

CREATE OR REPLACE FUNCTION post_provider_reported_transaction(
    p_transaction       JSONB,    -- ledger_transaction shape (see step 12)
    p_reservation_id    UUID,
    p_provider_amount   NUMERIC(38,0),
    p_pricing           JSONB,    -- 4 freeze fields supplied by caller for sanity
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
    v_commit_row       RECORD;
    v_reserve_entry    RECORD;
    v_delta            NUMERIC(38,0);
    v_account_committed UUID;
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
    -- 1) Idempotency authoritative replay (same as 0013).
    -- =========================================================
    SELECT ledger_transaction_id, request_hash
      INTO v_existing
      FROM ledger_transactions
     WHERE tenant_id      = v_tenant_id
       AND operation_kind = 'provider_report'
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
    -- 2) Fencing CAS (control_plane_writer scope_type required;
    --    SP 0012 step 2 guarded the same way for provider_report).
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
    IF v_current.scope_type <> 'control_plane_writer' THEN
        RAISE EXCEPTION
            'fencing_scope type % not allowed for operation provider_report',
            v_current.scope_type
            USING ERRCODE = '40P02';
    END IF;

    -- =========================================================
    -- 2b) Idempotency re-check AFTER fencing CAS (Codex challenge P1.1).
    --     Fencing FOR UPDATE serializes us with any prior winner; if
    --     the prior winner already inserted a ledger_transactions row
    --     with this idempotency_key, surface their tx_id as Replay
    --     rather than failing the commit-state CAS at step 4 with
    --     RESERVATION_STATE_CONFLICT.
    -- =========================================================
    SELECT ledger_transaction_id, request_hash
      INTO v_existing
      FROM ledger_transactions
     WHERE tenant_id      = v_tenant_id
       AND operation_kind = 'provider_report'
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
    -- 3) LOCK reservations row; assert tenant + current_state='committed'.
    -- =========================================================
    SELECT reservation_id, tenant_id, budget_id, window_instance_id,
           current_state
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
    IF v_reservation.current_state <> 'committed' THEN
        RAISE EXCEPTION
            'RESERVATION_STATE_CONFLICT: reservations.current_state=%, expected committed',
            v_reservation.current_state
            USING ERRCODE = 'P0001';
    END IF;

    -- =========================================================
    -- 4) LOCK commits row; CAS on latest_state='estimated'.
    --    Codex round 1 P1.1 fix.
    -- =========================================================
    SELECT commit_id, latest_state, estimated_amount_atomic,
           pricing_version, price_snapshot_hash, unit_id, budget_id
      INTO v_commit_row
      FROM commits
     WHERE tenant_id = v_tenant_id
       AND reservation_id = p_reservation_id
       FOR UPDATE;

    IF NOT FOUND THEN
        RAISE EXCEPTION
            'COMMIT_NOT_FOUND: provider_report requires prior commit_estimated for reservation %',
            p_reservation_id
            USING ERRCODE = 'P0001';
    END IF;
    IF v_commit_row.latest_state <> 'estimated' THEN
        RAISE EXCEPTION
            'RESERVATION_STATE_CONFLICT: commits.latest_state=%, expected estimated',
            v_commit_row.latest_state
            USING ERRCODE = 'P0001';
    END IF;

    -- =========================================================
    -- 5) Lookup the original reserve credit on reserved_hold to recover
    --    the FROZEN pricing tuple + original_reserved_amount.
    --    Same pattern as 0013 SP step 4 (M1.1 fix).
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
       AND le.reservation_id = p_reservation_id
       AND la.account_kind  = 'reserved_hold'
       AND le.direction     = 'credit'
     LIMIT 1;

    IF NOT FOUND THEN
        RAISE EXCEPTION
            'reserve credit entry not found for reservation %',
            p_reservation_id
            USING ERRCODE = 'P0001';
    END IF;

    -- =========================================================
    -- 5b) Validate caller-supplied unit_id matches original reserve.
    --     Codex Step 8 challenge P2.1: handler accepts unit on the
    --     wire but SP previously ignored it. Defense-in-depth.
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
    -- 6) Validate caller pricing == frozen tuple (IS DISTINCT FROM).
    --    Codex round 2 challenge P2.2 fix carried from 0013.
    -- =========================================================
    IF (p_pricing->>'pricing_version')      IS DISTINCT FROM v_reserve_entry.pricing_version
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
    -- 7) Validate 0 < provider_amount <= original_reserved.
    --    OVERRUN_RESERVATION: post-commit overrun must route through
    --    overrun_debt (deferred handler), NOT silent available dip.
    -- =========================================================
    IF p_provider_amount IS NULL OR p_provider_amount <= 0 THEN
        RAISE EXCEPTION
            'INVALID_AMOUNT: provider_amount must be > 0; got %',
            p_provider_amount
            USING ERRCODE = '22023';
    END IF;
    IF p_provider_amount > v_reserve_entry.amt THEN
        RAISE EXCEPTION
            'OVERRUN_RESERVATION: provider_amount % exceeds original_reserved %; \
             post-commit overrun must route through overrun_debt path (deferred)',
            p_provider_amount, v_reserve_entry.amt
            USING ERRCODE = 'P0001';
    END IF;

    -- =========================================================
    -- 8) Compute delta = provider - estimated. May be 0 / -ve / +ve.
    --    delta > 0 (provider > estimated):  debit available + credit committed
    --                                         (extra cost dipped from available)
    --    delta < 0 (provider < estimated):  debit committed + credit available
    --                                         (refund residual to available)
    --    delta == 0: no entries; transition still fires per spec.
    -- =========================================================
    v_delta := p_provider_amount - v_commit_row.estimated_amount_atomic;

    -- =========================================================
    -- 9) Resolve account_ids when delta != 0.
    -- =========================================================
    IF v_delta <> 0 THEN
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

        -- Acquire row locks in canonical order over the 2 accounts touched.
        PERFORM 1
          FROM ledger_accounts la
         WHERE la.ledger_account_id IN (v_account_committed, v_account_available)
         ORDER BY la.budget_id, la.window_instance_id, la.unit_id, la.account_kind
           FOR UPDATE OF la;

        -- Allocate sequences (2 entries: 1 debit + 1 credit).
        v_seq_a := nextval_per_shard(v_shard_id);
        v_seq_b := nextval_per_shard(v_shard_id);

        v_canonical_keys := v_reserve_entry.budget_id::TEXT
                            || ':' || v_reserve_entry.unit_id::TEXT
                            || ':available_budget,'
                            || v_reserve_entry.budget_id::TEXT
                            || ':' || v_reserve_entry.unit_id::TEXT
                            || ':committed_spend';
        v_lock_order_token := 'v1:' || encode(digest(v_canonical_keys, 'sha256'), 'hex');
    ELSE
        -- delta == 0: no entries; placeholder lock_order_token (not used).
        v_lock_order_token := 'v1:' || encode(digest('provider_report:noop', 'sha256'), 'hex');
    END IF;

    -- =========================================================
    -- 10) INSERT ledger_transactions (commit_estimated pattern).
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
            v_tx_id, v_tenant_id, 'provider_report',
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
           AND operation_kind = 'provider_report'
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
    -- 11) INSERT ledger_entries (only when delta != 0).
    -- =========================================================
    IF v_delta > 0 THEN
        -- provider > estimated: extra cost dipped from available.
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
            gen_random_uuid(), v_tx_id, v_account_available,
            v_tenant_id, v_reserve_entry.budget_id, v_reserve_entry.window_instance_id, v_reserve_entry.unit_id,
            'debit', v_delta,
            v_reserve_entry.pricing_version, v_reserve_entry.price_snapshot_hash,
            v_reserve_entry.fx_rate_version, v_reserve_entry.unit_conversion_version,
            p_reservation_id, 'provider_reported',
            v_shard_id, v_seq_a,
            v_effective_at, date_trunc('month', v_effective_at)::DATE,
            clock_timestamp(), date_trunc('month', clock_timestamp())::DATE
        ),
        (
            gen_random_uuid(), v_tx_id, v_account_committed,
            v_tenant_id, v_reserve_entry.budget_id, v_reserve_entry.window_instance_id, v_reserve_entry.unit_id,
            'credit', v_delta,
            v_reserve_entry.pricing_version, v_reserve_entry.price_snapshot_hash,
            v_reserve_entry.fx_rate_version, v_reserve_entry.unit_conversion_version,
            p_reservation_id, 'provider_reported',
            v_shard_id, v_seq_b,
            v_effective_at, date_trunc('month', v_effective_at)::DATE,
            clock_timestamp(), date_trunc('month', clock_timestamp())::DATE
        );
    ELSIF v_delta < 0 THEN
        -- provider < estimated: refund residual to available.
        -- Use ABS for amount; direction reversed.
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
            gen_random_uuid(), v_tx_id, v_account_committed,
            v_tenant_id, v_reserve_entry.budget_id, v_reserve_entry.window_instance_id, v_reserve_entry.unit_id,
            'debit', -v_delta,  -- ABS
            v_reserve_entry.pricing_version, v_reserve_entry.price_snapshot_hash,
            v_reserve_entry.fx_rate_version, v_reserve_entry.unit_conversion_version,
            p_reservation_id, 'provider_reported',
            v_shard_id, v_seq_a,
            v_effective_at, date_trunc('month', v_effective_at)::DATE,
            clock_timestamp(), date_trunc('month', clock_timestamp())::DATE
        ),
        (
            gen_random_uuid(), v_tx_id, v_account_available,
            v_tenant_id, v_reserve_entry.budget_id, v_reserve_entry.window_instance_id, v_reserve_entry.unit_id,
            'credit', -v_delta,
            v_reserve_entry.pricing_version, v_reserve_entry.price_snapshot_hash,
            v_reserve_entry.fx_rate_version, v_reserve_entry.unit_conversion_version,
            p_reservation_id, 'provider_reported',
            v_shard_id, v_seq_b,
            v_effective_at, date_trunc('month', v_effective_at)::DATE,
            clock_timestamp(), date_trunc('month', clock_timestamp())::DATE
        );
    END IF;
    -- delta == 0: no entries; per-unit balance vacuously satisfied.

    -- =========================================================
    -- 12) Per-unit balance check (vacuous if no entries).
    -- =========================================================
    PERFORM assert_per_unit_balance_now(v_tx_id);

    -- =========================================================
    -- 13) audit_outbox + audit_outbox_global_keys.
    --     event_type='spendguard.audit.decision' for transition events
    --     (Codex round 1 DD-A1; Stage 2 §10 audit.outcome reserved for
    --      final lifecycle close).
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
        'provider_report',
        v_workload_id,
        (p_audit_outbox_row->>'producer_sequence')::BIGINT,
        v_idempotency_key,
        date_trunc('month', clock_timestamp())::DATE,
        (p_audit_outbox_row->>'audit_outbox_id')::UUID
    );

    -- =========================================================
    -- 14) UPDATE commits projection: state transition + amount fields.
    --     CAS on latest_state='estimated' (defense in depth; row already
    --     locked at step 4).
    -- =========================================================
    UPDATE commits
       SET latest_state = 'provider_reported',
           provider_reported_amount_atomic = p_provider_amount,
           delta_to_reserved_atomic = p_provider_amount - v_reserve_entry.amt,
           provider_reported_at = clock_timestamp(),
           updated_at = clock_timestamp()
     WHERE commit_id = v_commit_row.commit_id
       AND latest_state = 'estimated';

    GET DIAGNOSTICS v_rowcount = ROW_COUNT;
    IF v_rowcount <> 1 THEN
        RAISE EXCEPTION
            'commit_lifecycle_race: UPDATE commits affected % rows (expected 1)',
            v_rowcount
            USING ERRCODE = 'P0001';
    END IF;

    -- reservations.current_state stays 'committed' (no UPDATE here).

    RETURN v_tx_id;
END;
$$ LANGUAGE plpgsql SECURITY DEFINER;

GRANT EXECUTE ON FUNCTION post_provider_reported_transaction(JSONB, UUID, NUMERIC, JSONB, JSONB)
    TO ledger_application_role;
