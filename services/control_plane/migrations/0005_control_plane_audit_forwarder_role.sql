-- ============================================================================
-- 0005_control_plane_audit_forwarder_role.sql
--
-- HARDEN_06: the audit forwarder drains control_plane_audit_outbox across
-- tenants. The table is FORCE RLS and the application role remains
-- tenant-scoped, so the forwarder gets its own least-privilege role and
-- explicit RLS policies. This is not BYPASSRLS: Postgres still evaluates
-- the role-scoped policies below.
-- ============================================================================

DO $$
BEGIN
    SET LOCAL search_path = pg_catalog, pg_temp;
    IF NOT EXISTS (
        SELECT 1 FROM pg_roles WHERE rolname = 'control_plane_audit_forwarder_role'
    ) THEN
        CREATE ROLE control_plane_audit_forwarder_role NOLOGIN;
    END IF;
END $$;

GRANT SELECT, UPDATE ON control_plane_audit_outbox
    TO control_plane_audit_forwarder_role;

CREATE POLICY control_plane_audit_outbox_forwarder_select
    ON control_plane_audit_outbox
    FOR SELECT
    TO control_plane_audit_forwarder_role
    USING (true);

CREATE POLICY control_plane_audit_outbox_forwarder_update
    ON control_plane_audit_outbox
    FOR UPDATE
    TO control_plane_audit_forwarder_role
    USING (true)
    WITH CHECK (true);

COMMENT ON POLICY control_plane_audit_outbox_forwarder_select
    ON control_plane_audit_outbox IS
    'HARDEN_06: dedicated audit forwarder role may read pending rows across tenants without BYPASSRLS.';

COMMENT ON POLICY control_plane_audit_outbox_forwarder_update
    ON control_plane_audit_outbox IS
    'HARDEN_06: dedicated audit forwarder role may set forwarded_at/signature across tenants without BYPASSRLS.';

DO $$
BEGIN
    SET LOCAL search_path = pg_catalog, pg_temp;
    PERFORM 1
      FROM pg_policy
     WHERE polname = 'control_plane_audit_outbox_forwarder_select';
    IF NOT FOUND THEN
        RAISE EXCEPTION 'control_plane_audit_outbox_forwarder_select policy missing';
    END IF;

    PERFORM 1
      FROM pg_policy
     WHERE polname = 'control_plane_audit_outbox_forwarder_update';
    IF NOT FOUND THEN
        RAISE EXCEPTION 'control_plane_audit_outbox_forwarder_update policy missing';
    END IF;
END $$;
