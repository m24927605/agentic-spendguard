-- D16 COV_88 — Widen audit_outbox.import_source CHECK to include
-- 'genspark_team_api'.
--
-- This migration is purely ADDITIVE per D16 design §6 locked decision
-- #8: it widens the existing CHECK from 7 values (D13 mig 0058 + D14
-- mig 0059 + the D15 widen at mig 0060 that lands in parallel) to 8,
-- without dropping any column, narrowing any other constraint, or
-- touching the partial index `idx_audit_outbox_import_source`
-- (review-standards G3, G6).
--
-- Migration number: 0061. D14 reserved 0059 for `devin_team_api`; D15
-- reserves 0060 for `manus_team_api` (parallel work-stream); D16
-- takes 0061. Reviewer cross-checks `ls services/ledger/migrations/006*.sql`
-- to confirm exclusive ownership of 0061.
--
-- D13 mig 0058 enum + D14 mig 0059 widen + D15 mig 0060 widen (assumed):
--   anthropic_console_usage
--   openai_admin_usage
--   devin_admin_usage     ← D13 placeholder
--   manus_admin_usage     ← D13 placeholder
--   genspark_admin_usage  ← D13 placeholder
--   devin_team_api        ← D14 live value (mig 0059)
--   manus_team_api        ← D15 live value (mig 0060, parallel)
--
-- D16 adds:
--   genspark_team_api     ← the value the Genspark importer writes
--
-- Note on naming: the spec design.md §3.2 originally proposed
-- `genspark_billing`, but the on-disk pattern from D13/D14 is
-- `<vendor>_team_api` / `<vendor>_admin_usage`. We use
-- `genspark_team_api` for cross-vendor consistency with D14's
-- `devin_team_api` and the assumed D15 `manus_team_api`.
--
-- The down-migration drops `genspark_team_api` from the CHECK; it
-- will fail (intentionally) when any 'genspark_team_api' rows exist —
-- the operator must purge or re-migrate before downgrading.
--
-- Spec: docs/specs/coverage/D16_genspark_importer/design.md §6 + §3.2.

ALTER TABLE audit_outbox
    DROP CONSTRAINT IF EXISTS audit_outbox_import_source_check;

ALTER TABLE audit_outbox
    ADD CONSTRAINT audit_outbox_import_source_check
        CHECK (import_source IS NULL OR import_source IN (
            'anthropic_console_usage',
            'openai_admin_usage',
            'devin_admin_usage',
            'manus_admin_usage',
            'genspark_admin_usage',
            'devin_team_api',
            'manus_team_api',
            'genspark_team_api'
        ));

COMMENT ON COLUMN audit_outbox.import_source IS
    'D13 §5 / D14 §4.3 / D15 §4.3 / D16 §3.2 — set by importer crates only; live proxy/sidecar rows leave NULL. genspark_team_api is the live D16 value (D13 left genspark_admin_usage as a placeholder; both pass CHECK).';
