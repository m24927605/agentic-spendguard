#!/bin/bash
# =====================================================================
# DEMO_MODE=lobechat_real smoke - D34 SLICE 1.
# =====================================================================
#
# Tests the LobeChat drop-in claim: an unmodified LobeChat 1.40+
# container with OPENAI_PROXY_URL set in the env block routes every
# server-side chat through the SpendGuard egress proxy. Three steps
# (no admin API call — env var did the work at boot):
#
#   0. LobeChat /api/health reachable
#   1. (inferential — env var honoured iff Step 3 finds the audit row)
#   2. /api/chat/openai - one chat round-trip via SpendGuard
#   3. SpendGuard ledger assertion: reserve + commit_estimated rows
#
# Requires OPENAI_API_KEY in the calling environment (proxy forwards
# to real OpenAI gpt-4o-mini).
#
# Pattern-locked from deploy/demo/anythingllm_smoke.sh (D33). Simpler
# than D33 because LobeChat has no admin update-env API — the env var
# is read at container boot and never mutated at runtime.

set -euo pipefail

LOBECHAT_URL="${LOBECHAT_URL:-http://lobechat:3210}"
LOBECHAT_ACCESS_CODE="${LOBECHAT_ACCESS_CODE:-spendguard-lobechat-smoke}"
PROXY_URL="${PROXY_URL:-http://egress-proxy:9000}"
POSTGRES_URL="${POSTGRES_URL:-postgres://spendguard:spendguard_demo@postgres:5432/spendguard_ledger}"

log()  { echo "[lobechat-smoke] $*" >&2; }
fail() { log "FAIL: $*"; exit 1; }

# ---------------------------------------------------------------------
# Step 0: LobeChat /api/health
# ---------------------------------------------------------------------
log "step 0: LobeChat /api/health..."
# LobeChat /api/health returns 200 with body {"status":"ok"} on
# server-mode boot once the Next.js runtime is ready. Retry up to 6x
# with a 5s sleep because LobeChat's cold start is slow on first boot
# (it pulls embeddings/assets on first request even though the smoke
# never uses them).
for i in 1 2 3 4 5 6; do
    if curl -sS --max-time 5 "${LOBECHAT_URL}/api/health" >/dev/null 2>&1; then
        log "  health OK on attempt ${i}"
        break
    fi
    [ "${i}" -lt 6 ] || fail "LobeChat /api/health never returned 200 after 6 attempts"
    log "  attempt ${i} failed; retrying in 5s..."
    sleep 5
done

# ---------------------------------------------------------------------
# Step 1: confirm OPENAI_PROXY_URL was honoured at boot. LobeChat has
# no admin endpoint that exposes the env var directly, so we treat
# Step 3's audit-row assertion as the load-bearing witness: if the
# env var was dropped, the call goes to api.openai.com directly and
# the ledger stays empty.
# ---------------------------------------------------------------------
log "step 1: confirm OPENAI_PROXY_URL on the container..."
log "  (env-var presence inferred from Step 3 audit row — Step 2 is the probe)"

# ---------------------------------------------------------------------
# Step 2: send one chat through /api/chat/openai
# ---------------------------------------------------------------------
log "step 2: chat round-trip via SpendGuard..."
RESP=$(curl -sS --max-time 30 -X POST "${LOBECHAT_URL}/api/chat/openai" \
    -H 'Content-Type: application/json' \
    -H "X-LOBE-CHAT-AUTH: ${LOBECHAT_ACCESS_CODE}" \
    -d '{
        "model": "gpt-4o-mini",
        "messages": [{"role":"user","content":"Say hi in two words."}],
        "stream": false
    }' || true)

# LobeChat may return either a JSON envelope or a streamed body even
# with stream:false on some versions. Accept any non-empty body and
# lean on the Step 3 audit assertion as the load-bearing check.
if [ -z "${RESP}" ]; then
    fail "empty response from LobeChat /api/chat/openai"
fi

# Best-effort positive-shape parse: log content if it looks
# OpenAI-shaped; otherwise log size and continue.
CONTENT=$(echo "${RESP}" | jq -r '.choices[0].message.content // empty' 2>/dev/null || echo "")
if [ -n "${CONTENT}" ]; then
    log "  chat OK: $(echo "${CONTENT}" | head -c 80)"
else
    log "  chat returned non-JSON body ($(echo -n "${RESP}" | wc -c) bytes); audit row arbitrates"
fi

# ---------------------------------------------------------------------
# Step 3: verify SpendGuard ledger has reserve + commit_estimated rows
# ---------------------------------------------------------------------
log "step 3: verify reserve+commit in ledger..."
psql "${POSTGRES_URL}" -v ON_ERROR_STOP=1 -f /smoke/verify.sql \
    || fail "verify SQL failed"

log "OK: reserve+commit verified"
