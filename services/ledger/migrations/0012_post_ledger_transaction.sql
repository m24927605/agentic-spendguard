-- post_ledger_transaction stored procedure (v2 — addresses Codex challenge findings).
--
-- Per Ledger §6.3 (server-side derivation) + Stage 2 §4 (audit_outbox atomic
-- write) + §8.2.1.1 (lock_order_token derivation/validation).
--
-- Changes vs v1:
--   * Fencing CAS verifies tenant + active_owner + TTL + scope_type (not
--     just scope_id + epoch).
--   * Entries reference accounts via (budget_id, window_instance_id,
--     unit_id, account_kind); proc resolves account_id and ASSERTS that all
--     inputs resolve.
--   * Pricing validation iterates over ALL distinct pricing_version /
--     price_snapshot_hash tuples, not just the first.
--   * Sequence allocation uses WITH ORDINALITY + ordered materialization.
--   * Idempotent replay path uses INSERT ... ON CONFLICT and returns
--     existing tx instead of raising 23505 in races.
--   * Atomically inserts into audit_outbox_global_keys.
--   * Asserts entries count > 0 and resolution count == input count.

CREATE OR REPLACE FUNCTION post_ledger_transaction(
    p_transaction        JSONB,
    p_entries            JSONB,
    p_reservations       JSONB,    -- nullable (Release / Commit / etc. pass NULL)
    p_audit_outbox_row   JSONB,
    p_caller_lock_token  TEXT
) RETURNS UUID AS $$
DECLARE
    v_tenant_id        UUID := (p_transaction->>'tenant_id')::UUID;
    v_operation_kind   TEXT :=  p_transaction->>'operation_kind';
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
    v_derived_token    TEXT;
    v_lock_order_token TEXT;
    v_tx_id            UUID;

    v_input_count      INT;
    v_resolved_count   INT;
    v_pricing_unknown  INT;
    v_canonical_keys   TEXT;
