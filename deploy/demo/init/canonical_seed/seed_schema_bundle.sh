#!/bin/sh
# Seed spendguard_canonical.schema_bundles after bundles-init produces
# runtime.env with the actual hash. Phase 2B Outbox Forwarder closure
# (Codex r2 V2.1).
#
# Run as a one-shot container:
#   - depends_on: postgres healthy + bundles-init completed
#   - mounts: bundles-data (read /var/lib/spendguard/bundles/runtime.env)
#   - psql client connects to spendguard_canonical and INSERT row.

set -eu

RUNTIME_ENV=/var/lib/spendguard/bundles/runtime.env
if [ ! -f "$RUNTIME_ENV" ]; then
    echo "[canonical-seed] FATAL: $RUNTIME_ENV not found" >&2
    exit 1
fi

# Source for SPENDGUARD_SCHEMA_BUNDLE_HASH_HEX.
set -a
. "$RUNTIME_ENV"
set +a

if [ -z "${SPENDGUARD_SCHEMA_BUNDLE_HASH_HEX:-}" ]; then
    echo "[canonical-seed] FATAL: SPENDGUARD_SCHEMA_BUNDLE_HASH_HEX not in runtime.env" >&2
    exit 1
fi
if [ -z "${SCHEMA_BUNDLE_ID:-}" ]; then
    echo "[canonical-seed] FATAL: SCHEMA_BUNDLE_ID env var not set" >&2
    exit 1
fi

echo "[canonical-seed] inserting schema_bundle_id=$SCHEMA_BUNDLE_ID hash=$SPENDGUARD_SCHEMA_BUNDLE_HASH_HEX"

PGPASSWORD="${POSTGRES_PASSWORD:-spendguard_demo}" \
psql -v ON_ERROR_STOP=1 \
    --host="${POSTGRES_HOST:-postgres}" \
    --username="${POSTGRES_USER:-spendguard}" \
    --dbname="spendguard_canonical" \
    -c "INSERT INTO schema_bundles ( \
            schema_bundle_id, schema_bundle_hash, canonical_schema_version, \
            profile_versions, fetched_at, cosign_verified_at \
        ) VALUES ( \
            '$SCHEMA_BUNDLE_ID'::UUID, \
            decode('$SPENDGUARD_SCHEMA_BUNDLE_HASH_HEX', 'hex'), \
            'spendguard.v1alpha1', \
            '{}'::JSONB, \
            now(), \
            now() \
        ) ON CONFLICT DO NOTHING;"

echo "[canonical-seed] done"
