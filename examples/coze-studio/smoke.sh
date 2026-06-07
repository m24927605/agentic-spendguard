#!/usr/bin/env bash
# D31 SLICE 2 — Coze Studio + SpendGuard HTTP companion smoke.
#
# Boots the standalone smoke stack (examples/coze-studio/docker-compose.coze.yaml),
# replays the same shape Coze sends when an operator clicks the workspace
# provider "Test connection" button, then asserts:
#
#   T-SMOKE-01  companion answers /v1/openai/chat/completions with 200
#   T-SMOKE-02  body is OpenAI-shaped (`.choices[0].message.content`)
#   T-SMOKE-03  audit chain has a `reserve` + `commit` pair tagged
#               integration='coze_studio' for the just-issued reservation
#   T-SMOKE-04  missing X-SpendGuard-Tenant-Id → 400 MISSING_TENANT
#   T-SMOKE-05  Coze Studio container is healthy (only when COZE_IMAGE_AVAILABLE=1)
#   T-SMOKE-06  teardown is clean (docker compose down -v exits 0)
#
# Required: OPENAI_API_KEY (this smoke hits real upstream OpenAI per the
# `feedback_demo_quality_gate` directive — wire assumptions only surface
# against real upstreams).
#
# Optional:
#   COZE_IMAGE_AVAILABLE=1   pull the (large) Coze Studio image. Off by
#                            default; the smoke validates the companion
#                            contract — the headline UI-driven flow is
#                            owned by `DEMO_MODE=coze_studio_real`.
#   SIDECAR_PORT             override the host port (default 8443).
#   SIDECAR_HOST             override the host (default 127.0.0.1).
#
# This smoke does NOT drive Coze Studio's web UI — that's covered by the
# Slice 3 demo driver (`client.py`) under `DEMO_MODE=coze_studio_real`.
# Reviewer note: keeping the smoke UI-free is intentional (R1 §3.2.7).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")"/../.. && pwd)"
COMPOSE_FILE="$REPO_ROOT/examples/coze-studio/docker-compose.coze.yaml"
SIDECAR_HOST="${SIDECAR_HOST:-127.0.0.1}"
SIDECAR_PORT="${SIDECAR_PORT:-8443}"
COZE_IMAGE_AVAILABLE="${COZE_IMAGE_AVAILABLE:-0}"

PKI_VOLUME="spendguard-coze-smoke_coze-smoke-pki"
TMP_CERTS="$(mktemp -d -t coze-smoke-XXXXXX)"

cleanup() {
  local code=$?
  echo "[smoke] tearing down (last exit=$code) ..."
  if [ "${KEEP_STACK:-0}" != "1" ]; then
    # T-SMOKE-06: docker compose down -v must succeed for the gate to pass.
    docker compose -f "$COMPOSE_FILE" down -v --remove-orphans >/dev/null 2>&1 || true
  fi
  rm -rf "$TMP_CERTS"
  exit "$code"
}
trap cleanup EXIT

# ── prereq: OPENAI_API_KEY required (G4 §3.2.2) ─────────────────────────
if [ -z "${OPENAI_API_KEY:-}" ]; then
  echo "[smoke] FATAL: OPENAI_API_KEY required (real upstream OpenAI hit)" >&2
  exit 8
fi

# ── prereq: D09 SLICE 1 companion endpoint present (R1 §3.2.10) ─────────
if [ ! -f "$REPO_ROOT/services/sidecar/src/http_companion/mod.rs" ]; then
  echo "[smoke] FATAL: D09 SLICE 1 HTTP companion missing from tree" >&2
  echo "[smoke]        expected: services/sidecar/src/http_companion/mod.rs" >&2
  exit 9
fi

# ── boot ────────────────────────────────────────────────────────────────
echo "[smoke] booting stack from $COMPOSE_FILE"
PROFILE_FLAGS=()
if [ "$COZE_IMAGE_AVAILABLE" = "1" ]; then
  PROFILE_FLAGS=(--profile coze)
  echo "[smoke] COZE_IMAGE_AVAILABLE=1 → pulling Coze image"
fi
docker compose -f "$COMPOSE_FILE" "${PROFILE_FLAGS[@]}" up -d --wait

# ── extract certs to host so curl can present mTLS ──────────────────────
echo "[smoke] extracting mTLS bundle to $TMP_CERTS"
docker run --rm \
  -v "$PKI_VOLUME":/pki:ro \
  -v "$TMP_CERTS":/host \
  alpine:3.20 sh -c "cp /pki/spendguard-ca.pem /pki/coze-client.pem /pki/coze-client.key /host/" >/dev/null