BEGIN
    -- =========================================================
    -- 1) Idempotency authoritative replay.
    -- =========================================================
    SELECT ledger_transaction_id, request_hash
      INTO v_existing
      FROM ledger_transactions
     WHERE tenant_id      = v_tenant_id
       AND operation_kind = v_operation_kind
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
    -- 2) Strict fencing CAS.
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
    -- Reject epoch 0: brand-new scopes default to current_epoch=0, but a
    -- properly-acquired lease has CAS-incremented at least once.
    IF v_caller_epoch = 0 THEN
        RAISE EXCEPTION 'FENCING_EPOCH_STALE: epoch 0 is not a valid lease'
            USING ERRCODE = '40P02';
    END IF;
    -- scope_type must match the operation. Sidecar-driven ops use
    -- reservation/budget_window; webhook/control-plane-driven ops use
    -- control_plane_writer.
    IF v_operation_kind IN ('reserve', 'release',
                            'commit_estimated', 'overrun_debt')
       AND v_current.scope_type NOT IN ('reservation', 'budget_window') THEN
        RAISE EXCEPTION
            'fencing_scope type % not allowed for operation %',
            v_current.scope_type, v_operation_kind
            USING ERRCODE = '40P02';
    END IF;
    IF v_operation_kind IN ('provider_report', 'invoice_reconcile',
                            'refund_credit', 'dispute_adjustment',
                            'compensating', 'adjustment')
       AND v_current.scope_type <> 'control_plane_writer' THEN
        RAISE EXCEPTION
            'fencing_scope type % not allowed for operation %',
            v_current.scope_type, v_operation_kind
            USING ERRCODE = '40P02';
    END IF;

    -- =========================================================
    -- 3) Resolve and order entries deterministically.
    --    Resolve account_id via JOIN on (tenant, budget, window, kind, unit).
    --    Materialize ORDERED rows in a temp table for downstream steps.
    -- =========================================================
    v_input_count := jsonb_array_length(p_entries);
    IF v_input_count IS NULL OR v_input_count = 0 THEN
        RAISE EXCEPTION 'p_entries must be a non-empty array'
            USING ERRCODE = '22023';
    END IF;

    -- POC: this proc must be called once per Postgres transaction.
    -- DROP IF EXISTS guards against accidental nested calls in the same
    -- transaction (would otherwise fail with "relation already exists").
    DROP TABLE IF EXISTS _entries_resolved;
    CREATE TEMP TABLE _entries_resolved (
        ord                      INT,
        budget_id                UUID,
        window_instance_id       UUID,
        unit_id                  UUID,
        account_kind             TEXT,
        ledger_account_id        UUID,
        ledger_entry_id          UUID,
        direction                TEXT,
        amount_atomic            NUMERIC(38,0),
        pricing_version          TEXT,
        price_snapshot_hash      BYTEA,
        fx_rate_version          TEXT,
        unit_conversion_version  TEXT,
        reservation_id           UUID,
        commit_event_kind        TEXT,
        invoice_line_item_ref    TEXT,
        ledger_shard_id          SMALLINT,
        ledger_sequence          BIGINT
    ) ON COMMIT DROP;

    INSERT INTO _entries_resolved (
        ord, budget_id, window_instance_id, unit_id, account_kind,
        ledger_account_id, ledger_entry_id, direction, amount_atomic,
        pricing_version, price_snapshot_hash, fx_rate_version,
        unit_conversion_version, reservation_id, commit_event_kind,
        invoice_line_item_ref, ledger_shard_id
        -- ledger_sequence is NULL until step 7 fills it.
    )
    SELECT
        ord,
        (entry->>'budget_id')::UUID,
        (entry->>'window_instance_id')::UUID,
        (entry->>'unit_id')::UUID,
        entry->>'account_kind',
        la.ledger_account_id,
        (entry->>'ledger_entry_id')::UUID,
        entry->>'direction',
        (entry->>'amount_atomic')::NUMERIC(38,0),
        entry->>'pricing_version',
        decode(entry->>'price_snapshot_hash_hex', 'hex'),
        entry->>'fx_rate_version',
        entry->>'unit_conversion_version',
        (entry->>'reservation_id')::UUID,
        entry->>'commit_event_kind',
        entry->>'invoice_line_item_ref',
        (entry->>'ledger_shard_id')::SMALLINT
    FROM jsonb_array_elements(p_entries) WITH ORDINALITY AS t(entry, ord)
    LEFT JOIN ledger_accounts la
      ON la.tenant_id           = v_tenant_id
     AND la.budget_id           = (entry->>'budget_id')::UUID
     AND la.window_instance_id  = (entry->>'window_instance_id')::UUID
     AND la.unit_id             = (entry->>'unit_id')::UUID
     AND la.account_kind        = entry->>'account_kind'
    ORDER BY ord;

    -- Assert all entries resolved.
    SELECT COUNT(*) INTO v_resolved_count
      FROM _entries_resolved
     WHERE ledger_account_id IS NOT NULL;

    IF v_resolved_count <> v_input_count THEN
        RAISE EXCEPTION
            'ledger_accounts resolution failed: input=%, resolved=%',
            v_input_count, v_resolved_count
            USING ERRCODE = '22023';
    END IF;

    -- =========================================================
    -- 4) Validate ALL pricing_version / price_snapshot_hash tuples.
    -- =========================================================
    SELECT COUNT(*) INTO v_pricing_unknown
      FROM (
          SELECT DISTINCT pricing_version, price_snapshot_hash
            FROM _entries_resolved
      ) e
      LEFT JOIN pricing_snapshots ps
        ON ps.pricing_version = e.pricing_version
       AND ps.price_snapshot_hash = e.price_snapshot_hash
     WHERE ps.pricing_version IS NULL;

    IF v_pricing_unknown > 0 THEN
        RAISE EXCEPTION 'PRICING_VERSION_UNKNOWN: % unknown tuple(s)',
            v_pricing_unknown
            USING ERRCODE = 'P0001';
    END IF;

    -- =========================================================
    -- 5) Derive lock_order_token from canonical key set.
    -- =========================================================
    SELECT string_agg(
               (budget_id::TEXT || ':' || unit_id::TEXT || ':' || account_kind),
               ',' ORDER BY budget_id, unit_id, account_kind)
      INTO v_canonical_keys
      FROM (
          SELECT DISTINCT budget_id, unit_id, account_kind
            FROM _entries_resolved
      ) k;

    v_derived_token := 'v1:' || encode(digest(v_canonical_keys, 'sha256'), 'hex');

    IF p_caller_lock_token IS NOT NULL
       AND p_caller_lock_token <> v_derived_token THEN
        RAISE EXCEPTION
            'LOCK_ORDER_TOKEN_MISMATCH: caller=%, derived=%',
            p_caller_lock_token, v_derived_token
            USING ERRCODE = '40P03';
    END IF;
    v_lock_order_token := v_derived_token;

    -- =========================================================
    -- 6) Acquire row locks in canonical order.
    --    Lock the EXACT account rows we will write to, including
    --    window_instance_id (different windows = different rows).
    -- =========================================================
    PERFORM 1
      FROM ledger_accounts la
      JOIN (
          SELECT DISTINCT budget_id, window_instance_id, unit_id, account_kind
            FROM _entries_resolved
      ) e
        ON la.tenant_id          = v_tenant_id
       AND la.budget_id          = e.budget_id
       AND la.window_instance_id = e.window_instance_id
       AND la.unit_id            = e.unit_id
       AND la.account_kind       = e.account_kind
     ORDER BY la.budget_id, la.window_instance_id, la.unit_id, la.account_kind
       FOR UPDATE OF la;

    -- =========================================================
    -- 7) Allocate sequences in deterministic ORDINAL order.
    --    Procedural FOR loop guarantees one nextval_per_shard call per
    --    row and a deterministic invocation order matching `ord`.
    --    (ledger_sequence column was created up-front in step 3 so we
    --    avoid an ALTER TABLE on the temp table mid-procedure.)
    -- =========================================================
    DECLARE
        r RECORD;
    BEGIN
        FOR r IN
            SELECT ord, ledger_shard_id
              FROM _entries_resolved
             ORDER BY ord
        LOOP
            UPDATE _entries_resolved
               SET ledger_sequence = nextval_per_shard(r.ledger_shard_id)
             WHERE ord = r.ord;
        END LOOP;
    END;

    -- =========================================================
    -- 8) INSERT ledger_transactions ON CONFLICT (idempotency race).
    -- =========================================================
    v_tx_id := COALESCE(v_caller_tx_id, gen_random_uuid());

    WITH ins AS (
        INSERT INTO ledger_transactions (
            ledger_transaction_id, tenant_id, operation_kind,
            posting_state, posted_at,
            idempotency_key, request_hash, minimal_replay_response,
            trace_event_id, audit_decision_event_id, decision_id,
            effective_at, recorded_at,
            lock_order_token, fencing_scope_id, fencing_epoch_at_post,
            provider_dispute_id, case_state, resolved_at
        ) VALUES (
            v_tx_id, v_tenant_id, v_operation_kind,
            'posted', clock_timestamp(),
            v_idempotency_key, v_request_hash,
            COALESCE(p_transaction->'minimal_replay_response', '{}'::JSONB),
            (p_transaction->>'trace_event_id')::UUID,
            v_audit_event_id, v_decision_id,
            v_effective_at, clock_timestamp(),
            v_lock_order_token, v_fencing_scope_id, v_caller_epoch,
            p_transaction->>'provider_dispute_id',
            p_transaction->>'case_state',
            (p_transaction->>'resolved_at')::TIMESTAMPTZ
        )
        ON CONFLICT (tenant_id, operation_kind, idempotency_key) DO NOTHING
        RETURNING ledger_transaction_id, request_hash
    )
    SELECT ledger_transaction_id, request_hash
      INTO v_existing
      FROM ins;

    IF NOT FOUND THEN
        -- Concurrent insert won the race; fetch existing.
        SELECT ledger_transaction_id, request_hash
          INTO v_existing
          FROM ledger_transactions
         WHERE tenant_id = v_tenant_id
           AND operation_kind = v_operation_kind
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
    -- 9) INSERT ledger_entries from materialized table.
    -- =========================================================
    INSERT INTO ledger_entries (
        ledger_entry_id, ledger_transaction_id, ledger_account_id,
        tenant_id, budget_id, window_instance_id, unit_id,
        direction, amount_atomic,
        pricing_version, price_snapshot_hash, fx_rate_version, unit_conversion_version,
        reservation_id, commit_event_kind, invoice_line_item_ref,
        ledger_shard_id, ledger_sequence,
        effective_at, effective_month, recorded_at, recorded_month
    )
    SELECT
        er.ledger_entry_id, v_tx_id, er.ledger_account_id,
        v_tenant_id, er.budget_id, er.window_instance_id, er.unit_id,
        er.direction, er.amount_atomic,
        er.pricing_version, er.price_snapshot_hash, er.fx_rate_version, er.unit_conversion_version,
        er.reservation_id, er.commit_event_kind, er.invoice_line_item_ref,
        er.ledger_shard_id, er.ledger_sequence,
        v_effective_at,
        date_trunc('month', v_effective_at)::DATE,
        clock_timestamp(),
        date_trunc('month', clock_timestamp())::DATE
    FROM _entries_resolved er
    ORDER BY er.ord;

    -- =========================================================
    -- 10) Per-unit balance check (statement-level; constraint trigger backstop).
    -- =========================================================
    PERFORM assert_per_unit_balance_now(v_tx_id);

    -- =========================================================
    -- 11) audit_outbox + audit_outbox_global_keys atomic insert.
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
        v_operation_kind,
        v_workload_id,
        (p_audit_outbox_row->>'producer_sequence')::BIGINT,
        v_idempotency_key,
        date_trunc('month', clock_timestamp())::DATE,
        (p_audit_outbox_row->>'audit_outbox_id')::UUID
    );

    -- =========================================================
    -- 12) Persist reservations projection (when caller provides them).
    --     Idempotent via reservation_id PK (handler derives it deterministically
    --     from (decision_id, claim ordinal); retries DO NOTHING on the PK).
    -- =========================================================
    IF p_reservations IS NOT NULL
       AND jsonb_typeof(p_reservations) = 'array'
       AND jsonb_array_length(p_reservations) > 0 THEN

        INSERT INTO reservations (
            reservation_id, tenant_id, budget_id, window_instance_id,
            current_state,
            trace_run_id, trace_step_id, trace_llm_call_id,
            source_ledger_transaction_id,
            ttl_expires_at, idempotency_key
        )
        SELECT
            (r->>'reservation_id')::UUID,
            v_tenant_id,
            (r->>'budget_id')::UUID,
            (r->>'window_instance_id')::UUID,
            'reserved',
            (r->>'trace_run_id')::UUID,
            (r->>'trace_step_id')::UUID,
            (r->>'trace_llm_call_id')::UUID,
            v_tx_id,
            (r->>'ttl_expires_at')::TIMESTAMPTZ,
            r->>'idempotency_key'
        FROM jsonb_array_elements(p_reservations) AS r
        -- Use reservation_id PK as the idempotency target. Handler derives
        -- reservation_id deterministically from (decision_id, claim ordinal),
        -- so an idempotent retry of the same ReserveSet generates the same
        -- reservation_ids and DO NOTHING gracefully skips them.
        ON CONFLICT (reservation_id) DO NOTHING;
    END IF;

    RETURN v_tx_id;
