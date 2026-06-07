#!/usr/bin/env bash
# =====================================================================
# D15 COV_74 — `demo-verify-import-manus-fixture` runner.
# =====================================================================
#
# Self-contained demo for the Manus billing importer's fixture flow.
# The importer is reconciliation-only (Manus runs the agent loop
# entirely inside a vendor-managed VM — SpendGuard cannot gate), so the
# demo here is narrow:
#
#   1. Spin up a throwaway postgres:16-alpine container.
#   2. Apply minimal schema (audit_outbox + ledger_entries).
#   3. Apply mig 0059 (D14 widen) + 0060 (D15 widen).
#   4. Run `cargo run -p spendguard-importer-manus` against the
#      committed sanitized fixture; capture the CloudEvent envelopes.
#   5. INSERT one audit_outbox row per envelope (no canonical_ingest
#      gRPC handoff — the runner short-circuits because the import
#      contract is fully captured by the envelope).
#   6. Run `verify_step_import_manus_fixture.sql` against the
#      container's postgres.
#   7. Clean up the container on exit.
#
# The runner is idempotent: a re-run skips the prereq schema if the
# audit_outbox table already exists. INSERTs use ON CONFLICT DO NOTHING
# keyed on (event_id) — the deterministic UUIDv5 makes re-runs land
# zero new rows (acceptance A8.2).
#
# Exit codes:
#   0  full fixture-replay round-trip verified
#   1  any step failed (docker / psql / cargo / assertion mismatch)

set -euo pipefail

log()  { echo "[d15-demo] $*" >&2; }
fail() { log "FAIL: $*"; exit 1; }

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
CRATE_DIR="$REPO_ROOT/services/importer_manus"
FIXTURE_PATH="$CRATE_DIR/tests/fixtures/manus_usage.json"
VERIFY_SQL="$REPO_ROOT/deploy/demo/verify_step_import_manus_fixture.sql"

CONTAINER_NAME="${SPENDGUARD_D15_PG_CONTAINER:-spendguard-d15-import-manus-pg}"
PG_USER="spendguard"
PG_PASSWORD="spendguard_demo"
PG_DB="spendguard_ledger"
PG_HOST_PORT="${SPENDGUARD_D15_PG_PORT:-5440}"  # avoid colliding with main demo (5433) and D14 demo (5439)

# --------------------------------------------------------------------
# Step 0 — preflight
# --------------------------------------------------------------------
command -v docker >/dev/null 2>&1 || fail "docker not on PATH; required for demo postgres"
command -v cargo  >/dev/null 2>&1 || fail "cargo not on PATH; required to build importer_manus"
[ -f "$FIXTURE_PATH" ] || fail "missing fixture at $FIXTURE_PATH"
[ -f "$VERIFY_SQL" ]   || fail "missing verifier at $VERIFY_SQL"

