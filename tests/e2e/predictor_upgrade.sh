#!/usr/bin/env bash
# SLICE_15 — End-to-end deployment script for the full predictor-upgrade
# demo stack.
#
# Spec ancestors:
#   - docs/slices/SLICE_15_end_to_end_benchmark.md §2 (E2E deployment + agent run)
#   - docs/predictor-architecture-spec-v1alpha1.md §0.2 lock criteria #4
#     (verify-chain regression green after new column writes)
#
# What this script does:
#   1. Tears down any pre-existing demo cluster (idempotent).
#   2. Brings up the full demo compose topology under deploy/demo/, which
#      includes every predictor-upgrade service:
#         - postgres (ledger + canonical_ingest databases)
#         - canonical-ingest (mTLS gRPC)
#         - ledger (mTLS gRPC)
#         - tokenizer (SLICE_03)
#         - output-predictor (SLICE_06)
#         - run-cost-projector (SLICE_09)
#         - stats-aggregator (SLICE_06+)
#         - sidecar (UDS gRPC; SLICE_07 + SLICE_10 wiring)
#         - egress-proxy (axum :9000)
#         - control-plane (REST API)
#         - outbox-forwarder (audit chain replication)
#         - bundle-registry / webhook-receiver / dashboard
#   3. Waits for every healthcheck-bearing service to reach `healthy`.
#   4. Returns exit 0 on ready, exit 1 on timeout.
#
# Per `feedback_demo_quality_gate.md`:
#   Codex green is not enough — every service must really start. This
#   script is the integration-time gate that surfaces wire-assumption
#   mismatches between the 11 predictor-upgrade specs and reality.
#
# Per `feedback_demo_quality_gate.md` known flake (project_known_demo_flakes):
#   ttl_sweep mode is known to flake in the downstream reserve flow.
#   This script intentionally uses the default mode (no SIDECAR_TTL_SECONDS
#   override), so the known flake does not apply here.
#
# Usage:
#   bash tests/e2e/predictor_upgrade.sh           # bring up + wait + exit 0
#   bash tests/e2e/predictor_upgrade.sh --no-up   # only wait + verify (assumes already up)
#   bash tests/e2e/predictor_upgrade.sh --down    # tear down + exit
#
# Exit codes:
#   0 = stack healthy, ready for predictor_upgrade_agent.py
#   1 = timeout waiting for healthchecks
#   2 = docker / docker-compose not available
#   3 = invalid argument

set -euo pipefail

# --------------------------------------------------------------------------
# Locate repo root robustly. This script lives at tests/e2e/, so go up two.
# --------------------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
COMPOSE_DIR="${REPO_ROOT}/deploy/demo"
COMPOSE_FILE="${COMPOSE_DIR}/compose.yaml"

# --------------------------------------------------------------------------
# Healthcheck wait budget. Default 5 minutes (300s) — first build pulls
# heavy Rust images and can take a while on cold cache. Override via
# E2E_HEALTH_TIMEOUT_S if running in CI with a warm image cache.
# --------------------------------------------------------------------------
HEALTH_TIMEOUT_S="${E2E_HEALTH_TIMEOUT_S:-300}"
POLL_INTERVAL_S="${E2E_POLL_INTERVAL_S:-3}"

# Services we explicitly wait on — these all expose healthchecks in
# deploy/demo/compose.yaml. The dependency graph (depends_on with
# `service_healthy`) propagates readiness to leaves; we only need to
# verify the "core" services that downstream tests interact with.
WAIT_FOR_SERVICES=(
    "spendguard-postgres"
    "spendguard-endpoint-catalog"
    # Note: ledger / canonical-ingest do not expose healthchecks (no
    # gRPC reflection enabled), so we rely on the sidecar /readyz
    # cascade instead.
    "spendguard-sidecar"
    "spendguard-tokenizer"
    "spendguard-output-predictor"
    "spendguard-run-cost-projector"
    "spendguard-stats-aggregator"
)

log() {
    # Plain stdout so CI captures the line. Avoid stderr to keep the
    # signal clean for downstream `python tests/e2e/...` pipes.
    printf '[predictor_upgrade.sh] %s\n' "$*"
}

err() {
    printf '[predictor_upgrade.sh] ERROR: %s\n' "$*" >&2
}

require_docker() {
    if ! command -v docker >/dev/null 2>&1; then
        err "docker not installed; this E2E gate requires docker + docker-compose v2"
        err "Skip with: SKIP_E2E=1 (downstream scripts will document as N/A)"
        exit 2
    fi
    # docker-compose v2 is `docker compose`, not `docker-compose`. The
    # demo Makefile uses the v2 form throughout; we follow.
    if ! docker compose version >/dev/null 2>&1; then
        err "docker compose v2 plugin not installed"
        exit 2
    fi
}

