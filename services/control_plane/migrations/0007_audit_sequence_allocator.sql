-- ============================================================================
-- 0007_audit_sequence_allocator.sql
--
-- Replace the three SELECT COALESCE(MAX(producer_sequence), 0) + 1 hot-paths
-- in services/control_plane/src (predictor_plugins audit emit + tokenizer
-- sampling-rate audit emit + tokenizer shadow-security audit emit) with a
-- serialized per-tenant allocator SP.
--
-- Previous pattern lost-update-raced on concurrent operator writes for the
-- same tenant: both transactions read the same MAX, both computed +1, both
-- INSERTed, and the UNIQUE(tenant_id, producer_sequence) constraint forced
-- one to roll back. The handler surfaced the rollback as `500`, even though
-- the operator's request was logically valid — just unlucky.
--
-- Fix: a per-tenant counter row in `control_plane_audit_sequence_counter`,
-- bumped under row-level lock by `control_plane_allocate_audit_sequence()`.
-- Concurrent callers serialize on the row lock and each receive a distinct
-- monotonically-increasing number.
-- ============================================================================

CREATE TABLE IF NOT EXISTS control_plane_audit_sequence_counter (
    tenant_id       UUID PRIMARY KEY,
    next_sequence   BIGINT NOT NULL CHECK (next_sequence >= 1)
);

COMMENT ON TABLE control_plane_audit_sequence_counter IS
    'Per-tenant monotonic counter for control_plane_audit_outbox.producer_sequence allocation. Each call to control_plane_allocate_audit_sequence() bumps the row under exclusive row lock so concurrent allocations cannot race onto the same sequence number.';

COMMENT ON COLUMN control_plane_audit_sequence_counter.next_sequence IS
    'The NEXT producer_sequence value to hand out. The allocator returns (next_sequence - 1) and increments. Seeded from MAX(producer_sequence)+1 of any pre-existing audit_outbox rows so we never regress over already-committed sequences.';

-- Seed from existing rows so allocations continue from MAX+1 rather than 1.
-- Idempotent: re-running the migration leaves any existing counter row
-- untouched (an ALREADY-seeded counter is the authoritative source going
-- forward, the historical MAX may have fallen further behind it).
INSERT INTO control_plane_audit_sequence_counter (tenant_id, next_sequence)
SELECT tenant_id, MAX(producer_sequence) + 1
  FROM control_plane_audit_outbox
 GROUP BY tenant_id
ON CONFLICT (tenant_id) DO NOTHING;

-- RLS: the counter table is read+written exclusively by the application
-- role (control_plane_application_role) via the SP below. We turn RLS on
-- with a permissive policy so a future BYPASSRLS-less role cannot bypass
-- accidental tenant_id mismatches at the application layer.
ALTER TABLE control_plane_audit_sequence_counter ENABLE ROW LEVEL SECURITY;
ALTER TABLE control_plane_audit_sequence_counter FORCE ROW LEVEL SECURITY;

CREATE POLICY control_plane_audit_sequence_counter_app_rw
    ON control_plane_audit_sequence_counter
    FOR ALL
    USING (true)
    WITH CHECK (true);

COMMENT ON POLICY control_plane_audit_sequence_counter_app_rw
    ON control_plane_audit_sequence_counter IS
    'Permissive policy: the table is reached only via control_plane_allocate_audit_sequence() (which the application invokes with the per-request tenant_id pre-checked at the handler layer). The policy exists so a future least-privilege role cannot accidentally bypass the structure.';

-- Application role permissions: SELECT, INSERT, UPDATE only — DELETEs
-- would silently regress the counter for that tenant, breaking the
-- monotonicity invariant.
REVOKE ALL ON control_plane_audit_sequence_counter FROM PUBLIC;
GRANT SELECT, INSERT, UPDATE ON control_plane_audit_sequence_counter
    TO control_plane_application_role;

-- The allocator itself. UPDATE ... RETURNING grabs an exclusive row
-- lock; concurrent callers queue on that lock and each receives a
-- distinct number.
CREATE OR REPLACE FUNCTION control_plane_allocate_audit_sequence(p_tenant_id UUID)
RETURNS BIGINT
LANGUAGE plpgsql
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    v_allocated BIGINT;
BEGIN
    -- Lazily seed the counter for a tenant that hasn't written any
    -- audit events yet. Idempotent: ON CONFLICT keeps any
    -- previously-seeded value (which the bulk seed above set to
    -- MAX(producer_sequence)+1 for tenants present at deploy time).
    INSERT INTO control_plane_audit_sequence_counter (tenant_id, next_sequence)
    SELECT p_tenant_id, COALESCE(MAX(producer_sequence), 0) + 1
      FROM control_plane_audit_outbox
     WHERE tenant_id = p_tenant_id
    ON CONFLICT (tenant_id) DO NOTHING;

    -- UPDATE acquires FOR UPDATE on the row. Concurrent callers
    -- serialize. RETURNING (next_sequence - 1) gives the caller the
    -- value JUST allocated (the next_sequence column already reflects
    -- the post-increment value).
    UPDATE control_plane_audit_sequence_counter
       SET next_sequence = next_sequence + 1
     WHERE tenant_id = p_tenant_id
    RETURNING next_sequence - 1 INTO v_allocated;

    IF v_allocated IS NULL THEN
        -- Defensive: the INSERT-then-UPDATE above always leaves a row.
        RAISE EXCEPTION
            'control_plane_allocate_audit_sequence: counter row vanished for tenant %',
            p_tenant_id
            USING ERRCODE = 'P0002';
    END IF;

    RETURN v_allocated;
END;
$$;

COMMENT ON FUNCTION control_plane_allocate_audit_sequence IS
    'Serialized per-tenant producer_sequence allocator for control_plane_audit_outbox. Replaces the SELECT COALESCE(MAX,0)+1 anti-pattern that lost-update-raced under concurrent operator writes. Call inside the same transaction as the audit_outbox INSERT so a rolled-back transaction returns its allocated number to the unused pool (the counter advanced — but the gap is benign for downstream forwarder ordering, which only requires monotonicity, not density).';

REVOKE ALL ON FUNCTION control_plane_allocate_audit_sequence(UUID) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION control_plane_allocate_audit_sequence(UUID)
    TO control_plane_application_role;
