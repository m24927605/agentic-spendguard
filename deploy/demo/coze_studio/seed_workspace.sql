-- D31 SLICE 3 — `DEMO_MODE=coze_studio_real` seed.
--
-- Runs against the shared `spendguard_ledger` Postgres BEFORE the demo
-- driver fires. Idempotent: every statement is `ON CONFLICT DO NOTHING`
-- or a soft-update guard, so re-running this file against an already-
-- seeded ledger is safe.
--
-- What gets seeded:
--   1. Tenant `00000000-0000-4000-8000-000000000001` (the canonical
--      demo tenant — already populated by `30_seed_demo_state.sh`,
--      this file just guarantees the row exists so the verify gate
--      can resolve it without bleeding from prior state).
--   2. Budget `44444444-4444-4444-8444-444444444444` + window-instance
--      `55555555-5555-4555-8555-555555555555` (also canonical demo
--      values from `30_seed_demo_state.sh`).
--   3. A `coze_workspace_provider_seeds` marker row (one per demo run)
--      that lets the verify SQL prove the Slice 3 demo touched the
--      ledger DB rather than re-using a stale row from another mode.
--
-- NOTE: Coze workspace + project + chat-flow seeds live in `coze_db`
-- (the SEPARATE Postgres per INV-8), and are only populated when the
-- `coze` profile is active (`COMPOSE_PROFILES=coze make demo-up ...`).
-- The default demo path skips Coze's own DB entirely — the demo
-- driver proves the companion contract directly. See the deviation
-- note in `compose.override.yaml` header.

\set ON_ERROR_STOP 1

BEGIN;

-- 1. Tenant existence (idempotent).
INSERT INTO tenants (tenant_id, display_name, created_at, updated_at)
VALUES (
    '00000000-0000-4000-8000-000000000001',
    'SpendGuard demo tenant (D31 coze_studio_real)',
    now(), now()
)
ON CONFLICT (tenant_id) DO NOTHING;

-- 2. Budget existence (idempotent). The canonical demo budget is
--    already seeded by the shared `30_seed_demo_state.sh`; this is a
--    safety guard so D31 can run after a partial demo-down.
INSERT INTO budgets (
    budget_id, tenant_id, display_name,
    ceiling_micro_usd, fencing_scope_id,
    created_at, updated_at
)
VALUES (
    '44444444-4444-4444-8444-444444444444',
    '00000000-0000-4000-8000-000000000001',
    'D31 coze_studio_real demo budget',
    1000000,  -- $1 USD ceiling
    '33333333-3333-4333-8333-333333333333',
    now(), now()
)
ON CONFLICT (budget_id) DO NOTHING;

-- 3. Open window instance (idempotent).
INSERT INTO window_instances (
    window_instance_id, budget_id, tenant_id,
    window_start_at, window_end_at, state,
    available_micro_usd, reserved_micro_usd, committed_micro_usd,
    created_at, updated_at
)
VALUES (
    '55555555-5555-4555-8555-555555555555',
    '44444444-4444-4444-8444-444444444444',
    '00000000-0000-4000-8000-000000000001',
    date_trunc('day', now()),
    date_trunc('day', now()) + interval '1 day',
    'open',
    1000000, 0, 0,
    now(), now()
)
ON CONFLICT (window_instance_id) DO NOTHING;

-- 4. D31 seed marker — a per-run tag so the verify SQL can prove the
--    demo touched the ledger DB in the last 5 minutes. Uses a no-op
--    table via the existing demo_run_markers helper if present; falls
--    back to a NOTICE if the table doesn't exist yet (older ledger
--    schemas) so the seed never blocks the demo on harmless absence.
DO $$
BEGIN
  IF EXISTS (
    SELECT 1 FROM pg_tables WHERE tablename = 'demo_run_markers'
  ) THEN
    INSERT INTO demo_run_markers (mode, tenant_id, started_at)
    VALUES (
      'coze_studio_real',
      '00000000-0000-4000-8000-000000000001',
      now()
    )
    ON CONFLICT DO NOTHING;
    RAISE NOTICE 'D31 seed: demo_run_markers row inserted for coze_studio_real';
  ELSE
    RAISE NOTICE 'D31 seed: demo_run_markers table absent — skipping marker (harmless)';
  END IF;
END;
$$;

COMMIT;

\echo
\echo D31 seed: tenant + budget + window-instance ready for coze_studio_real
