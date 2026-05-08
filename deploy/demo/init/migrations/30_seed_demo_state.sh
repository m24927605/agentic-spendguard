#!/bin/bash
# Seed the minimum demo state so that `Sidecar.RequestDecision` →
# `Ledger.ReserveSet` succeeds end-to-end.
#
# Required rows (matched against actual schema columns; Round 1 Codex
# review caught a number of column drift bugs in the prior version):
#   - 1 ledger_units                     (per 0001_ledger_units.sql)
#   - 1 pricing_snapshots row            (per 0005)
#   - 1 fencing_scopes row, scope_type='budget_window' (per 0006 +
#                                         post_ledger_transaction.sproc)
#   - 1 budget_window_instances row      (per 0003)
#   - 2 ledger_accounts (available_budget + reserved_hold) (per 0004)
#
# Notes:
#   * The reserve_set sproc REQUIRES scope_type IN ('reservation',
#     'budget_window'). 'budget_window' is the natural fit for the
#     sidecar's pre-provisioned scope; CHECK demands budget_id +
#     window_instance_id be set.
#   * Available-budget OPENING balance is now established by inserting
#     a seed ledger_entry against the available_budget account, NOT by
#     a phantom column. The sproc's per-unit balance check requires a
#     matching credit so we insert an offsetting credit on a synthetic
#     "adjustment" account so the per-unit-balance assertion passes.
#     Phase 2 will replace this with a proper top-up ledger op.
set -euo pipefail

# Demo identity (kept in sync with compose.yaml).
TENANT_ID="00000000-0000-4000-8000-000000000001"
WORKLOAD_INSTANCE_ID="sidecar-demo-1"
FENCING_SCOPE_ID="33333333-3333-4333-8333-333333333333"
UNIT_ID="66666666-6666-4666-8666-666666666666"
BUDGET_ID="44444444-4444-4444-8444-444444444444"
WINDOW_INSTANCE_ID="55555555-5555-4555-8555-555555555555"
PRICING_VERSION="demo-pricing-v1"
FX_RATE_VERSION="demo-fx-v1"
UNIT_CONVERSION_VERSION="demo-units-v1"
PRICE_SNAPSHOT_HASH_HEX=$(printf '%s' \
    "${PRICING_VERSION}:${FX_RATE_VERSION}:${UNIT_CONVERSION_VERSION}:demo-prices" \
    | sha256sum | awk '{print $1}')

psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" \
     --dbname spendguard_ledger <<EOSQL
\\set ON_ERROR_STOP on

-- 0) Ledger shard + sequence allocator. The reserve_set handler hardcodes
-- ledger_shard_id=1, and post_ledger_transaction() calls
-- nextval_per_shard(1) which UPDATEs ledger_sequence_allocators. Both
-- tables must have a row for shard 1 or every reserve fails. (Codex
-- Round 2 caught this hole.)
INSERT INTO ledger_shards (
    ledger_shard_id, shard_generation, status
) VALUES (1, 1, 'active')
ON CONFLICT DO NOTHING;

INSERT INTO ledger_sequence_allocators (
    ledger_shard_id, last_sequence
) VALUES (1, 0)
ON CONFLICT DO NOTHING;

-- 1) Ledger unit. token_kind+model_family required for token kind
-- (Contract §12.1). scale=0 (whole tokens). rounding_mode=truncate
-- per §3.4 default for token charges.
INSERT INTO ledger_units (
    unit_id, tenant_id, unit_kind, scale, rounding_mode,
    token_kind, model_family
) VALUES (
    '${UNIT_ID}'::UUID,
    '${TENANT_ID}'::UUID,
    'token',
    0,
    'truncate',
    'output_token',
    'gpt-4'
) ON CONFLICT DO NOTHING;

-- 2) Pricing snapshot. schema_json carries the demo's price table —
-- structure is opaque to the seed; the contract bundle's price hash
-- is what binds them. signature/signing_key_id/deployed_by are
-- required NOT NULL by the migration.
INSERT INTO pricing_snapshots (
    pricing_version, price_snapshot_hash, fx_rate_version,
    unit_conversion_version, schema_json, signature, signing_key_id,
    deployed_by
) VALUES (
    '${PRICING_VERSION}',
    decode('${PRICE_SNAPSHOT_HASH_HEX}', 'hex'),
    '${FX_RATE_VERSION}',
    '${UNIT_CONVERSION_VERSION}',
    '{"demo": "stub price table — real version comes from Pricing Authority"}'::JSONB,
    decode('00', 'hex'),
    'demo-key-1',
    'demo-seed'
) ON CONFLICT DO NOTHING;

