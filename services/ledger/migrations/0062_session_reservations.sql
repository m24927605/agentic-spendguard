-- D41 session reservation substrate (COV_D41S_02).
--
-- Authority model:
--   * Postgres is the sole authority for session reserve, streaming commit,
--     release, and expiry accounting.
--   * Accepted reserve writes real double-entry ledger rows:
--       available_budget debit -> reserved_hold credit.
--   * Accepted streaming commit writes real double-entry ledger rows:
--       reserved_hold debit -> committed_spend credit.
--   * Release/expiry writes real double-entry ledger rows for the uncommitted
--     remainder:
--       reserved_hold debit -> available_budget credit.
--   * Every commit/release/expiry function locks the session_reservations row
--     FOR UPDATE before mutating balances.
--   * Idempotency replays return the original JSONB outcome. Same tuple with a
--     different request hash raises SQLSTATE 40P03.

CREATE TABLE IF NOT EXISTS session_reservations (
    session_reservation_id        UUID PRIMARY KEY,
    tenant_id                     UUID NOT NULL,
    budget_id                     UUID NOT NULL,
    window_instance_id            UUID NOT NULL
        REFERENCES budget_window_instances(window_instance_id),
    unit_id                       UUID NOT NULL REFERENCES ledger_units(unit_id),
    pricing_version               TEXT NOT NULL,
    price_snapshot_hash           BYTEA NOT NULL,
    fx_rate_version               TEXT NOT NULL,
    unit_conversion_version       TEXT NOT NULL,
    session_id                    TEXT NOT NULL,
    route                         TEXT NOT NULL,
    reserved_amount_atomic        NUMERIC(38,0) NOT NULL CHECK (reserved_amount_atomic > 0),
    committed_amount_atomic       NUMERIC(38,0) NOT NULL DEFAULT 0 CHECK (committed_amount_atomic >= 0),
    released_amount_atomic        NUMERIC(38,0) NOT NULL DEFAULT 0 CHECK (released_amount_atomic >= 0),
    status                        TEXT NOT NULL CHECK (status IN ('active', 'released', 'expired', 'denied')),
    ttl_expires_at                TIMESTAMPTZ NOT NULL,
    reserve_idempotency_key       TEXT NOT NULL,
    reserve_request_hash          BYTEA NOT NULL,
    reserve_ledger_transaction_id UUID REFERENCES ledger_transactions(ledger_transaction_id),
    reserve_outcome               JSONB NOT NULL,
    created_at                    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at                    TIMESTAMPTZ NOT NULL DEFAULT now(),

    UNIQUE (tenant_id, session_id, route, reserve_idempotency_key),
    CHECK (committed_amount_atomic <= reserved_amount_atomic),
    CHECK (released_amount_atomic <= reserved_amount_atomic),
    CHECK (committed_amount_atomic + released_amount_atomic <= reserved_amount_atomic)
);

CREATE INDEX IF NOT EXISTS idx_session_reservations_active
    ON session_reservations (tenant_id, budget_id, window_instance_id, unit_id)
    WHERE status = 'active';

CREATE INDEX IF NOT EXISTS idx_session_reservations_ttl
    ON session_reservations (ttl_expires_at)
    WHERE status = 'active';

CREATE TABLE IF NOT EXISTS session_commit_deltas (
    session_reservation_id     UUID NOT NULL
        REFERENCES session_reservations(session_reservation_id),
    streaming_commit_id        TEXT NOT NULL,
    amount_atomic_delta        NUMERIC(38,0) NOT NULL CHECK (amount_atomic_delta > 0),
    request_outcome            TEXT NOT NULL,
    event_time                 TIMESTAMPTZ NOT NULL,
    idempotency_key            TEXT NOT NULL,
    request_hash               BYTEA NOT NULL,
    applied                    BOOLEAN NOT NULL,
    ledger_transaction_id      UUID REFERENCES ledger_transactions(ledger_transaction_id),
    commit_outcome             JSONB NOT NULL,
    created_at                 TIMESTAMPTZ NOT NULL DEFAULT now(),

    PRIMARY KEY (session_reservation_id, streaming_commit_id)
);

CREATE TABLE IF NOT EXISTS session_reservation_events (
    session_reservation_event_id UUID PRIMARY KEY,
    session_reservation_id       UUID NOT NULL
        REFERENCES session_reservations(session_reservation_id),
    operation_kind               TEXT NOT NULL CHECK (operation_kind IN (
                                    'reserve',
                                    'commit_delta',
                                    'release',
                                    'expire'
                                  )),
    event_type                   TEXT NOT NULL CHECK (event_type IN (
                                    'spendguard.audit.session.reserve',
                                    'spendguard.audit.session.commit_delta',
                                    'spendguard.audit.session.release',
                                    'spendguard.audit.session.expired',
                                    'spendguard.audit.session.denied'
                                  )),
    idempotency_key              TEXT NOT NULL,
    request_hash                 BYTEA NOT NULL,
    ledger_transaction_id        UUID REFERENCES ledger_transactions(ledger_transaction_id),
    amount_atomic                NUMERIC(38,0),
    event_time                   TIMESTAMPTZ NOT NULL,
    event_outcome                JSONB NOT NULL,
    created_at                   TIMESTAMPTZ NOT NULL DEFAULT now(),

    UNIQUE (session_reservation_id, operation_kind, idempotency_key)
);

CREATE OR REPLACE FUNCTION session_reservation_request_hash(p_request JSONB)
RETURNS BYTEA AS $$
    SELECT digest(convert_to(p_request::TEXT, 'UTF8'), 'sha256');
$$ LANGUAGE SQL IMMUTABLE SECURITY DEFINER;

CREATE OR REPLACE FUNCTION session_account_balance(p_ledger_account_id UUID)
RETURNS NUMERIC(38,0) AS $$
DECLARE
    v_balance NUMERIC(38,0);
BEGIN
    SELECT COALESCE(
               SUM(
                   CASE direction
                       WHEN 'credit' THEN amount_atomic
                       WHEN 'debit' THEN -amount_atomic
                   END
               ),
               0
           )
      INTO v_balance
      FROM ledger_entries
     WHERE ledger_account_id = p_ledger_account_id;

    RETURN v_balance;
