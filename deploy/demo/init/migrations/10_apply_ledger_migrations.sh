#!/bin/bash
# Apply ledger migrations against `spendguard_ledger`.
# Source SQL files come from a read-only bind mount populated by compose
# (see compose.yaml: `../../services/ledger/migrations`).
set -euo pipefail

MIG_DIR=/var/spendguard/ledger-migrations
if [ ! -d "$MIG_DIR" ]; then
    echo "[init] ledger migrations dir $MIG_DIR not mounted; skipping" >&2
    exit 0
fi

for f in $(ls -1 "$MIG_DIR"/*.sql | sort); do
    echo "[init] applying ledger migration: $f"
    psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" \
         --dbname spendguard_ledger -f "$f"
done

echo "[init] ledger migrations applied"
