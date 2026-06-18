-- ============================================================================
-- 0001_predictor_plugin_endpoints.sql — Customer plugin endpoint registry.
--
-- Spec ancestors:
--   - docs/output-predictor-plugin-contract-v1alpha1.md §8 (control plane API)
--   - docs/output-predictor-plugin-contract-v1alpha1.md §7 (multi-tenant isolation)
--   - docs/output-predictor-plugin-contract-v1alpha1.md §3 (mTLS cert pinning)
--   - docs/internal/slices/SLICE_07_output_predictor_plugin_c.md §5
--
-- ## Why this table lives in the control_plane DB
--
-- Control plane owns the endpoint configuration surface (POST/PUT/DELETE per
-- spec §8.1). The output_predictor reads from this table via a read-only
-- connection (endpoint_cache.rs in SLICE_07 Phase C). Co-locating the
-- registry in the control_plane DB matches the existing convention where
-- tenant + budget + fencing rows are owned by the control plane.
--
-- This is the first migration file living under
-- `services/control_plane/migrations/`. Pre-existing schema (tenants,
-- budgets, ledger_units) lives under `services/ledger/migrations/`
-- because the original POC topology shared a single Postgres database.
-- SLICE_07 introduces the control_plane-specific schema directory; the
-- migration runner is wired to consume both directories per the helm
-- chart's migrations job.
--
-- ## Per-tenant isolation (spec §7.1)
--
-- Each tenant has at most ONE plugin endpoint. Enforced via UNIQUE on
-- tenant_id (not via composite key) — this is the structural guarantee
-- that prevents customer cross-tenant fan-in at the registry layer.
-- Multi-region or HA customer deployments register the same logical
-- endpoint URL behind a load balancer; SpendGuard does not multi-write.
--
-- ## RLS posture (mirror of canonical_ingest/0016 R2 B1 pattern)
--
-- Row-Level Security ON with FOR ALL policy: every SELECT and every
-- INSERT/UPDATE/DELETE requires the caller's session to have
-- `SET LOCAL app.current_tenant_id = '<uuid>'` before the query.
-- The output_predictor read-through cache invokes
-- `SELECT set_config('app.current_tenant_id', tenant, true)` before
-- the SELECT (per SLICE_07 Phase C endpoint_cache.rs); the control
-- plane handlers do the same before INSERT/UPDATE/DELETE.
--
-- Failure mode: a forgotten SET LOCAL produces a 0-row read (or a
-- WITH CHECK violation on write) — never a silent cross-tenant leak.
-- The nil-UUID sentinel
-- ('00000000-0000-0000-0000-000000000000') in the COALESCE never
-- matches a production tenant_id (all tenants mint UUIDv7 with
-- timestamp > 0).
--
-- ## mTLS cert pinning (spec §3.2)
--
-- server_cert_fingerprint stores the SHA-256 fingerprint (64-hex)
-- of the plugin endpoint's TLS server cert. SpendGuard pins on this
-- value at connection-time; a mismatch is treated as a TLS error
-- and falls to Strategy B per spec §5.1 (mode
-- `customer_predictor_tls_error`).
--
-- client_cert_id references the SpendGuard-issued client cert
-- identifier — SpendGuard maintains a separate cert store keyed by
-- this id (cert rotation drill per spec §3.2 swaps cert content,
-- keeps id stable). v1alpha1 ships the column shape; actual cert
-- rotation is wired in SLICE_14 / cert_issuer follow-up.
--
-- ## Stylistic alignment (per SLICE_01 R5 conventions)
--
-- - psql autocommit per migration (no BEGIN/COMMIT wrapping)
-- - SET LOCAL search_path = pg_catalog, pg_temp in DO blocks
--   (CVE-2018-1058 hardening)
-- - TIMESTAMPTZ with TZ-explicit `+00` defaults
-- - UUIDv7 minted application-side (no DEFAULT gen_random_uuid())
-- - No down migration file per SLICE_03 R2 M3 convention; rollback
--   via `DROP TABLE predictor_plugin_endpoints CASCADE` (operator
--   one-liner documented in this header).
--
-- ## Privilege boundary
--
-- - control_plane_application_role: full DML (register / update /
--   delete endpoints via the REST API in SLICE_07 Phase E)
-- - control_plane_reader_role: SELECT only (output_predictor's
--   read-through cache + ad-hoc operator queries)
--
-- REVOKE PUBLIC ensures no role inherits the default PUBLIC grants
-- and bypasses the RLS policy via the role-attribute layer.
--
-- ## Not partitioned
--
-- Scale estimate: ≤ N tenants × 1 endpoint = small (<10K rows even
-- at extreme tenant counts). UPSERT performance on a single heap is
-- excellent at this scale; partition overhead would only hurt.
-- ============================================================================