END;
$$ LANGUAGE plpgsql SECURITY DEFINER;

CREATE OR REPLACE FUNCTION session_ledger_tx(
    p_tenant_id UUID,
    p_operation_kind TEXT,
    p_idempotency_key TEXT,
    p_request_hash BYTEA,
    p_minimal_replay_response JSONB,
    p_session_event_type TEXT,
    p_session_reservation_id UUID
) RETURNS UUID AS $$
DECLARE
    v_tx_id UUID := gen_random_uuid();
    v_audit_outbox_id UUID := gen_random_uuid();
    v_audit_event_id UUID := gen_random_uuid();
    v_decision_id UUID := gen_random_uuid();
    v_recorded_month DATE := date_trunc('month', clock_timestamp())::DATE;
    v_workload_id TEXT := 'session-reservation-ledger';
    v_producer_sequence BIGINT := nextval_per_shard(1::smallint);
    v_payload JSONB;
BEGIN
    v_payload := jsonb_build_object(
        'specversion', '1.0',
        'id', v_audit_event_id::TEXT,
        'type', p_session_event_type,
        'source', 'urn:spendguard:ledger:session-reservations',
        'subject', p_session_reservation_id::TEXT,
        'time', to_jsonb(clock_timestamp()),
        'datacontenttype', 'application/json',
        'signing_key_id', 'ledger-server-mint:session-reservation:v1',
        'data', p_minimal_replay_response
    );

    INSERT INTO ledger_transactions (
        ledger_transaction_id, tenant_id, operation_kind, posting_state,
        posted_at, idempotency_key, request_hash, minimal_replay_response,
        audit_decision_event_id, decision_id,
        effective_at, recorded_at, lock_order_token
    ) VALUES (
        v_tx_id, p_tenant_id, p_operation_kind, 'posted', clock_timestamp(),
        p_idempotency_key, p_request_hash, p_minimal_replay_response,
        v_audit_event_id, v_decision_id,
        clock_timestamp(), clock_timestamp(),
        'session:' || encode(digest(p_idempotency_key, 'sha256'), 'hex')
    );

    INSERT INTO audit_outbox (
        audit_outbox_id, audit_decision_event_id, decision_id,
        tenant_id, ledger_transaction_id,
        event_type, cloudevent_payload, cloudevent_payload_signature,
        ledger_fencing_epoch, workload_instance_id,
        pending_forward, forward_attempts,
        recorded_at, recorded_month,
        producer_sequence, idempotency_key
    ) VALUES (
        v_audit_outbox_id, v_audit_event_id, v_decision_id,
        p_tenant_id, v_tx_id,
        'spendguard.audit.outcome', v_payload,
        digest(convert_to(v_payload::TEXT, 'UTF8'), 'sha256'),
        1, v_workload_id,
        TRUE, 0,
        clock_timestamp(), v_recorded_month,
        v_producer_sequence, p_idempotency_key
    );

    INSERT INTO audit_outbox_global_keys (
        audit_decision_event_id, tenant_id, decision_id,
        event_type, operation_kind,
        workload_instance_id, producer_sequence,
        idempotency_key, recorded_month, audit_outbox_id
    ) VALUES (
        v_audit_event_id, p_tenant_id, v_decision_id,
        'spendguard.audit.outcome', p_operation_kind,
        v_workload_id, v_producer_sequence,
        p_idempotency_key, v_recorded_month, v_audit_outbox_id
    );

    RETURN v_tx_id;
END;
$$ LANGUAGE plpgsql SECURITY DEFINER;

CREATE OR REPLACE FUNCTION session_post_two_entries(
    p_tx_id UUID,
    p_tenant_id UUID,
    p_budget_id UUID,
    p_window_instance_id UUID,
    p_unit_id UUID,
    p_debit_account_id UUID,
    p_credit_account_id UUID,
    p_amount NUMERIC(38,0),
    p_pricing_version TEXT,
    p_price_snapshot_hash BYTEA,
    p_fx_rate_version TEXT,
    p_unit_conversion_version TEXT,
    p_session_reservation_id UUID,
    p_commit_event_kind TEXT
) RETURNS VOID AS $$
DECLARE
    v_shard_id SMALLINT := 1;
    v_effective_at TIMESTAMPTZ := clock_timestamp();
BEGIN
    IF p_amount IS NULL OR p_amount <= 0 THEN
        RETURN;
    END IF;

    INSERT INTO ledger_entries (
        ledger_entry_id, ledger_transaction_id, ledger_account_id,
        tenant_id, budget_id, window_instance_id, unit_id,
        direction, amount_atomic,
        pricing_version, price_snapshot_hash, fx_rate_version, unit_conversion_version,
        reservation_id, commit_event_kind, ledger_shard_id, ledger_sequence,
        effective_at, effective_month, recorded_at, recorded_month
    ) VALUES (
        gen_random_uuid(), p_tx_id, p_debit_account_id,
        p_tenant_id, p_budget_id, p_window_instance_id, p_unit_id,
        'debit', p_amount,
        p_pricing_version, p_price_snapshot_hash, p_fx_rate_version,
        p_unit_conversion_version, p_session_reservation_id, p_commit_event_kind,
        v_shard_id, nextval_per_shard(v_shard_id),
        v_effective_at, date_trunc('month', v_effective_at)::DATE,
        clock_timestamp(), date_trunc('month', clock_timestamp())::DATE
    ), (
        gen_random_uuid(), p_tx_id, p_credit_account_id,
        p_tenant_id, p_budget_id, p_window_instance_id, p_unit_id,
        'credit', p_amount,
        p_pricing_version, p_price_snapshot_hash, p_fx_rate_version,
        p_unit_conversion_version, p_session_reservation_id, p_commit_event_kind,
        v_shard_id, nextval_per_shard(v_shard_id),
        v_effective_at, date_trunc('month', v_effective_at)::DATE,
        clock_timestamp(), date_trunc('month', clock_timestamp())::DATE
    );

    PERFORM assert_per_unit_balance_now(p_tx_id);
