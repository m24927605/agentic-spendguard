-- =====================================================================
-- 0032: Relax invoice_reconcile decision-signature emptiness check
--       (Codex round-8 P1)
-- =====================================================================
--
-- Round 7 (migration 0031) relaxed the legacy literal pin on
-- `signing_key_id`. Codex round 8 caught the symmetric leftover:
-- line 414 still rejects any non-empty decision signature with
--
--   'AUDIT_INVARIANT_VIOLATED: decision signature must be empty
--    (server-minted POC)'
--
-- That was correct in the pre-S6 stub world where the ledger minted
-- the decision row internally with no real signature. After S6,
-- invoice_reconcile.rs passes
--
--   hex::encode(&decision_signature.bytes)
--
-- where decision_signature is produced by the configured Signer
-- (LocalEd25519 / KMS / Disabled). Local Ed25519 / KMS signatures
-- are non-empty, so every InvoiceReconcile call with a real signer
-- fails before the audit rows are written. Only demo Disabled mode
-- (which produces zero-length signatures) currently passes.
--
-- Fix: drop the empty-only constraint. The hex-decode itself already
-- validates well-formed hex (line 408 above). Actual signature
-- verification is the canonical_ingest verifier's job (S8) — the
-- ledger SP is only doing payload-shape checks.
--
-- This migration CREATE OR REPLACEs the SP. Body is copied verbatim
-- from 0031 (the round-7 SP) except for the relaxed signature
-- emptiness check.

-- copied verbatim from 0016 except for the relaxed check on what
-- was line 546 ('signing_key_id' literal) and a new non-empty
-- assertion that runs alongside the other literal checks.