END;
$$ LANGUAGE plpgsql SECURITY DEFINER;

-- =========================================================
-- Helpers (unchanged from v1).
-- =========================================================
CREATE OR REPLACE FUNCTION nextval_per_shard(p_shard_id SMALLINT)
RETURNS BIGINT AS $$
DECLARE v_next BIGINT;
BEGIN
    UPDATE ledger_sequence_allocators
       SET last_sequence = last_sequence + 1
     WHERE ledger_shard_id = p_shard_id
    RETURNING last_sequence INTO v_next;

    IF NOT FOUND THEN
        RAISE EXCEPTION 'ledger_shard_id % has no sequence allocator', p_shard_id;
    END IF;
    RETURN v_next;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION assert_per_unit_balance_now(p_tx_id UUID)
RETURNS VOID AS $$
DECLARE v_imbalanced TEXT;
BEGIN
    SELECT string_agg(unit_id::TEXT || ':' || diff::TEXT, ', ')
      INTO v_imbalanced
      FROM (
          SELECT unit_id,
                 SUM(CASE WHEN direction = 'debit'  THEN amount_atomic
                          WHEN direction = 'credit' THEN -amount_atomic END) AS diff
            FROM ledger_entries
           WHERE ledger_transaction_id = p_tx_id
           GROUP BY unit_id
      ) per_unit
     WHERE diff <> 0;

    IF v_imbalanced IS NOT NULL THEN
        RAISE EXCEPTION 'per-unit balance violation (statement-level): %', v_imbalanced
            USING ERRCODE = '23514';
    END IF;
END;
$$ LANGUAGE plpgsql;

-- =========================================================
-- Roles + grants.
-- =========================================================
CREATE ROLE ledger_application_role NOINHERIT;

GRANT EXECUTE ON FUNCTION post_ledger_transaction(JSONB, JSONB, JSONB, JSONB, TEXT)
    TO ledger_application_role;
GRANT EXECUTE ON FUNCTION nextval_per_shard(SMALLINT)
    TO ledger_application_role;

REVOKE INSERT, UPDATE, DELETE ON ledger_entries FROM PUBLIC;
REVOKE INSERT, UPDATE, DELETE ON ledger_transactions FROM PUBLIC;
REVOKE INSERT, UPDATE, DELETE ON audit_outbox FROM PUBLIC;
REVOKE INSERT, UPDATE, DELETE ON audit_outbox_global_keys FROM PUBLIC;

CREATE ROLE ledger_reader_role;
GRANT SELECT ON
    ledger_units, ledger_shards, ledger_sequence_allocators,
    budget_window_instances, ledger_accounts, pricing_snapshots,
    ledger_transactions, ledger_entries, fencing_scopes,
    fencing_scope_events, audit_outbox, audit_outbox_global_keys,
    spending_window_projections, reservations, commits
TO ledger_reader_role;