END;
$$ LANGUAGE plpgsql SECURITY DEFINER;

CREATE OR REPLACE FUNCTION session_account_id(
    p_tenant_id UUID,
    p_budget_id UUID,
    p_window_instance_id UUID,
    p_unit_id UUID,
    p_account_kind TEXT
) RETURNS UUID AS $$
DECLARE
    v_account_id UUID;
BEGIN
    SELECT ledger_account_id
      INTO v_account_id
      FROM ledger_accounts
     WHERE tenant_id = p_tenant_id
       AND budget_id = p_budget_id
       AND window_instance_id = p_window_instance_id
       AND unit_id = p_unit_id
       AND account_kind = p_account_kind;

    IF NOT FOUND THEN
        RAISE EXCEPTION 'ledger_account not found for kind=% tenant=% budget=% window=% unit=%',
            p_account_kind, p_tenant_id, p_budget_id, p_window_instance_id, p_unit_id
            USING ERRCODE = '22023';
    END IF;

    RETURN v_account_id;
END;
$$ LANGUAGE plpgsql SECURITY DEFINER;

CREATE OR REPLACE FUNCTION post_session_reserve(p_request JSONB)
RETURNS JSONB AS $$
DECLARE
    v_tenant_id               UUID := (p_request->>'tenant_id')::UUID;
    v_budget_id               UUID := (p_request->>'budget_id')::UUID;
    v_window_instance_id      UUID := (p_request->>'window_instance_id')::UUID;
    v_unit_id                 UUID := (p_request->>'unit_id')::UUID;
    v_pricing_version         TEXT := p_request->>'pricing_version';
    v_price_snapshot_hash     BYTEA := decode(p_request->>'price_snapshot_hash_hex', 'hex');
    v_fx_rate_version         TEXT := p_request->>'fx_rate_version';
    v_unit_conversion_version TEXT := p_request->>'unit_conversion_version';
    v_session_id              TEXT := p_request->>'session_id';
    v_route                   TEXT := p_request->>'route';
    v_amount                  NUMERIC(38,0) := (p_request->>'estimated_amount_atomic')::NUMERIC(38,0);
    v_ttl_seconds             BIGINT := (p_request->>'ttl_seconds')::BIGINT;
    v_idempotency_key         TEXT := p_request->>'idempotency_key';
    v_request_hash            BYTEA := session_reservation_request_hash(p_request);
    v_existing                RECORD;
    v_session_reservation_id  UUID := gen_random_uuid();
    v_ttl_expires_at          TIMESTAMPTZ;
    v_available_account_id    UUID;
    v_reserved_account_id     UUID;
    v_available_balance       NUMERIC(38,0);
    v_tx_id                   UUID;
    v_tx_key                  TEXT;
    v_outcome                 JSONB;
    v_status                  TEXT;