-- ============================================================================
-- Role bootstrap (idempotent — re-run safe).
-- Mirrors the canonical_ingest 0001_extensions.sql convention: role
-- creation lives in the first migration so subsequent GRANTs target
-- a known principal.
-- ============================================================================

DO $$
BEGIN
    SET LOCAL search_path = pg_catalog, pg_temp;
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'control_plane_application_role') THEN
        CREATE ROLE control_plane_application_role NOLOGIN;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'control_plane_reader_role') THEN
        CREATE ROLE control_plane_reader_role NOLOGIN;
    END IF;
END $$;

-- ============================================================================
-- Main table.
-- ============================================================================

CREATE TABLE predictor_plugin_endpoints (
    -- Application-minted UUIDv7 per the SLICE_01 R5 convention. No
    -- DEFAULT so the writer (control plane handler) is forced to mint
    -- it explicitly — surfaces missing-uuid bugs at the writer rather
    -- than papering over with a server-side gen_random_uuid().
    plugin_endpoint_id      UUID         PRIMARY KEY,

    -- One endpoint per tenant (spec §7.1). UNIQUE on tenant_id
    -- structurally prevents customer cross-tenant fan-in at the
    -- registry layer.
    tenant_id               UUID         NOT NULL UNIQUE,

    -- Plugin gRPC endpoint URL. CHECK enforces an http or https scheme
    -- (mTLS = https; plaintext = http only allowed in demo profile,
    -- production Helm gate blocks via env validation in SLICE_07
    -- Phase E). 2048-char limit prevents accidental megabyte URLs.
    endpoint_url            TEXT         NOT NULL
                            CHECK (endpoint_url ~ '^https?://')
                            CHECK (octet_length(endpoint_url) <= 2048),

    -- SHA-256 fingerprint of the plugin endpoint's TLS server cert
    -- (lowercase hex, 64 chars). SpendGuard pins on this at connect
    -- time; mismatch = `customer_predictor_tls_error` + fall to B.
    -- CHECK enforces format (defensive — the REST handler also validates
    -- before INSERT).
    server_cert_fingerprint TEXT         NOT NULL
                            CHECK (server_cert_fingerprint ~ '^[0-9a-f]{64}$'),

    -- SpendGuard-issued client cert identifier. References the
    -- cert_issuer store (v1alpha1 ships the column shape; SLICE_14
    -- wires the rotation pipeline). Format opaque; CHECK enforces
    -- non-empty + length cap to prevent accidentally huge values.
    client_cert_id          TEXT         NOT NULL
                            CHECK (octet_length(client_cert_id) BETWEEN 1 AND 256),

    -- Operator kill-switch. Setting `enabled = FALSE` causes the
    -- output_predictor to skip C for this tenant (fall to B silently
    -- per spec §11). Used during incident response or migration drills.
    enabled                 BOOLEAN      NOT NULL DEFAULT TRUE,

    -- Registration timestamp; TZ-explicit per SLICE_01 R5.
    registered_at           TIMESTAMPTZ  NOT NULL DEFAULT clock_timestamp(),

    -- Last successful HealthCheck timestamp. NULL = never probed.
    -- Updated by the output_predictor's 30s health-check loop per
    -- spec §6.3 (wired in SLICE_07 Phase B as part of the breaker
    -- state machine).
    last_health_check_at    TIMESTAMPTZ,

    -- Last observed HealthCheckResponse.Status as a lowercase string
    -- ('serving' | 'degraded' | 'not_serving' | 'unreachable').
    -- NULL = never probed. CHECK constrains the allowed values so
    -- a typo in the writer raises immediately.
    current_health_status   TEXT
                            CHECK (current_health_status IS NULL
                                   OR current_health_status IN (
                                       'serving',
                                       'degraded',
                                       'not_serving',
                                       'unreachable'
                                   ))
);

-- ============================================================================
-- Indexes
-- ============================================================================

-- Periodic health-check sweep query: pick the next batch of enabled
-- endpoints whose last_health_check_at is stale. Partial index on
-- enabled = TRUE makes the sweep cheap even at extreme tenant counts.
-- NULLS FIRST ensures never-probed endpoints (last_health_check_at IS
-- NULL) come first so cold-start churn does not starve them.
CREATE INDEX predictor_plugin_endpoints_health_sweep_idx
    ON predictor_plugin_endpoints (last_health_check_at NULLS FIRST)
    WHERE enabled = TRUE;

-- ============================================================================
-- Row-Level Security (spec §7.1 enforcement).
--
-- Mirror of canonical_ingest 0016 R2 B1: ENABLE + FORCE; FOR ALL
-- policy enforces USING for SELECT + WITH CHECK for INSERT/UPDATE
-- so a writer who forgets the SET LOCAL still fails closed (cannot
-- insert a row whose tenant_id mismatches the session variable).
--
-- nil-UUID sentinel in COALESCE: a missing session variable produces
-- a clean tenant-mismatch (0 rows / WITH CHECK violation) rather
-- than a silent NULL-match cross-tenant leak. Production tenants
-- mint UUIDv7 with timestamp > 0; the nil UUID never matches.
-- ============================================================================

