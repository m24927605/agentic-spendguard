-- Cached schema bundle index.
-- Per Trace §12: producers emit `schema_bundle_id` + `schema_bundle_hash`;
-- ingest validates against this table. Bundles themselves live in the
-- Bundle Registry (OCI); this table is a cache for fast lookup.

CREATE TABLE schema_bundles (
    schema_bundle_id          UUID PRIMARY KEY,
    schema_bundle_hash        BYTEA NOT NULL,
    canonical_schema_version  TEXT  NOT NULL,
    profile_versions          JSONB NOT NULL DEFAULT '{}'::JSONB,
    fetched_at                TIMESTAMPTZ NOT NULL DEFAULT now(),
    cosign_verified_at        TIMESTAMPTZ,
    UNIQUE (schema_bundle_id, schema_bundle_hash)
);

COMMENT ON TABLE schema_bundles IS
    'Cache from Bundle Registry. Append-only; bundle versions are immutable.';
