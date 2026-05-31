#!/usr/bin/env bash
# Apply ledger, canonical_ingest, and control_plane migrations from a clean
# Postgres 16 container and run schema/RLS smoke checks.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${REPO_ROOT}"

CONTAINER="${CONTAINER:-spendguard-harden07-migrations}"
IMAGE="${POSTGRES_IMAGE:-postgres:16-alpine}"
PASSWORD="${POSTGRES_PASSWORD:-spendguard_migrations}"
EVIDENCE_PREFIX="${EVIDENCE_PREFIX:-/tmp/spendguard-migrations}"

log() { echo "[verify-migrations] $*" >&2; }

cleanup() {
    docker rm -f "${CONTAINER}" >/dev/null 2>&1 || true
}
trap cleanup EXIT

cleanup
scripts/release/verify-migration-inventory.sh

log "starting ${IMAGE}"
docker run -d --name "${CONTAINER}" \
    -e POSTGRES_USER=spendguard \
    -e POSTGRES_PASSWORD="${PASSWORD}" \
    -e POSTGRES_DB=postgres \
    "${IMAGE}" >/dev/null

for _ in $(seq 1 60); do
    if docker exec "${CONTAINER}" pg_isready -U spendguard -d postgres >/dev/null 2>&1; then
        break
    fi
    sleep 1
done
docker exec "${CONTAINER}" pg_isready -U spendguard -d postgres >/dev/null

psql_exec() {
    local db="$1"
    shift
    docker exec -e PGPASSWORD="${PASSWORD}" -i "${CONTAINER}" \
        psql -v ON_ERROR_STOP=1 -U spendguard -d "${db}" "$@"
}

psql_exec postgres -c "CREATE DATABASE spendguard_ledger;"
psql_exec postgres -c "CREATE DATABASE spendguard_canonical;"
psql_exec postgres -c "CREATE DATABASE spendguard_control_plane;"

apply_dir() {
    local db="$1"
    local dir="$2"
    local label="$3"
    log "applying ${label} migrations from ${dir}"
    local count=0
    local f
    for f in "${dir}"/*.sql; do
        [ -e "${f}" ] || continue
        count=$((count + 1))
        log "${label}: $(basename "${f}")"
        psql_exec "${db}" <"${f}" >/dev/null
    done
    if [ "${count}" -eq 0 ]; then
        log "FATAL: no SQL files found for ${label}"
        exit 1
    fi
    log "${label}: applied ${count} files"
}

apply_dir spendguard_ledger services/ledger/migrations ledger
apply_dir spendguard_canonical services/canonical_ingest/migrations canonical_ingest
apply_dir spendguard_control_plane services/control_plane/migrations control_plane

log "ledger smoke checks"
psql_exec spendguard_ledger -c "
DO \$\$
DECLARE
  missing text[];
BEGIN
  IF to_regclass('public.audit_outbox') IS NULL THEN
    RAISE EXCEPTION 'missing ledger audit_outbox';
  END IF;
  IF to_regclass('public.tokenizer_t1_samples') IS NULL THEN
    RAISE EXCEPTION 'missing ledger tokenizer_t1_samples';
  END IF;
  SELECT ARRAY(
    SELECT c FROM unnest(ARRAY[
      'predicted_a_tokens',
      'run_projection_at_decision_atomic',
      'prediction_strategy_used'
    ]) AS c
    WHERE NOT EXISTS (
      SELECT 1 FROM information_schema.columns
      WHERE table_schema='public'
        AND table_name='audit_outbox'
        AND column_name=c
    )
  ) INTO missing;
  IF array_length(missing, 1) IS NOT NULL THEN
    RAISE EXCEPTION 'missing ledger audit_outbox prediction columns: %', missing;
  END IF;
END
\$\$;
"
psql_exec spendguard_ledger -c "
SELECT
  to_regclass('public.audit_outbox') AS audit_outbox,
  to_regclass('public.tokenizer_t1_samples') AS tokenizer_t1_samples,
  (
    SELECT COUNT(*) = 3 FROM information_schema.columns
    WHERE table_name='audit_outbox'
      AND column_name IN ('predicted_a_tokens', 'run_projection_at_decision_atomic', 'prediction_strategy_used')
  ) AS has_prediction_columns;
" | tee "${EVIDENCE_PREFIX}-ledger-smoke.txt"

log "canonical smoke checks"
psql_exec spendguard_canonical -c "
DO \$\$
DECLARE
  missing text[];
BEGIN
  IF to_regclass('public.canonical_events') IS NULL THEN
    RAISE EXCEPTION 'missing canonical_events';
  END IF;
  IF to_regclass('public.canonical_event_replay_dedup') IS NULL THEN
    RAISE EXCEPTION 'missing canonical_event_replay_dedup';
  END IF;
  SELECT ARRAY(
    SELECT c FROM unnest(ARRAY[
      'payload_json',
      'prediction_strategy_used',
      'run_id_mirror'
    ]) AS c
    WHERE NOT EXISTS (
      SELECT 1 FROM information_schema.columns
      WHERE table_schema='public'
        AND table_name='canonical_events'
        AND column_name=c
    )
  ) INTO missing;
  IF array_length(missing, 1) IS NOT NULL THEN
    RAISE EXCEPTION 'missing canonical_events mirror columns: %', missing;
  END IF;
END
\$\$;
"
psql_exec spendguard_canonical -c "
SELECT
  to_regclass('public.canonical_events') AS canonical_events,
  to_regclass('public.canonical_event_replay_dedup') AS replay_dedup,
  (
    SELECT COUNT(*) = 3 FROM information_schema.columns
    WHERE table_name='canonical_events'
      AND column_name IN ('payload_json', 'prediction_strategy_used', 'run_id_mirror')
  ) AS has_mirror_columns;
" | tee "${EVIDENCE_PREFIX}-canonical-smoke.txt"

log "control-plane smoke checks"
psql_exec spendguard_control_plane -c "
DO \$\$
BEGIN
  IF to_regclass('public.predictor_plugin_endpoints') IS NULL THEN
    RAISE EXCEPTION 'missing predictor_plugin_endpoints';
  END IF;
  IF to_regclass('public.control_plane_audit_outbox') IS NULL THEN
    RAISE EXCEPTION 'missing control_plane_audit_outbox';
  END IF;
  IF NOT EXISTS (
    SELECT 1 FROM pg_policies
    WHERE schemaname='public'
      AND tablename='control_plane_audit_outbox'
      AND policyname='control_plane_audit_outbox_forwarder_update'
  ) THEN
    RAISE EXCEPTION 'missing control_plane_audit_outbox_forwarder_update RLS policy';
  END IF;
END
\$\$;
"
psql_exec spendguard_control_plane -c "
SELECT
  to_regclass('public.predictor_plugin_endpoints') AS predictor_plugin_endpoints,
  to_regclass('public.control_plane_audit_outbox') AS control_plane_audit_outbox,
  EXISTS (
    SELECT 1 FROM pg_policies
    WHERE tablename='control_plane_audit_outbox'
      AND policyname='control_plane_audit_outbox_forwarder_update'
  ) AS has_forwarder_update_policy;
" | tee "${EVIDENCE_PREFIX}-control-plane-smoke.txt"

log "PASS"