BEGIN
    IF v_amount IS NULL OR v_amount <= 0 THEN
        RAISE EXCEPTION 'INVALID_AMOUNT: estimated_amount_atomic must be > 0'
            USING ERRCODE = '22023';
    END IF;
    IF v_ttl_seconds IS NULL OR v_ttl_seconds <= 0 THEN
        RAISE EXCEPTION 'INVALID_TTL: ttl_seconds must be > 0'
            USING ERRCODE = '22023';
    END IF;
    IF v_session_id IS NULL OR v_session_id = ''
       OR v_route IS NULL OR v_route = ''
       OR v_idempotency_key IS NULL OR v_idempotency_key = ''
       OR v_pricing_version IS NULL OR v_pricing_version = ''
       OR v_fx_rate_version IS NULL OR v_fx_rate_version = ''
       OR v_unit_conversion_version IS NULL OR v_unit_conversion_version = ''
    THEN
        RAISE EXCEPTION 'INVALID_REQUEST: required session, route, idempotency, and pricing fields must be non-empty'
            USING ERRCODE = '22023';
    END IF;

    SELECT reserve_request_hash, reserve_outcome
      INTO v_existing
      FROM session_reservations
     WHERE tenant_id = v_tenant_id
       AND session_id = v_session_id
       AND route = v_route
       AND reserve_idempotency_key = v_idempotency_key;

    IF FOUND THEN
        IF v_existing.reserve_request_hash <> v_request_hash THEN
            RAISE EXCEPTION 'idempotency_key reused with different session reserve payload'
                USING ERRCODE = '40P03';
        END IF;
        RETURN v_existing.reserve_outcome;
    END IF;

    IF NOT EXISTS (
        SELECT 1
          FROM pricing_snapshots
         WHERE pricing_version = v_pricing_version
           AND price_snapshot_hash = v_price_snapshot_hash
           AND fx_rate_version = v_fx_rate_version
           AND unit_conversion_version = v_unit_conversion_version
    ) THEN
        RAISE EXCEPTION 'PRICING_VERSION_UNKNOWN: pricing freeze not present in ledger cache'
            USING ERRCODE = 'P0001';
    END IF;

    PERFORM 1
      FROM ledger_accounts
     WHERE tenant_id = v_tenant_id
       AND budget_id = v_budget_id
       AND window_instance_id = v_window_instance_id
       AND unit_id = v_unit_id
       AND account_kind IN ('available_budget', 'reserved_hold')
     ORDER BY account_kind
       FOR UPDATE;

    SELECT reserve_request_hash, reserve_outcome
      INTO v_existing
      FROM session_reservations
     WHERE tenant_id = v_tenant_id
       AND session_id = v_session_id
       AND route = v_route
       AND reserve_idempotency_key = v_idempotency_key;

    IF FOUND THEN
        IF v_existing.reserve_request_hash <> v_request_hash THEN
            RAISE EXCEPTION 'idempotency_key reused with different session reserve payload'
                USING ERRCODE = '40P03';
        END IF;
        RETURN v_existing.reserve_outcome;
    END IF;

    v_available_account_id := session_account_id(
        v_tenant_id, v_budget_id, v_window_instance_id, v_unit_id, 'available_budget'
    );
    v_reserved_account_id := session_account_id(
        v_tenant_id, v_budget_id, v_window_instance_id, v_unit_id, 'reserved_hold'
    );
    v_available_balance := session_account_balance(v_available_account_id);
    v_ttl_expires_at := clock_timestamp() + (v_ttl_seconds * INTERVAL '1 second');

    IF v_available_balance < v_amount THEN
        v_status := 'denied';
        v_outcome := jsonb_build_object(
            'status', 'denied',
            'reason', 'INSUFFICIENT_AVAILABLE_BUDGET',
            'session_reservation_id', v_session_reservation_id::TEXT,
            'tenant_id', v_tenant_id::TEXT,
            'budget_id', v_budget_id::TEXT,
            'window_instance_id', v_window_instance_id::TEXT,
            'unit_id', v_unit_id::TEXT,
            'requested_amount_atomic', v_amount::TEXT,
            'available_amount_atomic', v_available_balance::TEXT,
            'committed_amount_atomic', '0',
            'remaining_amount_atomic', '0',
            'released_amount_atomic', '0',
            'ttl_expires_at', to_jsonb(v_ttl_expires_at)
        );
    ELSE
        v_status := 'active';
        v_outcome := jsonb_build_object(
            'status', 'accepted',
            'session_reservation_id', v_session_reservation_id::TEXT,
            'tenant_id', v_tenant_id::TEXT,
            'budget_id', v_budget_id::TEXT,
            'window_instance_id', v_window_instance_id::TEXT,
            'unit_id', v_unit_id::TEXT,
            'reserved_amount_atomic', v_amount::TEXT,
            'committed_amount_atomic', '0',
            'remaining_amount_atomic', v_amount::TEXT,
            'released_amount_atomic', '0',
            'ttl_expires_at', to_jsonb(v_ttl_expires_at)
        );
    END IF;

    INSERT INTO session_reservations (
        session_reservation_id, tenant_id, budget_id, window_instance_id,
        unit_id, pricing_version, price_snapshot_hash, fx_rate_version,
        unit_conversion_version, session_id, route, reserved_amount_atomic,
        committed_amount_atomic, released_amount_atomic, status,
        ttl_expires_at, reserve_idempotency_key, reserve_request_hash,
        reserve_outcome
    ) VALUES (
        v_session_reservation_id, v_tenant_id, v_budget_id, v_window_instance_id,
        v_unit_id, v_pricing_version, v_price_snapshot_hash, v_fx_rate_version,
        v_unit_conversion_version, v_session_id, v_route, v_amount,
        0, 0, v_status, v_ttl_expires_at, v_idempotency_key,
        v_request_hash, v_outcome
    )
    ON CONFLICT (tenant_id, session_id, route, reserve_idempotency_key) DO NOTHING;

    IF NOT FOUND THEN
        SELECT reserve_request_hash, reserve_outcome
          INTO v_existing
          FROM session_reservations
         WHERE tenant_id = v_tenant_id
           AND session_id = v_session_id
           AND route = v_route
           AND reserve_idempotency_key = v_idempotency_key;

        IF v_existing.reserve_request_hash <> v_request_hash THEN
            RAISE EXCEPTION 'idempotency_key reused with different session reserve payload'
                USING ERRCODE = '40P03';
        END IF;
        RETURN v_existing.reserve_outcome;
    END IF;

    IF v_status = 'active' THEN
        v_tx_key := 'session_reserve:' || v_session_reservation_id::TEXT;
        v_tx_id := session_ledger_tx(
            v_tenant_id, 'reserve', v_tx_key, v_request_hash, v_outcome,
            'spendguard.audit.session.reserve', v_session_reservation_id
        );
        PERFORM session_post_two_entries(
            v_tx_id, v_tenant_id, v_budget_id, v_window_instance_id, v_unit_id,
            v_available_account_id, v_reserved_account_id, v_amount,
            v_pricing_version, v_price_snapshot_hash, v_fx_rate_version,
            v_unit_conversion_version, v_session_reservation_id, 'session_reserve'
        );
        UPDATE session_reservations
           SET reserve_ledger_transaction_id = v_tx_id,
               reserve_outcome = v_outcome || jsonb_build_object('ledger_transaction_id', v_tx_id::TEXT),
               updated_at = clock_timestamp()
         WHERE session_reservation_id = v_session_reservation_id;
        v_outcome := v_outcome || jsonb_build_object('ledger_transaction_id', v_tx_id::TEXT);
    END IF;

    INSERT INTO session_reservation_events (
        session_reservation_event_id, session_reservation_id, operation_kind,
        event_type,
        idempotency_key, request_hash, ledger_transaction_id, amount_atomic,
        event_time, event_outcome
    ) VALUES (
        gen_random_uuid(), v_session_reservation_id,
        'reserve',
        CASE
            WHEN v_status = 'active' THEN 'spendguard.audit.session.reserve'
            ELSE 'spendguard.audit.session.denied'
        END,
        v_idempotency_key, v_request_hash, v_tx_id, v_amount,
        clock_timestamp(), v_outcome
    );

    RETURN v_outcome;
END;
$$ LANGUAGE plpgsql SECURITY DEFINER;

CREATE OR REPLACE FUNCTION post_session_commit_delta(p_request JSONB)
RETURNS JSONB AS $$
DECLARE
    v_session_reservation_id UUID := (p_request->>'session_reservation_id')::UUID;
    v_streaming_commit_id    TEXT := p_request->>'streaming_commit_id';
    v_delta                  NUMERIC(38,0) := (p_request->>'amount_atomic_delta')::NUMERIC(38,0);
    v_request_outcome        TEXT := COALESCE(p_request->>'outcome', 'estimated');
    v_event_time             TIMESTAMPTZ := COALESCE((p_request->>'event_time')::TIMESTAMPTZ, clock_timestamp());
    v_idempotency_key        TEXT := p_request->>'idempotency_key';
    v_request_hash           BYTEA := session_reservation_request_hash(p_request);
    v_existing               RECORD;
    v_session                RECORD;
    v_reserved_account_id    UUID;
    v_committed_account_id   UUID;
    v_new_committed          NUMERIC(38,0);
    v_remaining              NUMERIC(38,0);
    v_tx_id                  UUID;
    v_tx_key                 TEXT;
    v_outcome                JSONB;
    v_applied                BOOLEAN;
    v_event_type             TEXT;
