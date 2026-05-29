-- Down-migration: reverse 0049_tokenizer_versions_initial_seed.sql
--
-- Apply BEFORE 0048_tokenizer_versions_down.sql per SLICE_03 §11
-- rollback order. The 0048 down-migration drops the
-- tokenizer_versions table outright, which would implicitly delete
-- these rows — but per the immutability trigger on tokenizer_versions
-- (CREATE TRIGGER tokenizer_versions_no_update_delete BEFORE UPDATE
-- OR DELETE ...) a direct DELETE on the rows would fail. So the
-- down-migration cannot DELETE; it can only verify that the caller
-- ALSO ran the destructive 0048_down guard (which will DROP TABLE
-- and bypass the trigger).
--
-- This file exists for migration-runner symmetry; it's a no-op when
-- run between 0049 and 0048 in the rollback chain.
--
-- Round-1 self-review note: this is intentionally a no-op. The
-- alternative — disabling the trigger, DELETEing the four rows,
-- re-enabling — was rejected because:
--   1. It widens the immutability invariant ("rotation = INSERT new
--      + flip retired_at, never DELETE") that the spec §6.2 +
--      0048's trigger encode.
--   2. The whole-table 0048_down DROP gets the same end-state
--      atomically without any reachability window for a
--      mid-rollback DELETE to corrupt audit_outbox FK lineage.

DO $$
BEGIN
    -- Verify the row sanity: if the seed rows are present but the
    -- table is otherwise non-empty (SLICE_04+ added rows), warn the
    -- operator that the destructive 0048_down will drop those too.
    DECLARE
        seed_count INTEGER;
        total_count INTEGER;
    BEGIN
        SELECT COUNT(*) INTO seed_count
        FROM tokenizer_versions
        WHERE tokenizer_version_id IN (
            '01918000-0000-7c10-0c10-000000000001'::uuid,
            '01918000-0000-7c10-0c10-000000000002'::uuid,
            '01918000-0000-7c10-0c10-000000000003'::uuid,
            '01918000-0000-7c10-0c10-00000000000f'::uuid
        );
        SELECT COUNT(*) INTO total_count FROM tokenizer_versions;

        RAISE NOTICE 'tokenizer_versions rollback: seed_rows=%, total_rows=%', seed_count, total_count;

        IF total_count > seed_count THEN
            RAISE NOTICE
                'tokenizer_versions has % rows beyond the SLICE_03 seed (likely SLICE_04+ encoders); 0048_down will drop them too',
                total_count - seed_count;
        END IF;
    END;
END $$;
