-- =====================================================================
-- DEMO_MODE=cost_advisor — seed 10 budget-scoped TTL'd reservations
-- =====================================================================
--
-- Reuses the existing demo tenant + budget seeded by
-- `30_seed_demo_state.sh`. Adds 10 reservations on `:demo_date` (UTC),
-- 7 of which TTL'd at 30s median so idle_reservation_rate_v1 fires:
--   total=10 ≥ 5  ✓
--   ttl_expired/total = 0.7 > 0.20  ✓
--   median_ttl = 30 ≤ 60  ✓
--
-- COMMITs at the end (unlike verify_p1_cost_advisor.sql which rolls
-- back) so the cost_advisor binary can read the data and the
-- resulting cost_findings + approval_requests rows persist for the
-- verify step.
--
-- Wire-time note (codex-blind-spot caught in demo): psql client-side
-- variable interpolation does NOT happen inside dollar-quoted DO
-- blocks. We push the bound values through Postgres GUC settings
-- (`SET LOCAL ...`) and read them inside DO blocks via
-- `current_setting('cost_advisor_demo.X')`.

\set ON_ERROR_STOP 1

BEGIN;

-- Bind the demo parameters as session GUCs so DO blocks can see them.
-- tenant + budget come from `psql -v` (driver script env overrides);
-- window_inst + fencing_scope are tied to the demo's hardcoded
-- 30_seed_demo_state.sh values and are NOT overridable here.
SELECT set_config('cost_advisor_demo.tenant',        :'tenant',    false);
SELECT set_config('cost_advisor_demo.budget',        :'budget',    false);
SELECT set_config('cost_advisor_demo.window_inst',   '55555555-5555-4555-8555-555555555555', false);
SELECT set_config('cost_advisor_demo.fencing_scope', '33333333-3333-4333-8333-333333333333', false);
SELECT set_config('cost_advisor_demo.demo_date',     :'demo_date', false);

-- Light cleanup of PENDING cost_advisor approvals for this
-- tenant/date so two back-to-back invocations on the same volume
-- don't trip the (tenant, decision_id) UNIQUE. Note: this demo is
-- NOT fully idempotent across same-volume re-runs after a
-- successful `make demo-up DEMO_MODE=cost_advisor` — the resulting
-- approval is RESOLVED in step 4, and approval_events FK makes
-- terminal approvals audit-protected (per the demo's step-0
-- pre-check). For a clean re-run, use `make demo-down -v` first.
--
-- Constraints from the audit chain:
--   * approval_events has FK → approval_requests; once a transition
--     event lands (resolve → approved/denied), the approval row is
--     audit-protected and can't be deleted (caught by wire-time demo).
--     We therefore only clean up `state='pending'` approvals; any
--     terminal ones from prior runs stay (the new seed produces a
--     different finding_id → different decision_id → ON CONFLICT
--     idempotency in cost_advisor_create_proposal will return
--     'already_exists' for that finding-version, but the per-run
--     nonce in v_seed_iter creates fresh ledger_transactions so a
--     fresh fingerprint → fresh finding → fresh approval lands).
--   * cost_findings_id_keys cascades from cost_findings (back-FK
--     from 0042); cost_findings_fingerprint_keys does NOT (linked
--     only by the upsert SP), so we delete it explicitly to avoid
--     a 'reinstated' outcome on re-run.
--   * The ledger reservations + audit chain themselves stay
--     (immutable / append-only).
DO $$
DECLARE
    v_tenant UUID := current_setting('cost_advisor_demo.tenant')::uuid;
    v_date   DATE := current_setting('cost_advisor_demo.demo_date')::date;
    v_fids UUID[];
BEGIN
    -- Drop only PENDING cost_advisor approvals — terminal ones are
    -- audit-protected via approval_events FK.
    DELETE FROM approval_requests
     WHERE tenant_id = v_tenant
       AND proposal_source = 'cost_advisor'
       AND state = 'pending';

    -- Collect findings whose only approval reference (if any) is
    -- now gone, so we can safely delete them. A finding referenced
    -- by a remaining (terminal) approval would fail the
    -- cost_findings_id_keys back-FK CASCADE → approval_requests
    -- RESTRICT chain. Filter those out.
    --
    -- Filter by `evidence->>'time_bucket'` (the rule's --date arg,
    -- not `detected_at` which is NOW()) — codex CA-demo r2 P3.
    SELECT array_agg(cf.finding_id) INTO v_fids
      FROM cost_findings cf
     WHERE cf.tenant_id = v_tenant
       AND cf.evidence ->> 'time_bucket' = v_date::text
       AND NOT EXISTS (
           SELECT 1 FROM approval_requests ar
            WHERE ar.proposing_finding_id = cf.finding_id
       );

    IF v_fids IS NOT NULL THEN
        DELETE FROM cost_findings
         WHERE tenant_id = v_tenant
           AND finding_id = ANY(v_fids);
        DELETE FROM cost_findings_fingerprint_keys
         WHERE tenant_id = v_tenant
           AND finding_id = ANY(v_fids);
    END IF;
