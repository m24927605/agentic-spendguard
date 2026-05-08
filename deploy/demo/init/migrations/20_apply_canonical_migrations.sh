#!/bin/bash
# Apply canonical-ingest migrations against `spendguard_canonical`.
set -euo pipefail

MIG_DIR=/var/spendguard/canonical-migrations
if [ ! -d "$MIG_DIR" ]; then
    echo "[init] canonical-ingest migrations dir $MIG_DIR not mounted; skipping" >&2
    exit 0
fi

for f in $(ls -1 "$MIG_DIR"/*.sql | sort); do
    echo "[init] applying canonical migration: $f"
    psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" \
         --dbname spendguard_canonical -f "$f"
done

echo "[init] canonical migrations applied"
