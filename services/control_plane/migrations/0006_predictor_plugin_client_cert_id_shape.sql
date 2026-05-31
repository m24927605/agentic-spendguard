-- 0006_predictor_plugin_client_cert_id_shape.sql
--
-- HARDEN_08: align durable predictor_plugin_endpoints.client_cert_id
-- with the runtime and Helm SVID mount contract.
--
-- Shape rationale:
--   - output_predictor maps client_cert_id to an on-disk subdirectory.
--   - Helm renders volume/mount/Certificate names as
--     plugin-client-svid-<client_cert_id>; 44 bytes leaves the composed
--     name at Kubernetes' 63-byte DNS label limit.
--   - [A-Za-z0-9_-] is intentionally stricter than a full DNS label so
--     the same value is safe as a path segment and chart identifier.

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM predictor_plugin_endpoints
        WHERE client_cert_id !~ '^[A-Za-z0-9_-]{1,44}$'
    ) THEN
        RAISE EXCEPTION
            'HARDEN_08 precondition failed: predictor_plugin_endpoints has invalid client_cert_id values; fix rows to ^[A-Za-z0-9_-]{1,44}$ before applying migration 0006';
    END IF;
END $$;

ALTER TABLE predictor_plugin_endpoints
    DROP CONSTRAINT IF EXISTS predictor_plugin_endpoints_client_cert_id_check;

ALTER TABLE predictor_plugin_endpoints
    ADD CONSTRAINT predictor_plugin_endpoints_client_cert_id_check
    CHECK (client_cert_id ~ '^[A-Za-z0-9_-]{1,44}$');

COMMENT ON COLUMN predictor_plugin_endpoints.client_cert_id IS
    'SpendGuard-issued per-tenant SVID client cert identifier. HARDEN_08 constrains this to ^[A-Za-z0-9_-]{1,44}$ so it is safe as an output_predictor mount path segment and Helm resource suffix.';
