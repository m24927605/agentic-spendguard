-- Phase 5 GA Hardening S3: Ledger.AcquireFencingLease SP.
--
-- Replaces the seeded-current_epoch=1 model with an SP-owned CAS
-- lease primitive. The ledger remains the single source of fencing
-- authority — sidecar / webhook-receiver / ttl-sweeper call this SP
-- via the AcquireFencingLease RPC at startup AND on a renew schedule.
--
-- Invariants enforced:
--   * Renewal by the current holder MUST NOT change current_epoch.
--   * Takeover (lease expired) bumps current_epoch by exactly 1.
--   * Takeover before expiry is rejected unless p_force=TRUE
--     (operator-driven recovery only; should be rare).
--   * Every successful acquire/renew appends a fencing_scope_events
--     row inside the same DB transaction (auditable history).
--   * The fencing_scope row MUST exist before this is called — the
--     SP returns NOT_FOUND error rather than auto-creating, because
--     scope_type + budget_id binding is operator policy.

CREATE OR REPLACE FUNCTION acquire_fencing_lease(
    p_scope_id      UUID,
    p_tenant_id     UUID,
    p_workload_id   TEXT,
    p_ttl_seconds   INT,
    p_force         BOOLEAN DEFAULT FALSE,
    p_audit_event_id UUID DEFAULT NULL
) RETURNS TABLE(
    granted          BOOLEAN,
    new_epoch        BIGINT,
    expires_at       TIMESTAMPTZ,
    action           TEXT,
    holder_instance_id TEXT
) AS $$
DECLARE
    v_now      TIMESTAMPTZ := clock_timestamp();
    v_ttl      INTERVAL := (p_ttl_seconds || ' seconds')::INTERVAL;
    v_existing RECORD;
    v_audit_id UUID := COALESCE(p_audit_event_id, gen_random_uuid());
    v_action   TEXT;
    v_new_epoch BIGINT;
BEGIN
    IF p_workload_id IS NULL OR length(p_workload_id) = 0 THEN
        RAISE EXCEPTION 'workload_id required' USING ERRCODE = '22023';
    END IF;
    IF p_ttl_seconds <= 0 THEN
        RAISE EXCEPTION 'ttl_seconds must be > 0' USING ERRCODE = '22023';
    END IF;

    SELECT * INTO v_existing
      FROM fencing_scopes
     WHERE fencing_scope_id = p_scope_id
       FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'fencing_scope_id not found: %', p_scope_id
            USING ERRCODE = '40P02';
    END IF;
    IF v_existing.tenant_id <> p_tenant_id THEN
        RAISE EXCEPTION 'fencing_scope tenant mismatch (scope.tenant=%, caller.tenant=%)',
            v_existing.tenant_id, p_tenant_id
            USING ERRCODE = '40P02';
    END IF;

    -- Path A: renewal by current holder, lease still alive.
    IF v_existing.active_owner_instance_id = p_workload_id
       AND v_existing.ttl_expires_at IS NOT NULL
       AND v_existing.ttl_expires_at > v_now THEN
        UPDATE fencing_scopes
           SET ttl_expires_at = v_now + v_ttl,
               updated_at = v_now
         WHERE fencing_scope_id = p_scope_id;
        v_action := 'renew';
        v_new_epoch := v_existing.current_epoch;

        INSERT INTO fencing_scope_events
            (fencing_event_id, fencing_scope_id, old_epoch, new_epoch,
             owner_instance_id, action, audit_event_id)
        VALUES (gen_random_uuid(), p_scope_id, v_existing.current_epoch,
                v_new_epoch, p_workload_id, v_action, v_audit_id);

        RETURN QUERY SELECT true, v_new_epoch, v_now + v_ttl,
                            v_action, p_workload_id;
        RETURN;
    END IF;

    -- Path B: takeover — lease expired OR no current holder OR
    -- p_force=TRUE for operator override.
    IF v_existing.ttl_expires_at IS NULL
       OR v_existing.ttl_expires_at <= v_now
       OR v_existing.active_owner_instance_id IS NULL
       OR p_force THEN
        v_new_epoch := v_existing.current_epoch + 1;
        v_action := CASE
                       WHEN v_existing.active_owner_instance_id IS NULL THEN 'acquire'
                       WHEN v_existing.active_owner_instance_id = p_workload_id THEN 'recover'
                       WHEN p_force THEN 'revoke'
                       ELSE 'promote'  -- previous holder expired naturally
                   END;

        UPDATE fencing_scopes
           SET active_owner_instance_id = p_workload_id,
               current_epoch = v_new_epoch,
               ttl_expires_at = v_now + v_ttl,
               updated_at = v_now
         WHERE fencing_scope_id = p_scope_id;

        INSERT INTO fencing_scope_events
            (fencing_event_id, fencing_scope_id, old_epoch, new_epoch,
             owner_instance_id, action, audit_event_id)
        VALUES (gen_random_uuid(), p_scope_id, v_existing.current_epoch,
                v_new_epoch, p_workload_id, v_action, v_audit_id);

        RETURN QUERY SELECT true, v_new_epoch, v_now + v_ttl,
                            v_action, p_workload_id;
        RETURN;
    END IF;

    -- Path C: held by someone else, not yet expired, no force flag.
    -- Caller MUST back off; the rightful holder is identified for
    -- runbook diagnostics.
    RETURN QUERY SELECT false, v_existing.current_epoch,
                        v_existing.ttl_expires_at,
                        'denied'::TEXT,
                        v_existing.active_owner_instance_id;
END;
$$ LANGUAGE plpgsql SECURITY DEFINER;

GRANT EXECUTE ON FUNCTION acquire_fencing_lease(UUID, UUID, TEXT, INT, BOOLEAN, UUID)
    TO PUBLIC;

COMMENT ON FUNCTION acquire_fencing_lease IS
    'Phase 5 S3: SP-owned fencing lease CAS. Renewal preserves epoch;
     takeover bumps epoch by 1. fencing_scope_events appended atomically.';