-- 3) Budget window instance. window_type='rolling' avoids needing a
-- timezone; tzdb_version still required NOT NULL.
INSERT INTO budget_window_instances (
    window_instance_id, tenant_id, budget_id, window_type, tzdb_version,
    boundary_start, boundary_end, computed_from_snapshot_at
) VALUES (
    '${WINDOW_INSTANCE_ID}'::UUID,
    '${TENANT_ID}'::UUID,
    '${BUDGET_ID}'::UUID,
    'rolling',
    '2025c',
    now() - interval '1 hour',
    now() + interval '24 hours',
    now()
) ON CONFLICT DO NOTHING;

-- 4) Fencing scope (scope_type='budget_window'; sproc allows this for
-- 'reserve' operation_kind). active_owner_instance_id matches the
-- sidecar's SPENDGUARD_SIDECAR_WORKLOAD_INSTANCE_ID env var so the CAS
-- owner check passes. current_epoch=1 because post_ledger_transaction()
-- explicitly rejects epoch 0 ("brand-new scopes default to 0; a
-- properly-acquired lease has CAS-incremented at least once") — see
-- 0012_post_ledger_transaction.sql:108. The sidecar's
-- SPENDGUARD_SIDECAR_FENCING_INITIAL_EPOCH must match this value.
INSERT INTO fencing_scopes (
    fencing_scope_id, scope_type, tenant_id, budget_id,
    window_instance_id, current_epoch, active_owner_instance_id,
    ttl_expires_at, epoch_source_authority
) VALUES (
    '${FENCING_SCOPE_ID}'::UUID,
    'budget_window',
    '${TENANT_ID}'::UUID,
    '${BUDGET_ID}'::UUID,
    '${WINDOW_INSTANCE_ID}'::UUID,
    1,
    '${WORKLOAD_INSTANCE_ID}',
    now() + interval '24 hours',
    'ledger_lease'
) ON CONFLICT DO NOTHING;

-- 5) Ledger accounts: one row per account_kind the demo will touch.
-- Phase 2B Step 7 commit lifecycle requires committed_spend + adjustment
-- in addition to available_budget + reserved_hold. ledger_account_id is
-- uuid-v4 (gen_random_uuid).
INSERT INTO ledger_accounts (
    ledger_account_id, tenant_id, budget_id, window_instance_id,
    account_kind, unit_id
) VALUES (
    gen_random_uuid(), '${TENANT_ID}'::UUID, '${BUDGET_ID}'::UUID,
    '${WINDOW_INSTANCE_ID}'::UUID, 'available_budget', '${UNIT_ID}'::UUID
), (
    gen_random_uuid(), '${TENANT_ID}'::UUID, '${BUDGET_ID}'::UUID,
    '${WINDOW_INSTANCE_ID}'::UUID, 'reserved_hold', '${UNIT_ID}'::UUID
), (
    gen_random_uuid(), '${TENANT_ID}'::UUID, '${BUDGET_ID}'::UUID,
    '${WINDOW_INSTANCE_ID}'::UUID, 'committed_spend', '${UNIT_ID}'::UUID
), (
    gen_random_uuid(), '${TENANT_ID}'::UUID, '${BUDGET_ID}'::UUID,
    '${WINDOW_INSTANCE_ID}'::UUID, 'adjustment', '${UNIT_ID}'::UUID
) ON CONFLICT DO NOTHING;

-- 6) Phase 2B Step 7: opening deposit so available_budget starts at 500.
-- The post_ledger_transaction SP enforces fencing CAS + audit_outbox
-- atomicity, so we seed a 'control_plane_writer' fencing scope first
-- (sproc allows this scope_type for operation_kind='adjustment').
-- audit_outbox event_type='spendguard.audit.decision' per Codex round 3
-- L1.3 fix (operator deposits are decisions, not orphan outcomes).
INSERT INTO fencing_scopes (
    fencing_scope_id, scope_type, tenant_id, workload_kind,
    current_epoch, active_owner_instance_id,
    ttl_expires_at, epoch_source_authority
) VALUES (
    '00000000-0000-7000-a000-000000000010'::UUID,
    'control_plane_writer',
    '${TENANT_ID}'::UUID,
    'demo_seed_runner',
    1,
    'demo-seed-runner',
    'infinity'::timestamptz,
    'ledger_lease'
) ON CONFLICT DO NOTHING;

-- 6b) Phase 2B Step 8: separate cp_writer fencing scope for the webhook
-- simulator. Distinct workload_instance_id keeps producer_sequence space
-- independent (audit_outbox_global_producer_seq_uq is keyed by
-- (tenant, workload_instance_id, producer_sequence)). Codex round 1
-- P2.2 fix.
INSERT INTO fencing_scopes (
    fencing_scope_id, scope_type, tenant_id, workload_kind,
    current_epoch, active_owner_instance_id,
    ttl_expires_at, epoch_source_authority
) VALUES (
    '00000000-0000-7000-a000-000000000050'::UUID,
    'control_plane_writer',
    '${TENANT_ID}'::UUID,
    'demo_webhook_receiver',
    1,
    'demo-webhook-receiver',
    'infinity'::timestamptz,
    'ledger_lease'
) ON CONFLICT DO NOTHING;