bring_down() {
    log "Bringing down demo stack (idempotent)..."
    (cd "${COMPOSE_DIR}" && docker compose -f "${COMPOSE_FILE}" down -v --remove-orphans) \
        || log "down returned non-zero (likely nothing to tear down — continuing)"
}

bring_up() {
    log "Bringing up demo stack from ${COMPOSE_FILE}..."
    log "Note: first build can take 5+ minutes (Rust image cold cache)."
    (cd "${COMPOSE_DIR}" && docker compose -f "${COMPOSE_FILE}" up -d --build) \
        || {
            err "docker compose up failed; see logs above"
            exit 1
        }
}

# Wait for one named container to reach `healthy`. Returns 0 on healthy,
# non-zero on timeout. We poll `docker inspect` rather than `docker
# compose ps --format json` because the latter's format keys differ
# across docker versions; `docker inspect` is stable since v1.x.
wait_for_healthy() {
    local container="$1"
    local deadline=$(( $(date +%s) + HEALTH_TIMEOUT_S ))
    log "  waiting for ${container}..."
    while (( $(date +%s) < deadline )); do
        local state
        # If the container doesn't exist yet (still creating), inspect
        # returns 1 → treat as "not yet ready, keep polling".
        state="$(docker inspect --format='{{.State.Health.Status}}' "${container}" 2>/dev/null || echo 'creating')"
        case "${state}" in
            healthy)
                log "    ${container} → healthy"
                return 0
                ;;
            unhealthy)
                err "    ${container} → UNHEALTHY (see docker logs ${container})"
                return 1
                ;;
            *)
                # creating / starting / "" → keep waiting
                sleep "${POLL_INTERVAL_S}"
                ;;
        esac
    done
    err "    ${container} did not become healthy within ${HEALTH_TIMEOUT_S}s"
    return 1
}

wait_for_all() {
    log "Waiting for healthchecks (budget: ${HEALTH_TIMEOUT_S}s per service)..."
    local failed=0
    for svc in "${WAIT_FOR_SERVICES[@]}"; do
        if ! wait_for_healthy "${svc}"; then
            failed=1
        fi
    done
    if (( failed != 0 )); then
        err "One or more services failed to become healthy."
        err "Run: docker compose -f ${COMPOSE_FILE} logs"
        exit 1
    fi
    log "All ${#WAIT_FOR_SERVICES[@]} services healthy."
}

verify_postgres_predictor_columns() {
    # Sanity check: the predictor mirror columns exist (migrations 0013 +
    # 0018 + ledger 0046 applied). This is a cheap pre-flight before
    # python tests/e2e/verify_audit_columns.py runs the full 21-column
    # population check.
    log "Verifying predictor mirror columns exist in canonical_events..."
    local pg_container="spendguard-postgres"
    local sql="SELECT count(*) FROM information_schema.columns WHERE table_name = 'canonical_events' AND column_name IN ('predicted_a_tokens','predicted_b_tokens','predicted_c_tokens','reserved_strategy','tokenizer_tier','run_projection_at_decision_atomic','actual_input_tokens','actual_output_tokens','delta_b_ratio','delta_c_ratio','prompt_class_fingerprint');"
    local got
    got=$(docker exec -e PGPASSWORD=spendguard_demo "${pg_container}" \
        psql -U spendguard -d spendguard_canonical -tAc "${sql}" 2>/dev/null \
        || echo "0")
    if [[ "${got}" -lt 11 ]]; then
        err "Expected >= 11 predictor mirror columns; found: ${got}"
        err "Migrations 0013 / 0018 / ledger-0046 may not have run."
        exit 1
    fi
    log "  predictor mirror columns present (${got}/11)."
}

main() {
    local mode="up"
    if [[ $# -gt 0 ]]; then
        case "$1" in
            --no-up) mode="wait_only" ;;
            --down) mode="down" ;;
            -h|--help)
                cat <<EOF
Usage: $0 [--no-up | --down | --help]

  (no arg)   Bring up demo + wait + verify.
  --no-up    Only wait + verify (stack assumed already up).
  --down     Tear down + exit.
EOF
                exit 0
                ;;
            *) err "Unknown arg: $1"; exit 3 ;;
        esac
    fi

    require_docker

    case "${mode}" in
        down)
            bring_down
            log "Tear-down complete."
            exit 0
            ;;
        wait_only)
            log "Skipping bring-up (--no-up); waiting on existing stack."
            wait_for_all
            verify_postgres_predictor_columns
            ;;
        up)
            bring_down  # ensure clean slate
            bring_up
            wait_for_all
            verify_postgres_predictor_columns
            ;;
    esac

    log "READY."
    log "  - Next: python3 tests/e2e/predictor_upgrade_agent.py"
    log "  - Then: python3 tests/e2e/verify_audit_columns.py --tenant 00000000-0000-4000-8000-000000000001"
    exit 0
}

main "$@"