END $$;

DO $$
DECLARE
    v_tenant        UUID  := current_setting('cost_advisor_demo.tenant')::uuid;
    v_budget        UUID  := current_setting('cost_advisor_demo.budget')::uuid;
    v_window        UUID  := current_setting('cost_advisor_demo.window_inst')::uuid;
    v_fencing_scope UUID  := current_setting('cost_advisor_demo.fencing_scope')::uuid;
    v_demo_date     DATE  := current_setting('cost_advisor_demo.demo_date')::date;
    v_i INT;
    v_decision_id UUID;
    v_reserve_tx_id UUID;
    v_release_tx_id UUID;
    v_audit_decision_id UUID;
    v_audit_outcome_id UUID;
    v_reservation_id UUID;
    v_created_at TIMESTAMPTZ;
    v_ttl_expires_at TIMESTAMPTZ;
    v_seq BIGINT;
    v_seed_iter TEXT;
BEGIN
    -- Per-run nonce so re-runs on the same day don't collide on the
    -- ledger_transactions UNIQUE (tenant, kind, idempotency_key)
    -- constraint. Caught by wire-time demo: cleanup deletes
    -- cost_findings + approval_requests but ledger_transactions are
    -- immutable / audit-protected, so a fresh idempotency_key suffix
    -- is required for re-runnability.
    v_seed_iter := 'demo-' || to_char(v_demo_date, 'YYYYMMDD')
                || '-' || substr(gen_random_uuid()::text, 1, 8) || '-';

    -- v_seq must be globally unique for (tenant, workload_instance_id)
    -- across all audit_outbox rows ever inserted (per
    -- audit_outbox_global_producer_seq_uq). Derive from MAX+1 under a
    -- transaction-scoped advisory lock so concurrent demo runs (or
    -- fast back-to-back re-runs after demo-down) can't collide on
    -- the wire (codex CA-demo r1 P2).
    PERFORM pg_advisory_xact_lock(
        ('x' || substr(md5('cost-advisor-demo:' || v_tenant::text), 1, 16))::bit(64)::bigint
    );
    SELECT COALESCE(MAX(producer_sequence), 0) + 1
      INTO v_seq
      FROM audit_outbox
     WHERE tenant_id = v_tenant
       AND workload_instance_id = 'cost-advisor-demo';

    FOR v_i IN 1..10 LOOP
        v_decision_id := gen_random_uuid();
        v_reserve_tx_id := gen_random_uuid();
        v_release_tx_id := gen_random_uuid();
        v_audit_decision_id := gen_random_uuid();
        v_audit_outcome_id := gen_random_uuid();
        v_reservation_id := gen_random_uuid();
        v_created_at := (v_demo_date::timestamptz + INTERVAL '12 hours') + (v_i * INTERVAL '1 minute');
        v_ttl_expires_at := v_created_at + INTERVAL '30 seconds';

        INSERT INTO ledger_transactions (
            ledger_transaction_id, tenant_id, operation_kind, posting_state,
            idempotency_key, request_hash, lock_order_token, decision_id,
            audit_decision_event_id, effective_at, fencing_scope_id, fencing_epoch_at_post
        ) VALUES (
            v_reserve_tx_id, v_tenant, 'reserve', 'posted',
            v_seed_iter || 'reserve-' || v_i,
            '\x00'::bytea, 'lock-' || v_seed_iter || v_i, v_decision_id,
            v_audit_decision_id, v_created_at,
            v_fencing_scope, 1
        );

        INSERT INTO audit_outbox (
            audit_outbox_id, audit_decision_event_id, decision_id, tenant_id,
            ledger_transaction_id, event_type, cloudevent_payload,
            cloudevent_payload_signature, ledger_fencing_epoch, workload_instance_id,
            recorded_at, recorded_month, producer_sequence, idempotency_key,
            pending_forward
        ) VALUES (
            gen_random_uuid(), v_audit_decision_id, v_decision_id,
            v_tenant, v_reserve_tx_id,
            'spendguard.audit.decision',
            ('{"specversion":"1.0","type":"spendguard.audit.decision","data_b64":"' ||
              encode('{"kind":"reserve"}'::bytea, 'base64') || '"}')::jsonb,
            '\x00'::bytea, 1, 'cost-advisor-demo',
            v_created_at, date_trunc('month', v_demo_date)::date, v_seq,
            v_seed_iter || 'reserve-' || v_i,
            -- Fixture rows are not forwardable (synthetic payload, no signing
            -- key). Mark pending_forward=FALSE so the outbox-forwarder skips
            -- them (codex CA-demo r1 P2).
            FALSE
        );

        -- Mirror into the global keys table (codex CA-demo r2 P2). The
        -- post_ledger_transaction SP normally inserts both atomically;
        -- this fixture bypasses the SP so we must mirror manually to
        -- preserve the global uniqueness invariants.
        INSERT INTO audit_outbox_global_keys (
            audit_decision_event_id, tenant_id, decision_id, event_type,
            operation_kind, workload_instance_id, producer_sequence,
            idempotency_key, recorded_month, audit_outbox_id
        ) VALUES (
            v_audit_decision_id, v_tenant, v_decision_id,
            'spendguard.audit.decision',
            'reserve', 'cost-advisor-demo', v_seq,
            v_seed_iter || 'reserve-' || v_i,
            date_trunc('month', v_demo_date)::date,
            (SELECT audit_outbox_id FROM audit_outbox
              WHERE audit_decision_event_id = v_audit_decision_id)
        );
        v_seq := v_seq + 1;

        INSERT INTO reservations (
            reservation_id, tenant_id, budget_id, window_instance_id, current_state,
            source_ledger_transaction_id, ttl_expires_at, idempotency_key, created_at
        ) VALUES (
            v_reservation_id, v_tenant, v_budget, v_window,
            CASE WHEN v_i <= 7 THEN 'released' ELSE 'committed' END,
            v_reserve_tx_id, v_ttl_expires_at,
            v_seed_iter || 'reserve-' || v_i, v_created_at
        );

        IF v_i <= 7 THEN
            INSERT INTO ledger_transactions (
                ledger_transaction_id, tenant_id, operation_kind, posting_state,
                idempotency_key, request_hash, lock_order_token, decision_id,
                audit_decision_event_id, effective_at, fencing_scope_id, fencing_epoch_at_post
            ) VALUES (
                v_release_tx_id, v_tenant, 'release', 'posted',
                v_seed_iter || 'release-' || v_i,
                '\x01'::bytea, 'lock-rel-' || v_seed_iter || v_i, v_decision_id,
                v_audit_outcome_id, v_ttl_expires_at,
                v_fencing_scope, 1
            );

            INSERT INTO audit_outbox (
                audit_outbox_id, audit_decision_event_id, decision_id, tenant_id,
                ledger_transaction_id, event_type, cloudevent_payload,
                cloudevent_payload_signature, ledger_fencing_epoch, workload_instance_id,
                recorded_at, recorded_month, producer_sequence, idempotency_key,
                pending_forward
            ) VALUES (
                gen_random_uuid(), v_audit_outcome_id, v_decision_id,
                v_tenant, v_release_tx_id,
                'spendguard.audit.outcome',
                ('{"specversion":"1.0","type":"spendguard.audit.outcome","data_b64":"' ||
                  encode('{"kind":"release","reason":"TTL_EXPIRED"}'::bytea, 'base64') || '"}')::jsonb,
                '\x00'::bytea, 1, 'cost-advisor-demo',
                v_ttl_expires_at, date_trunc('month', v_demo_date)::date, v_seq,
                v_seed_iter || 'release-' || v_i,
                FALSE
            );

            -- Mirror into audit_outbox_global_keys (codex CA-demo r2 P2).
            INSERT INTO audit_outbox_global_keys (
                audit_decision_event_id, tenant_id, decision_id, event_type,
                operation_kind, workload_instance_id, producer_sequence,
                idempotency_key, recorded_month, audit_outbox_id
            ) VALUES (
                v_audit_outcome_id, v_tenant, v_decision_id,
                'spendguard.audit.outcome',
                'release', 'cost-advisor-demo', v_seq,
                v_seed_iter || 'release-' || v_i,
                date_trunc('month', v_demo_date)::date,
                (SELECT audit_outbox_id FROM audit_outbox
                  WHERE audit_decision_event_id = v_audit_outcome_id)
            );
            v_seq := v_seq + 1;
        END IF;
    END LOOP;
    RAISE NOTICE 'cost-advisor-demo seed: 10 reservations (7 TTL_EXPIRED) for tenant=% budget=% date=%',
        v_tenant, v_budget, v_demo_date;
END $$;

COMMIT;