-- 6c) TTL Sweeper fencing scope (Phase 2B closes Step 7.5 P1.1).
-- Distinct workload from sidecar + webhook so producer_sequence space
-- is independent. control_plane_writer scope_type per Migration 0019.
INSERT INTO fencing_scopes (
    fencing_scope_id, scope_type, tenant_id, workload_kind,
    current_epoch, active_owner_instance_id,
    ttl_expires_at, epoch_source_authority
) VALUES (
    '00000000-0000-7000-a000-000000000060'::UUID,
    'control_plane_writer',
    '${TENANT_ID}'::UUID,
    'demo_ttl_sweeper',
    1,
    'demo-ttl-sweeper',
    'infinity'::timestamptz,
    'ledger_lease'
) ON CONFLICT DO NOTHING;

-- 7) Operator deposit: credit available_budget 500, debit adjustment 500.
-- This routes through post_ledger_transaction (the SP) so the per-unit
-- balance + fencing + audit_outbox invariants are exercised end-to-end
-- exactly the way the runtime exercises them.
SELECT post_ledger_transaction(
    jsonb_build_object(
        'tenant_id',                '${TENANT_ID}',
        'operation_kind',           'adjustment',
        'idempotency_key',          'demo-seed-deposit-1',
        'request_hash_hex',         encode(digest('demo-seed-deposit-1', 'sha256'), 'hex'),
        'decision_id',              '00000000-0000-7000-a000-000000000020',
        'audit_decision_event_id',  '00000000-0000-7000-a000-000000000021',
        'fencing_scope_id',         '00000000-0000-7000-a000-000000000010',
        'fencing_epoch',            1,
        'workload_instance_id',     'demo-seed-runner',
        'effective_at',             now()::text,
        'ledger_transaction_id',    '00000000-0000-7000-a000-000000000022',
        'minimal_replay_response',  '{}'::jsonb
    ),
    jsonb_build_array(
        jsonb_build_object(
            'budget_id',                '${BUDGET_ID}',
            'window_instance_id',       '${WINDOW_INSTANCE_ID}',
            'unit_id',                  '${UNIT_ID}',
            'account_kind',             'available_budget',
            'direction',                'credit',
            'amount_atomic',            '500',
            'pricing_version',          '${PRICING_VERSION}',
            'price_snapshot_hash_hex',  '${PRICE_SNAPSHOT_HASH_HEX}',
            'fx_rate_version',          '${FX_RATE_VERSION}',
            'unit_conversion_version',  '${UNIT_CONVERSION_VERSION}',
            'ledger_entry_id',          '00000000-0000-7000-a000-000000000030',
            'ledger_shard_id',          1
        ),
        jsonb_build_object(
            'budget_id',                '${BUDGET_ID}',
            'window_instance_id',       '${WINDOW_INSTANCE_ID}',
            'unit_id',                  '${UNIT_ID}',
            'account_kind',             'adjustment',
            'direction',                'debit',
            'amount_atomic',            '500',
            'pricing_version',          '${PRICING_VERSION}',
            'price_snapshot_hash_hex',  '${PRICE_SNAPSHOT_HASH_HEX}',
            'fx_rate_version',          '${FX_RATE_VERSION}',
            'unit_conversion_version',  '${UNIT_CONVERSION_VERSION}',
            'ledger_entry_id',          '00000000-0000-7000-a000-000000000031',
            'ledger_shard_id',          1
        )
    ),
    NULL,
    jsonb_build_object(
        'audit_outbox_id',                  '00000000-0000-7000-a000-000000000040',
        'event_type',                       'spendguard.audit.decision',
        'cloudevent_payload',               jsonb_build_object(
            'specversion',  '1.0',
            'type',         'spendguard.audit.decision',
            'id',           '00000000-0000-7000-a000-000000000021',
            'source',       'demo-seed-runner',
            'tenantid',     '${TENANT_ID}',
            'data_b64',     encode(convert_to(
                '{"kind":"operator_adjustment","reason":"demo_seed_opening_balance","amount_atomic":"500"}',
                'utf8'), 'base64'),
            'producer_sequence', 1
        ),
        'cloudevent_payload_signature_hex', '',
        'producer_sequence',                1
    ),
    NULL
);
EOSQL

echo "[init] demo seed inserted into spendguard_ledger"
