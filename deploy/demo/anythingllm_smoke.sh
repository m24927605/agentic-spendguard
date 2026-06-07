#!/bin/bash
# =====================================================================
# DEMO_MODE=anythingllm_real smoke — D33 SLICE 1.
# =====================================================================
#
# Tests the AnythingLLM drop-in claim: an unmodified AnythingLLM 1.8+
# pointing its Generic OpenAI provider at the SpendGuard egress proxy
# routes every Workspace chat through SpendGuard's pre-call gate and
# commit lane. Five steps:
#
#   0. AnythingLLM /api/ping reachable
#   1. /api/setup-account (first-run idempotent)
#   2. /api/v1/system/update-env → Generic OpenAI provider configured
#   3. /api/v1/workspace/new → smoke workspace
#   4. /api/v1/workspace/{slug}/chat → one chat round-trip via proxy
#   5. SpendGuard ledger assertion: reserve + commit_estimated rows
#
# Requires OPENAI_API_KEY in the calling environment (proxy forwards
# to real OpenAI gpt-4o-mini).

set -euo pipefail

ANYTHINGLLM_URL="${ANYTHINGLLM_URL:-http://anythingllm:3001}"
PROXY_URL="${PROXY_URL:-http://egress-proxy:9000}"
POSTGRES_URL="${POSTGRES_URL:-postgres://spendguard:spendguard_demo@postgres:5432/spendguard_ledger}"

log()  { echo "[anythingllm-smoke] $*" >&2; }
fail() { log "FAIL: $*"; exit 1; }

# ---------------------------------------------------------------------
# Step 0: AnythingLLM /api/ping
# ---------------------------------------------------------------------
log "step 0: AnythingLLM /api/ping..."
PING=$(curl -sS --max-time 5 "${ANYTHINGLLM_URL}/api/ping" || true)
echo "${PING}" | grep -q online || fail "AnythingLLM /api/ping not online: ${PING}"

# ---------------------------------------------------------------------
# Step 1: bootstrap admin account (first-run idempotent — AnythingLLM
# 1.8+ requires an initial account before /api/v1 routes accept POSTs;
# a 4xx on re-run means the account already exists).
# ---------------------------------------------------------------------
log "step 1: bootstrap admin account..."
curl -sS --max-time 10 -X POST "${ANYTHINGLLM_URL}/api/setup-account" \
    -H 'Content-Type: application/json' \
    -d '{"username":"smoke","password":"smoke-pw-1234"}' \
    >/dev/null || true

# ---------------------------------------------------------------------
# Step 2: configure Generic OpenAI provider via /api/v1/system/update-env
# ---------------------------------------------------------------------
log "step 2: configure Generic OpenAI provider → ${PROXY_URL}/v1..."
RESP=$(curl -sS --max-time 10 -X POST "${ANYTHINGLLM_URL}/api/v1/system/update-env" \
    -H 'Content-Type: application/json' \
    -d "{
        \"LLMProvider\": \"generic-openai\",
        \"GenericOpenAiBasePath\": \"${PROXY_URL}/v1\",
        \"GenericOpenAiKey\": \"sk-anythingllm-spendguard\",
        \"GenericOpenAiModelPref\": \"gpt-4o-mini\",
        \"GenericOpenAiTokenLimit\": 128000
    }")
echo "${RESP}" | jq -e '.newValues' >/dev/null \
    || fail "update-env failed: ${RESP}"

# ---------------------------------------------------------------------
# Step 3: create a smoke Workspace
# ---------------------------------------------------------------------
log "step 3: create workspace..."
WS=$(curl -sS --max-time 10 -X POST "${ANYTHINGLLM_URL}/api/v1/workspace/new" \
    -H 'Content-Type: application/json' \
    -d '{"name":"smoke-ws"}' | jq -r '.workspace.slug')
[ -n "${WS}" ] && [ "${WS}" != "null" ] || fail "workspace not created"
log "  workspace=${WS}"

# ---------------------------------------------------------------------
# Step 4: one chat round-trip — AnythingLLM → SpendGuard → OpenAI
# ---------------------------------------------------------------------
log "step 4: chat round-trip via SpendGuard..."
RESP=$(curl -sS --max-time 30 -X POST "${ANYTHINGLLM_URL}/api/v1/workspace/${WS}/chat" \
    -H 'Content-Type: application/json' \
    -d '{"message":"Say hi in two words.","mode":"chat"}')
echo "${RESP}" | jq -e '.textResponse | length > 0' >/dev/null \
    || fail "no chat response: ${RESP}"
log "  chat OK"

# ---------------------------------------------------------------------
# Step 5: verify SpendGuard ledger has reserve + commit_estimated rows
# ---------------------------------------------------------------------
log "step 5: verify reserve+commit in ledger..."
psql "${POSTGRES_URL}" -v ON_ERROR_STOP=1 -f /smoke/verify.sql \
    || fail "verify SQL failed"

log "OK: reserve+commit verified"
