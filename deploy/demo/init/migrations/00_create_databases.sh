#!/bin/bash
# Postgres docker-entrypoint-initdb.d hook (runs once on first boot).
# Creates the two application databases used by the demo. Each service's
# migrations land in its own DB so that schema namespaces stay clean and
# we can later split onto separate Postgres instances without a rename.
set -euo pipefail

psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname postgres <<-EOSQL
    CREATE DATABASE spendguard_ledger;
    CREATE DATABASE spendguard_canonical;
EOSQL

echo "[init] created spendguard_ledger + spendguard_canonical"