BEGIN
    IF v_streaming_commit_id IS NULL OR v_streaming_commit_id = ''
       OR v_idempotency_key IS NULL OR v_idempotency_key = ''
    THEN
        RAISE EXCEPTION 'INVALID_REQUEST: streaming_commit_id and idempotency_key are required'
            USING ERRCODE = '22023';
    END IF;
    IF v_delta IS NULL OR v_delta <= 0 THEN
        RAISE EXCEPTION 'INVALID_AMOUNT: amount_atomic_delta must be > 0'
            USING ERRCODE = '22023';
    END IF;

    SELECT request_hash, commit_outcome
      INTO v_existing
      FROM session_commit_deltas
     WHERE session_reservation_id = v_session_reservation_id
       AND streaming_commit_id = v_streaming_commit_id;

    IF FOUND THEN
        IF v_existing.request_hash <> v_request_hash THEN
            RAISE EXCEPTION 'streaming_commit_id reused with different payload'
                USING ERRCODE = '40P03';
        END IF;
        RETURN v_existing.commit_outcome;
    END IF;

    SELECT *
      INTO v_session
      FROM session_reservations
     WHERE session_reservation_id = v_session_reservation_id
     FOR UPDATE;

    IF NOT FOUND THEN
        RAISE EXCEPTION 'SESSION_RESERVATION_NOT_FOUND: %', v_session_reservation_id
            USING ERRCODE = 'P0001';
    END IF;

    SELECT request_hash, commit_outcome
      INTO v_existing
      FROM session_commit_deltas
     WHERE session_reservation_id = v_session_reservation_id
       AND streaming_commit_id = v_streaming_commit_id;

    IF FOUND THEN
        IF v_existing.request_hash <> v_request_hash THEN
            RAISE EXCEPTION 'streaming_commit_id reused with different payload'
                USING ERRCODE = '40P03';
        END IF;
        RETURN v_existing.commit_outcome;
    END IF;

    IF p_request ? 'tenant_id'
       AND (p_request->>'tenant_id')::UUID IS DISTINCT FROM v_session.tenant_id
    THEN
        RAISE EXCEPTION 'TENANT_MISMATCH: commit tenant_id differs from session reservation'
            USING ERRCODE = 'P0001';
    END IF;
    IF p_request ? 'budget_id'
       AND (p_request->>'budget_id')::UUID IS DISTINCT FROM v_session.budget_id
    THEN
        RAISE EXCEPTION 'BUDGET_MISMATCH: commit budget_id differs from session reservation'
            USING ERRCODE = 'P0001';
    END IF;
    IF p_request ? 'window_instance_id'
       AND (p_request->>'window_instance_id')::UUID IS DISTINCT FROM v_session.window_instance_id
    THEN
        RAISE EXCEPTION 'WINDOW_MISMATCH: commit window_instance_id differs from session reservation'
            USING ERRCODE = 'P0001';
    END IF;
    IF p_request ? 'unit_id'
       AND (p_request->>'unit_id')::UUID IS DISTINCT FROM v_session.unit_id
    THEN
        RAISE EXCEPTION 'UNIT_MISMATCH: commit unit_id differs from session reservation'
            USING ERRCODE = 'P0001';
    END IF;
    IF p_request ? 'pricing_version'
       AND (p_request->>'pricing_version') IS DISTINCT FROM v_session.pricing_version
    THEN
        RAISE EXCEPTION 'PRICING_VERSION_MISMATCH: commit pricing_version differs from session reservation'
            USING ERRCODE = 'P0001';
    END IF;
    IF p_request ? 'price_snapshot_hash_hex'
       AND decode(p_request->>'price_snapshot_hash_hex', 'hex') IS DISTINCT FROM v_session.price_snapshot_hash
    THEN
        RAISE EXCEPTION 'PRICE_SNAPSHOT_HASH_MISMATCH: commit price_snapshot_hash differs from session reservation'
            USING ERRCODE = 'P0001';
    END IF;
    IF p_request ? 'fx_rate_version'
       AND (p_request->>'fx_rate_version') IS DISTINCT FROM v_session.fx_rate_version
    THEN
        RAISE EXCEPTION 'FX_RATE_VERSION_MISMATCH: commit fx_rate_version differs from session reservation'
            USING ERRCODE = 'P0001';
    END IF;
    IF p_request ? 'unit_conversion_version'
       AND (p_request->>'unit_conversion_version') IS DISTINCT FROM v_session.unit_conversion_version
    THEN
        RAISE EXCEPTION 'UNIT_CONVERSION_VERSION_MISMATCH: commit unit_conversion_version differs from session reservation'
            USING ERRCODE = 'P0001';
    END IF;

    IF v_session.status <> 'active' THEN
        v_applied := FALSE;
        v_event_type := 'spendguard.audit.session.denied';
        v_outcome := jsonb_build_object(
            'status', 'denied',
            'reason', 'SESSION_NOT_ACTIVE',
            'session_status', v_session.status,
            'session_reservation_id', v_session_reservation_id::TEXT,
            'committed_amount_atomic', v_session.committed_amount_atomic::TEXT,
            'remaining_amount_atomic', (v_session.reserved_amount_atomic - v_session.committed_amount_atomic - v_session.released_amount_atomic)::TEXT
        );
    ELSIF v_session.ttl_expires_at <= clock_timestamp() THEN
        v_applied := FALSE;
        v_event_type := 'spendguard.audit.session.denied';
        v_outcome := jsonb_build_object(
            'status', 'denied',
            'reason', 'SESSION_TTL_EXPIRED',
            'session_reservation_id', v_session_reservation_id::TEXT,
            'ttl_expires_at', to_jsonb(v_session.ttl_expires_at),
            'committed_amount_atomic', v_session.committed_amount_atomic::TEXT,
            'remaining_amount_atomic', (v_session.reserved_amount_atomic - v_session.committed_amount_atomic - v_session.released_amount_atomic)::TEXT
        );
    ELSIF v_session.committed_amount_atomic + v_delta > v_session.reserved_amount_atomic THEN
        v_applied := FALSE;
        v_event_type := 'spendguard.audit.session.denied';
        v_outcome := jsonb_build_object(
            'status', 'denied',
            'reason', 'OVERRUN_RESERVATION',
            'session_reservation_id', v_session_reservation_id::TEXT,
            'attempted_amount_atomic_delta', v_delta::TEXT,
            'committed_amount_atomic', v_session.committed_amount_atomic::TEXT,
            'remaining_amount_atomic', (v_session.reserved_amount_atomic - v_session.committed_amount_atomic)::TEXT
        );
    ELSE
        v_applied := TRUE;
        v_event_type := 'spendguard.audit.session.commit_delta';
        v_new_committed := v_session.committed_amount_atomic + v_delta;
        v_remaining := v_session.reserved_amount_atomic - v_new_committed;

        PERFORM 1
          FROM ledger_accounts
         WHERE tenant_id = v_session.tenant_id
           AND budget_id = v_session.budget_id
           AND window_instance_id = v_session.window_instance_id
           AND unit_id = v_session.unit_id
           AND account_kind IN ('reserved_hold', 'committed_spend')
         ORDER BY account_kind
           FOR UPDATE;

        v_reserved_account_id := session_account_id(
            v_session.tenant_id, v_session.budget_id, v_session.window_instance_id,
            v_session.unit_id, 'reserved_hold'
        );
        v_committed_account_id := session_account_id(
            v_session.tenant_id, v_session.budget_id, v_session.window_instance_id,
            v_session.unit_id, 'committed_spend'
        );
        v_tx_key := 'session_commit:' || v_session_reservation_id::TEXT || ':' || v_streaming_commit_id;
        v_outcome := jsonb_build_object(
            'status', 'accepted',
            'session_reservation_id', v_session_reservation_id::TEXT,
            'streaming_commit_id', v_streaming_commit_id,
            'amount_atomic_delta', v_delta::TEXT,
            'committed_amount_atomic', v_new_committed::TEXT,
            'remaining_amount_atomic', v_remaining::TEXT
        );
        v_tx_id := session_ledger_tx(
            v_session.tenant_id, 'commit_estimated', v_tx_key, v_request_hash, v_outcome,
            'spendguard.audit.session.commit_delta', v_session_reservation_id
        );
        PERFORM session_post_two_entries(
            v_tx_id, v_session.tenant_id, v_session.budget_id,
            v_session.window_instance_id, v_session.unit_id,
            v_reserved_account_id, v_committed_account_id, v_delta,
            v_session.pricing_version, v_session.price_snapshot_hash,
            v_session.fx_rate_version, v_session.unit_conversion_version,
            v_session_reservation_id, 'session_commit_delta'
        );

        UPDATE session_reservations
           SET committed_amount_atomic = v_new_committed,
               updated_at = clock_timestamp()
         WHERE session_reservation_id = v_session_reservation_id;

        v_outcome := v_outcome || jsonb_build_object('ledger_transaction_id', v_tx_id::TEXT);
    END IF;

    INSERT INTO session_commit_deltas (
        session_reservation_id, streaming_commit_id, amount_atomic_delta,
        request_outcome, event_time, idempotency_key, request_hash,
        applied, ledger_transaction_id, commit_outcome
    ) VALUES (
        v_session_reservation_id, v_streaming_commit_id, v_delta,
        v_request_outcome, v_event_time, v_idempotency_key, v_request_hash,
        v_applied, v_tx_id, v_outcome
    );

    INSERT INTO session_reservation_events (
        session_reservation_event_id, session_reservation_id, operation_kind,
        event_type,
        idempotency_key, request_hash, ledger_transaction_id, amount_atomic,
        event_time, event_outcome
    ) VALUES (
        gen_random_uuid(), v_session_reservation_id, 'commit_delta', v_event_type,
        v_streaming_commit_id, v_request_hash, v_tx_id,
        CASE WHEN v_applied THEN v_delta ELSE NULL END,
        v_event_time, v_outcome
    );

    RETURN v_outcome;
