#!/bin/bash
# Apply cost_advisor migrations against `spendguard_canonical`.
# Source SQL files come from a read-only bind mount populated by compose
# (see compose.yaml: `../../services/cost_advisor/migrations`).
#
# Cost Advisor tables (cost_findings + cost_baselines) live in
# spendguard_canonical alongside canonical_events per spec §4.1.
# Run AFTER 20_apply_canonical_migrations.sh — the failure_class
# column 0011 from canonical_ingest is a prerequisite for rules that
# join canonical_events.failure_class with cost_findings.evidence.
set -euo pipefail

MIG_DIR=/var/spendguard/cost-advisor-migrations
if [ ! -d "$MIG_DIR" ]; then
    echo "[init] cost_advisor migrations dir $MIG_DIR not mounted; skipping" >&2
    exit 0
fi

for f in $(ls -1 "$MIG_DIR"/*.sql | sort); do
    echo "[init] applying cost_advisor migration: $f"
    psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" \
         --dbname spendguard_canonical -f "$f"
done

echo "[init] cost_advisor migrations applied"
