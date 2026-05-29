-- Down-migration: reverse 0014_schema_bundle_prediction_v1alpha1.sql
-- (round-2 fix m2).
--
-- Removes the rotated schema_bundle row. Note: schema_bundles is
-- append-only in production (see 0005:25-28 immutability trigger);
-- this down-migration only works on a fresh / demo cluster where the
-- immutability trigger is bypassed during teardown. The schema_bundles
-- trigger fires BEFORE DELETE so we temporarily drop + recreate it.
--
-- Production rollback should NEVER use this — instead, leave the row in
-- place; it is forward-compatible with any pre-SLICE_01 reader (per
-- Trace §6 dual_read).

BEGIN;

DROP TRIGGER IF EXISTS schema_bundles_no_update_delete ON schema_bundles;

DELETE FROM schema_bundles
 WHERE schema_bundle_id = '01999d60-0001-7000-8000-000000000001'::uuid;

-- Restore the immutability trigger from 0005.
CREATE TRIGGER schema_bundles_no_update_delete
    BEFORE UPDATE OR DELETE ON schema_bundles
    FOR EACH ROW EXECUTE FUNCTION reject_canonical_event_mutation();

COMMIT;