END;
$$ LANGUAGE plpgsql SECURITY DEFINER;

CREATE OR REPLACE FUNCTION post_session_release(p_request JSONB)
RETURNS JSONB AS $$
DECLARE
    v_session_reservation_id UUID := (p_request->>'session_reservation_id')::UUID;
    v_reason_code            TEXT := p_request->>'reason_code';
    v_event_time             TIMESTAMPTZ := COALESCE((p_request->>'event_time')::TIMESTAMPTZ, clock_timestamp());
    v_idempotency_key        TEXT := p_request->>'idempotency_key';
    v_request_hash           BYTEA := session_reservation_request_hash(p_request);
    v_existing               RECORD;
    v_session                RECORD;
    v_reserved_account_id    UUID;
    v_available_account_id   UUID;
    v_release_amount         NUMERIC(38,0);
    v_tx_id                  UUID;
    v_tx_key                 TEXT;
    v_outcome                JSONB;
BEGIN
    IF v_reason_code IS NULL OR v_reason_code = ''
       OR v_idempotency_key IS NULL OR v_idempotency_key = ''
    THEN
        RAISE EXCEPTION 'INVALID_REQUEST: reason_code and idempotency_key are required'
            USING ERRCODE = '22023';
    END IF;

    SELECT request_hash, event_outcome
      INTO v_existing
      FROM session_reservation_events
     WHERE session_reservation_id = v_session_reservation_id
       AND operation_kind = 'release'
       AND idempotency_key = v_idempotency_key;

    IF FOUND THEN
        IF v_existing.request_hash <> v_request_hash THEN
            RAISE EXCEPTION 'release idempotency_key reused with different payload'
                USING ERRCODE = '40P03';
        END IF;
        RETURN v_existing.event_outcome;
    END IF;

    SELECT *
      INTO v_session
      FROM session_reservations
     WHERE session_reservation_id = v_session_reservation_id
     FOR UPDATE;

    IF NOT FOUND THEN
        RAISE EXCEPTION 'SESSION_RESERVATION_NOT_FOUND: %', v_session_reservation_id
            USING ERRCODE = 'P0001';
    END IF;

    SELECT request_hash, event_outcome
      INTO v_existing
      FROM session_reservation_events
     WHERE session_reservation_id = v_session_reservation_id
       AND operation_kind = 'release'
       AND idempotency_key = v_idempotency_key;

    IF FOUND THEN
        IF v_existing.request_hash <> v_request_hash THEN
            RAISE EXCEPTION 'release idempotency_key reused with different payload'
                USING ERRCODE = '40P03';
        END IF;
        RETURN v_existing.event_outcome;
    END IF;

    IF v_session.status = 'active' THEN
        v_release_amount := v_session.reserved_amount_atomic - v_session.committed_amount_atomic;

        PERFORM 1
          FROM ledger_accounts
         WHERE tenant_id = v_session.tenant_id
           AND budget_id = v_session.budget_id
           AND window_instance_id = v_session.window_instance_id
           AND unit_id = v_session.unit_id
           AND account_kind IN ('reserved_hold', 'available_budget')
         ORDER BY account_kind
           FOR UPDATE;

        v_reserved_account_id := session_account_id(
            v_session.tenant_id, v_session.budget_id, v_session.window_instance_id,
            v_session.unit_id, 'reserved_hold'
        );
        v_available_account_id := session_account_id(
            v_session.tenant_id, v_session.budget_id, v_session.window_instance_id,
            v_session.unit_id, 'available_budget'
        );
        v_tx_key := 'session_release:' || v_session_reservation_id::TEXT || ':' || v_idempotency_key;
        v_tx_id := session_ledger_tx(
            v_session.tenant_id, 'release', v_tx_key, v_request_hash,
            jsonb_build_object('session_reservation_id', v_session_reservation_id::TEXT),
            'spendguard.audit.session.release', v_session_reservation_id
        );
        PERFORM session_post_two_entries(
            v_tx_id, v_session.tenant_id, v_session.budget_id,
            v_session.window_instance_id, v_session.unit_id,
            v_reserved_account_id, v_available_account_id, v_release_amount,
            v_session.pricing_version, v_session.price_snapshot_hash,
            v_session.fx_rate_version, v_session.unit_conversion_version,
            v_session_reservation_id, 'session_release'
        );

        UPDATE session_reservations
           SET status = 'released',
               released_amount_atomic = v_release_amount,
               updated_at = clock_timestamp()
         WHERE session_reservation_id = v_session_reservation_id;
    ELSE
        v_release_amount := 0;
    END IF;

    SELECT *
      INTO v_session
      FROM session_reservations
     WHERE session_reservation_id = v_session_reservation_id;

    v_outcome := jsonb_build_object(
        'status', 'accepted',
        'session_reservation_id', v_session_reservation_id::TEXT,
        'reason_code', v_reason_code,
        'session_status', v_session.status,
        'released_this_call_atomic', v_release_amount::TEXT,
        'released_amount_atomic', v_session.released_amount_atomic::TEXT,
        'committed_amount_atomic', v_session.committed_amount_atomic::TEXT,
        'remaining_amount_atomic', (v_session.reserved_amount_atomic - v_session.committed_amount_atomic - v_session.released_amount_atomic)::TEXT
    );
    IF v_tx_id IS NOT NULL THEN
        v_outcome := v_outcome || jsonb_build_object('ledger_transaction_id', v_tx_id::TEXT);
    END IF;

    INSERT INTO session_reservation_events (
        session_reservation_event_id, session_reservation_id, operation_kind,
        event_type,
        idempotency_key, request_hash, ledger_transaction_id, amount_atomic,
        event_time, event_outcome
    ) VALUES (
        gen_random_uuid(), v_session_reservation_id,
        'release', 'spendguard.audit.session.release', v_idempotency_key,
        v_request_hash, v_tx_id, v_release_amount, v_event_time, v_outcome
    );

    RETURN v_outcome;