# --------------------------------------------------------------------
# Step 1 — spin up postgres (or reuse if already running)
# --------------------------------------------------------------------
cleanup() {
    if [ "${KEEP_CONTAINER:-0}" != "1" ]; then
        docker rm -f "$CONTAINER_NAME" >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT

if ! docker ps --format '{{.Names}}' | grep -q "^${CONTAINER_NAME}$"; then
    log "starting throwaway postgres:16-alpine as $CONTAINER_NAME on :$PG_HOST_PORT"
    docker rm -f "$CONTAINER_NAME" >/dev/null 2>&1 || true
    docker run -d --rm \
        --name "$CONTAINER_NAME" \
        -e POSTGRES_USER="$PG_USER" \
        -e POSTGRES_PASSWORD="$PG_PASSWORD" \
        -e POSTGRES_DB="$PG_DB" \
        -p "${PG_HOST_PORT}:5432" \
        postgres:16-alpine >/dev/null
else
    log "reusing existing postgres container $CONTAINER_NAME on :$PG_HOST_PORT"
fi

# Wait for postgres to accept connections.
log "waiting for postgres to be ready"
for i in $(seq 1 30); do
    if docker exec "$CONTAINER_NAME" pg_isready -U "$PG_USER" -d "$PG_DB" >/dev/null 2>&1; then
        log "postgres ready"
        break
    fi
    sleep 1
    if [ "$i" = "30" ]; then
        fail "postgres did not become ready within 30s"
    fi
done

PSQL="docker exec -i $CONTAINER_NAME psql -v ON_ERROR_STOP=1 -X -U $PG_USER -d $PG_DB"

# --------------------------------------------------------------------
# Step 2 — bootstrap minimal schema: audit_outbox + ledger_entries
# --------------------------------------------------------------------
log "bootstrapping minimal demo schema (audit_outbox + ledger_entries)"
$PSQL <<'SQL'
CREATE TABLE IF NOT EXISTS audit_outbox (
    event_id                  UUID         PRIMARY KEY,
    tenant_id                 TEXT         NOT NULL,
    reservation_source        TEXT         NOT NULL DEFAULT 'byok',
    import_source             TEXT         NULL,
    model                     TEXT         NULL,
    credits_consumed          BIGINT       NULL,
    credit_cost_micro_usd     BIGINT       NULL,
    amount_micro_usd          BIGINT       NULL,
    pricing_version           TEXT         NULL,
    tier                      TEXT         NULL,
    status                    TEXT         NULL,
    session_id                TEXT         NULL,
    workspace_id              TEXT         NULL,
    input_tokens              BIGINT       NULL,
    output_tokens             BIGINT       NULL,
    occurred_at               TIMESTAMPTZ  NOT NULL,
    ingestion_mode            TEXT         NULL,
    fixture_provenance_sha256 TEXT         NULL,
    dedupe_key                TEXT         NULL,
    recorded_at               TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS ledger_entries (
    entry_id   UUID         PRIMARY KEY,
    tenant_id  TEXT         NOT NULL,
    created_at TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);
SQL

# --------------------------------------------------------------------
# Step 3 — apply the D14 mig 0059 + D15 mig 0060 widenings (idempotent)
# --------------------------------------------------------------------
log "applying mig 0059 + 0060 (D14 + D15 audit_outbox.import_source widen)"
$PSQL <<'SQL'
ALTER TABLE audit_outbox
    DROP CONSTRAINT IF EXISTS audit_outbox_import_source_check;
ALTER TABLE audit_outbox
    ADD CONSTRAINT audit_outbox_import_source_check
        CHECK (import_source IS NULL OR import_source IN (
            'anthropic_console_usage',
            'openai_admin_usage',
            'devin_admin_usage',
            'manus_admin_usage',
            'genspark_admin_usage',
            'devin_team_api',
            'manus_team_api'
        ));

ALTER TABLE audit_outbox
    DROP CONSTRAINT IF EXISTS audit_outbox_reservation_source_check;
ALTER TABLE audit_outbox
    ADD CONSTRAINT audit_outbox_reservation_source_check
        CHECK (reservation_source IN ('byok', 'subscription_meter'));
SQL

# --------------------------------------------------------------------
# Step 4 — run the importer against the committed fixture
# --------------------------------------------------------------------
log "running importer in fixture mode"
ENVELOPES_JSON="$(cargo run --quiet --manifest-path "$CRATE_DIR/Cargo.toml" --bin importer_manus -- \
    --mode fixture \
    --fixture "$FIXTURE_PATH" 2>/dev/null)"

ENV_COUNT="$(printf '%s' "$ENVELOPES_JSON" | python3 -c 'import json,sys;print(len(json.load(sys.stdin)))')"
log "importer emitted $ENV_COUNT envelope(s)"
[ "$ENV_COUNT" = "7" ] || fail "expected exactly 7 envelopes (in_progress filtered); got $ENV_COUNT"

# --------------------------------------------------------------------
# Step 5 — translate envelopes → audit_outbox INSERTs
# --------------------------------------------------------------------
log "inserting audit_outbox rows from envelopes"
INSERT_SQL="$(printf '%s' "$ENVELOPES_JSON" | python3 "$REPO_ROOT/deploy/demo/import_manus_fixture_emit_sql.py")"
printf '%s' "$INSERT_SQL" | $PSQL

# --------------------------------------------------------------------
# Step 6 — run the verifier SQL
# --------------------------------------------------------------------
log "running verifier SQL"
$PSQL < "$VERIFY_SQL"

# --------------------------------------------------------------------
# Step 7 — idempotency check (A8.2)
# --------------------------------------------------------------------
ROWCOUNT_BEFORE="$(docker exec -i "$CONTAINER_NAME" psql -At -U "$PG_USER" -d "$PG_DB" -c "SELECT COUNT(*) FROM audit_outbox WHERE import_source='manus_team_api';")"
printf '%s' "$INSERT_SQL" | $PSQL >/dev/null
ROWCOUNT_AFTER="$(docker exec -i "$CONTAINER_NAME" psql -At -U "$PG_USER" -d "$PG_DB" -c "SELECT COUNT(*) FROM audit_outbox WHERE import_source='manus_team_api';")"
[ "$ROWCOUNT_BEFORE" = "$ROWCOUNT_AFTER" ] \
    || fail "idempotency violated: count was $ROWCOUNT_BEFORE before re-run, $ROWCOUNT_AFTER after"
log "idempotency: $ROWCOUNT_BEFORE rows before, $ROWCOUNT_AFTER after re-run (HOLD)"

log "D15 import_manus_fixture demo verified — exiting 0"
