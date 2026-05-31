#!/bin/bash
# Apply control-plane migrations against `spendguard_ledger`.
#
# The demo topology keeps the control-plane tables in the ledger database so
# the REST API and its audit forwarder can run without another Postgres DB.
set -euo pipefail

MIG_DIR=/var/spendguard/control-plane-migrations
if [ ! -d "$MIG_DIR" ]; then
    echo "[init] control-plane migrations dir $MIG_DIR not mounted; skipping" >&2
    exit 0
fi

for f in $(ls -1 "$MIG_DIR"/*.sql | sort); do
    echo "[init] applying control-plane migration: $f"
    psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" \
         --dbname spendguard_ledger -f "$f"
done

echo "[init] control-plane migrations applied"