END;
$$ LANGUAGE plpgsql SECURITY DEFINER;

CREATE OR REPLACE FUNCTION post_session_expire(p_request JSONB)
RETURNS JSONB AS $$
DECLARE
    v_session_reservation_id UUID := (p_request->>'session_reservation_id')::UUID;
    v_event_time             TIMESTAMPTZ := COALESCE((p_request->>'event_time')::TIMESTAMPTZ, clock_timestamp());
    v_idempotency_key        TEXT := p_request->>'idempotency_key';
    v_request_hash           BYTEA := session_reservation_request_hash(p_request);
    v_existing               RECORD;
    v_session                RECORD;
    v_reserved_account_id    UUID;
    v_available_account_id   UUID;
    v_release_amount         NUMERIC(38,0);
    v_tx_id                  UUID;
    v_tx_key                 TEXT;
    v_outcome                JSONB;
BEGIN
    IF v_idempotency_key IS NULL OR v_idempotency_key = '' THEN
        RAISE EXCEPTION 'INVALID_REQUEST: idempotency_key is required'
            USING ERRCODE = '22023';
    END IF;

    SELECT request_hash, event_outcome
      INTO v_existing
      FROM session_reservation_events
     WHERE session_reservation_id = v_session_reservation_id
       AND operation_kind = 'expire'
       AND idempotency_key = v_idempotency_key;

    IF FOUND THEN
        IF v_existing.request_hash <> v_request_hash THEN
            RAISE EXCEPTION 'expire idempotency_key reused with different payload'
                USING ERRCODE = '40P03';
        END IF;
        RETURN v_existing.event_outcome;
    END IF;

    SELECT *
      INTO v_session
      FROM session_reservations
     WHERE session_reservation_id = v_session_reservation_id
     FOR UPDATE;

    IF NOT FOUND THEN
        RAISE EXCEPTION 'SESSION_RESERVATION_NOT_FOUND: %', v_session_reservation_id
            USING ERRCODE = 'P0001';
    END IF;

    SELECT request_hash, event_outcome
      INTO v_existing
      FROM session_reservation_events
     WHERE session_reservation_id = v_session_reservation_id
       AND operation_kind = 'expire'
       AND idempotency_key = v_idempotency_key;

    IF FOUND THEN
        IF v_existing.request_hash <> v_request_hash THEN
            RAISE EXCEPTION 'expire idempotency_key reused with different payload'
                USING ERRCODE = '40P03';
        END IF;
        RETURN v_existing.event_outcome;
    END IF;

    IF v_session.status = 'active' AND v_session.ttl_expires_at > clock_timestamp() THEN
        v_outcome := jsonb_build_object(
            'status', 'denied',
            'reason', 'SESSION_TTL_NOT_EXPIRED',
            'session_reservation_id', v_session_reservation_id::TEXT,
            'ttl_expires_at', to_jsonb(v_session.ttl_expires_at)
        );
        INSERT INTO session_reservation_events (
            session_reservation_event_id, session_reservation_id,
            operation_kind, event_type, idempotency_key, request_hash,
            ledger_transaction_id, amount_atomic, event_time, event_outcome
        ) VALUES (
            gen_random_uuid(), v_session_reservation_id,
            'expire', 'spendguard.audit.session.denied',
            v_idempotency_key, v_request_hash,
            NULL, NULL, v_event_time, v_outcome
        );
        RETURN v_outcome;
    END IF;

    IF v_session.status = 'active' THEN
        v_release_amount := v_session.reserved_amount_atomic - v_session.committed_amount_atomic;

        PERFORM 1
          FROM ledger_accounts
         WHERE tenant_id = v_session.tenant_id
           AND budget_id = v_session.budget_id
           AND window_instance_id = v_session.window_instance_id
           AND unit_id = v_session.unit_id
           AND account_kind IN ('reserved_hold', 'available_budget')
         ORDER BY account_kind
           FOR UPDATE;

        v_reserved_account_id := session_account_id(
            v_session.tenant_id, v_session.budget_id, v_session.window_instance_id,
            v_session.unit_id, 'reserved_hold'
        );
        v_available_account_id := session_account_id(
            v_session.tenant_id, v_session.budget_id, v_session.window_instance_id,
            v_session.unit_id, 'available_budget'
        );
        v_tx_key := 'session_expire:' || v_session_reservation_id::TEXT || ':' || v_idempotency_key;
        v_tx_id := session_ledger_tx(
            v_session.tenant_id, 'release', v_tx_key, v_request_hash,
            jsonb_build_object('session_reservation_id', v_session_reservation_id::TEXT),
            'spendguard.audit.session.expired', v_session_reservation_id
        );
        PERFORM session_post_two_entries(
            v_tx_id, v_session.tenant_id, v_session.budget_id,
            v_session.window_instance_id, v_session.unit_id,
            v_reserved_account_id, v_available_account_id, v_release_amount,
            v_session.pricing_version, v_session.price_snapshot_hash,
            v_session.fx_rate_version, v_session.unit_conversion_version,
            v_session_reservation_id, 'session_expired'
        );

        UPDATE session_reservations
           SET status = 'expired',
               released_amount_atomic = v_release_amount,
               updated_at = clock_timestamp()
         WHERE session_reservation_id = v_session_reservation_id;
    ELSE
        v_release_amount := 0;
    END IF;

    SELECT *
      INTO v_session
      FROM session_reservations
     WHERE session_reservation_id = v_session_reservation_id;

    v_outcome := jsonb_build_object(
        'status', 'accepted',
        'session_reservation_id', v_session_reservation_id::TEXT,
        'session_status', v_session.status,
        'released_this_call_atomic', v_release_amount::TEXT,
        'released_amount_atomic', v_session.released_amount_atomic::TEXT,
        'committed_amount_atomic', v_session.committed_amount_atomic::TEXT,
        'remaining_amount_atomic', (v_session.reserved_amount_atomic - v_session.committed_amount_atomic - v_session.released_amount_atomic)::TEXT
    );
    IF v_tx_id IS NOT NULL THEN
        v_outcome := v_outcome || jsonb_build_object('ledger_transaction_id', v_tx_id::TEXT);
    END IF;

    INSERT INTO session_reservation_events (
        session_reservation_event_id, session_reservation_id, operation_kind,
        event_type,
        idempotency_key, request_hash, ledger_transaction_id, amount_atomic,
        event_time, event_outcome
    ) VALUES (
        gen_random_uuid(), v_session_reservation_id,
        'expire', 'spendguard.audit.session.expired', v_idempotency_key,
        v_request_hash, v_tx_id, v_release_amount, v_event_time, v_outcome
    );

    RETURN v_outcome;