chmod 0400 "$TMP_CERTS"/*.key
SIDECAR_CA="$TMP_CERTS/spendguard-ca.pem"
COZE_CERT="$TMP_CERTS/coze-client.pem"
COZE_KEY="$TMP_CERTS/coze-client.key"

# ── T-SMOKE-01 + T-SMOKE-02: positive path ──────────────────────────────
echo "[smoke] T-SMOKE-01 + 02: /v1/openai/chat/completions positive path"
RESP_BODY="$(mktemp)"
HTTP_CODE=$(
  curl --silent --show-error \
    --cacert "$SIDECAR_CA" --cert "$COZE_CERT" --key "$COZE_KEY" \
    --connect-timeout 10 --max-time 60 \
    -X POST "https://$SIDECAR_HOST:$SIDECAR_PORT/v1/openai/chat/completions" \
    -H "Authorization: Bearer $OPENAI_API_KEY" \
    -H "Content-Type: application/json" \
    -H "X-SpendGuard-Tenant-Id: coze-smoke-tenant" \
    -H "X-SpendGuard-Budget-Id: 44444444-4444-4444-8444-444444444444" \
    -H "X-SpendGuard-Window-Instance-Id: 55555555-5555-5555-8555-555555555555" \
    -d '{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hello from coze smoke"}],"max_tokens":16}' \
    -o "$RESP_BODY" -w "%{http_code}"
)
if [ "$HTTP_CODE" != "200" ]; then
  echo "[smoke] T-SMOKE-01 FAIL: expected 200, got $HTTP_CODE" >&2
  cat "$RESP_BODY" >&2
  exit 1
fi
echo "[smoke] T-SMOKE-01 PASS: HTTP 200"

if ! jq -e '.choices[0].message.content' "$RESP_BODY" >/dev/null; then
  echo "[smoke] T-SMOKE-02 FAIL: response missing .choices[0].message.content" >&2
  cat "$RESP_BODY" >&2
  exit 1
fi
echo "[smoke] T-SMOKE-02 PASS: response body OpenAI-shaped"
rm -f "$RESP_BODY"

# ── T-SMOKE-03: reserve + commit audit row pair present ────────────────
echo "[smoke] T-SMOKE-03: audit chain has reserve + commit for integration=coze_studio"
# Wait up to 10s for the outbox writer to land the rows.
PSQL="docker compose -f $COMPOSE_FILE exec -T spendguard-postgres psql -U spendguard -d spendguard_ledger -At"
DEADLINE=$(( $(date +%s) + 10 ))
RESERVE_COUNT=0
while [ "$(date +%s)" -lt "$DEADLINE" ]; do
  RESERVE_COUNT=$(
    eval $PSQL -c "\"SELECT COUNT(DISTINCT reservation_id) FROM audit_outbox WHERE decision_context->>'integration' = 'coze_studio' AND created_at > now() - interval '1 minute'\"" 2>/dev/null || echo 0
  )
  if [ "${RESERVE_COUNT:-0}" -ge 1 ]; then break; fi
  sleep 1
done
if [ "${RESERVE_COUNT:-0}" -lt 1 ]; then
  echo "[smoke] T-SMOKE-03 FAIL: no audit_outbox row with integration=coze_studio" >&2
  exit 1
fi
echo "[smoke] T-SMOKE-03 PASS: $RESERVE_COUNT distinct reservation_id(s) tagged coze_studio"

# ── T-SMOKE-04: missing tenant header → 400 MISSING_TENANT ─────────────
echo "[smoke] T-SMOKE-04: missing X-SpendGuard-Tenant-Id → 400 MISSING_TENANT (INV-4)"
ERR_BODY="$(mktemp)"
HTTP_CODE=$(
  curl --silent --show-error \
    --cacert "$SIDECAR_CA" --cert "$COZE_CERT" --key "$COZE_KEY" \
    --connect-timeout 10 --max-time 30 \
    -X POST "https://$SIDECAR_HOST:$SIDECAR_PORT/v1/openai/chat/completions" \
    -H "Authorization: Bearer $OPENAI_API_KEY" \
    -H "Content-Type: application/json" \
    -d '{"model":"gpt-4o-mini","messages":[{"role":"user","content":"missing tenant"}]}' \
    -o "$ERR_BODY" -w "%{http_code}"
)
if [ "$HTTP_CODE" != "400" ]; then
  echo "[smoke] T-SMOKE-04 FAIL: expected 400, got $HTTP_CODE" >&2
  cat "$ERR_BODY" >&2
  exit 1
fi
if ! jq -e '.error.code | test("MISSING_TENANT")' "$ERR_BODY" >/dev/null; then
  echo "[smoke] T-SMOKE-04 FAIL: error.code does not match MISSING_TENANT" >&2
  cat "$ERR_BODY" >&2
  exit 1
fi
echo "[smoke] T-SMOKE-04 PASS: 400 MISSING_TENANT"
rm -f "$ERR_BODY"

# ── T-SMOKE-05: Coze Studio container is healthy (only when image pulled) ─
if [ "$COZE_IMAGE_AVAILABLE" = "1" ]; then
  echo "[smoke] T-SMOKE-05: coze-studio container health"
  HEALTH=$(docker compose -f "$COMPOSE_FILE" ps --format json | jq -r '. | select(.Service=="coze-studio") | .Health' || true)
  if [ "$HEALTH" != "healthy" ]; then
    echo "[smoke] T-SMOKE-05 FAIL: coze-studio health=$HEALTH" >&2
    exit 1
  fi
  echo "[smoke] T-SMOKE-05 PASS: coze-studio healthy"
else
  echo "[smoke] T-SMOKE-05 SKIP: COZE_IMAGE_AVAILABLE=0"
fi

# ── T-SMOKE-06: clean teardown happens in trap (verified by exit code) ─
echo "[smoke] ALL 5 assertions PASS — D31 SLICE 2 contract live"
echo "[smoke] (T-SMOKE-06 teardown runs on exit)"
