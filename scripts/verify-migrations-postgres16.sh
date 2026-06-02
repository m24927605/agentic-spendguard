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

clean_evidence() {
    sed -E 's/[[:blank:]]+$//'
}

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
image_repo_digest="$(docker image inspect "${IMAGE}" --format '{{range .RepoDigests}}{{println .}}{{end}}' 2>/dev/null | awk 'NF {print; exit}' || true)"
if [[ -z "${image_repo_digest}" && "${IMAGE}" == *@sha256:* ]]; then
    image_repo_digest="${IMAGE}"
fi

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

version_error_file="$(mktemp)"
server_version_num=""
server_version=""
for _ in $(seq 1 30); do
    if server_version_num="$(psql_exec postgres -At -c "SHOW server_version_num;" 2>"${version_error_file}" | tr -d '[:space:]')" &&
        server_version="$(psql_exec postgres -At -c "SHOW server_version;" 2>>"${version_error_file}" | tr -d '\r')"; then
        break
    fi
    sleep 1
done
if [[ -z "${server_version_num}" ]]; then
    log "FATAL: unable to query Postgres server version"
    cat "${version_error_file}" >&2
    exit 1
fi
rm -f "${version_error_file}"
if [[ ! "${server_version_num}" =~ ^[0-9]+$ || "${server_version_num}" -lt 160000 || "${server_version_num}" -ge 170000 ]]; then
    log "FATAL: expected Postgres 16.x, got server_version_num=${server_version_num} (${server_version})"
    exit 1
fi
{
    printf 'image=%s\n' "${IMAGE}"
    printf 'image_repo_digest=%s\n' "${image_repo_digest:-unavailable}"
    printf 'server_version_num=%s\n' "${server_version_num}"
    printf 'server_version=%s\n' "${server_version}"
} | tee "${EVIDENCE_PREFIX}-postgres-version.txt"

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
" | clean_evidence | tee "${EVIDENCE_PREFIX}-ledger-smoke.txt"

log "ledger tokenizer_t1_samples PUBLIC SELECT revoke smoke"
psql_exec spendguard_ledger -c "
DO \$\$
BEGIN
  IF EXISTS (
    SELECT 1
      FROM information_schema.role_table_grants
     WHERE table_schema='public'
       AND table_name IN (
         'tokenizer_t1_samples',
         'tokenizer_t1_samples_2026_05',
         'tokenizer_t1_samples_2026_06',
         'tokenizer_t1_samples_2026_07'
       )
       AND grantee='PUBLIC'
       AND privilege_type='SELECT'
  ) THEN
    RAISE EXCEPTION 'PUBLIC SELECT remains on tokenizer_t1_samples or a current partition';
  END IF;
END
\$\$;
SELECT 'tokenizer_t1_samples_public_select_revoked' AS check_name, TRUE AS passed;
" | clean_evidence | tee "${EVIDENCE_PREFIX}-ledger-tokenizer-public-revoke.txt"

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
" | clean_evidence | tee "${EVIDENCE_PREFIX}-canonical-smoke.txt"

log "canonical sample-size CHECK smoke"
psql_exec spendguard_canonical -c "
DO \$\$
DECLARE
  tenant uuid := '00000000-0000-4000-8000-000000000167'::uuid;
  output_cache_check_hit boolean := false;
  run_length_check_hit boolean := false;
BEGIN
  SET LOCAL search_path = pg_catalog, pg_temp;
  PERFORM set_config('app.current_tenant_id', tenant::text, TRUE);

  BEGIN
    INSERT INTO public.output_distribution_cache (
      tenant_id, model, agent_id, prompt_class,
      sample_size_7d, sample_size_30d, computed_at
    ) VALUES (
      tenant, 'post-ga-02-model', 'post-ga-02-agent', 'chat_short',
      -1, 0, clock_timestamp()
    );
  EXCEPTION WHEN check_violation THEN
    output_cache_check_hit := true;
  END;
  IF NOT output_cache_check_hit THEN
    RAISE EXCEPTION 'negative output_distribution_cache.sample_size_7d was accepted';
  END IF;

  BEGIN
    INSERT INTO public.run_length_distribution_cache (
      tenant_id, agent_id, sample_size_30d, computed_at
    ) VALUES (
      tenant, 'post-ga-02-agent', -1, clock_timestamp()
    );
  EXCEPTION WHEN check_violation THEN
    run_length_check_hit := true;
  END;
  IF NOT run_length_check_hit THEN
    RAISE EXCEPTION 'negative run_length_distribution_cache.sample_size_30d was accepted';
  END IF;
END
\$\$;
SELECT 'sample_size_check_constraints_reject_negative' AS check_name, TRUE AS passed;
" | clean_evidence | tee "${EVIDENCE_PREFIX}-canonical-sample-size-checks.txt"

log "canonical RLS no-nil-sentinel smoke"
psql_exec spendguard_canonical -c "
DO \$\$
DECLARE
  policy_sql text;
BEGIN
  SELECT pg_get_expr(polqual, polrelid) || ' ' || pg_get_expr(polwithcheck, polrelid)
    INTO policy_sql
    FROM pg_policy
   WHERE polname='output_distribution_cache_tenant_isolation';
  IF policy_sql IS NULL OR policy_sql LIKE '%00000000-0000-0000-0000-000000000000%' THEN
    RAISE EXCEPTION 'output_distribution_cache RLS policy missing or still uses nil sentinel: %', policy_sql;
  END IF;

  SELECT pg_get_expr(polqual, polrelid) || ' ' || pg_get_expr(polwithcheck, polrelid)
    INTO policy_sql
    FROM pg_policy
   WHERE polname='run_length_distribution_cache_tenant_isolation';
  IF policy_sql IS NULL OR policy_sql LIKE '%00000000-0000-0000-0000-000000000000%' THEN
    RAISE EXCEPTION 'run_length_distribution_cache RLS policy missing or still uses nil sentinel: %', policy_sql;
  END IF;
END
\$\$;
SELECT 'cache_rls_no_nil_uuid_sentinel' AS check_name, TRUE AS passed;
" | clean_evidence | tee "${EVIDENCE_PREFIX}-canonical-rls-no-nil-sentinel.txt"

log "canonical output_distribution_cache freshness index planner smoke"
psql_exec spendguard_canonical < scripts/db/explain-post-ga-08-cache-index.sql \
  | clean_evidence | tee "${EVIDENCE_PREFIX}-canonical-output-cache-index-plan.txt"

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
" | clean_evidence | tee "${EVIDENCE_PREFIX}-control-plane-smoke.txt"

log "PASS"