END;
$$ LANGUAGE plpgsql SECURITY DEFINER;

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'ledger_application_role') THEN
        CREATE ROLE ledger_application_role NOINHERIT;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'ledger_reader_role') THEN
        CREATE ROLE ledger_reader_role;
    END IF;
END;
$$;

REVOKE EXECUTE ON FUNCTION post_session_reserve(JSONB) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION post_session_commit_delta(JSONB) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION post_session_release(JSONB) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION post_session_expire(JSONB) FROM PUBLIC;

GRANT EXECUTE ON FUNCTION post_session_reserve(JSONB)
    TO ledger_application_role;
GRANT EXECUTE ON FUNCTION post_session_commit_delta(JSONB)
    TO ledger_application_role;
GRANT EXECUTE ON FUNCTION post_session_release(JSONB)
    TO ledger_application_role;
GRANT EXECUTE ON FUNCTION post_session_expire(JSONB)
    TO ledger_application_role;

REVOKE EXECUTE ON FUNCTION session_reservation_request_hash(JSONB) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION session_account_balance(UUID) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION session_ledger_tx(UUID, TEXT, TEXT, BYTEA, JSONB, TEXT, UUID) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION session_post_two_entries(UUID, UUID, UUID, UUID, UUID, UUID, UUID, NUMERIC, TEXT, BYTEA, TEXT, TEXT, UUID, TEXT) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION session_account_id(UUID, UUID, UUID, UUID, TEXT) FROM PUBLIC;

REVOKE INSERT, UPDATE, DELETE ON
    session_reservations,
    session_commit_deltas,
    session_reservation_events
FROM PUBLIC;

GRANT SELECT ON
    session_reservations,
    session_commit_deltas,
    session_reservation_events
TO ledger_reader_role;
