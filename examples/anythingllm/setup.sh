#!/bin/bash
# =====================================================================
# SpendGuard + AnythingLLM — Pattern 2 drop-in bootstrap (operator).
# =====================================================================
#
# Configures a running AnythingLLM instance to route every Workspace
# chat through a running SpendGuard egress proxy. Five steps:
#
#   0. /api/ping reachable
#   1. /api/setup-account (first-run only — idempotent: ignored if the
#      account already exists)
#   2. /api/v1/system/update-env with generic-openai-config.json
#   3. /api/v1/workspace/new for a smoke workspace
#   4. /api/v1/workspace/{slug}/chat — one chat round-trip
#   5. SpendGuard ledger assertion: reserve + commit_estimated rows
#
# Companion file: deploy/demo/anythingllm_smoke.sh (CI-driven version
# under the SpendGuard demo Makefile).
#
# Prerequisites:
#   - curl, jq, psql (postgresql-client) installed.
#   - AnythingLLM reachable at $ANYTHINGLLM_URL.
#   - SpendGuard egress proxy reachable at $PROXY_URL (the value
#     AnythingLLM's Generic OpenAI provider points at).
#   - SpendGuard demo Postgres reachable at $POSTGRES_URL (for the
#     ledger assertion in step 5). Skip step 5 with --no-verify if
#     you are configuring a production AnythingLLM and just want the
#     provider wiring without the assertion.

set -euo pipefail

ANYTHINGLLM_URL="${ANYTHINGLLM_URL:-http://localhost:3001}"
PROXY_URL="${PROXY_URL:-http://localhost:9000}"
POSTGRES_URL="${POSTGRES_URL:-postgres://spendguard:spendguard_demo@localhost:5432/spendguard_ledger}"
SMOKE_TENANT_ID="${SMOKE_TENANT_ID:-00000000-0000-4000-8000-000000000001}"
VERIFY=true

for arg in "$@"; do
    case "$arg" in
        --no-verify) VERIFY=false ;;
        --help|-h)
            echo "Usage: $0 [--no-verify]"
            echo
            echo "Environment:"
            echo "  ANYTHINGLLM_URL  AnythingLLM admin URL (default: http://localhost:3001)"
            echo "  PROXY_URL        SpendGuard egress proxy URL (default: http://localhost:9000)"
            echo "  POSTGRES_URL     SpendGuard ledger DB URL (only for --no-verify=false)"
            echo "  SMOKE_TENANT_ID  Tenant UUID the verify SQL filters on"
            exit 0 ;;
    esac
done

log() { echo "[anythingllm-setup] $*" >&2; }
fail() { log "FAIL: $*"; exit 1; }

# ---------------------------------------------------------------------
# Step 0: AnythingLLM reachable
# ---------------------------------------------------------------------
log "step 0: AnythingLLM /api/ping..."
curl -sS --max-time 5 "${ANYTHINGLLM_URL}/api/ping" | grep -q online \
    || fail "AnythingLLM not reachable at ${ANYTHINGLLM_URL}"

# ---------------------------------------------------------------------
# Step 1: bootstrap admin account (first-run only)
# ---------------------------------------------------------------------
# AnythingLLM 1.8+ requires an initial /api/setup-account call before
# the /api/v1 routes accept writes. Idempotent: a 4xx on re-run means
# the account already exists; we silently swallow it.
log "step 1: bootstrap admin account (idempotent)..."
curl -sS --max-time 10 -X POST "${ANYTHINGLLM_URL}/api/setup-account" \
    -H 'Content-Type: application/json' \
    -d '{"username":"spendguard","password":"spendguard-default-9000"}' \
    >/dev/null || true

# ---------------------------------------------------------------------
# Step 2: configure Generic OpenAI provider
# ---------------------------------------------------------------------
log "step 2: POST /api/v1/system/update-env with generic-openai-config.json..."
CONFIG_FILE="$(dirname "$0")/generic-openai-config.json"
[ -f "${CONFIG_FILE}" ] || fail "missing config: ${CONFIG_FILE}"

# Substitute the PROXY_URL into the config payload so a caller-override
# of PROXY_URL flows through to AnythingLLM without editing the JSON.
PAYLOAD=$(jq --arg base "${PROXY_URL}/v1" \
    'del(._comment) | .GenericOpenAiBasePath = $base' \
    "${CONFIG_FILE}")

RESP=$(curl -sS --max-time 10 -X POST "${ANYTHINGLLM_URL}/api/v1/system/update-env" \
    -H 'Content-Type: application/json' \
    -d "${PAYLOAD}")
echo "${RESP}" | jq -e '.newValues' >/dev/null \
    || fail "update-env failed: ${RESP}"
log "  provider configured -> ${PROXY_URL}/v1"

# ---------------------------------------------------------------------
# Step 3: create a smoke Workspace
# ---------------------------------------------------------------------
log "step 3: create smoke workspace..."
WS=$(curl -sS --max-time 10 -X POST "${ANYTHINGLLM_URL}/api/v1/workspace/new" \
    -H 'Content-Type: application/json' \
    -d '{"name":"spendguard-smoke"}' | jq -r '.workspace.slug')
[ -n "${WS}" ] && [ "${WS}" != "null" ] || fail "workspace not created"
log "  workspace=${WS}"

# ---------------------------------------------------------------------
# Step 4: smoke chat
# ---------------------------------------------------------------------
log "step 4: send one chat through SpendGuard..."
RESP=$(curl -sS --max-time 30 -X POST "${ANYTHINGLLM_URL}/api/v1/workspace/${WS}/chat" \
    -H 'Content-Type: application/json' \
    -d '{"message":"Say hi in two words.","mode":"chat"}')
echo "${RESP}" | jq -e '.textResponse | length > 0' >/dev/null \
    || fail "no chat response: ${RESP}"
log "  chat OK: $(echo "${RESP}" | jq -r '.textResponse' | head -c 80)"

# ---------------------------------------------------------------------
# Step 5: verify reserve + commit_estimated in the SpendGuard ledger
# ---------------------------------------------------------------------
if [ "${VERIFY}" = "false" ]; then
    log "step 5: SKIPPED (--no-verify)"
    log "OK: provider configured and chat round-trip succeeded"
    exit 0
fi

log "step 5: verify reserve+commit in the SpendGuard ledger..."
psql "${POSTGRES_URL}" -v ON_ERROR_STOP=1 -v tenant_id="${SMOKE_TENANT_ID}" <<'SQL'
DO $$
DECLARE
    v_reserve INT;
    v_commit INT;
    v_tenant UUID := current_setting('myapp.tenant_id', true)::UUID;
BEGIN
    IF v_tenant IS NULL THEN
        v_tenant := :'tenant_id'::UUID;
    END IF;
    SELECT COUNT(*) INTO v_reserve
      FROM ledger_transactions
     WHERE tenant_id = v_tenant
       AND operation_kind = 'reserve'
       AND event_time > now() - interval '10 minutes';
    SELECT COUNT(*) INTO v_commit
      FROM ledger_transactions
     WHERE tenant_id = v_tenant
       AND operation_kind = 'commit_estimated'
       AND event_time > now() - interval '10 minutes';

    IF v_reserve < 1 THEN
        RAISE EXCEPTION 'no reserve row in last 10m (tenant=%)', v_tenant;
    END IF;
    IF v_commit < 1 THEN
        RAISE EXCEPTION 'no commit_estimated row in last 10m (tenant=%)', v_tenant;
    END IF;
    RAISE NOTICE 'LEDGER OK: reserve=% commit_estimated=%', v_reserve, v_commit;
END $$;
SQL

log "OK: reserve+commit verified"
