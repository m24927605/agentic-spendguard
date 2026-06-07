-- D14 COV_69 — Widen audit_outbox.import_source CHECK to include
-- 'devin_team_api'.
--
-- Deviation from D14 implementation.md §2.1: the spec proposed
-- migration number 0047 against services/canonical_ingest/migrations/.
-- In the current tree the `import_source` column is declared by
-- D13 mig 0058 (services/ledger/migrations/0058_subscription_importer.sql),
-- NOT by a canonical_ingest migration. The next free number in the
-- ledger migrations sequence is 0059. Using 0047 would collide with
-- pre-existing 0048 (tokenizer_versions) — same pattern the D13
-- delivery hit when it went 0046→0058 instead of 0046→0047.
--
-- This migration is purely ADDITIVE per design §6 locked decision #1:
-- it widens the existing CHECK from 5 values to 6, without dropping
-- any column, narrowing any other constraint, or touching the partial
-- index `idx_audit_outbox_import_source` (review-standards G3, G5).
--
-- D13 mig 0058 enum:
--   anthropic_console_usage
--   openai_admin_usage
--   devin_admin_usage     ← D13 placeholder (per ImporterKind enum)
--   manus_admin_usage
--   genspark_admin_usage
--
-- D14 adds:
--   devin_team_api        ← the value the Devin importer writes
--
-- Spec: docs/specs/coverage/D14_devin_importer/design.md §6 + §4.3.

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
            'devin_team_api'
        ));

COMMENT ON COLUMN audit_outbox.import_source IS
    'D13 §5 / D14 §4.3 — set by importer crates only; live proxy/sidecar rows leave NULL. devin_team_api is the live D14 value (D13 left devin_admin_usage as a placeholder; both pass CHECK).';