ALTER TABLE predictor_plugin_endpoints ENABLE ROW LEVEL SECURITY;
ALTER TABLE predictor_plugin_endpoints FORCE ROW LEVEL SECURITY;

CREATE POLICY predictor_plugin_endpoints_tenant_isolation
    ON predictor_plugin_endpoints
    FOR ALL
    USING (
        tenant_id = COALESCE(
            NULLIF(current_setting('app.current_tenant_id', TRUE), ''),
            '00000000-0000-0000-0000-000000000000'
        )::uuid
    )
    WITH CHECK (
        tenant_id = COALESCE(
            NULLIF(current_setting('app.current_tenant_id', TRUE), ''),
            '00000000-0000-0000-0000-000000000000'
        )::uuid
    );

-- ============================================================================
-- Privilege boundary
-- ============================================================================

REVOKE SELECT, INSERT, UPDATE, DELETE ON predictor_plugin_endpoints FROM PUBLIC;

-- Control plane REST handlers (register / update / delete / force-reset).
GRANT SELECT, INSERT, UPDATE, DELETE
    ON predictor_plugin_endpoints
    TO control_plane_application_role;

-- output_predictor read-through cache + ad-hoc operator queries.
-- Read goes through the application role (RLS applies; reader_role
-- by default would bypass RLS, which is wrong here).
GRANT SELECT ON predictor_plugin_endpoints TO control_plane_application_role;
GRANT SELECT ON predictor_plugin_endpoints TO control_plane_reader_role;

COMMENT ON TABLE predictor_plugin_endpoints IS
    'Customer-configured Strategy C plugin endpoints per output-predictor-plugin-contract-v1alpha1.md §8. UNIQUE(tenant_id) enforces one-endpoint-per-tenant (spec §7.1); RLS FOR ALL policy enforces per-tenant isolation at both read AND write time. Demo profile may register http:// endpoints; production Helm gate rejects via env validation.';

COMMENT ON COLUMN predictor_plugin_endpoints.tenant_id IS
    'Tenant identifier (UUIDv7). UNIQUE — at most one endpoint per tenant per spec §7.1.';
COMMENT ON COLUMN predictor_plugin_endpoints.endpoint_url IS
    'Plugin gRPC endpoint URL. Production: https://*; demo: may be http://. Length-capped at 2048 bytes to prevent accidentally huge values.';
COMMENT ON COLUMN predictor_plugin_endpoints.server_cert_fingerprint IS
    'SHA-256 fingerprint (lowercase hex, 64 chars) of the plugin endpoint TLS server cert. SpendGuard pins at connect; mismatch = customer_predictor_tls_error.';
COMMENT ON COLUMN predictor_plugin_endpoints.client_cert_id IS
    'SpendGuard-issued client cert identifier. cert_issuer rotation pipeline (SLICE_14) swaps cert content while keeping this id stable.';
COMMENT ON COLUMN predictor_plugin_endpoints.enabled IS
    'Operator kill-switch. FALSE = output_predictor skips C for this tenant (fall to B silently per spec §11).';
COMMENT ON COLUMN predictor_plugin_endpoints.last_health_check_at IS
    'Last successful HealthCheck wallclock. NULL = never probed. Updated by output_predictor 30s health loop per spec §6.3.';
COMMENT ON COLUMN predictor_plugin_endpoints.current_health_status IS
    'Last observed HealthCheckResponse.Status (lowercase). NULL = never probed; ''unreachable'' = the gRPC dial itself failed.';

-- ============================================================================
-- DO-block smoke check: verify RLS is actually enabled and the policy
-- exists. CVE-2018-1058 hardening: SET LOCAL search_path so PostgreSQL
-- resolves built-in catalog names without consulting the runtime
-- search_path (per SLICE_01 R5).
-- ============================================================================

DO $$
BEGIN
    SET LOCAL search_path = pg_catalog, pg_temp;

    PERFORM 1 FROM pg_class
        WHERE relname = 'predictor_plugin_endpoints' AND relrowsecurity = TRUE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'predictor_plugin_endpoints RLS not enabled after migration';
    END IF;

    PERFORM 1 FROM pg_class
        WHERE relname = 'predictor_plugin_endpoints' AND relforcerowsecurity = TRUE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'predictor_plugin_endpoints FORCE RLS not set after migration';
    END IF;

    PERFORM 1 FROM pg_policy
        WHERE polname = 'predictor_plugin_endpoints_tenant_isolation';
    IF NOT FOUND THEN
        RAISE EXCEPTION 'predictor_plugin_endpoints_tenant_isolation policy missing';
    END IF;
END $$;