CREATE OR REPLACE FUNCTION post_invoice_reconciled_transaction(
    p_transaction               JSONB,    -- ledger_transaction shape
    p_reservation_id            UUID,
    p_invoice_amount            NUMERIC(38,0),
    p_pricing                   JSONB,    -- 4 freeze fields
    p_audit_decision_outbox_row JSONB,    -- handler-synthesized
    p_audit_outcome_outbox_row  JSONB,    -- caller-supplied (signed outcome)
    p_outcome_producer_seq      BIGINT    -- outcome seq; decision = this - 1
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
    v_baseline         NUMERIC(38,0);
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

    -- v7 design: SP-owned timestamp + month
    v_now              TIMESTAMPTZ;
    v_recorded_month   DATE;

    -- v7 Δ11 dual-row audit
    v_dec_pl                    JSONB;
    v_out_pl                    JSONB;
    v_outcome_event_id          UUID;
    v_derived_decision_event_id UUID;
    v_decision_audit_outbox_id  UUID;
    v_outcome_audit_outbox_id   UUID;
    v_decision_payload_sig      BYTEA;
    v_outcome_payload_sig       BYTEA;
    v_dec_data_bytes            BYTEA;
    v_out_data_bytes            BYTEA;
BEGIN
    -- =========================================================
    -- 1) Idempotency authoritative replay (mirror 0014).
    -- =========================================================
    SELECT ledger_transaction_id, request_hash
      INTO v_existing
      FROM ledger_transactions
     WHERE tenant_id      = v_tenant_id
       AND operation_kind = 'invoice_reconcile'
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
    -- 2) Fencing CAS (control_plane_writer scope).
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
            'fencing_scope type % not allowed for operation invoice_reconcile',
            v_current.scope_type
            USING ERRCODE = '40P02';
    END IF;

    -- =========================================================
    -- 2b) Idempotency re-check AFTER fencing CAS.
    -- =========================================================
    SELECT ledger_transaction_id, request_hash
      INTO v_existing
      FROM ledger_transactions
     WHERE tenant_id      = v_tenant_id
       AND operation_kind = 'invoice_reconcile'
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
    -- 2c) SP-owned timestamps (v7 Δ12).
    -- =========================================================
    v_now            := clock_timestamp();
    v_recorded_month := date_trunc('month', v_now)::DATE;

    -- =========================================================
    -- 3) LOCK reservations row; assert tenant + state='committed'.
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
    -- 4) LOCK commits row; CAS on latest_state IN ('estimated','provider_reported').
    -- =========================================================
    SELECT commit_id, latest_state, estimated_amount_atomic,
           provider_reported_amount_atomic,
           pricing_version, price_snapshot_hash, unit_id, budget_id
      INTO v_commit_row
      FROM commits
     WHERE tenant_id = v_tenant_id
       AND reservation_id = p_reservation_id
       FOR UPDATE;

    IF NOT FOUND THEN
        RAISE EXCEPTION
            'RESERVATION_STATE_CONFLICT: invoice_reconcile requires prior commit_estimated for reservation %',
            p_reservation_id
            USING ERRCODE = 'P0001';
    END IF;
    IF v_commit_row.latest_state NOT IN ('estimated', 'provider_reported') THEN
        RAISE EXCEPTION
            'RESERVATION_STATE_CONFLICT: commits.latest_state=%, expected estimated or provider_reported',
            v_commit_row.latest_state
            USING ERRCODE = 'P0001';
    END IF;

    -- =========================================================
    -- 5) Lookup reserve credit (mirror 0014:223-238).
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
    -- 5b) Unit defense-in-depth (mirror 0014:248-259).
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
    -- 6) Validate pricing 4 fields (mirror 0014:262-275).
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
    -- 7) Amount validation (POC: invoice <= original_reserved).
    -- =========================================================
    IF p_invoice_amount IS NULL OR p_invoice_amount <= 0 THEN
        RAISE EXCEPTION
            'INVALID_AMOUNT: invoice_amount must be > 0; got %',
            p_invoice_amount
            USING ERRCODE = '22023';
    END IF;
    IF p_invoice_amount > v_reserve_entry.amt THEN
        RAISE EXCEPTION
            'OVERRUN_RESERVATION: invoice_amount % exceeds original_reserved %; \
             post-commit overrun must route through overrun_debt path (deferred)',
            p_invoice_amount, v_reserve_entry.amt
            USING ERRCODE = 'P0001';
    END IF;

    -- =========================================================
    -- 8) Producer sequence validation (v7 Δ2).
    --    Caller pre-allocates 2 contiguous seqs (N, N+1) and passes N+1
    --    as p_outcome_producer_seq. SP back-derives decision seq = N.
    -- =========================================================
    IF p_outcome_producer_seq IS NULL OR p_outcome_producer_seq < 2 THEN
        RAISE EXCEPTION
            'INVALID_PRODUCER_SEQUENCE: outcome seq must be >= 2 to back-derive decision seq; got %',
            p_outcome_producer_seq
            USING ERRCODE = 'P0001';
    END IF;

    -- =========================================================
    -- 9) Δ11.0 / Δ11.1 typed-presence (v7 Δ14: IS DISTINCT FROM per field).
    -- =========================================================
    v_dec_pl := p_audit_decision_outbox_row->'cloudevent_payload';
    v_out_pl := p_audit_outcome_outbox_row ->'cloudevent_payload';

    IF jsonb_typeof(v_dec_pl) IS DISTINCT FROM 'object' THEN
        RAISE EXCEPTION 'AUDIT_INVARIANT_VIOLATED: decision payload not object'
          USING ERRCODE = 'P0001';
    END IF;
    IF jsonb_typeof(v_out_pl) IS DISTINCT FROM 'object' THEN
        RAISE EXCEPTION 'AUDIT_INVARIANT_VIOLATED: outcome payload not object'
          USING ERRCODE = 'P0001';
    END IF;

    -- Outcome payload typed presence (NULL-safe via IS DISTINCT FROM)
    IF       jsonb_typeof(v_out_pl->'specversion')      IS DISTINCT FROM 'string'
        OR   jsonb_typeof(v_out_pl->'type')             IS DISTINCT FROM 'string'
        OR   jsonb_typeof(v_out_pl->'source')           IS DISTINCT FROM 'string'
        OR   jsonb_typeof(v_out_pl->'id')               IS DISTINCT FROM 'string'
        OR   jsonb_typeof(v_out_pl->'time_seconds')     IS DISTINCT FROM 'number'
        OR   jsonb_typeof(v_out_pl->'time_nanos')       IS DISTINCT FROM 'number'
        OR   jsonb_typeof(v_out_pl->'datacontenttype')  IS DISTINCT FROM 'string'
        OR   jsonb_typeof(v_out_pl->'data_b64')         IS DISTINCT FROM 'string'
        OR   jsonb_typeof(v_out_pl->'tenantid')         IS DISTINCT FROM 'string'
        OR   jsonb_typeof(v_out_pl->'runid')            IS DISTINCT FROM 'string'
        OR   jsonb_typeof(v_out_pl->'decisionid')       IS DISTINCT FROM 'string'
        OR   jsonb_typeof(v_out_pl->'schema_bundle_id') IS DISTINCT FROM 'string'
        OR   jsonb_typeof(v_out_pl->'producer_id')      IS DISTINCT FROM 'string'
        OR   jsonb_typeof(v_out_pl->'producer_sequence') IS DISTINCT FROM 'number'
        OR   jsonb_typeof(v_out_pl->'signing_key_id')   IS DISTINCT FROM 'string'
    THEN
        RAISE EXCEPTION 'AUDIT_INVARIANT_VIOLATED: outcome payload field type/presence mismatch'
          USING ERRCODE = 'P0001';
    END IF;

    -- Decision payload typed presence
    IF       jsonb_typeof(v_dec_pl->'specversion')      IS DISTINCT FROM 'string'
        OR   jsonb_typeof(v_dec_pl->'type')             IS DISTINCT FROM 'string'
        OR   jsonb_typeof(v_dec_pl->'source')           IS DISTINCT FROM 'string'
        OR   jsonb_typeof(v_dec_pl->'id')               IS DISTINCT FROM 'string'
        OR   jsonb_typeof(v_dec_pl->'time_seconds')     IS DISTINCT FROM 'number'
        OR   jsonb_typeof(v_dec_pl->'time_nanos')       IS DISTINCT FROM 'number'
        OR   jsonb_typeof(v_dec_pl->'datacontenttype')  IS DISTINCT FROM 'string'
        OR   jsonb_typeof(v_dec_pl->'data_b64')         IS DISTINCT FROM 'string'
        OR   jsonb_typeof(v_dec_pl->'tenantid')         IS DISTINCT FROM 'string'
        OR   jsonb_typeof(v_dec_pl->'runid')            IS DISTINCT FROM 'string'
        OR   jsonb_typeof(v_dec_pl->'decisionid')       IS DISTINCT FROM 'string'
        OR   jsonb_typeof(v_dec_pl->'schema_bundle_id') IS DISTINCT FROM 'string'
        OR   jsonb_typeof(v_dec_pl->'producer_id')      IS DISTINCT FROM 'string'
        OR   jsonb_typeof(v_dec_pl->'producer_sequence') IS DISTINCT FROM 'number'
        OR   jsonb_typeof(v_dec_pl->'signing_key_id')   IS DISTINCT FROM 'string'
    THEN
        RAISE EXCEPTION 'AUDIT_INVARIANT_VIOLATED: decision payload field type/presence mismatch'
          USING ERRCODE = 'P0001';
    END IF;

    -- audit_outbox top-level typed presence
    IF jsonb_typeof(p_audit_outcome_outbox_row->'audit_decision_event_id') IS DISTINCT FROM 'string'
       OR jsonb_typeof(p_audit_outcome_outbox_row->'audit_outbox_id')      IS DISTINCT FROM 'string'
       OR jsonb_typeof(p_audit_outcome_outbox_row->'event_type')            IS DISTINCT FROM 'string'
       OR jsonb_typeof(p_audit_outcome_outbox_row->'cloudevent_payload_signature_hex') IS DISTINCT FROM 'string'
    THEN
        RAISE EXCEPTION 'AUDIT_INVARIANT_VIOLATED: outcome row top-level field type/presence mismatch'
          USING ERRCODE = 'P0001';
    END IF;

    IF jsonb_typeof(p_audit_decision_outbox_row->'audit_decision_event_id') IS DISTINCT FROM 'string'
       OR jsonb_typeof(p_audit_decision_outbox_row->'audit_outbox_id')      IS DISTINCT FROM 'string'
       OR jsonb_typeof(p_audit_decision_outbox_row->'event_type')            IS DISTINCT FROM 'string'
       OR jsonb_typeof(p_audit_decision_outbox_row->'cloudevent_payload_signature_hex') IS DISTINCT FROM 'string'
    THEN
        RAISE EXCEPTION 'AUDIT_INVARIANT_VIOLATED: decision row top-level field type/presence mismatch'
          USING ERRCODE = 'P0001';
    END IF;

    -- =========================================================
    -- 10) Δ15 wrapped casts (v7 Δ15.1 / Δ15.2).
    -- =========================================================
    BEGIN
        v_outcome_event_id := (p_audit_outcome_outbox_row->>'audit_decision_event_id')::UUID;
    EXCEPTION WHEN OTHERS THEN
        RAISE EXCEPTION 'AUDIT_INVARIANT_VIOLATED: outcome audit_decision_event_id not a UUID'
          USING ERRCODE = 'P0001';
    END;

    BEGIN
        v_decision_audit_outbox_id := (p_audit_decision_outbox_row->>'audit_outbox_id')::UUID;
    EXCEPTION WHEN OTHERS THEN
        RAISE EXCEPTION 'AUDIT_INVARIANT_VIOLATED: decision audit_outbox_id not a UUID'
          USING ERRCODE = 'P0001';
    END;

    BEGIN
        v_outcome_audit_outbox_id := (p_audit_outcome_outbox_row->>'audit_outbox_id')::UUID;
    EXCEPTION WHEN OTHERS THEN
        RAISE EXCEPTION 'AUDIT_INVARIANT_VIOLATED: outcome audit_outbox_id not a UUID'
          USING ERRCODE = 'P0001';
    END;

    IF v_decision_audit_outbox_id = v_outcome_audit_outbox_id THEN
        RAISE EXCEPTION 'AUDIT_INVARIANT_VIOLATED: decision and outcome share audit_outbox_id'
          USING ERRCODE = 'P0001';
    END IF;

    BEGIN
        v_decision_payload_sig := decode(p_audit_decision_outbox_row->>'cloudevent_payload_signature_hex', 'hex');
    EXCEPTION WHEN OTHERS THEN
        RAISE EXCEPTION 'AUDIT_INVARIANT_VIOLATED: decision signature_hex not hex-decodable'
          USING ERRCODE = 'P0001';
    END;
    -- Codex round-8 P1: decision row signature is no longer required
    -- to be empty. Pre-S6 the ledger minted decision rows internally
    -- with no signature; post-S6 invoice_reconcile.rs passes the
    -- producer signer's actual signature (Ed25519 / KMS / Disabled).
    -- The hex-decode above already validates well-formed hex.
    -- Actual signature trust is enforced by the canonical_ingest
    -- verifier (S8), not by this SP.
    IF v_decision_payload_sig IS NULL THEN
        v_decision_payload_sig := '\x'::BYTEA;
    END IF;

    BEGIN
        v_outcome_payload_sig := decode(p_audit_outcome_outbox_row->>'cloudevent_payload_signature_hex', 'hex');
    EXCEPTION WHEN OTHERS THEN
        RAISE EXCEPTION 'AUDIT_INVARIANT_VIOLATED: outcome signature_hex not hex-decodable'
          USING ERRCODE = 'P0001';
    END;
    IF v_outcome_payload_sig IS NULL THEN
        v_outcome_payload_sig := '\x'::BYTEA;
    END IF;

    -- =========================================================
    -- 11) Δ11.3 outcome literal + identity assertions.
    -- =========================================================
    IF       (v_out_pl->>'specversion')                IS DISTINCT FROM '1.0'
       OR    (v_out_pl->>'type')                       IS DISTINCT FROM 'spendguard.audit.outcome'
       OR    (v_out_pl->>'datacontenttype')            IS DISTINCT FROM 'application/json'
       OR    (p_audit_outcome_outbox_row->>'event_type') IS DISTINCT FROM 'spendguard.audit.outcome'
    THEN
        RAISE EXCEPTION 'AUDIT_INVARIANT_VIOLATED: outcome payload literal mismatch'
          USING ERRCODE = 'P0001';
    END IF;

    BEGIN
        IF      (v_out_pl->>'id')::UUID                  IS DISTINCT FROM v_outcome_event_id
           OR   (v_out_pl->>'tenantid')::UUID            IS DISTINCT FROM v_tenant_id
           OR   (v_out_pl->>'decisionid')::UUID          IS DISTINCT FROM v_decision_id
           OR   (v_out_pl->>'producer_sequence')::BIGINT IS DISTINCT FROM p_outcome_producer_seq
        THEN
            RAISE EXCEPTION 'AUDIT_INVARIANT_VIOLATED: outcome payload identity mismatch'
              USING ERRCODE = 'P0001';
        END IF;
    EXCEPTION WHEN invalid_text_representation OR numeric_value_out_of_range THEN
        RAISE EXCEPTION 'AUDIT_INVARIANT_VIOLATED: outcome payload cast failed'
          USING ERRCODE = 'P0001';
    END;

    -- =========================================================
    -- 12) Δ11.4 outcome data_b64 non-empty.
    -- =========================================================
    BEGIN
        v_out_data_bytes := decode(v_out_pl->>'data_b64', 'base64');
    EXCEPTION WHEN OTHERS THEN
        RAISE EXCEPTION 'AUDIT_INVARIANT_VIOLATED: outcome data_b64 not base64-decodable'
          USING ERRCODE = 'P0001';
    END;
    IF v_out_data_bytes IS NULL OR length(v_out_data_bytes) = 0 THEN
        RAISE EXCEPTION 'AUDIT_INVARIANT_VIOLATED: outcome data_b64 must be non-empty'
          USING ERRCODE = 'P0001';
    END IF;

    -- =========================================================
    -- 13) Δ11.5 ledger_tx anchor → outcome event_id.
    -- =========================================================
    BEGIN
        IF (p_transaction->>'audit_decision_event_id')::UUID IS DISTINCT FROM v_outcome_event_id THEN
            RAISE EXCEPTION 'AUDIT_INVARIANT_VIOLATED: ledger_tx anchor != outcome event id'
              USING ERRCODE = 'P0001';
        END IF;
    EXCEPTION WHEN invalid_text_representation THEN
        RAISE EXCEPTION 'AUDIT_INVARIANT_VIOLATED: p_transaction.audit_decision_event_id not a UUID'
          USING ERRCODE = 'P0001';
    END;

    -- =========================================================
    -- 14) Δ11.6 decision row identity (derivable from outcome).
    -- =========================================================
    v_derived_decision_event_id := encode(
        substring(digest(v_outcome_event_id::TEXT || ':decision', 'sha256') FROM 1 FOR 16),
        'hex')::UUID;

    BEGIN
        IF (p_audit_decision_outbox_row->>'audit_decision_event_id')::UUID
              IS DISTINCT FROM v_derived_decision_event_id
           OR (v_dec_pl->>'id')::UUID                    IS DISTINCT FROM v_derived_decision_event_id
           OR (v_dec_pl->>'tenantid')::UUID              IS DISTINCT FROM v_tenant_id
           OR (v_dec_pl->>'decisionid')::UUID            IS DISTINCT FROM v_decision_id
           OR (v_dec_pl->>'producer_sequence')::BIGINT   IS DISTINCT FROM (p_outcome_producer_seq - 1)
        THEN
            RAISE EXCEPTION 'AUDIT_INVARIANT_VIOLATED: decision payload identity mismatch'
              USING ERRCODE = 'P0001';
        END IF;
    EXCEPTION WHEN invalid_text_representation OR numeric_value_out_of_range THEN
        RAISE EXCEPTION 'AUDIT_INVARIANT_VIOLATED: decision payload cast failed'
          USING ERRCODE = 'P0001';
    END;

    -- Codex round-7 P1: removed legacy literal pin on signing_key_id.
    -- Lines above (decision payload typed-presence block) already
    -- enforce signing_key_id is a non-empty STRING. S6 producer
    -- signers may use any key_id (e.g. local Ed25519, KMS ARN, demo
    -- disabled-mode marker); the canonical_ingest verifier (S8) is
    -- what validates the key against the trust store, not this SP.
    -- Pinning to 'ledger-server-mint:v1' here was a pre-S6 invariant
    -- and broke every post-S6 InvoiceReconcile.
    IF      (v_dec_pl->>'specversion')                IS DISTINCT FROM '1.0'
       OR   (v_dec_pl->>'type')                       IS DISTINCT FROM 'spendguard.audit.decision'
       OR   (v_dec_pl->>'datacontenttype')            IS DISTINCT FROM 'application/json'
       OR   coalesce(length(v_dec_pl->>'signing_key_id'), 0) = 0
       OR   (p_audit_decision_outbox_row->>'event_type') IS DISTINCT FROM 'spendguard.audit.decision'
    THEN
        RAISE EXCEPTION 'AUDIT_INVARIANT_VIOLATED: decision payload literal mismatch'
          USING ERRCODE = 'P0001';
    END IF;

    BEGIN
        v_dec_data_bytes := decode(v_dec_pl->>'data_b64', 'base64');
    EXCEPTION WHEN OTHERS THEN
        RAISE EXCEPTION 'AUDIT_INVARIANT_VIOLATED: decision data_b64 not base64-decodable'
          USING ERRCODE = 'P0001';
    END;
    IF v_dec_data_bytes IS NULL OR length(v_dec_data_bytes) = 0 THEN
        RAISE EXCEPTION 'AUDIT_INVARIANT_VIOLATED: decision data_b64 must be non-empty'
          USING ERRCODE = 'P0001';
    END IF;

    -- =========================================================
    -- 15) Δ11.7 cross-row metadata consistency.
    -- =========================================================
    BEGIN
        IF      (v_dec_pl->>'producer_id')            IS DISTINCT FROM (v_out_pl->>'producer_id')
           OR   (v_dec_pl->>'runid')                  IS DISTINCT FROM (v_out_pl->>'runid')
           OR   (v_dec_pl->>'source')                 IS DISTINCT FROM (v_out_pl->>'source')
           OR   (v_dec_pl->>'time_seconds')::BIGINT
                  IS DISTINCT FROM (v_out_pl->>'time_seconds')::BIGINT
           OR   (v_dec_pl->>'time_nanos')::BIGINT
                  IS DISTINCT FROM (v_out_pl->>'time_nanos')::BIGINT
           OR   (v_dec_pl->>'schema_bundle_id')        IS DISTINCT FROM (v_out_pl->>'schema_bundle_id')
        THEN
            RAISE EXCEPTION 'AUDIT_INVARIANT_VIOLATED: decision/outcome metadata divergence'
              USING ERRCODE = 'P0001';
        END IF;
    EXCEPTION WHEN numeric_value_out_of_range OR invalid_text_representation THEN
        RAISE EXCEPTION 'AUDIT_INVARIANT_VIOLATED: cross-row metadata cast failed'
          USING ERRCODE = 'P0001';
    END;

    -- =========================================================
    -- 16) Compute baseline + delta.
    --     baseline = COALESCE(provider_reported, estimated)
    -- =========================================================
    v_baseline := COALESCE(v_commit_row.provider_reported_amount_atomic,
                           v_commit_row.estimated_amount_atomic);
    v_delta    := p_invoice_amount - v_baseline;

    -- =========================================================
    -- 17) Resolve account_ids when delta != 0.
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

        PERFORM 1
          FROM ledger_accounts la
         WHERE la.ledger_account_id IN (v_account_committed, v_account_available)
         ORDER BY la.budget_id, la.window_instance_id, la.unit_id, la.account_kind
           FOR UPDATE OF la;

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
        v_lock_order_token := 'v1:' || encode(digest('invoice_reconcile:noop', 'sha256'), 'hex');
    END IF;

    -- =========================================================
    -- 18) INSERT ledger_transactions.
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
            v_tx_id, v_tenant_id, 'invoice_reconcile',
            'posted', v_now,
            v_idempotency_key, v_request_hash,
            COALESCE(p_transaction->'minimal_replay_response', '{}'::JSONB),
            (p_transaction->>'trace_event_id')::UUID,
            v_audit_event_id, v_decision_id,
            v_effective_at, v_now,
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
           AND operation_kind = 'invoice_reconcile'
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
    -- 19) INSERT ledger_entries (mirror 0014 step 11; commit_event_kind='invoice_reconciled').
    -- =========================================================
    IF v_delta > 0 THEN
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
            p_reservation_id, 'invoice_reconciled',
            v_shard_id, v_seq_a,
            v_effective_at, date_trunc('month', v_effective_at)::DATE,
            v_now, v_recorded_month
        ),
        (
            gen_random_uuid(), v_tx_id, v_account_committed,
            v_tenant_id, v_reserve_entry.budget_id, v_reserve_entry.window_instance_id, v_reserve_entry.unit_id,
            'credit', v_delta,
            v_reserve_entry.pricing_version, v_reserve_entry.price_snapshot_hash,
            v_reserve_entry.fx_rate_version, v_reserve_entry.unit_conversion_version,
            p_reservation_id, 'invoice_reconciled',
            v_shard_id, v_seq_b,
            v_effective_at, date_trunc('month', v_effective_at)::DATE,
            v_now, v_recorded_month
        );
    ELSIF v_delta < 0 THEN
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
            'debit', -v_delta,
            v_reserve_entry.pricing_version, v_reserve_entry.price_snapshot_hash,
            v_reserve_entry.fx_rate_version, v_reserve_entry.unit_conversion_version,
            p_reservation_id, 'invoice_reconciled',
            v_shard_id, v_seq_a,
            v_effective_at, date_trunc('month', v_effective_at)::DATE,
            v_now, v_recorded_month
        ),
        (
            gen_random_uuid(), v_tx_id, v_account_available,
            v_tenant_id, v_reserve_entry.budget_id, v_reserve_entry.window_instance_id, v_reserve_entry.unit_id,
            'credit', -v_delta,
            v_reserve_entry.pricing_version, v_reserve_entry.price_snapshot_hash,
            v_reserve_entry.fx_rate_version, v_reserve_entry.unit_conversion_version,
            p_reservation_id, 'invoice_reconciled',
            v_shard_id, v_seq_b,
            v_effective_at, date_trunc('month', v_effective_at)::DATE,
            v_now, v_recorded_month
        );
    END IF;
    -- delta == 0: no entries; per-unit balance vacuously satisfied.

    -- =========================================================
    -- 20) Per-unit balance check (vacuous if no entries).
    -- =========================================================
    PERFORM assert_per_unit_balance_now(v_tx_id);

    -- =========================================================
    -- 21) INSERT audit_outbox row 1 (decision; server-minted).
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
        v_decision_audit_outbox_id, v_derived_decision_event_id, v_decision_id,
        v_tenant_id, v_tx_id,
        'spendguard.audit.decision',
        v_dec_pl, v_decision_payload_sig,
        v_caller_epoch, v_workload_id,
        TRUE, 0,
        v_now, v_recorded_month,
        (p_outcome_producer_seq - 1), v_idempotency_key
    );

    -- =========================================================
    -- 22) INSERT audit_outbox row 2 (outcome; caller-signed).
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
        v_outcome_audit_outbox_id, v_outcome_event_id, v_decision_id,
        v_tenant_id, v_tx_id,
        'spendguard.audit.outcome',
        v_out_pl, v_outcome_payload_sig,
        v_caller_epoch, v_workload_id,
        TRUE, 0,
        v_now, v_recorded_month,
        p_outcome_producer_seq, v_idempotency_key
    );

    -- =========================================================
    -- 23) INSERT audit_outbox_global_keys row 1 (decision; suffix).
    -- =========================================================
    INSERT INTO audit_outbox_global_keys (
        audit_decision_event_id, tenant_id, decision_id,
        event_type, operation_kind,
        workload_instance_id, producer_sequence,
        idempotency_key, recorded_month, audit_outbox_id
    ) VALUES (
        v_derived_decision_event_id, v_tenant_id, v_decision_id,
        'spendguard.audit.decision', 'invoice_reconcile',
        v_workload_id, (p_outcome_producer_seq - 1),
        v_idempotency_key || ':decision',
        v_recorded_month, v_decision_audit_outbox_id
    );

    -- =========================================================
    -- 24) INSERT audit_outbox_global_keys row 2 (outcome; suffix).
    -- =========================================================
    INSERT INTO audit_outbox_global_keys (
        audit_decision_event_id, tenant_id, decision_id,
        event_type, operation_kind,
        workload_instance_id, producer_sequence,
        idempotency_key, recorded_month, audit_outbox_id
    ) VALUES (
        v_outcome_event_id, v_tenant_id, v_decision_id,
        'spendguard.audit.outcome', 'invoice_reconcile',
        v_workload_id, p_outcome_producer_seq,
        v_idempotency_key || ':outcome',
        v_recorded_month, v_outcome_audit_outbox_id
    );

    -- =========================================================
    -- 25) UPDATE commits projection by commit_id (v7 Δ4).
    -- =========================================================
    UPDATE commits
       SET latest_state = 'invoice_reconciled',
           invoice_reconciled_amount_atomic = p_invoice_amount,
           delta_to_reserved_atomic = p_invoice_amount - v_reserve_entry.amt,
           invoice_reconciled_at = v_now,
           updated_at = v_now
     WHERE commit_id = v_commit_row.commit_id
       AND tenant_id = v_tenant_id
       AND latest_state IN ('estimated', 'provider_reported');

    GET DIAGNOSTICS v_rowcount = ROW_COUNT;
    IF v_rowcount <> 1 THEN
        RAISE EXCEPTION
            'RESERVATION_STATE_CONFLICT: UPDATE commits affected % rows (expected 1)',
            v_rowcount
            USING ERRCODE = 'P0001';
    END IF;

    -- reservations.current_state stays 'committed' (no UPDATE here).

    RETURN v_tx_id;
END;
$$ LANGUAGE plpgsql SECURITY DEFINER;

GRANT EXECUTE ON FUNCTION post_invoice_reconciled_transaction(JSONB, UUID, NUMERIC, JSONB, JSONB, JSONB, BIGINT)
    TO ledger_application_role;
