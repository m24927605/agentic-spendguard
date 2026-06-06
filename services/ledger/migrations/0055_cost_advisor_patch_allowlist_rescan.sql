-- =====================================================================
-- 0055: Force re-validation of cost_advisor proposed_dsl_patch rows
--       under the migration-0044 validator function.
-- =====================================================================
--
-- Migration 0044 redefined cost_advisor_validate_proposed_dsl_patch
-- (the function backing the approval_requests_cost_advisor_patch_allowlist
-- CHECK constraint) with two changes vs. 0043:
--
--   1. Added the `test` op + same-index pinning invariant.
--   2. Changed the path prefix from `/budgets/...` to `/spec/budgets/...`
--      to match the contract YAML schema (`spec.budgets[]`).
--
-- 0044 issued CREATE OR REPLACE FUNCTION but did NOT re-validate the
-- existing approval_requests rows. PostgreSQL CHECK constraints
-- referencing a redefined function will use the NEW function the next
-- time a row is INSERTed or UPDATEd, but already-stored rows that
-- passed validation under the OLD function remain in place untouched.
--
-- In production this matters because:
--   * Any cost_advisor row inserted between 0043 and 0044 was emitted
--     by an older cost_advisor binary that wrote `/budgets/...` paths.
--     Those rows pass the 0043 validator but FAIL the 0044 validator.
--   * The next UPDATE on such a row (e.g. resolve_approval_request
--     transitioning it pending → approved) triggers the CHECK on the
--     new function definition and ROLLS BACK with errcode 23514 —
--     which the operator sees as a confusing 409 / 500.
--   * A row in this state cannot be resolved through any handler,
--     cannot be swept by the TTL sweeper (which UPDATEs the row), and
--     cannot be deleted (immutability + RESTRICT FK from approval_events).
--     The only way out is a hand-written admin SQL transition.
--
-- This migration:
--   (a) Runs the 0044 validator over every existing cost_advisor row
--       and either RAISES (operator inspects + cleans up) or proceeds.
--   (b) DROPs + ADDs the CHECK constraint with VALIDATE so PostgreSQL
--       formally records the rescan in the catalog; once that lands,
--       any operator running pg_dump --schema-only sees the validated
--       state.
--
-- Idempotent: the DROP IF EXISTS + re-ADD ensures the migration can be
-- re-applied without error.

DO $$
DECLARE
    v_invalid_row RECORD;
    v_invalid_count INT := 0;
    v_examples TEXT := '';
BEGIN
    -- Scan: any row that the current validator rejects?
    FOR v_invalid_row IN
        SELECT approval_id, tenant_id, state, jsonb_typeof(proposed_dsl_patch) AS patch_type
          FROM approval_requests
         WHERE proposal_source = 'cost_advisor'
           AND NOT cost_advisor_validate_proposed_dsl_patch(proposed_dsl_patch)
         ORDER BY created_at ASC
         LIMIT 5
    LOOP
        v_invalid_count := v_invalid_count + 1;
        v_examples := v_examples
            || format(
                E'\n  - approval_id=% tenant=% state=% patch_type=%',
                v_invalid_row.approval_id,
                v_invalid_row.tenant_id,
                v_invalid_row.state,
                COALESCE(v_invalid_row.patch_type, 'null')
            );
    END LOOP;

    IF v_invalid_count > 0 THEN
        -- Don't silently auto-quarantine — that would mask a real
        -- data-integrity problem. Surface it loudly so the operator
        -- decides: dismiss the row manually, write a one-off
        -- transition SQL, or roll the cost_advisor binary back.
        RAISE EXCEPTION
            E'0055: % cost_advisor approval_requests row(s) fail the current cost_advisor_validate_proposed_dsl_patch validator. Examples (first 5):%\nResolve by manually transitioning these rows to a terminal state (denied/cancelled/expired) via resolve_approval_request, OR by writing an explicit UPDATE under operator audit. Re-run this migration after cleanup.',
            v_invalid_count,
            v_examples
            USING ERRCODE = '23514';   -- check_violation
    END IF;
END $$;

-- DROP + re-ADD the CHECK with NOT VALID + VALIDATE so PostgreSQL
-- formally records the rescan against the current function definition.
-- Both ALTERs are metadata-only on a hot table (ADD CONSTRAINT NOT VALID
-- takes SHARE UPDATE EXCLUSIVE briefly; VALIDATE CONSTRAINT scans
-- under SHARE UPDATE EXCLUSIVE — concurrent reads and writes proceed).
ALTER TABLE approval_requests
    DROP CONSTRAINT IF EXISTS approval_requests_cost_advisor_patch_allowlist;

ALTER TABLE approval_requests
    ADD CONSTRAINT approval_requests_cost_advisor_patch_allowlist
    CHECK (
        proposal_source <> 'cost_advisor'
        OR cost_advisor_validate_proposed_dsl_patch(proposed_dsl_patch)
    ) NOT VALID;

ALTER TABLE approval_requests
    VALIDATE CONSTRAINT approval_requests_cost_advisor_patch_allowlist;

COMMENT ON CONSTRAINT approval_requests_cost_advisor_patch_allowlist
    ON approval_requests IS
    'Re-validated under the 0044 validator function in migration 0055. See 0043 (initial), 0044 (function redefinition with /spec/ prefix + test op + same-index pinning).';
