-- D15 COV_74 — Widen audit_outbox.import_source CHECK to include
-- 'manus_team_api'.
--
-- Deviation from D15 implementation.md §4.2: the spec proposed
-- migration number 0048 against services/canonical_ingest/migrations/.
-- In the current tree the `import_source` column is declared by
-- D13 mig 0058 (services/ledger/migrations/0058_subscription_importer.sql),
-- NOT by a canonical_ingest migration. The next free number is 0060;
-- D14 took 0059 for `devin_team_api` and D16 has reserved 0061 for
-- `genspark_team_api`. Reviewer cross-checks
-- `ls services/ledger/migrations/006*.sql` to confirm exclusive
-- ownership of 0060.
--
-- This migration is purely ADDITIVE per design §6 spirit: it widens
-- the existing CHECK from 6 values (D13 mig 0058 + D14 mig 0059) to 7,
-- without dropping any column, narrowing any other constraint, or
-- touching the partial index `idx_audit_outbox_import_source`
-- (review-standards G3, G5).
--
-- D13 mig 0058 enum + D14 mig 0059 widen:
--   anthropic_console_usage
--   openai_admin_usage
--   devin_admin_usage     ← D13 placeholder
--   manus_admin_usage     ← D13 placeholder (still valid; not removed)
--   genspark_admin_usage  ← D13 placeholder
--   devin_team_api        ← D14 live value (mig 0059)
--
-- D15 adds:
--   manus_team_api        ← the value the Manus importer writes
--
-- Cross-vendor consistency: D14's `devin_team_api` (mig 0059) +
-- D15's `manus_team_api` (this mig) + D16's `genspark_team_api`
-- (mig 0061) follow the locked `<vendor>_team_api` family pattern.
--
-- Note on reservation_source: D15 ships `subscription_meter` (D14 /
-- D16 family-aligned) on every emitted row; per-importer filtering
-- uses `import_source`. NO `reservation_source` CHECK widening is
-- required (acceptance §1 of D14 / D16; review-standards on import
-- family).
--
-- Spec: docs/specs/coverage/D15_manus_importer/design.md §5 + §3.3.

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
            'manus_team_api'
        ));

COMMENT ON COLUMN audit_outbox.import_source IS
    'D13 §5 / D14 §4.3 / D15 §3.3 — set by importer crates only; live proxy/sidecar rows leave NULL. manus_team_api is the live D15 value (D13 left manus_admin_usage as a placeholder; both pass CHECK).';
